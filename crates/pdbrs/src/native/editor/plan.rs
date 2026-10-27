//! The write plan of a [`PdbEditor`]: the streams, the report and the layout.

use alloc::vec::Vec;

use super::report::{PlanReport, RemapReport, StreamChange, StreamReport};
use super::{EditOptions, PdbEditor};
use crate::io::{ReadAt, Write};
use crate::native::container::Stream;
use crate::native::msf::{MsfBuilder, MsfWriteOptions, MsfWritePlan, StreamId};
use crate::native::output::no_memory;
use crate::native::{Identity, Pdb};
use crate::{Error, ErrorKind, Result};

mod checks;
mod cow;
mod layout;
mod modules;
mod opaque;
mod patch;
mod paths;
mod shared;
mod strip;
mod types;

/// The bytes of one planned stream.
pub(super) enum Planned<'e, S> {
    /// An unchanged stream of the source.
    Source(Stream<'e, S>),
    /// Bytes that the plan encoded.
    Owned(Vec<u8>),
    /// Bytes that the editor model holds.
    Borrowed(&'e [u8]),
    /// Base bytes with new fixed-width values (#48).
    Patched(patch::Patched<'e, S>),
}

impl<S: ReadAt> ReadAt for Planned<'_, S> {
    fn len(&self) -> Result<u64> {
        match self {
            Self::Source(stream) => stream.len(),
            Self::Owned(bytes) => ReadAt::len(bytes.as_slice()),
            Self::Borrowed(bytes) => ReadAt::len(*bytes),
            Self::Patched(patched) => patched.len(),
        }
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        match self {
            Self::Source(stream) => stream.read_exact_at(offset, buffer),
            Self::Owned(bytes) => bytes.as_slice().read_exact_at(offset, buffer),
            Self::Borrowed(bytes) => bytes.read_exact_at(offset, buffer),
            Self::Patched(patched) => patched.read_exact_at(offset, buffer),
        }
    }
}

/// The identity of a plan.
enum PlanIdentity<'e, S> {
    /// The plan knows the identity.
    Known(Identity),
    /// The identity has the source DBI age, which the plan did not read.
    SourceAge {
        /// The GUID.
        guid: [u8; 16],
        /// The Info age.
        info_age: u32,
        /// The source PDB.
        pdb: &'e Pdb<S>,
    },
}

/// The streams of a plan in stream ID order, with their report entries.
pub(super) struct Assembly<'e, S> {
    /// The planned streams. `None` is a nil stream.
    streams: Vec<Option<Planned<'e, S>>>,
    /// The report entries of the output streams.
    reports: Vec<StreamReport>,
    /// The report entries of the source streams that the output does not keep.
    dropped: Vec<StreamReport>,
}

impl<'e, S> Assembly<'e, S> {
    /// Create an empty assembly.
    pub(super) const fn new() -> Self {
        Self {
            streams: Vec::new(),
            reports: Vec::new(),
            dropped: Vec::new(),
        }
    }

    /// Return the stream ID of the next stream.
    pub(super) fn next_id(&self) -> Result<StreamId> {
        u32::try_from(self.streams.len())
            .map(StreamId)
            .map_err(|_| {
                Error::new(ErrorKind::LimitExceeded, None).with_context("PDB has too many streams")
            })
    }

    /// Add a stream with its source stream and its change.
    pub(super) fn push(
        &mut self,
        stream: Option<Planned<'e, S>>,
        source: Option<StreamId>,
        change: StreamChange,
    ) -> Result<StreamId> {
        let id = self.next_id()?;
        self.streams.try_reserve(1).map_err(|_| no_memory())?;
        self.reports.try_reserve(1).map_err(|_| no_memory())?;
        self.streams.push(stream);
        self.reports.push(StreamReport {
            source,
            output: Some(id),
            change,
        });
        Ok(id)
    }

    /// Report a source stream that the output does not keep.
    pub(super) fn drop_source(&mut self, source: StreamId) -> Result<()> {
        self.dropped.try_reserve(1).map_err(|_| no_memory())?;
        self.dropped.push(StreamReport {
            source: Some(source),
            output: None,
            change: StreamChange::Removed,
        });
        Ok(())
    }
}

/// Return `true` if the source stream `reader` has the bytes `bytes`.
pub(super) fn same_bytes<S: ReadAt>(reader: &Stream<'_, S>, bytes: &[u8]) -> Result<bool> {
    if ReadAt::len(reader)? != u64::try_from(bytes.len()).unwrap_or(u64::MAX) {
        return Ok(false);
    }
    let mut buffer = [0_u8; 4096];
    let mut offset = 0_u64;
    for chunk in bytes.chunks(buffer.len()) {
        let part = &mut buffer[..chunk.len()];
        reader.read_exact_at(offset, part)?;
        if part != chunk {
            return Ok(false);
        }
        offset += chunk.len() as u64;
    }
    Ok(true)
}

/// Return the error for a stream that the plan needs.
pub(super) fn missing(context: &'static str) -> Error {
    Error::new(ErrorKind::MissingStream, None).with_context(context)
}

/// Return an [`ErrorKind::UnsupportedEdit`] error.
pub(super) fn unsupported(context: &'static str) -> Error {
    Error::new(ErrorKind::UnsupportedEdit, None).with_context(context)
}

/// A frozen native PDB: the streams, the report and the checked container
/// layout.
///
/// [`PdbEditor::plan`] makes the plan. The plan encoded all changed streams
/// and checked the MSF layout, so the write functions do only output work.
/// They read each copied stream from the source. A stream that gets only
/// new fixed-width values (for example the global references of a module
/// after a move of symbol records) is a patched stream: the plan keeps the
/// new values only. The write functions read the stream from the source (or
/// from the bytes that the source session holds) and write the new values
/// over it.
///
/// # Errors and partial output
///
/// [`PdbWritePlan::write_to`] writes in file order. A read error or an
/// output error can leave a partial output. Under `std`,
/// `PdbWritePlan::write_file` writes a temporary file and renames it. On
/// error, the destination does not change. Writing over the source file is
/// not supported.
pub struct PdbWritePlan<'e, S = &'static [u8]> {
    /// The streams in stream ID order.
    streams: MsfBuilder<Planned<'e, S>>,
    /// The container options.
    options: MsfWriteOptions,
    /// The identity for the `RSDS` debug entry of the image.
    identity: PlanIdentity<'e, S>,
    /// The number of streams.
    stream_count: u32,
    /// The size of the output in bytes.
    file_size: u64,
    /// The report.
    report: PlanReport,
}

impl<S> core::fmt::Debug for PdbWritePlan<'_, S> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PdbWritePlan")
            .field("streams", &self.stream_count)
            .field("options", &self.options)
            .field("file_size", &self.file_size)
            .field("report", &self.report)
            .finish_non_exhaustive()
    }
}

impl<S: ReadAt> PdbWritePlan<'_, S> {
    /// Return the identity for the `RSDS` debug entry of the matching image:
    /// the GUID and the DBI age.
    ///
    /// If no change needs the DBI stream, the plan does not read it. This
    /// function then reads the age of the source DBI stream.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the source has no DBI stream.
    /// - [`ErrorKind::InvalidFormat`] if the source DBI age is larger than
    ///   the Info age.
    /// - The errors of the stream reader.
    pub fn identity(&self) -> Result<Identity> {
        match &self.identity {
            PlanIdentity::Known(identity) => Ok(*identity),
            PlanIdentity::SourceAge {
                guid,
                info_age,
                pdb,
            } => {
                let age = source_dbi_age(pdb)?;
                if age > *info_age {
                    return Err(Error::new(ErrorKind::InvalidFormat, None)
                        .with_context("DBI age is larger than the PDB Info age"));
                }
                Ok(Identity::Modern { guid: *guid, age })
            }
        }
    }

    /// Return the report: how the plan writes each stream.
    #[must_use]
    pub const fn report(&self) -> &PlanReport {
        &self.report
    }

    /// Return the number of streams, including nil streams.
    #[must_use]
    pub const fn stream_count(&self) -> u32 {
        self.stream_count
    }

    /// Return the size of the output in bytes.
    #[must_use]
    pub const fn file_size(&self) -> u64 {
        self.file_size
    }

    /// Return the MSF plan of the streams.
    fn msf(&self) -> Result<MsfWritePlan<'_>> {
        self.streams.plan(self.options)
    }

    /// Write the PDB to `output` in page order.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`MsfWritePlan::write_to`].
    pub fn write_to<W: Write + ?Sized>(&self, output: &mut W) -> Result<()> {
        self.msf()?.write_to(output)
    }

    /// Write the PDB to a new vector.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`MsfWritePlan::to_vec`].
    pub fn to_vec(&self) -> Result<Vec<u8>> {
        self.msf()?.to_vec()
    }

    /// Write the PDB to the file `path` through a temporary sibling file.
    /// Requires the `std` feature.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`MsfWritePlan::write_file`].
    #[cfg(feature = "std")]
    pub fn write_file(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        self.msf()?.write_file(path)
    }
}

/// The offset of the age in the DBI header.
const DBI_AGE_OFFSET: u64 = 8;

/// Read the age of the source DBI stream.
fn source_dbi_age<S: ReadAt>(pdb: &Pdb<S>) -> Result<u32> {
    let stream = pdb
        .container()
        .stream(StreamId(3))?
        .ok_or_else(|| missing("PDB has no DBI stream"))?;
    stream.read_u32_at(DBI_AGE_OFFSET)
}

/// Freeze an assembly into a plan.
fn freeze<'e, S: ReadAt>(
    assembly: Assembly<'e, S>,
    options: MsfWriteOptions,
    identity: PlanIdentity<'e, S>,
    remap: Option<RemapReport>,
) -> Result<PdbWritePlan<'e, S>> {
    let Assembly {
        streams: list,
        mut reports,
        dropped,
    } = assembly;
    let mut streams = MsfBuilder::new();
    for stream in list {
        match stream {
            Some(stream) => streams.push(stream)?,
            None => streams.push_nil()?,
        };
    }
    reports
        .try_reserve(dropped.len())
        .map_err(|_| no_memory())?;
    reports.extend(dropped);
    let msf = streams.plan(options)?;
    let (stream_count, file_size) = (msf.stream_count(), msf.file_size());
    Ok(PdbWritePlan {
        streams,
        options,
        identity,
        stream_count,
        file_size,
        report: PlanReport::new(reports, remap),
    })
}

impl<S: ReadAt> PdbEditor<'_, S> {
    /// Check the model, encode the streams and plan the container.
    ///
    /// See [`PdbEditor`] for the layouts, the derived streams and the
    /// rejected edits.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedEdit`] for an edit that the plan does not
    ///   support, for example an edit that can make an opaque reference
    ///   invalid, a forward type reference or a strip together with other
    ///   edits.
    /// - [`ErrorKind::InvalidReference`] for a reference that does not
    ///   resolve, for example a reference record whose target is removed.
    /// - [`ErrorKind::InvalidFormat`] if a record or a stream breaks a rule
    ///   of its encoder.
    /// - [`ErrorKind::LimitExceeded`] if a value does not fit its field, a
    ///   stream is too large or an allocation fails.
    /// - [`ErrorKind::UnsupportedVersion`] for a source page size that the
    ///   writer does not support.
    /// - [`ErrorKind::MissingStream`] if the source has no stream that a
    ///   change needs.
    /// - The errors of the stream readers and of [`MsfBuilder::plan`].
    pub fn plan(&self, options: EditOptions) -> Result<PdbWritePlan<'_, S>> {
        if self.paths_remapped {
            paths::check_model(self)?;
        }
        self.prepare_remap()?;
        let remap = self.remapped().map(|remapped| remapped.report.clone());
        if self.is_new_layout() {
            if options.strip_private() {
                return Err(unsupported(
                    "a strip needs a source and cannot follow decode_all",
                ));
            }
            let (assembly, msf, identity) = layout::plan(self, options)?;
            return freeze(assembly, msf, PlanIdentity::Known(identity), remap);
        }
        let (assembly, msf, identity) = cow::plan(self, options)?;
        let identity = match identity {
            cow::Identity::Known(identity) => PlanIdentity::Known(identity),
            cow::Identity::SourceAge { guid, info_age } => PlanIdentity::SourceAge {
                guid,
                info_age,
                pdb: self.cow_source()?,
            },
        };
        freeze(assembly, msf, identity, remap)
    }
}
