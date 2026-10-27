//! Read the parts of a path remap: a copy of each part that the model has,
//! or the decoded source part.

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::{HeaderBlock, NamedBytes, NewFpo, Parts, TypeRecordStream, push};
use crate::io::ReadAt;
use crate::native::auxiliary::NamedStreamKind;
use crate::native::codeview::symbol::SymbolData;
use crate::native::dbi::DebugStream;
use crate::native::editor::module::{ModuleContents, Symbols};
use crate::native::editor::named::Content;
use crate::native::editor::streams::DebugContent;
use crate::native::editor::{
    DebugInfo, IdList, ModuleSlot, NamedStreams, Names, PdbEditor, TypeList, missing, source, tree,
};
use crate::native::injected_source::SourceHeaderBlock;
use crate::native::msf::StreamId;
use crate::native::names::StringTableBuilder;
use crate::native::types::HashAdjuster;
use crate::native::{Pdb, Producer};
use crate::{Bytes, Result};

/// Read the parts of the model that hold paths or `/names` offsets.
pub(super) fn load<S: ReadAt>(editor: &PdbEditor<'_, S>) -> Result<Parts> {
    let source = editor.source;
    let named = match &editor.named {
        Some(named) => named.clone(),
        None => NamedStreams::from_info(editor.cow_source()?.info())?,
    };
    let slots = modules(editor)?;
    let ids = match (&editor.ids, source) {
        (Some(ids), _) => Some(ids.clone()),
        (None, Some(pdb)) if pdb.id_offsets()?.is_some() => {
            Some(IdList::from_list(source::id_list(pdb)?))
        }
        _ => None,
    };
    let string_table = match (&editor.names, source) {
        (Some(Names::Table(table)), _) => Some(table.clone()),
        (Some(Names::Absent), _) | (None, None) => None,
        (None, Some(pdb)) => pdb
            .names()?
            .map(|table| StringTableBuilder::from_table(&table))
            .transpose()?,
    };
    let dbi = match (&editor.dbi, source) {
        (Some(dbi), _) => Some(dbi.clone()),
        (None, Some(pdb)) => pdb.debug_info()?.map(DebugInfo::decode).transpose()?,
        (None, None) => None,
    };
    let type_adjusters = match (&editor.types, source) {
        (Some(types), _) => types.list.adjusters.clone(),
        (None, Some(pdb)) => source_adjusters(pdb)?,
        (None, None) => Vec::new(),
    };
    let producer = match source {
        Some(pdb) => pdb
            .type_offsets()?
            .map_or(Producer::Microsoft, |offsets| offsets.header().producer()),
        None => Producer::Microsoft,
    };
    let header_block = header_block(source, &named, producer)?;
    let virtual_names = virtual_names(header_block.as_ref(), string_table.as_ref())?;
    let module_types = module_types(editor, &slots)?;
    let source_server = named_bytes(source, &named, &NamedStreamKind::SourceServer)?;
    let link_info = named_bytes(source, &named, &NamedStreamKind::LinkInfo)?;
    Ok(Parts {
        fpo: new_fpo(editor)?,
        named,
        slots,
        ids,
        types: None,
        type_adjusters,
        names: string_table,
        dbi,
        header_block,
        module_types,
        virtual_names,
        source_server,
        link_info,
    })
}

/// Load the TPI records if the hash adjusters can change.
pub(super) fn load_types<S: ReadAt>(editor: &PdbEditor<'_, S>, parts: &mut Parts) -> Result<()> {
    if parts.type_adjusters.is_empty() || parts.types.is_some() {
        return Ok(());
    }
    parts.types = Some(match &editor.types {
        Some(types) => types.clone(),
        None => TypeList::from_list(source::type_list(editor.cow_source()?)?),
    });
    Ok(())
}

/// Return the hash adjusters of the source TPI stream. The function reads
/// the TPI stream only if its header has hash adjusters.
fn source_adjusters<S: ReadAt>(pdb: &Pdb<S>) -> Result<Vec<HashAdjuster>> {
    let has_adjusters = pdb
        .type_offsets()?
        .is_some_and(|offsets| offsets.header().hash_adjustments.length != 0);
    if !has_adjusters {
        return Ok(Vec::new());
    }
    match pdb.types()? {
        Some(stream) => source::adjusters(pdb, &stream),
        None => Ok(Vec::new()),
    }
}

/// Return every module of the model, decoded.
fn modules<S: ReadAt>(editor: &PdbEditor<'_, S>) -> Result<Vec<ModuleSlot>> {
    let mut slots = match &editor.modules {
        Some(slots) => slots.clone(),
        None => source::module_slots(editor.cow_source()?)?,
    };
    if !slots.iter().any(|slot| matches!(slot, ModuleSlot::Source)) {
        return Ok(slots);
    }
    let pdb = editor.cow_source()?;
    let dbi = pdb
        .debug_info()?
        .ok_or_else(|| missing("PDB has no DBI stream"))?;
    let info = dbi.source_info()?;
    for (position, module) in dbi.modules().enumerate() {
        let (index, record) = module?;
        let Some(slot) = slots.get_mut(position) else {
            break;
        };
        if !matches!(slot, ModuleSlot::Source) {
            continue;
        }
        let mut files = Vec::new();
        if let Some(info) = &info
            && let Some(list) = info.modules.get(position)
        {
            for name in info.file_names(list) {
                push(&mut files, Bytes::from(name?).into_owned())?;
            }
        }
        let stream = pdb.load_module(&record)?;
        *slot = ModuleSlot::Decoded(Box::new(ModuleContents::decode(
            index,
            record.into_owned(),
            files,
            stream,
        )?));
    }
    Ok(slots)
}

/// Return the new FPO stream of the model.
fn new_fpo<S: ReadAt>(editor: &PdbEditor<'_, S>) -> Result<Option<NewFpo>> {
    let source = editor.source;
    let content = match &editor.debug {
        Some(list) => list
            .entries
            .iter()
            .find(|(kind, _)| *kind == DebugStream::NewFpo)
            .map(|(_, content)| content.clone()),
        None => match source {
            Some(pdb) => pdb
                .debug_info()?
                .and_then(|dbi| dbi.debug_stream(DebugStream::NewFpo))
                .map(DebugContent::Source),
            None => None,
        },
    };
    let (stream, bytes) = match content {
        None => return Ok(None),
        Some(DebugContent::Source(id)) => {
            let pdb = editor.cow_source()?;
            (id, pdb.stream_bytes(id)?)
        }
        Some(DebugContent::Bytes(bytes)) => {
            let slot = match source {
                Some(pdb) => pdb
                    .debug_info()?
                    .and_then(|dbi| dbi.debug_stream(DebugStream::NewFpo)),
                None => None,
            };
            (slot.unwrap_or_default(), bytes)
        }
    };
    Ok(Some(NewFpo {
        stream,
        bytes,
        changed: false,
    }))
}

/// Return the first named stream of the kind `kind`, if it has bytes.
fn named_bytes<S: ReadAt>(
    source: Option<&Pdb<S>>,
    named: &NamedStreams,
    kind: &NamedStreamKind<'_>,
) -> Result<Option<NamedBytes>> {
    let Some((entry, found)) = named.entries.iter().enumerate().find(|(_, entry)| {
        entry.content != Content::Removed && NamedStreamKind::classify(&entry.name) == *kind
    }) else {
        return Ok(None);
    };
    let stream = found.source.unwrap_or_default();
    let bytes = match &found.content {
        Content::Removed => return Ok(None),
        Content::Bytes(bytes) => bytes.clone(),
        Content::Source => source
            .ok_or_else(|| missing("the editor has no source for a named stream"))?
            .stream_bytes(stream)?,
    };
    if bytes.is_empty() {
        return Ok(None);
    }
    Ok(Some(NamedBytes {
        entry,
        stream,
        bytes,
    }))
}

/// Return the `/src/headerblock` stream of the named streams, if it has
/// bytes.
fn header_block<S: ReadAt>(
    source: Option<&Pdb<S>>,
    named: &NamedStreams,
    producer: Producer,
) -> Result<Option<HeaderBlock>> {
    Ok(
        named_bytes(source, named, &NamedStreamKind::SourceHeaderBlock)?.map(|found| HeaderBlock {
            entry: found.entry,
            stream: found.stream,
            bytes: found.bytes,
            producer,
            changed: false,
        }),
    )
}

/// Return the virtual file names of the injected source files.
fn virtual_names(
    block: Option<&HeaderBlock>,
    names: Option<&StringTableBuilder>,
) -> Result<Vec<Vec<u8>>> {
    let mut list = Vec::new();
    let (Some(block), Some(names)) = (block, names) else {
        return Ok(list);
    };
    let Some(decoded) = SourceHeaderBlock::parse(&block.bytes, block.stream)? else {
        return Ok(list);
    };
    for entry in decoded.entries() {
        if let Some(name) = names.get(entry.value.virtual_file_name_offset) {
            push(&mut list, name.to_vec())?;
        }
    }
    Ok(list)
}

/// Return the streams that the `S_MOD_TYPEREF` records of the modules name.
fn module_types<S: ReadAt>(
    editor: &PdbEditor<'_, S>,
    slots: &[ModuleSlot],
) -> Result<Vec<TypeRecordStream>> {
    let mut ids: Vec<StreamId> = Vec::new();
    for slot in slots {
        let ModuleSlot::Decoded(module) = slot else {
            continue;
        };
        let Symbols::Tree(nodes) = module.symbol_region() else {
            continue;
        };
        tree::walk(nodes, &mut |record| {
            if let SymbolData::ModuleTypeReference(reference) = record.data() {
                for id in reference.source().streams() {
                    if !ids.contains(&id) {
                        push(&mut ids, id)?;
                    }
                }
            }
            Ok(())
        })?;
    }
    let mut streams = Vec::new();
    for id in ids {
        let model = editor.unnamed.as_ref().and_then(|unnamed| {
            unnamed
                .entries
                .iter()
                .map(|entry| (entry, true))
                .chain(unnamed.replaced.iter().map(|entry| (entry, false)))
                .find(|((stream, _), _)| *stream == id)
        });
        let (bytes, in_model) = match model {
            Some(((_, bytes), in_model)) => (bytes.clone(), in_model),
            None => (editor.cow_source()?.stream_bytes(id)?, false),
        };
        push(
            &mut streams,
            TypeRecordStream {
                id,
                bytes,
                in_model,
                changed: false,
            },
        )?;
    }
    Ok(streams)
}
