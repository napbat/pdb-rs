//! The write plan of a [`PdbEditor`]: the stream changes and the layout.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::{PdbEditor, named, strip, symbols, types, unsupported};
use crate::io::{ReadAt, Write};
use crate::native::container::Stream;
use crate::native::dbi::{Dbi, DbiBuilder};
use crate::native::info::InfoBuilder;
use crate::native::msf::{MsfBuilder, MsfWriteOptions, MsfWritePlan, StreamId};
use crate::native::output::no_memory;
use crate::native::types::TypeStream;
use crate::native::{Identity, Producer};
use crate::{Error, ErrorKind, Result};

/// The options of the plan of a [`PdbEditor`].
///
/// [`Default`] keeps the page size of an MSF 7.00 source. Set a value with
/// its `with_*` function and read it with the function of the same name. A
/// later version can add options, for example the output container or an
/// output profile, without a change for callers.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct EditOptions {
    /// The container options, or `None` for the page size of the source.
    msf: Option<MsfWriteOptions>,
}

impl EditOptions {
    /// Return the container options, or `None` if the plan uses the page
    /// size of the source.
    ///
    /// With `None`, the plan uses the default [`MsfWriteOptions`] with the
    /// page size of an MSF 7.00 source. For an MSFZ source, it uses the
    /// default page size of [`MsfWriteOptions`].
    #[must_use]
    pub const fn msf(self) -> Option<MsfWriteOptions> {
        self.msf
    }

    /// Set the container options: the page size and the limits.
    #[must_use]
    pub const fn with_msf(mut self, msf: MsfWriteOptions) -> Self {
        self.msf = Some(msf);
        self
    }
}

/// The new contents of one source stream.
#[derive(Clone, Debug)]
enum Change {
    /// New bytes.
    Replace(Vec<u8>),
    /// A nil stream.
    Remove,
}

/// The stream changes of a plan, by stream ID.
#[derive(Clone, Debug)]
pub(super) struct Changes {
    /// The number of source streams.
    source_count: u32,
    /// The changes of source streams.
    source: BTreeMap<u32, Change>,
    /// The new streams after the source streams. `None` is a nil stream.
    added: Vec<Option<Vec<u8>>>,
}

impl Changes {
    /// Create an empty change list for a source with `source_count` streams.
    const fn new(source_count: u32) -> Self {
        Self {
            source_count,
            source: BTreeMap::new(),
            added: Vec::new(),
        }
    }

    /// Return the position of an added stream, or `None` for a source stream.
    fn added_position(&self, id: StreamId) -> Option<usize> {
        id.0.checked_sub(self.source_count)
            .and_then(|position| usize::try_from(position).ok())
    }

    /// Set the bytes of stream `id`.
    pub(super) fn replace(&mut self, id: StreamId, bytes: Vec<u8>) -> Result<()> {
        self.set(id, Some(bytes))
    }

    /// Make stream `id` a nil stream.
    pub(super) fn remove(&mut self, id: StreamId) -> Result<()> {
        self.set(id, None)
    }

    /// Set stream `id` to `bytes`, or to a nil stream for `None`.
    fn set(&mut self, id: StreamId, bytes: Option<Vec<u8>>) -> Result<()> {
        if let Some(position) = self.added_position(id) {
            let slot = self.added.get_mut(position).ok_or_else(|| {
                Error::new(ErrorKind::InvalidStreamIndex, None)
                    .with_context("edited stream is outside the stream directory")
            })?;
            *slot = bytes;
        } else {
            let change = bytes.map_or(Change::Remove, Change::Replace);
            self.source.insert(id.0, change);
        }
        Ok(())
    }

    /// Add a new stream with `bytes` and return its stream ID.
    pub(super) fn add(&mut self, bytes: Vec<u8>) -> Result<StreamId> {
        let id = u32::try_from(self.added.len())
            .ok()
            .and_then(|count| self.source_count.checked_add(count))
            .ok_or_else(|| {
                Error::new(ErrorKind::LimitExceeded, None).with_context("PDB has too many streams")
            })?;
        self.added.try_reserve(1).map_err(|_| no_memory())?;
        self.added.push(Some(bytes));
        Ok(StreamId(id))
    }
}

/// The bytes of one planned stream: a source stream or new bytes.
enum EditedStream<'p, S> {
    /// An unchanged stream of the source.
    Source(Stream<'p, S>),
    /// Bytes that the plan encoded.
    Owned(Vec<u8>),
}

impl<S: ReadAt> ReadAt for EditedStream<'_, S> {
    fn len(&self) -> Result<u64> {
        match self {
            Self::Source(stream) => stream.len(),
            Self::Owned(bytes) => ReadAt::len(bytes.as_slice()),
        }
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        match self {
            Self::Source(stream) => stream.read_exact_at(offset, buffer),
            Self::Owned(bytes) => bytes.as_slice().read_exact_at(offset, buffer),
        }
    }
}

/// A frozen edited PDB: the new streams, the source streams and the checked
/// container layout.
///
/// [`PdbEditor::plan`] makes the plan. The plan encoded all changed streams
/// and checked the MSF layout, so the write functions do only output work.
/// They read each unchanged stream from the source.
///
/// # Errors and partial output
///
/// [`PdbEditPlan::write_to`] writes in file order. A read error or an output
/// error can leave a partial output. Under `std`, `PdbEditPlan::write_file`
/// writes a temporary file and renames it. On error, the destination does
/// not change. Writing over the source file is not supported.
pub struct PdbEditPlan<'p, S> {
    /// The streams in stream ID order.
    streams: MsfBuilder<EditedStream<'p, S>>,
    /// The container options.
    options: MsfWriteOptions,
    /// The identity for the `RSDS` debug entry of the image.
    identity: Identity,
    /// The number of streams.
    stream_count: u32,
    /// The size of the output in bytes.
    file_size: u64,
    /// The streams after stream 0 that have new bytes, are removed or are new.
    changed: Vec<StreamId>,
}

impl<S> core::fmt::Debug for PdbEditPlan<'_, S> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PdbEditPlan")
            .field("streams", &self.stream_count)
            .field("options", &self.options)
            .field("identity", &self.identity)
            .field("file_size", &self.file_size)
            .field("changed", &self.changed)
            .finish()
    }
}

impl<S: ReadAt> PdbEditPlan<'_, S> {
    /// Return the identity for the `RSDS` debug entry of the matching image:
    /// the GUID and the DBI age.
    #[must_use]
    pub const fn identity(&self) -> Identity {
        self.identity
    }

    /// Return the streams after stream 0 that the plan changes: the streams
    /// with new bytes, the removed streams and the new streams, in stream ID
    /// order. All other streams have the source bytes.
    #[must_use]
    pub fn changed_streams(&self) -> &[StreamId] {
        &self.changed
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

/// The source streams that most edits read, and the producer conventions.
pub(super) struct Context<'p> {
    /// The TPI stream.
    pub(super) tpi: TypeStream,
    /// The DBI stream.
    pub(super) dbi: &'p Dbi,
    /// The conventions of the symbol indexes.
    pub(super) index_producer: Producer,
}

/// Return the error for a stream that an edit needs.
pub(super) fn missing(context: &'static str) -> Error {
    Error::new(ErrorKind::MissingStream, None).with_context(context)
}

/// Return `true` if the source stream `reader` has the bytes `bytes`.
///
/// The plan keeps the source stream for an encoded stream with the source
/// bytes, so that the plan does not report it as changed.
fn same_bytes<S: ReadAt>(reader: &Stream<'_, S>, bytes: &[u8]) -> Result<bool> {
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

impl<'p, S: ReadAt> PdbEditor<'p, S> {
    /// Check all changes, encode the changed streams and plan the container
    /// with the container options of `options`.
    ///
    /// See [`PdbEditor`] for the rules.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedEdit`] for an edit that the editor does not
    ///   support, for example an edit that can make an opaque reference
    ///   invalid.
    /// - [`ErrorKind::InvalidReference`] for a reference that does not
    ///   resolve or a name that is not found or not unique.
    /// - [`ErrorKind::UnsupportedVersion`] for a source page size that the
    ///   writer does not support.
    /// - [`ErrorKind::MissingStream`] if the PDB has no TPI or DBI stream, or
    ///   no stream that an edit needs.
    /// - The errors of the stream readers, the record encoders and
    ///   [`MsfBuilder::plan`].
    pub fn plan(&self, options: EditOptions) -> Result<PdbEditPlan<'p, S>> {
        let options = options.msf().unwrap_or_else(|| {
            MsfWriteOptions::default().with_page_size(self.source.output_page_size())
        });
        let pdb = self.source;
        if self.strip
            && (!self.symbols.is_empty()
                || !self.renames.is_empty()
                || self.types.count != 0
                || self.ids.count != 0)
        {
            return Err(unsupported(
                "a strip cannot be combined with a symbol, type or ID edit",
            ));
        }
        let tpi = pdb
            .types()?
            .ok_or_else(|| missing("PDB has no TPI stream"))?;
        let dbi = pdb
            .debug_info()?
            .ok_or_else(|| missing("PDB has no DBI stream"))?;
        let producer = tpi.header().producer();
        let context = Context {
            tpi,
            dbi,
            index_producer: producer,
        };
        let source_info = InfoBuilder::from_info_with(pdb.info(), producer)?;
        let mut info = source_info.clone();
        let mut dbi_age = context.dbi.header().age;
        let identity = info.apply_identity(self.identity, &mut dbi_age)?;
        let mut changes = Changes::new(pdb.container().stream_count());
        let mut dbi_builder = None;
        if self.strip {
            dbi_builder = Some(strip::plan(pdb, &context, &mut info, &mut changes)?);
        }
        named::plan(&self.named, &mut info, &mut changes)?;
        let ends = types::plan(pdb, &context, &self.types, &self.ids, &mut changes)?;
        if !self.symbols.is_empty() || !self.renames.is_empty() {
            symbols::plan(
                pdb,
                &context,
                &self.symbols,
                &self.renames,
                ends,
                &mut changes,
            )?;
        }
        if dbi_builder.is_none() && dbi_age != context.dbi.header().age {
            dbi_builder = Some(DbiBuilder::from_dbi(context.dbi)?);
        }
        if let Some(mut builder) = dbi_builder {
            builder.header.age = dbi_age;
            changes.replace(StreamId(3), builder.to_bytes()?)?;
        }
        if info != source_info {
            let mut bytes = Vec::new();
            info.encode(&mut bytes)?;
            changes.replace(StreamId(1), bytes)?;
        }
        self.freeze(changes, options, identity)
    }

    /// Put the source streams and the changes in a container plan.
    fn freeze(
        &self,
        changes: Changes,
        options: MsfWriteOptions,
        identity: Identity,
    ) -> Result<PdbEditPlan<'p, S>> {
        let container = self.source.container();
        let mut streams = MsfBuilder::new();
        let mut edited = Vec::new();
        // Stream 0 is the previous stream directory. Its pages describe the
        // source layout, so the plan writes it empty.
        streams.push(EditedStream::Owned(Vec::new()))?;
        let Changes {
            source_count,
            mut source,
            added,
        } = changes;
        for id in 1..source_count {
            let stream = StreamId(id);
            let reader = container.stream(stream)?;
            let (planned, differs) = match (source.remove(&id), reader) {
                (Some(Change::Replace(bytes)), Some(reader)) if same_bytes(&reader, &bytes)? => {
                    (Some(EditedStream::Source(reader)), false)
                }
                (Some(Change::Replace(bytes)), _) => (Some(EditedStream::Owned(bytes)), true),
                (Some(Change::Remove), reader) => (None, reader.is_some()),
                (None, reader) => (reader.map(EditedStream::Source), false),
            };
            if differs {
                edited.try_reserve(1).map_err(|_| no_memory())?;
                edited.push(stream);
            }
            match planned {
                Some(planned) => streams.push(planned)?,
                None => streams.push_nil()?,
            };
        }
        for (offset, bytes) in (source_count..).zip(added) {
            edited.try_reserve(1).map_err(|_| no_memory())?;
            edited.push(StreamId(offset));
            match bytes {
                Some(bytes) => streams.push(EditedStream::Owned(bytes))?,
                None => streams.push_nil()?,
            };
        }
        let msf = streams.plan(options)?;
        let (stream_count, file_size) = (msf.stream_count(), msf.file_size());
        Ok(PdbEditPlan {
            streams,
            options,
            identity,
            stream_count,
            file_size,
            changed: edited,
        })
    }
}
