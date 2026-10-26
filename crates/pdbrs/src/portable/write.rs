//! Portable PDB writing: exact copies and new layouts from raw parts.
//!
//! # Write operations
//!
//! | Operation | Guarantee | Not guaranteed |
//! | --- | --- | --- |
//! | [`Pdb::copy_exact`] | Every source byte, including padding and unreferenced heap data | Any validation of the source beyond the open checks |
//! | [`PdbParts::plan`] | A Portable PDB with the given rows and heaps, in a checked and deterministic layout | Byte-identical output for input from other producers |
//!
//! [`Pdb::to_parts`] reads all rows and heap entries of a Portable PDB into new heaps. The
//! row IDs and the row order do not change. A plan of these parts writes the same records.
//!
//! # Layout
//!
//! [`PdbParts::plan`] checks all parts and computes the full layout before it writes a byte.
//! It uses the layout of `System.Reflection.Metadata` (`PortablePdbBuilder`):
//!
//! 1. The metadata root (version 1.1) has six stream headers in this order: `#Pdb`, `#~`,
//!    `#Strings`, `#US`, `#GUID` and `#Blob`. Each stream starts at a 4-byte boundary.
//!    Zero bytes pad each stream to a multiple of 4.
//! 2. `#Pdb` has the PDB ID, the entry point and the external tables with a row count other
//!    than 0.
//! 3. `#~` has version 2.0 and the reserved byte 1. `Valid` has the debug tables with rows.
//!    `Sorted` has `LocalScope`, `StateMachineMethod` and `CustomDebugInformation` if they
//!    have rows. One zero byte follows the rows.
//! 4. A heap index has 4 bytes if the heap has more than `0xFFFF` bytes without padding. A
//!    table index has 4 bytes if the table has `0x10000` rows or more. The row count of a
//!    type-system table is its external row count. A `HasCustomDebugInformation` coded index
//!    has 4 bytes if one of its tables has `0x800` rows or more.
//!
//! The same parts and the same [`IdPolicy`] thus give the same bytes.
//!
//! # Checks
//!
//! The plan checks the metadata version string, the external row counts, the entry point and
//! each row:
//!
//! - Each heap reference: string offsets inside `#Strings`, blob offsets at the start of a
//!   `#Blob` entry, GUID indexes inside `#GUID`.
//! - Each row reference, list start and coded parent. Nil is valid only where the
//!   specification allows it. A document name is not nil.
//! - `MethodDebugInformation` has no rows or one row for each external `MethodDef` row.
//! - The order of `LocalScope` (method, start offset, then length with the longest first),
//!   non-decreasing variable and constant lists, `StateMachineMethod` (unique `MoveNext`
//!   methods in ascending order, unique kickoff methods) and `CustomDebugInformation`
//!   (coded parent in ascending order).
//! - Import scope parents do not form a cycle.
//!
//! The plan does not decode blobs.
//!
//! # Identity
//!
//! The plan sets the PDB ID from its [`IdPolicy`]. [`WritePlan::identity`] and
//! [`WritePlan::codeview`] return the values for the debug directory of the assembly.
//! [`WritePlan::checksum`] returns the PDB checksum. Updating the assembly is the task of the
//! caller.
//!
//! # Errors and partial output
//!
//! A planning error writes nothing. [`WritePlan::write_to`] writes in file order. An output
//! error can leave a partial output in the writer. Under `std`, `WritePlan::write_file` and
//! `Pdb::copy_exact_file` write a temporary file and rename it. On error, they delete the
//! temporary file and do not change the destination. Writing over the source file is not
//! supported.

use alloc::vec::Vec;

use super::embedded::encode_embedded;
use super::{Identity, Pdb};
use crate::io::{ReadAt, Write, copy_all};
use crate::{Error, ErrorKind, Result};

mod check;
#[cfg(feature = "std")]
mod file;
mod heaps;
mod identity;
mod layout;
mod parts;
mod rebuild;

pub use heaps::{BlobHeap, GuidHeap, StringHeap, StringHeapBuilder, UserStringHeap};
pub use identity::{ChecksumAlgorithm, CodeViewFields, IdPolicy, PdbChecksum};
pub use parts::{DebugTables, ExternalRowCounts, PDB_VERSION, PdbParts};

/// Zero bytes for the PDB ID and the stream padding.
const ZEROS: [u8; 20] = [0; 20];

/// The size of the PDB ID at the start of the `#Pdb` stream.
const ID_LEN: usize = 20;

/// The frozen layout and the encoded bytes of a Portable PDB.
///
/// See the [module documentation](self) for the layout, the checks and the error contract.
pub struct WritePlan {
    root: Vec<u8>,
    pdb: Vec<u8>,
    tables: Vec<u8>,
    /// `#Strings`, `#US`, `#GUID` and `#Blob` without padding.
    heaps: [Vec<u8>; 4],
    identity: Identity,
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
    pub fn plan(self, id: IdPolicy) -> Result<WritePlan> {
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
        let mut plan = WritePlan {
            root,
            pdb,
            tables,
            heaps,
            identity: Identity::from_bytes(ZEROS),
        };
        plan.identity = match id {
            IdPolicy::Explicit(identity) => identity,
            IdPolicy::ContentDerived => identity::content_id(&identity::sha256(plan.pieces(true))),
        };
        plan.pdb[..ID_LEN].copy_from_slice(&plan.identity.to_bytes());
        Ok(plan)
    }
}

impl WritePlan {
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
    /// - The errors of [`WritePlan::write_to`].
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

    /// Encode the Portable PDB as an `MPDB` payload for an embedded Portable PDB debug
    /// directory entry (type 17). See [`encode_embedded`].
    ///
    /// # Errors
    ///
    /// The errors of [`WritePlan::to_vec`] and [`encode_embedded`].
    pub fn to_embedded(&self) -> Result<Vec<u8>> {
        encode_embedded(&self.to_vec()?)
    }
}

impl core::fmt::Debug for WritePlan {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("WritePlan")
            .field("identity", &self.identity)
            .field("file_size", &self.file_size())
            .finish_non_exhaustive()
    }
}

impl<S: ReadAt> Pdb<S> {
    /// Copy all bytes of the source to `output` without a change. Return the number of bytes.
    ///
    /// The copy uses one buffer of at most 64 KiB. It does not check more than the open
    /// checks of the Portable PDB.
    ///
    /// # Errors
    ///
    /// - An error from the source if a read fails.
    /// - [`ErrorKind::Io`] if `output` fails. The error offset is the file offset of the
    ///   buffer.
    /// - [`ErrorKind::LimitExceeded`] if the buffer allocation fails.
    ///
    /// After an error, `output` can hold a part of the source.
    pub fn copy_exact<W: Write + ?Sized>(&self, output: &mut W) -> Result<u64> {
        copy_all(self.source(), output)
    }
}
