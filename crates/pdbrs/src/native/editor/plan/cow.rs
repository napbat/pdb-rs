//! The copy-on-write layout: each source stream keeps its stream ID, and the
//! plan copies each stream that no change touches (see
//! [`PdbEditor`](crate::native::PdbEditor#layouts)).

use core::cell::Cell;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::patch::{Base, Patch, Patched};
use super::types::write_cow;
use super::{Assembly, Planned, checks, missing, same_bytes};
use crate::io::ReadAt;
use crate::native::auxiliary::{LinkInfo, NamedStreamKind, SourceLink, SourceServer};
use crate::native::codeview::id_record::IdTable;
use crate::native::codeview::index::{FIRST_RECORD_INDEX, IdIndex, TypeIndex};
use crate::native::container::Stream;
use crate::native::dbi::{Dbi, DbiBuilder};
use crate::native::editor::named::Content as NamedContent;
use crate::native::editor::report::{RebuildReason, StreamChange};
use crate::native::editor::{EditOptions, PdbEditor};
use crate::native::info::InfoBuilder;
use crate::native::msf::{MsfWriteOptions, StreamId};
use crate::native::output::no_memory;
use crate::native::types::{HEADER_SIZE, TypeStream, TypeTable, parse_header_prefix};
use crate::native::{IdentityPolicy, Pdb, Producer};
use crate::{Error, ErrorKind, Result};

mod modules;

/// The stream ID of the TPI stream.
pub(super) const TPI_STREAM: StreamId = StreamId(2);
/// The stream ID of the IPI stream.
pub(super) const IPI_STREAM: StreamId = StreamId(4);

/// Push a value with a checked allocation.
pub(super) fn push<T>(list: &mut Vec<T>, value: T) -> Result<()> {
    list.try_reserve(1).map_err(|_| no_memory())?;
    list.push(value);
    Ok(())
}

/// The identity of a copy-on-write plan.
pub(super) enum Identity {
    /// The plan knows the identity.
    Known(crate::native::Identity),
    /// The identity has the source DBI age, which the plan did not read.
    SourceAge {
        /// The GUID.
        guid: [u8; 16],
        /// The Info age.
        info_age: u32,
    },
}

/// The bytes of a changed stream.
pub(super) enum Bytes<'e> {
    /// Bytes that the plan encoded.
    Owned(Vec<u8>),
    /// Bytes that the model holds.
    Borrowed(&'e [u8]),
    /// The source stream with the same stream ID, with patches (#48).
    SourcePatched(Vec<Patch>),
    /// Bytes that the source session holds, with patches (#48).
    SessionPatched(&'e [u8], Vec<Patch>),
}

impl Bytes<'_> {
    /// Return the bytes, or `None` for patched bytes. A patched stream
    /// differs from its source stream, because each patch has a new value.
    fn as_slice(&self) -> Option<&[u8]> {
        match self {
            Self::Owned(bytes) => Some(bytes),
            Self::Borrowed(bytes) => Some(bytes),
            Self::SourcePatched(_) | Self::SessionPatched(..) => None,
        }
    }
}

/// The new contents of one source stream.
enum Change<'e> {
    /// New bytes and the reason.
    Replace(Bytes<'e>, StreamChange),
    /// A nil stream.
    Remove,
}

/// The stream changes of a plan, by stream ID.
pub(super) struct Changes<'e> {
    /// The number of source streams.
    source_count: u32,
    /// The changes of source streams.
    source: BTreeMap<u32, Change<'e>>,
    /// The new streams after the source streams, or `None` for a stream that
    /// the plan reserved and did not fill yet.
    added: Vec<Option<Bytes<'e>>>,
    /// The reason of [`Changes::replace`].
    pub(super) reason: StreamChange,
}

impl<'e> Changes<'e> {
    /// Create an empty change list for a source with `source_count` streams.
    const fn new(source_count: u32) -> Self {
        Self {
            source_count,
            source: BTreeMap::new(),
            added: Vec::new(),
            reason: StreamChange::Edited,
        }
    }

    /// Set the bytes of the source stream `id` with the current reason.
    pub(super) fn replace(&mut self, id: StreamId, bytes: Vec<u8>) -> Result<()> {
        let reason = self.reason;
        self.set(id, Bytes::Owned(bytes), reason)
    }

    /// Set the bytes of stream `id` with `reason`.
    pub(super) fn set(
        &mut self,
        id: StreamId,
        bytes: Bytes<'e>,
        reason: StreamChange,
    ) -> Result<()> {
        match id.0.checked_sub(self.source_count) {
            Some(position) => {
                let slot = self.added.get_mut(position as usize).ok_or_else(|| {
                    Error::new(ErrorKind::InvalidStreamIndex, None)
                        .with_context("edited stream is outside the stream directory")
                })?;
                *slot = Some(bytes);
            }
            None => {
                self.source.insert(id.0, Change::Replace(bytes, reason));
            }
        }
        Ok(())
    }

    /// Make the source stream `id` a nil stream.
    pub(super) fn remove(&mut self, id: StreamId) -> Result<()> {
        if id.0 >= self.source_count {
            return Err(Error::new(ErrorKind::InvalidStreamIndex, None)
                .with_context("removed stream is not a source stream"));
        }
        self.source.insert(id.0, Change::Remove);
        Ok(())
    }

    /// Add a new stream and return its stream ID. `None` reserves the ID.
    pub(super) fn add(&mut self, bytes: Option<Bytes<'e>>) -> Result<StreamId> {
        let id = u32::try_from(self.added.len())
            .ok()
            .and_then(|count| self.source_count.checked_add(count))
            .ok_or_else(|| {
                Error::new(ErrorKind::LimitExceeded, None).with_context("PDB has too many streams")
            })?;
        push(&mut self.added, bytes)?;
        Ok(StreamId(id))
    }

    /// Put the source streams and the changes in an assembly.
    fn freeze<S: ReadAt>(self, pdb: &'e Pdb<S>) -> Result<Assembly<'e, S>> {
        let container = pdb.container();
        let mut assembly = Assembly::new();
        // Stream 0 is the previous stream directory. Its pages describe the
        // source layout, so the plan writes it empty.
        let zero = match container.stream(StreamId(0))? {
            Some(reader) if ReadAt::len(&reader)? != 0 => {
                StreamChange::Rebuilt(RebuildReason::Container)
            }
            _ => StreamChange::Copied,
        };
        assembly.push(Some(Planned::Owned(Vec::new())), Some(StreamId(0)), zero)?;
        let Self {
            source_count,
            mut source,
            added,
            ..
        } = self;
        for id in 1..source_count {
            let stream = StreamId(id);
            let reader = container.stream(stream)?;
            let (planned, change) = match (source.remove(&id), reader) {
                (Some(Change::Replace(bytes, reason)), Some(reader)) => {
                    let same = match bytes.as_slice() {
                        Some(slice) => same_bytes(&reader, slice)?,
                        None => false,
                    };
                    if same {
                        (Some(Planned::Source(reader)), StreamChange::Copied)
                    } else {
                        (Some(planned(bytes, Some(reader))?), reason)
                    }
                }
                (Some(Change::Replace(bytes, reason)), None) => {
                    (Some(planned(bytes, None)?), reason)
                }
                (Some(Change::Remove), Some(_)) => (None, StreamChange::Removed),
                (Some(Change::Remove), None) => (None, StreamChange::Copied),
                (None, reader) => (reader.map(Planned::Source), StreamChange::Copied),
            };
            assembly.push(planned, Some(stream), change)?;
        }
        for bytes in added {
            let bytes = bytes.ok_or_else(|| {
                Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("the plan reserved a stream and did not fill it")
            })?;
            assembly.push(Some(planned(bytes, None)?), None, StreamChange::Added)?;
        }
        Ok(assembly)
    }
}

/// Return the planned stream of changed bytes. `source` is the source
/// stream with the same stream ID, the base of [`Bytes::SourcePatched`].
fn planned<'e, S: ReadAt>(
    bytes: Bytes<'e>,
    source: Option<Stream<'e, S>>,
) -> Result<Planned<'e, S>> {
    Ok(match bytes {
        Bytes::Owned(bytes) => Planned::Owned(bytes),
        Bytes::Borrowed(bytes) => Planned::Borrowed(bytes),
        Bytes::SourcePatched(patches) => {
            let source = source.ok_or_else(|| missing("a patched stream has no source stream"))?;
            Planned::Patched(Patched::new(Base::Source(source), patches)?)
        }
        Bytes::SessionPatched(base, patches) => {
            Planned::Patched(Patched::new(Base::Session(base), patches)?)
        }
    })
}

/// The source streams that the strip and the opaque check read.
pub(super) struct Context<'p> {
    /// The TPI stream.
    pub(super) tpi: TypeStream,
    /// The DBI stream.
    pub(super) dbi: &'p Dbi,
    /// The conventions of the symbol indexes.
    pub(super) index_producer: Producer,
}

/// The state of a copy-on-write plan.
pub(super) struct Work<'e, S> {
    /// The source.
    pub(super) pdb: &'e Pdb<S>,
    /// The editor.
    pub(super) editor: &'e PdbEditor<'e, S>,
    /// The stream changes.
    pub(super) changes: Changes<'e>,
    /// The source streams of the strip and the opaque check.
    context: Option<Context<'e>>,
    /// The new DBI stream values, if a change needs the DBI stream.
    pub(super) dbi: Option<(DbiBuilder, StreamChange)>,
    /// The index after the last TPI record.
    pub(super) type_end: Option<TypeIndex>,
    /// The index after the last IPI record.
    pub(super) id_end: Option<IdIndex>,
    /// The module records and debug header slots that the DBI stream gets.
    pub(super) updates: modules::Updates,
    /// The producer of the source TPI stream, after the first read.
    producer: Cell<Option<Producer>>,
}

impl<'e, S: ReadAt> Work<'e, S> {
    /// Return the source streams of the strip and the opaque check.
    pub(super) fn context(&mut self) -> Result<&Context<'e>> {
        if self.context.is_none() {
            let tpi = self
                .pdb
                .types()?
                .ok_or_else(|| missing("PDB has no TPI stream"))?;
            let dbi = self.source_dbi()?;
            let index_producer = tpi.header().producer();
            self.context = Some(Context {
                tpi,
                dbi,
                index_producer,
            });
        }
        self.context.as_ref().ok_or_else(|| missing("TPI"))
    }

    /// Return the source DBI stream.
    pub(super) fn source_dbi(&self) -> Result<&'e Dbi> {
        self.pdb
            .debug_info()?
            .ok_or_else(|| missing("PDB has no DBI stream"))
    }

    /// Return the producer of the source TPI stream, from its header.
    ///
    /// The function reads only the header, once: a named stream change then
    /// reads no other part of the TPI stream and no hash stream.
    pub(super) fn producer(&self) -> Result<Producer> {
        if let Some(producer) = self.producer.get() {
            return Ok(producer);
        }
        let container = self.pdb.container();
        let stream = if TPI_STREAM.0 < container.stream_count() {
            container.stream(TPI_STREAM)?
        } else {
            None
        };
        let producer = match stream {
            None => Producer::Microsoft,
            Some(stream) => {
                let length = usize::try_from(ReadAt::len(&stream)?).unwrap_or(usize::MAX);
                let mut head = [0; HEADER_SIZE];
                let head = &mut head[..length.min(HEADER_SIZE)];
                stream.read_exact_at(0, head)?;
                parse_header_prefix(head, length)
                    .map_err(|error| error.with_stream(TPI_STREAM, 0))?
                    .producer()
            }
        };
        self.producer.set(Some(producer));
        Ok(producer)
    }

    /// Return the new DBI stream values, made from the source if needed.
    pub(super) fn dbi(&mut self, reason: StreamChange) -> Result<&mut DbiBuilder> {
        if self.dbi.is_none() {
            let builder = match self.editor.plan_dbi() {
                Some(values) => values.dbi.clone(),
                None => DbiBuilder::from_dbi(self.source_dbi()?)?,
            };
            self.dbi = Some((builder, reason));
        }
        match &mut self.dbi {
            Some((builder, current)) => {
                if *current == StreamChange::Rebuilt(RebuildReason::Identity) {
                    *current = reason;
                }
                Ok(builder)
            }
            None => Err(missing("DBI")),
        }
    }

    /// Return the index after the last TPI record of the output.
    pub(super) fn type_end(&self) -> Result<TypeIndex> {
        if let Some(end) = self.type_end {
            return Ok(end);
        }
        Ok(TypeIndex(
            self.pdb
                .type_offsets()?
                .map_or(FIRST_RECORD_INDEX, |offsets| offsets.header().index_end),
        ))
    }

    /// Return the index after the last IPI record of the output.
    pub(super) fn id_end(&self) -> Result<IdIndex> {
        if let Some(end) = self.id_end {
            return Ok(end);
        }
        Ok(IdIndex(
            self.pdb
                .id_offsets()?
                .map_or(FIRST_RECORD_INDEX, |offsets| offsets.header().index_end),
        ))
    }
}

/// Plan the copy-on-write layout.
pub(super) fn plan<'e, S: ReadAt>(
    editor: &'e PdbEditor<'_, S>,
    options: EditOptions,
) -> Result<(Assembly<'e, S>, MsfWriteOptions, Identity)> {
    let pdb = editor.cow_source()?;
    let msf = options
        .msf()
        .unwrap_or_else(|| MsfWriteOptions::default().with_page_size(pdb.output_page_size()));
    let strip = options.strip_private();
    if strip && editor.has_model_edits() {
        return Err(super::unsupported(
            "a strip cannot be combined with a symbol, module, type or ID edit",
        ));
    }
    let mut work = Work {
        pdb,
        editor: shorten(editor),
        changes: Changes::new(pdb.container().stream_count()),
        context: None,
        dbi: None,
        type_end: None,
        id_end: None,
        updates: modules::Updates::default(),
        producer: Cell::new(None),
    };
    if let Some(unnamed) = &editor.unnamed {
        for (id, bytes) in &unnamed.entries {
            let added = work.changes.add(Some(Bytes::Borrowed(bytes)))?;
            if added != *id {
                return Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("stream without a name has another stream ID than the plan"));
            }
        }
    }
    let mut info = None;
    if strip {
        let producer = work.context()?.index_producer;
        let mut builder = InfoBuilder::from_info_with(pdb.info(), producer)?;
        work.changes.reason = StreamChange::Rebuilt(RebuildReason::Strip);
        let context = work.context.take().ok_or_else(|| missing("TPI"))?;
        let dbi = super::strip::plan(pdb, &context, &mut builder, &mut work.changes)?;
        work.context = Some(context);
        work.changes.reason = StreamChange::Edited;
        work.dbi = Some((dbi, StreamChange::Rebuilt(RebuildReason::Strip)));
        info = Some(builder);
    }
    plan_named(&mut work, &mut info)?;
    plan_types(&mut work)?;
    modules::plan(&mut work)?;
    plan_names(&mut work)?;
    let identity = plan_identity(&mut work, &mut info)?;
    modules::plan_dbi(&mut work)?;
    if let Some(info) = info {
        let source = InfoBuilder::from_info_with(pdb.info(), work.producer()?)?;
        if info != source {
            let mut bytes = Vec::new();
            info.encode(&mut bytes)?;
            let reason = if strip {
                StreamChange::Rebuilt(RebuildReason::Strip)
            } else if editor.named.is_some() || editor.features.is_some() {
                StreamChange::Rebuilt(RebuildReason::NamedStreamMap)
            } else {
                StreamChange::Rebuilt(RebuildReason::Identity)
            };
            work.changes.set(StreamId(1), Bytes::Owned(bytes), reason)?;
        }
    }
    let assembly = work.changes.freeze(pdb)?;
    Ok((assembly, msf, identity))
}

/// Return the editor with the lifetime of the plan.
const fn shorten<'e, 'p: 'e, S>(editor: &'e PdbEditor<'p, S>) -> &'e PdbEditor<'e, S> {
    editor
}

/// Return the Info model, made from the source if needed.
fn info_model<'i, S: ReadAt>(
    work: &Work<'_, S>,
    info: &'i mut Option<InfoBuilder>,
) -> Result<&'i mut InfoBuilder> {
    if info.is_none() {
        *info = Some(InfoBuilder::from_info_with(
            work.pdb.info(),
            work.producer()?,
        )?);
    }
    info.as_mut().ok_or_else(|| missing("Info"))
}

/// Decode `bytes` with the decoder of the kind of `name`, if the kind has one.
fn check_contents(name: &[u8], bytes: &[u8], stream: StreamId) -> Result<()> {
    match NamedStreamKind::classify(name) {
        NamedStreamKind::SourceServer => SourceServer::parse(bytes, stream).map(|_| ()),
        NamedStreamKind::SourceLink => SourceLink::parse(bytes, stream).map(|_| ()),
        NamedStreamKind::NumberedSourceLink { .. } => {
            SourceLink::parse_numbered(bytes, stream).map(|_| ())
        }
        NamedStreamKind::LinkInfo => LinkInfo::parse(bytes, stream).map(|_| ()),
        _ => Ok(()),
    }
}

/// Apply the named stream changes: first the new names in model order, then
/// the removals in source order.
fn plan_named<S: ReadAt>(work: &mut Work<'_, S>, info: &mut Option<InfoBuilder>) -> Result<()> {
    let Some(named) = &work.editor.named else {
        return Ok(());
    };
    for entry in &named.entries {
        if let (Some(stream), NamedContent::Bytes(bytes)) = (entry.source, &entry.content) {
            check_contents(&entry.name, bytes, stream)?;
            work.changes
                .set(stream, Bytes::Borrowed(bytes), StreamChange::Edited)?;
        }
    }
    for entry in &named.entries {
        if let (None, NamedContent::Bytes(bytes)) = (entry.source, &entry.content) {
            let stream = work.changes.add(Some(Bytes::Borrowed(bytes)))?;
            check_contents(&entry.name, bytes, stream)?;
            info_model(work, info)?.set_named_stream(&entry.name, stream)?;
        }
    }
    for entry in &named.entries {
        if let (Some(stream), NamedContent::Removed) = (entry.source, &entry.content) {
            info_model(work, info)?.remove_named_stream(&entry.name)?;
            work.changes.remove(stream)?;
        }
    }
    Ok(())
}

/// Plan the TPI and IPI streams.
fn plan_types<S: ReadAt>(work: &mut Work<'_, S>) -> Result<()> {
    let pdb = work.pdb;
    if let Some(types) = &work.editor.types
        && let Some(planned) = write_cow(&types.list, |id| pdb.stream_bytes(id))?
    {
        checks::check_types(&TypeTable::new(&planned.stream)?)?;
        work.type_end = Some(TypeIndex(planned.index_end()));
        store(work, planned, TPI_STREAM)?;
    }
    if let Some(ids) = &work.editor.ids
        && let Some(planned) = write_cow(&ids.list, |id| pdb.stream_bytes(id))?
    {
        IdTable::new(&planned.stream)?.check(work.type_end()?)?;
        checks::check_id_order(ids.iter())?;
        work.id_end = Some(IdIndex(planned.index_end()));
        store(work, planned, IPI_STREAM)?;
    }
    Ok(())
}

/// Give a planned type stream and its hash stream to the changes.
fn store<S>(
    work: &mut Work<'_, S>,
    planned: super::types::PlannedTypes,
    id: StreamId,
) -> Result<()> {
    if let (Some(hash), Some(stream)) = (planned.hash, planned.stream.header().hash_stream) {
        work.changes.set(
            stream,
            Bytes::Owned(hash),
            StreamChange::Rebuilt(RebuildReason::TypeHashes),
        )?;
    }
    work.changes.set(
        id,
        Bytes::Owned(planned.stream.into_bytes()),
        StreamChange::Edited,
    )
}

/// Plan the `/names` stream.
fn plan_names<S: ReadAt>(work: &mut Work<'_, S>) -> Result<()> {
    let Some(crate::native::editor::Names::Table(names)) = &work.editor.names else {
        return Ok(());
    };
    let stream = work
        .pdb
        .info()
        .named_stream(b"/names")
        .ok_or_else(|| missing("the PDB has no /names table"))?;
    let mut bytes = Vec::new();
    names.encode(&mut bytes)?;
    work.changes
        .set(stream, Bytes::Owned(bytes), StreamChange::Edited)
}

/// Apply the identity policy and the feature codes. Return the identity.
fn plan_identity<S: ReadAt>(
    work: &mut Work<'_, S>,
    info: &mut Option<InfoBuilder>,
) -> Result<Identity> {
    let editor = work.editor;
    if let Some(features) = &editor.features {
        info_model(work, info)?.set_features(features)?;
    }
    let policy = editor.policy;
    if policy == IdentityPolicy::Preserve && work.dbi.is_none() && !modules::needs_dbi(work) {
        let crate::native::Identity::Modern { guid, age } = work.pdb.info().identity() else {
            return Err(Error::new(ErrorKind::UnsupportedVersion, None)
                .with_context("PDB Info stream has no GUID identity"));
        };
        return Ok(Identity::SourceAge {
            guid,
            info_age: age,
        });
    }
    let source_age = work.source_dbi()?.header().age;
    let mut dbi_age = source_age;
    let identity = info_model(work, info)?.apply_identity(policy, &mut dbi_age)?;
    if dbi_age != source_age || work.dbi.is_some() {
        work.dbi(StreamChange::Rebuilt(RebuildReason::Identity))?
            .header
            .age = dbi_age;
    }
    Ok(Identity::Known(identity))
}
