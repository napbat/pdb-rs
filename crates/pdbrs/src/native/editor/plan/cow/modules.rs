//! The modules, the symbol record stream, the symbol indexes, the debug
//! streams and the DBI stream of a copy-on-write plan.

use alloc::vec::Vec;

use super::super::layout::global_refs;
use super::super::modules::{self as encode, EncodedModule, OffsetMap};
use super::super::opaque::{self, Scope};
use super::super::patch::Patch;
use super::super::shared::{self, EncodedShared, ModuleTargets};
use super::super::{checks, missing};
use super::{Bytes, Work, push};
use crate::io::ReadAt;
use crate::native::codeview::index::{ModuleIndex, ModuleSymbolHandle, ModuleSymbolOffset};
use crate::native::codeview::symbol::validate::{
    ValidationContext, validate_module_symbols, validate_shared_symbols,
};
use crate::native::codeview::symbol::{ModuleSymbolTarget, ModuleTypeReference};
use crate::native::dbi::{GlobalRefRegion, Module, SourceInfo, read_global_refs};
use crate::native::editor::ModuleSlot;
use crate::native::editor::module::{ModuleContents, SYMBOL_BASE};
use crate::native::editor::report::{IndexSpace, RebuildReason, StreamChange};
use crate::native::editor::streams::{DebugContent, kind_of};
use crate::native::msf::StreamId;
use crate::native::symbols::{GlobalSymbolIndex, PublicSymbolIndex, RecordOffsets, SymbolOffset};
use crate::{Error, ErrorKind, Result};

/// The changes of the DBI stream that the modules and the debug streams
/// make.
#[derive(Debug, Default)]
pub(in super::super) struct Updates {
    /// The new module records by module index.
    records: Vec<(u32, Module<'static>)>,
    /// `true` if the source file list of a module changed.
    files: bool,
    /// The new debug header slots.
    debug: Vec<(usize, StreamId)>,
    /// The debug header slots whose source stream the plan removes.
    removed_debug: Vec<usize>,
}

/// The decoded modules and their encoded regions.
struct Encoded<'e> {
    /// The module, its regions and its source stream.
    modules: Vec<(&'e ModuleContents, Option<EncodedModule>)>,
}

impl ModuleTargets for Encoded<'_> {
    fn handle(&self, handle: ModuleSymbolHandle) -> Result<ModuleSymbolOffset> {
        self.modules
            .iter()
            .find(|(module, _)| module.index().0 == handle.module)
            .and_then(|(_, regions)| regions.as_ref())
            .and_then(|regions| regions.ids.get(handle.position as usize).copied().flatten())
            .map(ModuleSymbolOffset)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidReference, None)
                    .with_context("module symbol handle has no record in the editor")
            })
    }

    fn map(&self, module: ModuleIndex) -> Option<&OffsetMap> {
        self.modules
            .iter()
            .find(|(contents, _)| contents.index() == module && contents.source().is_some())
            .and_then(|(_, regions)| regions.as_ref())
            .map(|regions| &regions.map)
    }

    fn name(&self, target: ModuleSymbolTarget) -> Option<&[u8]> {
        let module = shared::target_module(target)?;
        self.modules
            .iter()
            .find(|(contents, _)| contents.index() == module)
            .and_then(|(contents, _)| contents.target_name(target))
    }
}

/// Return the source module stream of `module`.
fn source_stream(module: &ModuleContents) -> Option<StreamId> {
    module
        .source()
        .and_then(|source| source.stream.as_ref())
        .map(crate::native::dbi::ModuleStream::stream)
}

/// Return `true` if the DBI stream must change for the modules or the
/// debug streams.
pub(in super::super) fn needs_dbi<S: ReadAt>(work: &Work<'_, S>) -> bool {
    !work.updates.records.is_empty()
        || work.updates.files
        || !work.updates.debug.is_empty()
        || !work.updates.removed_debug.is_empty()
        || work.editor.plan_dbi().is_some()
}

/// Run the opaque check of moved records.
fn check_opaque<S: ReadAt>(work: &mut Work<'_, S>, space: IndexSpace) -> Result<()> {
    let pdb = work.pdb;
    let context = work.context()?;
    opaque::check(pdb, context, Scope::Records).map_err(|error| {
        if error.kind() == ErrorKind::UnsupportedEdit {
            error.with_index_space(space)
        } else {
            error
        }
    })?;
    Ok(())
}

/// Encode the regions of the decoded modules and check their symbols.
fn encode_modules<'e, S: ReadAt>(work: &mut Work<'e, S>) -> Result<Encoded<'e>> {
    let mut encoded = Encoded {
        modules: Vec::new(),
    };
    let Some(slots) = work.editor.plan_modules() else {
        return Ok(encoded);
    };
    let count = slots.len();
    let context = ValidationContext::new(work.type_end()?, work.id_end()?).with_module_count(count);
    let mut moved = false;
    for slot in slots {
        let ModuleSlot::Decoded(module) = slot else {
            continue;
        };
        let regions = if module.has_stream() {
            let regions = encode::encode_regions(module, &|id| Ok(id))?;
            moved |= regions.map.moved(regions.source_count);
            let source = module
                .source()
                .and_then(|source| source.stream.as_ref())
                .map(crate::native::dbi::ModuleStream::symbol_data);
            if let Some(symbols) = &regions.symbols
                && source != Some(symbols.as_slice())
                && ModuleTypeReference::find(symbols)?.is_none()
            {
                let stream = source_stream(module).unwrap_or(StreamId(0));
                checks::check(&validate_module_symbols(
                    symbols,
                    stream,
                    ModuleSymbolOffset(SYMBOL_BASE),
                    &context,
                )?)?;
            }
            Some(regions)
        } else {
            None
        };
        push(&mut encoded.modules, (&**module, regions))?;
    }
    if moved {
        check_opaque(work, IndexSpace::ModuleSymbolOffset)?;
    }
    Ok(encoded)
}

/// Plan the modules, the symbol record stream and its indexes, and the
/// debug streams.
pub(in super::super) fn plan<S: ReadAt>(work: &mut Work<'_, S>) -> Result<()> {
    let encoded = encode_modules(work)?;
    let shared = plan_shared(work, &encoded)?;
    if let Some(shared) = &shared
        && (shared.moved() || shared.pairs.len() != shared.source_count)
    {
        move_global_refs(work, shared)?;
    }
    join_modules(work, encoded, shared.as_ref())?;
    plan_debug(work)
}

/// Plan the symbol record stream and its indexes. Return the encoded stream
/// if the model has it.
fn plan_shared<S: ReadAt>(
    work: &mut Work<'_, S>,
    modules: &Encoded<'_>,
) -> Result<Option<EncodedShared>> {
    let pdb = work.pdb;
    let module_moved = modules
        .modules
        .iter()
        .filter_map(|(_, regions)| regions.as_ref())
        .any(|regions| regions.map.moved(regions.source_count));
    let Some(model) = work.editor.plan_symbols() else {
        if module_moved
            && let Some(stream) = work.source_dbi()?.header().symbol_stream
            && let Some(records) = pdb.symbol_records()?
        {
            let patches = shared::patch_targets(records, modules, stream)?;
            if !patches.is_empty() {
                work.changes.set(
                    stream,
                    Bytes::SessionPatched(records, patches),
                    StreamChange::Remapped(IndexSpace::ModuleSymbolOffset),
                )?;
            }
        }
        return Ok(None);
    };
    let header = *work.source_dbi()?.header();
    let stream = header
        .symbol_stream
        .ok_or_else(|| missing("PDB has no symbol record stream"))?;
    let encoded = shared::encode_records(model, modules, Some(stream))?;
    if encoded.moved() {
        check_opaque(work, IndexSpace::SymbolOffset)?;
    }
    let module_count = work.source_dbi()?.modules().count();
    checks::check(&validate_shared_symbols(
        &encoded.bytes,
        stream,
        &ValidationContext::new(work.type_end()?, work.id_end()?).with_module_count(module_count),
    )?)?;
    let offsets = RecordOffsets::frame(&encoded.bytes, stream)?;
    let buckets = pdb.symbol_bucket_count();
    let producer = work.producer()?;
    let source = model.source.as_ref();
    let rebuilt = StreamChange::Rebuilt(RebuildReason::SymbolIndexes);
    if encoded.moved()
        || shared::index_changed(
            model,
            &model.globals,
            source.map(|source| &source.globals[..]),
        )
    {
        let id = header
            .global_stream
            .ok_or_else(|| missing("PDB has no global symbol index"))?;
        let bytes = shared::encode_globals(model, &encoded, buckets, producer)?;
        let parsed = GlobalSymbolIndex::parse(bytes, id, buckets)?;
        parsed.hash_table().check_records(&offsets)?;
        work.changes
            .set(id, Bytes::Owned(parsed.into_bytes()), rebuilt)?;
    }
    if encoded.moved() || shared::publics_changed(model) {
        let id = header
            .public_stream
            .ok_or_else(|| missing("PDB has no public symbol index"))?;
        let bytes = shared::encode_publics(model, &encoded, buckets, producer)?;
        let parsed = PublicSymbolIndex::parse(bytes, id, buckets)?;
        parsed.check_records(&offsets)?;
        work.changes
            .set(id, Bytes::Owned(parsed.into_bytes()), rebuilt)?;
    }
    work.changes.set(
        stream,
        Bytes::Owned(encoded.bytes.clone()),
        StreamChange::Edited,
    )?;
    Ok(Some(encoded))
}

/// Write the new offsets into the global references of each module stream
/// that the model did not decode. A reference to a removed record goes away.
///
/// The plan reads only the global reference region of each module stream.
/// A new offset becomes a patch over the source stream (#48). A removed
/// reference makes the stream shorter, so that stream gets new bytes.
fn move_global_refs<S: ReadAt>(work: &mut Work<'_, S>, shared: &EncodedShared) -> Result<()> {
    let pdb = work.pdb;
    let decoded = |index: u32| {
        work.editor
            .plan_modules()
            .is_some_and(|slots| matches!(slots.get(index as usize), Some(ModuleSlot::Decoded(_))))
    };
    let mut changed = Vec::new();
    for module in work.source_dbi()?.modules() {
        let (index, module) = module?;
        if decoded(index.0) {
            continue;
        }
        let Some(id) = module.stream else {
            continue;
        };
        let reader = pdb
            .container()
            .stream(id)?
            .ok_or_else(|| missing("PDB has no stream for a module"))?;
        let region = read_global_refs(&reader, &module)?;
        let mut patches = Vec::new();
        let mut removed = false;
        for (position, &offset) in region.offsets.iter().enumerate() {
            match shared.get(offset) {
                Some(new) if new != offset => {
                    push(&mut patches, Patch::new(region.start + position * 4, new.0))?;
                }
                Some(_) => {}
                None => removed = true,
            }
        }
        let bytes = if removed {
            Bytes::Owned(shorten_global_refs(work, &module, &region, shared)?)
        } else if patches.is_empty() {
            continue;
        } else {
            Bytes::SourcePatched(patches)
        };
        push(&mut changed, (id, bytes))?;
    }
    for (id, bytes) in changed {
        work.changes
            .set(id, bytes, StreamChange::Remapped(IndexSpace::SymbolOffset))?;
    }
    Ok(())
}

/// Return the bytes of the module stream of `module` without the global
/// references to removed records and with the new offsets of the others.
fn shorten_global_refs<S: ReadAt>(
    work: &Work<'_, S>,
    module: &Module<'_>,
    region: &GlobalRefRegion,
    shared: &EncodedShared,
) -> Result<Vec<u8>> {
    let stream = work
        .pdb
        .load_module(module)?
        .ok_or_else(|| missing("PDB has no stream for a module"))?;
    let mut moved = Vec::new();
    moved
        .try_reserve_exact(region.offsets.len())
        .map_err(|_| crate::native::output::no_memory())?;
    moved.extend(
        region
            .offsets
            .iter()
            .filter_map(|&offset| shared.get(offset)),
    );
    let mut bytes = stream.into_bytes();
    bytes.truncate(region.start - 4);
    let size = u32::try_from(moved.len() * 4).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("global references do not fit in 32 bits")
    })?;
    bytes.extend_from_slice(&size.to_le_bytes());
    for offset in &moved {
        bytes.extend_from_slice(&offset.0.to_le_bytes());
    }
    Ok(bytes)
}

/// Join the decoded modules with their global references and store the
/// module streams and the module records.
fn join_modules<S: ReadAt>(
    work: &mut Work<'_, S>,
    encoded: Encoded<'_>,
    shared: Option<&EncodedShared>,
) -> Result<()> {
    for (module, regions) in encoded.modules {
        let source = module.source();
        let record = match regions {
            None => encode::streamless(module)?,
            Some(regions) => {
                let refs = match shared {
                    Some(shared) => global_refs(module, shared)?,
                    None => source_refs(module)?,
                };
                let id = match source_stream(module) {
                    Some(id) => id,
                    None => work.changes.add(None)?,
                };
                let (mut record, bytes) = encode::join(module, &regions, &refs, id)?;
                if let Some(source) = source
                    && !module.files_changed()
                {
                    record.source_file_count = source.record.source_file_count;
                }
                work.changes
                    .set(id, Bytes::Owned(bytes), StreamChange::Edited)?;
                record
            }
        };
        work.updates.files |= module.files_changed();
        if source.is_none_or(|source| source.record != record) {
            push(&mut work.updates.records, (module.index().0, record))?;
        }
    }
    Ok(())
}

/// Return the global references of a module whose shared records the model
/// did not decode.
fn source_refs(module: &ModuleContents) -> Result<Vec<SymbolOffset>> {
    let mut refs = Vec::new();
    for reference in module.global_refs() {
        match reference {
            crate::native::editor::module::GlobalRef::Offset(offset) => push(&mut refs, *offset)?,
            crate::native::editor::module::GlobalRef::Handle(_) => {
                return Err(Error::new(ErrorKind::InvalidReference, None)
                    .with_context("global reference has no symbol record"));
            }
        }
    }
    Ok(refs)
}

/// Plan the optional debug streams. A source slot whose kind the list does
/// not have any more becomes a nil stream, and its DBI slot becomes empty.
fn plan_debug<S: ReadAt>(work: &mut Work<'_, S>) -> Result<()> {
    let Some(list) = work.editor.plan_debug() else {
        return Ok(());
    };
    let slots: Vec<Option<StreamId>> = work.source_dbi()?.debug_streams().collect();
    for (slot, id) in slots.iter().enumerate() {
        if let (Some(id), Some(kind)) = (id, kind_of(slot))
            && !list.contains(kind)
        {
            work.changes.remove(*id)?;
            push(&mut work.updates.removed_debug, slot)?;
        }
    }
    for (kind, content) in &list.entries {
        let DebugContent::Bytes(bytes) = content else {
            continue;
        };
        let slot = kind.index();
        if let Some(id) = slots.get(slot).copied().flatten() {
            work.changes
                .set(id, Bytes::Borrowed(bytes), StreamChange::Edited)?;
        } else {
            let id = work.changes.add(Some(Bytes::Borrowed(bytes)))?;
            push(&mut work.updates.debug, (slot, id))?;
        }
    }
    Ok(())
}

/// Plan the DBI stream: the module records, the source info, the debug
/// header slots and the age.
pub(in super::super) fn plan_dbi<S: ReadAt>(work: &mut Work<'_, S>) -> Result<()> {
    if work.dbi.is_none() && !needs_dbi(work) {
        return Ok(());
    }
    let reason = if work.editor.plan_dbi().is_some() {
        StreamChange::Edited
    } else if !work.updates.records.is_empty() || work.updates.files {
        StreamChange::Rebuilt(RebuildReason::ModuleRecords)
    } else if !work.updates.debug.is_empty() || !work.updates.removed_debug.is_empty() {
        StreamChange::Rebuilt(RebuildReason::DebugHeader)
    } else {
        StreamChange::Rebuilt(RebuildReason::Identity)
    };
    let files = if work.updates.files {
        Some(file_lists(work)?)
    } else {
        None
    };
    let updates = core::mem::take(&mut work.updates);
    let builder = work.dbi(reason)?;
    for (index, record) in updates.records {
        match builder.modules.get_mut(index as usize) {
            Some(slot) => *slot = record,
            None => push(&mut builder.modules, record)?,
        }
    }
    if let Some(files) = files {
        builder.source_info = Some(SourceInfo::from_file_names(
            files.iter().map(|list| list.iter().map(Vec::as_slice)),
        )?);
    }
    for (slot, id) in updates.debug {
        if builder.debug_streams.len() <= slot {
            builder.debug_streams.resize(slot + 1, None);
        }
        builder.debug_streams[slot] = Some(id);
    }
    for slot in updates.removed_debug {
        if let Some(entry) = builder.debug_streams.get_mut(slot) {
            *entry = None;
        }
    }
    let debug_slots = work
        .editor
        .plan_dbi()
        .map_or(0, |values| values.debug_slots);
    let (builder, reason) = work.dbi.as_mut().ok_or_else(|| missing("DBI"))?;
    if builder.debug_streams.len() < debug_slots {
        builder.debug_streams.resize(debug_slots, None);
    }
    let bytes = builder.to_bytes()?;
    let reason = *reason;
    work.changes.set(StreamId(3), Bytes::Owned(bytes), reason)
}

/// Return the source file names of each module of the output.
fn file_lists<S: ReadAt>(work: &Work<'_, S>) -> Result<Vec<Vec<Vec<u8>>>> {
    let dbi = work.source_dbi()?;
    let info = dbi.source_info()?;
    let slots = work.editor.plan_modules().unwrap_or_default();
    let mut lists = Vec::new();
    for (index, slot) in slots.iter().enumerate() {
        let mut list = Vec::new();
        match slot {
            ModuleSlot::Decoded(module) => {
                for name in module.files() {
                    push(&mut list, name.as_bytes().to_vec())?;
                }
            }
            ModuleSlot::Source => {
                if let Some(info) = &info
                    && let Some(sources) = info.modules.get(index)
                {
                    for name in info.file_names(sources) {
                        push(&mut list, name?.to_vec())?;
                    }
                }
            }
        }
        push(&mut lists, list)?;
    }
    Ok(lists)
}
