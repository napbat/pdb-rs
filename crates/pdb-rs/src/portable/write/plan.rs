//! The write plan of a Portable PDB: the checked layout and the encoded bytes.

use alloc::vec::Vec;

use super::identity::{self, ChecksumAlgorithm, CodeViewFields, PdbChecksum};
use super::parts::PdbParts;
use super::report::{PlanReport, RebuildReason, STREAMS, StreamChange};
use super::{check, layout};
use crate::io::{ReadAt, Write};
use crate::portable::embedded::encode_embedded;
use crate::portable::{Identity, IdentityPolicy, Pdb};
use crate::{Error, ErrorKind, Result};

/// Zero bytes for the PDB ID and the stream padding.
const ZEROS: [u8; 20] = [0; 20];

/// The size of the PDB ID at the start of the `#Pdb` stream.
const ID_LEN: usize = 20;

/// The frozen layout, the encoded bytes and the report of a Portable PDB.
///
/// [`PdbParts::plan`] and [`PdbEditor::plan`](crate::portable::PdbEditor::plan) make the plan.
/// See [`crate::portable::write`] for the layout, the checks and the error contract.
pub struct PdbWritePlan {
    root: Vec<u8>,
    pdb: Vec<u8>,
    tables: Vec<u8>,
    /// `#Strings`, `#US`, `#GUID` and `#Blob` without padding.
    heaps: [Vec<u8>; 4],
    identity: Identity,
    report: PlanReport,
}

impl PdbParts {
    /// Check the parts, compute the layout and set the PDB ID.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] for a heap or row reference that does not identify
    ///   data, a nil document name, an entry point that is not an external `MethodDef` row,
    ///   or import scope parents that form a cycle.
    /// - [`ErrorKind::InvalidFormat`] for rows in the wrong order, a wrong
    ///   `MethodDebugInformation` row count, an external row count of a table that is not a
    ///   type-system table, or an invalid version string.
    /// - [`ErrorKind::LimitExceeded`] if a row count does not fit in a metadata token, the
    ///   metadata is larger than 4 GiB, or an allocation fails.
    ///
    /// The errors have a portable location.
    pub fn plan(self, id: IdentityPolicy) -> Result<PdbWritePlan> {
        let counts = check::counts(&self)?;
        check::rows(&self, &counts)?;
        let widths = layout::Widths::new(&self, &counts);
        let tables = layout::tables_stream(&self.tables, &widths)?;
        let pdb = layout::pdb_stream(self.entry_point, &self.external);
        let heaps = [
            self.strings.into_bytes(),
            self.user_strings.into_bytes(),
            self.guids.into_bytes(),
            self.blobs.into_bytes(),
        ];
        let sizes = [
            pdb.len(),
            tables.len(),
            heaps[0].len(),
            heaps[1].len(),
            heaps[2].len(),
            heaps[3].len(),
        ];
        let root = layout::root(&self.version, sizes)?;
        let mut plan = PdbWritePlan {
            root,
            pdb,
            tables,
            heaps,
            identity: Identity::from_bytes(ZEROS),
            report: PlanReport::new([StreamChange::Added; 6]),
        };
        plan.identity = match id {
            IdentityPolicy::Explicit(identity) => identity,
            IdentityPolicy::ContentDerived => {
                identity::content_id(&identity::sha256(plan.pieces(true)))
            }
        };
        plan.pdb[..ID_LEN].copy_from_slice(&plan.identity.to_bytes());
        Ok(plan)
    }
}

impl PdbWritePlan {
    /// Return the bytes of a stream of the output, without padding.
    fn stream(&self, index: usize) -> &[u8] {
        match index {
            0 => &self.pdb,
            1 => &self.tables,
            _ => &self.heaps[index - 2],
        }
    }

    /// Compare each output stream with the stream of `source` and set the report.
    ///
    /// # Errors
    ///
    /// The errors of [`Pdb::stream_bytes`].
    pub(crate) fn compare<S: ReadAt>(&mut self, source: &Pdb<S>) -> Result<()> {
        let mut changes = [StreamChange::Added; 6];
        for (index, (change, name)) in changes.iter_mut().zip(STREAMS).enumerate() {
            let Some(before) = source.stream_bytes(name)? else {
                continue;
            };
            let after = self.stream(index);
            // A source stream can end with up to 3 zero bytes of padding.
            let same = |before: &[u8], after: &[u8]| {
                before.starts_with(after)
                    && before.len() - after.len() < 4
                    && before[after.len()..].iter().all(|&byte| byte == 0)
            };
            *change = if same(&before, after) {
                StreamChange::Copied
            } else if index == 0
                && before.len() >= ID_LEN
                && after.len() >= ID_LEN
                && same(&before[ID_LEN..], &after[ID_LEN..])
            {
                StreamChange::Rebuilt(RebuildReason::Identity)
            } else {
                StreamChange::Edited
            };
        }
        self.report = PlanReport::new(changes);
        Ok(())
    }

    /// Return the change of each stream, compared with the source of an editor.
    #[must_use]
    pub const fn report(&self) -> &PlanReport {
        &self.report
    }

    /// Return the output in pieces, in file order. If `zero_id` is `true`, the PDB ID is zero.
    fn pieces(&self, zero_id: bool) -> [&[u8]; 12] {
        let pad = |bytes: &[u8]| &ZEROS[..layout::padding(bytes.len())];
        let [strings, user_strings, guids, blobs] = &self.heaps;
        let (id, pdb) = self.pdb.split_at(ID_LEN);
        [
            &self.root,
            if zero_id { &ZEROS } else { id },
            pdb,
            &self.tables,
            strings,
            pad(strings),
            user_strings,
            pad(user_strings),
            guids,
            pad(guids),
            blobs,
            pad(blobs),
        ]
    }

    /// Return the PDB ID of the output.
    #[must_use]
    pub const fn identity(&self) -> Identity {
        self.identity
    }

    /// Return the fields of the portable `CodeView` debug directory entry of the assembly.
    #[must_use]
    pub fn codeview(&self) -> CodeViewFields {
        CodeViewFields::from(self.identity)
    }

    /// Return the PDB checksum of the output: a digest of all bytes with the PDB ID set to
    /// zero.
    #[must_use]
    pub fn checksum(&self, algorithm: ChecksumAlgorithm) -> PdbChecksum {
        identity::checksum(algorithm, self.pieces(true))
    }

    /// Return the size of the output in bytes.
    #[must_use]
    pub fn file_size(&self) -> u64 {
        self.pieces(false)
            .iter()
            .map(|piece| u64::try_from(<[u8]>::len(piece)).unwrap_or(u64::MAX))
            .sum()
    }

    /// Write the Portable PDB to `output` in file order.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Io`] if `output` fails. The error offset is the file offset of the
    /// failed piece. After an error, `output` can hold a part of the Portable PDB.
    pub fn write_to<W: Write + ?Sized>(&self, output: &mut W) -> Result<()> {
        let mut offset = 0_u64;
        for piece in self.pieces(false) {
            output
                .write_all(piece)
                .map_err(|error| Error::from_io(error, Some(offset)))?;
            offset += u64::try_from(<[u8]>::len(piece)).unwrap_or(u64::MAX);
        }
        Ok(())
    }

    /// Write the Portable PDB to a new vector.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`] if the allocation fails. The function checks the
    ///   allocation before it writes.
    /// - The errors of [`PdbWritePlan::write_to`].
    pub fn to_vec(&self) -> Result<Vec<u8>> {
        let too_large = || {
            Error::new(ErrorKind::LimitExceeded, None).with_context("output does not fit in memory")
        };
        let size = usize::try_from(self.file_size()).map_err(|_| too_large())?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(size).map_err(|_| too_large())?;
        self.write_to(&mut bytes)?;
        Ok(bytes)
    }

    /// Write the Portable PDB to the file `path`. Requires the `std` feature.
    ///
    /// The function writes a temporary file in the same directory and renames it to `path`.
    /// The rename replaces an existing file.
    ///
    /// Caution: Do not give the path of the source file. This operation is not supported.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Io`] if the temporary file does not open, a write fails, or the rename
    ///   fails.
    /// - The errors of [`PdbWritePlan::write_to`].
    ///
    /// After an error, the function deletes the temporary file. The file at `path` does not
    /// change.
    #[cfg(feature = "std")]
    pub fn write_file(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        crate::io::write_new_file(path.as_ref(), |output| self.write_to(output))
    }

    /// Encode the Portable PDB as an `MPDB` payload for an embedded Portable PDB debug
    /// directory entry (type 17). See [`encode_embedded`].
    ///
    /// # Errors
    ///
    /// The errors of [`PdbWritePlan::to_vec`] and [`encode_embedded`].
    pub fn to_embedded(&self) -> Result<Vec<u8>> {
        encode_embedded(&self.to_vec()?)
    }
}

impl core::fmt::Debug for PdbWritePlan {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PdbWritePlan")
            .field("identity", &self.identity)
            .field("file_size", &self.file_size())
            .finish_non_exhaustive()
    }
}
