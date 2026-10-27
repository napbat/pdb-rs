//! The new layout: every stream is encoded from the model and gets a new
//! stream ID (see [`PdbEditor`](super::PdbEditor#layouts)).

use alloc::vec::Vec;

use super::modules::{self, EncodedModule, OffsetMap};
use super::shared::{self, ModuleTargets};
use super::types::{NewTypeOptions, PlannedTypes, write_new};
use super::{Assembly, Planned, checks, missing, same_bytes};
use crate::io::ReadAt;
use crate::native::auxiliary::{NamedStreamKind, TmCache, TmCacheBuilder};
use crate::native::codeview::LeafKind;
use crate::native::codeview::id_record::IdTable;
use crate::native::codeview::index::{
    IdIndex, ModuleIndex, ModuleSymbolHandle, ModuleSymbolOffset, TypeIndex,
};
use crate::native::codeview::symbol::ModuleSymbolTarget;
use crate::native::codeview::symbol::validate::{
    ModuleRecord, ValidationContext, validate_module_symbols, validate_shared_symbols,
};
use crate::native::dbi::{DbiBuilder, Module, SourceInfo};
use crate::native::editor::module::{GlobalRef, ModuleContents, SYMBOL_BASE};
use crate::native::editor::named::Content;
use crate::native::editor::report::RebuildReason;
use crate::native::editor::streams::DebugContent;
use crate::native::editor::{EditOptions, ModuleSlot, Names, PdbEditor, StreamChange};
use crate::native::info::InfoBuilder;
use crate::native::msf::{MsfWriteOptions, StreamId};
use crate::native::output::no_memory;
use crate::native::symbols::{BucketCount, SymbolOffset};
use crate::native::types::DEFAULT_HASH_BUCKETS;
use crate::native::{Identity, Pdb, Producer};
use crate::{Error, ErrorKind, Result};

mod module_types;

/// The options of the new layout after the conventions of the source.
#[derive(Clone, Copy, Debug)]
struct Resolved {
    /// The container options.
    msf: MsfWriteOptions,
    /// The type stream options.
    types: NewTypeOptions,
    /// The producer of the symbol index conventions.
    index_producer: Producer,
    /// The producer of the named-stream map conventions.
    info_producer: Producer,
    /// `true` to write the TPI hash stream.
    type_hash_stream: bool,
    /// `true` to write the IPI hash stream.
    id_hash_stream: bool,
}

/// Return the options with the conventions of the source for each option
/// without a value.
fn resolve<S>(editor: &PdbEditor<'_, S>, options: EditOptions) -> Resolved {
    let source = editor.rebuild.as_ref().map(|rebuild| rebuild.conventions);
    let producer = source.map(|source| source.producer);
    Resolved {
        msf: options.msf().unwrap_or_else(|| match source {
            Some(source) => MsfWriteOptions::default().with_page_size(source.page_size),
            None => MsfWriteOptions::default(),
        }),
        types: NewTypeOptions {
            hash_buckets: options
                .hash_buckets()
                .or(source.map(|source| source.hash_buckets))
                .unwrap_or(DEFAULT_HASH_BUCKETS),
            producer: options
                .type_hash_producer()
                .or(producer)
                .unwrap_or(Producer::Microsoft),
        },
        index_producer: options
            .index_producer()
            .or(producer)
            .unwrap_or(Producer::Microsoft),
        info_producer: options
            .info_producer()
            .or(producer)
            .unwrap_or(Producer::Microsoft),
        type_hash_stream: options
            .type_hash_stream()
            .or(source.map(|source| source.type_hash_stream))
            .unwrap_or(true),
        id_hash_stream: options
            .id_hash_stream()
            .or(source.map(|source| source.id_hash_stream))
            .unwrap_or(true),
    }
}

/// The stream IDs that the plan assigns.
struct StreamIds {
    /// The TPI hash stream.
    tpi_hash: Option<StreamId>,
    /// The IPI hash stream.
    ipi_hash: Option<StreamId>,
    /// The global symbol index.
    globals: StreamId,
    /// The public symbol index.
    publics: StreamId,
    /// The symbol record stream.
    symbols: StreamId,
    /// The module stream of each module, or `None`.
    modules: Vec<Option<StreamId>>,
    /// The stream of each optional debug stream, in slot order.
    debug: Vec<StreamId>,
    /// The `/names` stream.
    names: Option<StreamId>,
    /// The stream of each named stream that the output keeps, in model order.
    named: Vec<StreamId>,
    /// The output stream ID of each stream without a name, by model ID.
    unnamed: Vec<(StreamId, StreamId)>,
    /// The number of streams.
    count: u32,
}

/// A counter that gives the next stream ID.
struct Counter(u32);

impl Counter {
    /// Return the next stream ID.
    fn next(&mut self) -> Result<StreamId> {
        let id = StreamId(self.0);
        self.0 = self.0.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::LimitExceeded, None).with_context("PDB has too many streams")
        })?;
        Ok(id)
    }
}

/// The decoded parts of the model of the new layout.
struct Parts<'e> {
    /// The modules.
    modules: Vec<&'e ModuleContents>,
}

/// Return the decoded modules of the model.
fn parts<'e, S>(editor: &'e PdbEditor<'_, S>) -> Result<Parts<'e>> {
    let slots = editor.modules.as_deref().unwrap_or_default();
    let mut modules = Vec::new();
    modules
        .try_reserve_exact(slots.len())
        .map_err(|_| no_memory())?;
    for slot in slots {
        match slot {
            ModuleSlot::Decoded(module) => modules.push(&**module),
            ModuleSlot::Source => {
                return Err(missing("the new layout has a module that is not decoded"));
            }
        }
    }
    Ok(Parts { modules })
}

/// Assign the stream IDs in the order of the layout.
fn assign<S>(
    editor: &PdbEditor<'_, S>,
    parts: &Parts<'_>,
    resolved: &Resolved,
) -> Result<StreamIds> {
    let mut counter = Counter(5);
    let tpi_hash = resolved
        .type_hash_stream
        .then(|| counter.next())
        .transpose()?;
    let ipi_hash = resolved
        .id_hash_stream
        .then(|| counter.next())
        .transpose()?;
    let globals = counter.next()?;
    let publics = counter.next()?;
    let symbols = counter.next()?;
    let mut modules = Vec::new();
    modules
        .try_reserve_exact(parts.modules.len())
        .map_err(|_| no_memory())?;
    for module in &parts.modules {
        modules.push(module.has_stream().then(|| counter.next()).transpose()?);
    }
    let mut debug = Vec::new();
    for _ in editor.debug.iter().flat_map(|list| &list.entries) {
        debug.try_reserve(1).map_err(|_| no_memory())?;
        debug.push(counter.next()?);
    }
    let names_stream = matches!(editor.names, Some(Names::Table(_)))
        .then(|| counter.next())
        .transpose()?;
    let mut named = Vec::new();
    for entry in editor.named.iter().flat_map(|named| &named.entries) {
        if entry.content != Content::Removed {
            named.try_reserve(1).map_err(|_| no_memory())?;
            named.push(counter.next()?);
        }
    }
    let mut unnamed = Vec::new();
    for (id, _) in editor.unnamed.iter().flat_map(|list| &list.entries) {
        unnamed.try_reserve(1).map_err(|_| no_memory())?;
        unnamed.push((*id, counter.next()?));
    }
    Ok(StreamIds {
        tpi_hash,
        ipi_hash,
        globals,
        publics,
        symbols,
        modules,
        debug,
        names: names_stream,
        named,
        unnamed,
        count: counter.0,
    })
}

/// Return the output stream ID of a stream without a name.
fn map_stream(unnamed: &[(StreamId, StreamId)], id: StreamId) -> Result<StreamId> {
    unnamed
        .iter()
        .find(|(model, _)| *model == id)
        .map(|(_, output)| *output)
        .ok_or_else(|| {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("a record names a stream that is not a stream without a name")
        })
}

/// The module targets of the new layout.
struct Targets<'m> {
    /// The modules.
    modules: &'m [&'m ModuleContents],
    /// The encoded regions of each module, or `None`.
    regions: &'m [Option<EncodedModule>],
}

impl ModuleTargets for Targets<'_> {
    fn handle(&self, handle: ModuleSymbolHandle) -> Result<ModuleSymbolOffset> {
        self.regions
            .get(handle.module as usize)
            .and_then(Option::as_ref)
            .and_then(|regions| regions.ids.get(handle.position as usize).copied().flatten())
            .map(ModuleSymbolOffset)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidReference, None)
                    .with_context("module symbol handle has no record in the editor")
            })
    }

    fn map(&self, module: ModuleIndex) -> Option<&OffsetMap> {
        let index = module.0 as usize;
        self.modules.get(index)?.source()?;
        self.regions
            .get(index)?
            .as_ref()
            .map(|regions| &regions.map)
    }

    fn name(&self, target: ModuleSymbolTarget) -> Option<&[u8]> {
        let module = shared::target_module(target)?;
        self.modules.get(module.0 as usize)?.target_name(target)
    }
}

/// The identity of the new layout.
fn identity<S>(
    editor: &PdbEditor<'_, S>,
    producer: Producer,
) -> Result<(InfoBuilder, u32, Identity)> {
    let model = editor
        .info
        .as_ref()
        .ok_or_else(|| missing("the new layout has no identity"))?;
    let mut info = InfoBuilder::new(producer, model.signature, model.guid, model.age)?;
    info.set_features(&model.features)?;
    let mut dbi_age = model.dbi_age;
    let identity = info.apply_identity(editor.policy, &mut dbi_age)?;
    Ok((info, dbi_age, identity))
}

/// Plan the new layout.
pub(super) fn plan<'e, S: ReadAt>(
    editor: &'e PdbEditor<'_, S>,
    options: EditOptions,
) -> Result<(Assembly<'e, S>, MsfWriteOptions, Identity)> {
    let resolved = resolve(editor, options);
    let parts = parts(editor)?;
    let ids = assign(editor, &parts, &resolved)?;
    let (mut info, dbi_age, identity) = identity(editor, resolved.info_producer)?;
    let types = editor
        .types
        .as_ref()
        .ok_or_else(|| missing("the new layout has no TPI records"))?;
    let id_list = editor
        .ids
        .as_ref()
        .ok_or_else(|| missing("the new layout has no IPI records"))?;
    let tpi = write_new(&types.list, resolved.types, ids.tpi_hash)?;
    checks::check_types(&crate::native::types::TypeTable::new(&tpi.stream)?)?;
    let ipi = write_new(&id_list.list, resolved.types, ids.ipi_hash)?;
    IdTable::new(&ipi.stream)?.check(TypeIndex(tpi.index_end()))?;
    checks::check_id_order(id_list.iter())?;
    let encoded = encode_symbols(editor, &parts, &ids, &tpi, &ipi, &resolved, &info)?;
    let dbi = dbi_stream(editor, &parts, &ids, encoded.records, dbi_age)?;
    let mut assembly = Assembly::new();
    push_streams(
        editor,
        &mut assembly,
        Streams {
            info: &mut info,
            tpi,
            ipi,
            dbi,
            globals: encoded.globals,
            publics: encoded.publics,
            symbols: encoded.symbols,
            modules: encoded.streams,
            ids: &ids,
        },
    )?;
    report_dropped(editor, &mut assembly)?;
    Ok((assembly, resolved.msf, identity))
}

/// The encoded symbol data of the new layout.
struct EncodedSymbols {
    /// The module records in module order.
    records: Vec<Module<'static>>,
    /// The module streams in module order.
    streams: Vec<Option<Vec<u8>>>,
    /// The symbol record stream.
    symbols: Vec<u8>,
    /// The global symbol index stream.
    globals: Vec<u8>,
    /// The public symbol index stream.
    publics: Vec<u8>,
}

/// Encode the module streams, the symbol record stream and the indexes.
fn encode_symbols<S>(
    editor: &PdbEditor<'_, S>,
    parts: &Parts<'_>,
    ids: &StreamIds,
    tpi: &PlannedTypes,
    ipi: &PlannedTypes,
    resolved: &Resolved,
    info: &InfoBuilder,
) -> Result<EncodedSymbols> {
    let id_table = IdTable::new(&ipi.stream)?;
    let id_kind = |id: IdIndex| id_table.record(id).map(|record| LeafKind(record.kind));
    let base = ValidationContext::new(TypeIndex(tpi.index_end()), IdIndex(ipi.index_end()))
        .with_id_kinds(&id_kind)
        .with_module_count(parts.modules.len());
    let map = |id| map_stream(&ids.unnamed, id);
    let mut regions = Vec::new();
    regions
        .try_reserve_exact(parts.modules.len())
        .map_err(|_| no_memory())?;
    for (module, stream) in parts.modules.iter().zip(&ids.modules) {
        regions.push(match stream {
            Some(_) => Some(modules::encode_regions(module, &map)?),
            None => None,
        });
    }
    check_modules(editor, &regions, ids, &base)?;
    let mut kinds = Vec::new();
    for (index, region) in regions.iter().enumerate() {
        if let Some(symbols) = region.as_ref().and_then(|region| region.symbols.as_deref()) {
            for (offset, kind) in modules::record_kinds(symbols)? {
                kinds.try_reserve(1).map_err(|_| no_memory())?;
                kinds.push(((index, offset), kind));
            }
        }
    }
    kinds.sort_unstable_by_key(|(key, _)| *key);
    let module_record = |module: ModuleIndex, offset: ModuleSymbolOffset| {
        kinds
            .binary_search_by_key(&(module.0 as usize, offset.0), |(key, _)| *key)
            .map_or(ModuleRecord::Missing, |position| {
                ModuleRecord::Kind(kinds[position].1)
            })
    };
    let model = editor
        .symbols
        .as_ref()
        .ok_or_else(|| missing("the new layout has no symbol records"))?;
    let targets = Targets {
        modules: &parts.modules,
        regions: &regions,
    };
    let shared = shared::encode_records(model, &targets, Some(ids.symbols))?;
    checks::check(&validate_shared_symbols(
        &shared.bytes,
        ids.symbols,
        &base.with_module_records(&module_record),
    )?)?;
    let (records, streams) = join_modules(parts, regions, ids, &shared)?;
    let buckets = BucketCount::from_features(info.features());
    let globals = shared::encode_globals(model, &shared, buckets, resolved.index_producer)?;
    let publics = shared::encode_publics(model, &shared, buckets, resolved.index_producer)?;
    Ok(EncodedSymbols {
        records,
        streams,
        symbols: shared.bytes,
        globals,
        publics,
    })
}

/// Check the references of the module symbols.
///
/// The type and ID indices of a module with an `S_MOD_TYPEREF` record
/// resolve against the records that it names, not against TPI and IPI.
fn check_modules<S>(
    editor: &PdbEditor<'_, S>,
    regions: &[Option<EncodedModule>],
    ids: &StreamIds,
    base: &ValidationContext<'_>,
) -> Result<()> {
    let mut symbols = Vec::new();
    symbols
        .try_reserve_exact(regions.len())
        .map_err(|_| no_memory())?;
    symbols.extend(
        regions
            .iter()
            .map(|region| region.as_ref().and_then(|region| region.symbols.as_deref())),
    );
    let mut unnamed = Vec::new();
    for (model, bytes) in editor.unnamed.iter().flat_map(|list| &list.entries) {
        unnamed.try_reserve(1).map_err(|_| no_memory())?;
        unnamed.push((map_stream(&ids.unnamed, *model)?, bytes.as_slice()));
    }
    let (types, streams) = module_types::module_types(&symbols, &unnamed)?;
    for ((records, types), stream) in symbols.iter().zip(types).zip(&ids.modules) {
        let (Some(records), Some(stream)) = (records, stream) else {
            continue;
        };
        streams.with_context(types, base, regions.len(), |context| {
            checks::check(&validate_module_symbols(
                records,
                *stream,
                ModuleSymbolOffset(SYMBOL_BASE),
                context,
            )?)
        })?;
    }
    Ok(())
}

/// The module records and the module streams of the new layout.
type JoinedModules = (Vec<Module<'static>>, Vec<Option<Vec<u8>>>);

/// Join the module streams with their global references and set the module
/// records.
fn join_modules(
    parts: &Parts<'_>,
    regions: Vec<Option<EncodedModule>>,
    ids: &StreamIds,
    shared: &shared::EncodedShared,
) -> Result<JoinedModules> {
    let mut records = Vec::new();
    let mut streams = Vec::new();
    records
        .try_reserve_exact(parts.modules.len())
        .map_err(|_| no_memory())?;
    streams
        .try_reserve_exact(parts.modules.len())
        .map_err(|_| no_memory())?;
    for ((module, region), stream) in parts.modules.iter().zip(regions).zip(&ids.modules) {
        let (Some(region), Some(stream)) = (region, *stream) else {
            records.push(modules::streamless(module)?);
            streams.push(None);
            continue;
        };
        let refs = global_refs(module, shared)?;
        let (record, bytes) = modules::join(module, &region, &refs, stream)?;
        records.push(record);
        streams.push(Some(bytes));
    }
    Ok((records, streams))
}

/// Resolve the global references of a module. A reference to a removed
/// record goes away.
pub(super) fn global_refs(
    module: &ModuleContents,
    shared: &shared::EncodedShared,
) -> Result<Vec<SymbolOffset>> {
    let mut refs = Vec::new();
    refs.try_reserve_exact(module.global_refs().len())
        .map_err(|_| no_memory())?;
    for reference in module.global_refs() {
        let offset = match *reference {
            GlobalRef::Handle(handle) => shared
                .offsets
                .get(handle.0 as usize)
                .copied()
                .ok_or_else(|| {
                    Error::new(ErrorKind::InvalidReference, None)
                        .with_context("global reference has no symbol record")
                })?,
            GlobalRef::Offset(offset) => shared.get(offset),
        };
        if let Some(offset) = offset {
            refs.push(offset);
        }
    }
    Ok(refs)
}

/// Encode the DBI stream.
fn dbi_stream<S>(
    editor: &PdbEditor<'_, S>,
    parts: &Parts<'_>,
    ids: &StreamIds,
    modules: Vec<Module<'static>>,
    dbi_age: u32,
) -> Result<Vec<u8>> {
    let model = editor
        .dbi
        .as_ref()
        .ok_or_else(|| missing("the new layout has no DBI values"))?;
    let mut header = model.dbi.header;
    header.age = dbi_age;
    header.global_stream = Some(ids.globals);
    header.public_stream = Some(ids.publics);
    header.symbol_stream = Some(ids.symbols);
    let mut dbi = DbiBuilder::new(header);
    dbi.modules = modules;
    dbi.section_contribution_version = model.dbi.section_contribution_version;
    dbi.section_contributions
        .try_reserve_exact(model.dbi.section_contributions.len())
        .map_err(|_| no_memory())?;
    dbi.section_contributions
        .extend_from_slice(&model.dbi.section_contributions);
    dbi.section_map.clone_from(&model.dbi.section_map);
    dbi.type_servers.clone_from(&model.dbi.type_servers);
    dbi.ec_names.clone_from(&model.dbi.ec_names);
    if !parts.modules.is_empty() {
        dbi.source_info =
            Some(SourceInfo::from_file_names(parts.modules.iter().map(
                |module| module.files().iter().map(crate::Bytes::as_bytes),
            ))?);
    }
    let entries = editor.debug.as_ref().map_or(&[][..], |list| &list.entries);
    let slots = entries
        .iter()
        .map(|(kind, _)| kind.index() + 1)
        .fold(model.debug_slots, usize::max);
    dbi.debug_streams
        .try_reserve_exact(slots)
        .map_err(|_| no_memory())?;
    dbi.debug_streams.resize(slots, None);
    for ((kind, _), id) in entries.iter().zip(&ids.debug) {
        dbi.debug_streams[kind.index()] = Some(*id);
    }
    dbi.to_bytes()
}

/// The encoded streams of the new layout.
struct Streams<'i, 'a> {
    /// The Info model; the plan adds the named streams.
    info: &'i mut InfoBuilder,
    /// The TPI stream.
    tpi: PlannedTypes,
    /// The IPI stream.
    ipi: PlannedTypes,
    /// The DBI stream.
    dbi: Vec<u8>,
    /// The global symbol index.
    globals: Vec<u8>,
    /// The public symbol index.
    publics: Vec<u8>,
    /// The symbol record stream.
    symbols: Vec<u8>,
    /// The module streams.
    modules: Vec<Option<Vec<u8>>>,
    /// The stream IDs.
    ids: &'a StreamIds,
}

/// Return the report entry of an output stream with `source`.
fn change<S: ReadAt>(
    pdb: Option<&Pdb<S>>,
    source: Option<StreamId>,
    bytes: &[u8],
) -> Result<StreamChange> {
    let (Some(pdb), Some(source)) = (pdb, source) else {
        return Ok(StreamChange::Added);
    };
    match pdb.container().stream(source)? {
        Some(reader) if same_bytes(&reader, bytes)? => Ok(StreamChange::Copied),
        _ => Ok(StreamChange::Rebuilt(RebuildReason::Layout)),
    }
}

/// Push one stream with its report entry.
fn push<'e, S: ReadAt>(
    assembly: &mut Assembly<'e, S>,
    pdb: Option<&Pdb<S>>,
    stream: Planned<'e, S>,
    source: Option<StreamId>,
) -> Result<StreamId> {
    let bytes: &[u8] = match &stream {
        Planned::Owned(bytes) => bytes,
        Planned::Borrowed(bytes) => bytes,
        Planned::Source(_) | Planned::Patched(_) => &[],
    };
    let change = change(pdb, source, bytes)?;
    let source = if change == StreamChange::Added {
        None
    } else {
        source
    };
    assembly.push(Some(stream), source, change)
}

/// Push all streams in stream ID order and finish the Info stream.
fn push_streams<'e, S: ReadAt>(
    editor: &'e PdbEditor<'_, S>,
    assembly: &mut Assembly<'e, S>,
    streams: Streams<'_, '_>,
) -> Result<()> {
    let pdb = editor.source;
    let sources = editor.rebuild.as_ref().map(|rebuild| &rebuild.sources);
    let ids = streams.ids;
    // The Info stream is stream 1, but its named-stream map needs the IDs of
    // the later streams: `/names` first, then the named streams in model order.
    let named = named_streams(editor, ids)?;
    if let Some(id) = ids.names {
        streams.info.set_named_stream(b"/names", id)?;
    }
    for ((name, _, _), id) in named.iter().zip(&ids.named) {
        streams.info.set_named_stream(name, *id)?;
    }
    let mut info = Vec::new();
    streams.info.encode(&mut info)?;
    push(assembly, pdb, Planned::Owned(Vec::new()), Some(StreamId(0)))?;
    push(assembly, pdb, Planned::Owned(info), Some(StreamId(1)))?;
    push(
        assembly,
        pdb,
        Planned::Owned(streams.tpi.stream.into_bytes()),
        Some(StreamId(2)),
    )?;
    push(
        assembly,
        pdb,
        Planned::Owned(streams.dbi),
        Some(StreamId(3)),
    )?;
    let ipi_source = sources.and_then(|sources| sources.ipi).map(|_| StreamId(4));
    push(
        assembly,
        pdb,
        Planned::Owned(streams.ipi.stream.into_bytes()),
        ipi_source,
    )?;
    for (hash, source) in [
        (
            streams.tpi.hash,
            sources.and_then(|sources| sources.tpi_hash),
        ),
        (
            streams.ipi.hash,
            sources.and_then(|sources| sources.ipi_hash),
        ),
    ] {
        if let Some(hash) = hash {
            push(assembly, pdb, Planned::Owned(hash), source)?;
        }
    }
    push(
        assembly,
        pdb,
        Planned::Owned(streams.globals),
        sources.and_then(|sources| sources.globals),
    )?;
    push(
        assembly,
        pdb,
        Planned::Owned(streams.publics),
        sources.and_then(|sources| sources.publics),
    )?;
    push(
        assembly,
        pdb,
        Planned::Owned(streams.symbols),
        sources.and_then(|sources| sources.symbols),
    )?;
    let slots = editor.modules.as_deref().unwrap_or_default();
    for (stream, slot) in streams.modules.into_iter().zip(slots) {
        let Some(stream) = stream else { continue };
        let source = match slot {
            ModuleSlot::Decoded(module) => module
                .source()
                .and_then(|source| source.stream.as_ref())
                .map(crate::native::dbi::ModuleStream::stream),
            ModuleSlot::Source => None,
        };
        push(assembly, pdb, Planned::Owned(stream), source)?;
    }
    for (kind, content) in editor.debug.iter().flat_map(|list| &list.entries) {
        let DebugContent::Bytes(bytes) = content else {
            return Err(missing(
                "the new layout has a debug stream that is not decoded",
            ));
        };
        let source = sources.and_then(|sources| {
            sources
                .debug
                .iter()
                .find(|(slot, _)| slot == kind)
                .map(|(_, id)| *id)
        });
        push(assembly, pdb, Planned::Borrowed(bytes), source)?;
    }
    if let Some(Names::Table(names)) = &editor.names {
        let mut bytes = Vec::new();
        names.encode(&mut bytes)?;
        push(
            assembly,
            pdb,
            Planned::Owned(bytes),
            sources.and_then(|sources| sources.names),
        )?;
    }
    for (_, stream, source) in named {
        push(assembly, pdb, stream, source)?;
    }
    for (id, bytes) in editor.unnamed.iter().flat_map(|list| &list.entries) {
        let source = sources.and_then(|sources| (id.0 < sources.count).then_some(*id));
        push(assembly, pdb, Planned::Borrowed(bytes), source)?;
    }
    if assembly.next_id()?.0 != ids.count {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("planned stream count differs from the assigned stream IDs"));
    }
    Ok(())
}

/// A named stream of the new layout: the name, the bytes and the source
/// stream.
type NamedStream<'e, S> = (&'e [u8], Planned<'e, S>, Option<StreamId>);

/// Plan the named streams other than `/names`, in model order. A
/// `/TMCache` stream gets the new stream IDs of its type maps.
fn named_streams<'e, S>(
    editor: &'e PdbEditor<'_, S>,
    ids: &StreamIds,
) -> Result<Vec<NamedStream<'e, S>>> {
    let mut plan = Vec::new();
    for entry in editor.named.iter().flat_map(|named| &named.entries) {
        let bytes = match &entry.content {
            Content::Removed => continue,
            Content::Bytes(bytes) => bytes.as_slice(),
            Content::Source => {
                return Err(missing(
                    "the new layout has a named stream that is not decoded",
                ));
            }
        };
        let stream = if NamedStreamKind::classify(&entry.name) == NamedStreamKind::TmCache
            && !bytes.is_empty()
        {
            Planned::Owned(remap_cache(bytes, entry.source, &ids.unnamed)?)
        } else {
            Planned::Borrowed(bytes)
        };
        plan.try_reserve(1).map_err(|_| no_memory())?;
        plan.push((entry.name.as_bytes(), stream, entry.source));
    }
    Ok(plan)
}

/// Encode a `/TMCache` stream with the new stream IDs of its type maps.
fn remap_cache(
    bytes: &[u8],
    source: Option<StreamId>,
    unnamed: &[(StreamId, StreamId)],
) -> Result<Vec<u8>> {
    let cache = TmCache::parse(bytes, source.unwrap_or(StreamId(0)))?;
    let mut builder = TmCacheBuilder::new();
    for slot in cache.streams() {
        builder.push_stream(slot.map(|id| map_stream(unnamed, id)).transpose()?)?;
    }
    for entry in cache.entries() {
        builder.push_entry(entry.slot, entry.checksum)?;
    }
    let mut out = Vec::new();
    builder.encode(&mut out)?;
    Ok(out)
}

/// Report the source streams that the model does not keep.
fn report_dropped<S>(editor: &PdbEditor<'_, S>, assembly: &mut Assembly<'_, S>) -> Result<()> {
    if let Some(rebuild) = &editor.rebuild {
        for &stream in &rebuild.unclaimed {
            assembly.drop_source(stream)?;
        }
    }
    Ok(())
}
