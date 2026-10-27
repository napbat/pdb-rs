//! The `/names` offset fields of the parts of a path remap.
//!
//! [`map_name_offsets`] gives each `/names` offset field to a function, with
//! the role of the field, and writes the returned offset back. A part whose
//! offsets all stay keeps its bytes. The fields:
//!
//! | Part | Field | Role |
//! | --- | --- | --- |
//! | C13 file checksums | file name | path |
//! | C13 cross-scope imports | module name | path |
//! | C13 frame data | frame program | other |
//! | `S_FILESTATIC` | module file name | path |
//! | `S_DEFRANGE`, `S_DEFRANGE_SUBFIELD` | DIA program | other |
//! | `LF_UDT_MOD_SRC_LINE` (IPI and module type streams) | file name | path |
//! | TPI and IPI hash adjusters | UDT name | other |
//! | new FPO stream | frame program | other |
//! | `/src/headerblock` entries | original, object and virtual file name | path |
//!
//! A change that builds `/names` again, for example a later removal of
//! names, can use the walk to find and rewrite every offset.

use alloc::vec::Vec;

use super::{HeaderBlock, NewFpo, Parts, TypeRecordStream, invalid, push};
use crate::Result;
use crate::native::codeview::Records;
use crate::native::codeview::id_record::{IdRecord, LF_UDT_MOD_SRC_LINE};
use crate::native::codeview::index::NameOffset;
use crate::native::codeview::symbol::SymbolData;
use crate::native::editor::module::{ModuleContents, Symbols};
use crate::native::editor::record::SymbolRecord;
use crate::native::editor::types::RecordList;
use crate::native::editor::{ModuleSlot, tree};
use crate::native::fpo::{FrameData, FrameDataStream};
use crate::native::injected_source::{InjectedSource, SourceHeaderBlock, SourceHeaderBlockBuilder};
use crate::native::lines::{CrossScopeImports, FileChecksums, FrameDataSubsection, LineSubsection};
use crate::native::types::HashAdjuster;

/// The role of a `/names` offset field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Role {
    /// The field names a path.
    Path,
    /// The field names other text: a frame program or a UDT name.
    Other,
}

/// The parts with `/names` offset fields.
pub(super) struct NameFields<'p> {
    /// The modules.
    modules: &'p mut [ModuleSlot],
    /// The IPI records.
    ids: Option<&'p mut RecordList<IdRecord<'static>>>,
    /// The TPI hash adjusters.
    type_adjusters: &'p mut [HashAdjuster],
    /// The new FPO stream.
    fpo: Option<&'p mut NewFpo>,
    /// The `/src/headerblock` stream.
    header_block: Option<&'p mut HeaderBlock>,
    /// The streams that `S_MOD_TYPEREF` records name.
    module_types: &'p mut [TypeRecordStream],
}

/// Return the `/names` offset fields of `parts`.
pub(super) fn fields(parts: &mut Parts) -> NameFields<'_> {
    NameFields {
        modules: &mut parts.slots,
        ids: parts.ids.as_mut().map(|ids| &mut ids.list),
        type_adjusters: &mut parts.type_adjusters,
        fpo: parts.fpo.as_mut(),
        header_block: parts.header_block.as_mut(),
        module_types: &mut parts.module_types,
    }
}

/// The function that gives the new offset of a field.
type Map<'m> = dyn FnMut(Role, NameOffset) -> Result<NameOffset> + 'm;

/// Give each `/names` offset field of `fields` to `map` and write the
/// returned offset into the field.
pub(super) fn map_name_offsets(fields: &mut NameFields<'_>, map: &mut Map<'_>) -> Result<()> {
    for slot in fields.modules.iter_mut() {
        if let ModuleSlot::Decoded(module) = slot {
            map_module(module, map)?;
        }
    }
    if let Some(list) = fields.ids.as_deref_mut() {
        map_ids(list, map)?;
    }
    for adjuster in fields.type_adjusters.iter_mut() {
        adjuster.name_offset = map(Role::Other, adjuster.name_offset)?;
    }
    if let Some(fpo) = fields.fpo.as_deref_mut() {
        map_fpo(fpo, map)?;
    }
    if let Some(block) = fields.header_block.as_deref_mut() {
        map_header_block(block, map)?;
    }
    for stream in fields.module_types.iter_mut() {
        map_type_stream(stream, map)?;
    }
    Ok(())
}

/// Map the offsets of one module: its C13 subsections and its symbols.
fn map_module(module: &mut ModuleContents, map: &mut Map<'_>) -> Result<()> {
    let mut changed = Vec::new();
    for (position, subsection) in module.lines().iter().enumerate() {
        if let Some(new) = map_subsection(subsection, map)? {
            push(&mut changed, (position, new))?;
        }
    }
    if !changed.is_empty() {
        let lines = module.lines_mut();
        for (position, subsection) in changed {
            lines[position] = subsection;
        }
    }
    if let Symbols::Tree(nodes) = module.symbol_region_mut() {
        tree::walk_mut(nodes, &mut |record| map_symbol(record, map))?;
    }
    Ok(())
}

/// Write `value` into the 4 bytes of `bytes` at `at`.
fn patch(bytes: &mut [u8], at: usize, value: u32) -> Result<()> {
    let field = bytes
        .get_mut(at..at + 4)
        .ok_or_else(|| invalid("a /names offset field is outside its subsection"))?;
    field.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

/// Map the offsets of a C13 subsection. Return the new subsection, or
/// `None` if every offset stays.
fn map_subsection(
    subsection: &LineSubsection<'static>,
    map: &mut Map<'_>,
) -> Result<Option<LineSubsection<'static>>> {
    match subsection {
        LineSubsection::FileChecksums(checksums) => {
            let mut bytes = Vec::new();
            checksums.encode(&mut bytes)?;
            let mut changed = false;
            for entry in checksums {
                let new = map(Role::Path, entry.name_offset)?;
                if new != entry.name_offset {
                    patch(&mut bytes, entry.offset.0 as usize, new.0)?;
                    changed = true;
                }
            }
            if !changed {
                return Ok(None);
            }
            let parsed = FileChecksums::parse(&bytes)?;
            Ok(Some(LineSubsection::FileChecksums(parsed).into_owned()))
        }
        LineSubsection::CrossScopeImports(imports) => {
            let mut bytes = Vec::new();
            imports.encode(&mut bytes)?;
            let mut changed = false;
            let mut position = 0;
            for entry in imports {
                let new = map(Role::Path, entry.module_name_offset)?;
                if new != entry.module_name_offset {
                    patch(&mut bytes, position, new.0)?;
                    changed = true;
                }
                position += 8 + 4 * entry.ids.len();
            }
            if !changed {
                return Ok(None);
            }
            let parsed = CrossScopeImports::parse(&bytes)?;
            Ok(Some(LineSubsection::CrossScopeImports(parsed).into_owned()))
        }
        LineSubsection::FrameData(frames) => {
            let mut records = Vec::new();
            let mut changed = false;
            for mut record in frames.records() {
                let new = map(Role::Other, NameOffset(record.program))?;
                changed |= new.0 != record.program;
                record.program = new.0;
                push(&mut records, record)?;
            }
            if !changed {
                return Ok(None);
            }
            let mut bytes = Vec::new();
            FrameDataSubsection::encode_parts(frames.relocation(), &records, &mut bytes)?;
            let parsed = FrameDataSubsection::parse(&bytes)?;
            Ok(Some(LineSubsection::FrameData(parsed).into_owned()))
        }
        _ => Ok(None),
    }
}

/// Map the offsets of one symbol record.
fn map_symbol(record: &mut SymbolRecord, map: &mut Map<'_>) -> Result<()> {
    match record.data() {
        SymbolData::FileStatic(value) => {
            let new = map(Role::Path, value.module_name_offset)?;
            if new != value.module_name_offset
                && let SymbolData::FileStatic(value) = record.data_mut()
            {
                value.module_name_offset = new;
            }
        }
        SymbolData::DefRangeProgram(value) => {
            let new = map(Role::Other, NameOffset(value.program))?;
            if new.0 != value.program
                && let SymbolData::DefRangeProgram(value) = record.data_mut()
            {
                value.program = new.0;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Map the offsets of the IPI records and the IPI hash adjusters.
fn map_ids(list: &mut RecordList<IdRecord<'static>>, map: &mut Map<'_>) -> Result<()> {
    for entry in &mut list.entries {
        if let IdRecord::UdtModuleSourceLine(line) = &mut entry.value {
            let new = map(Role::Path, line.name_offset)?;
            if new != line.name_offset {
                line.name_offset = new;
                entry.touched = true;
            }
        }
    }
    let mut changed = false;
    for adjuster in &mut list.adjusters {
        let new = map(Role::Other, adjuster.name_offset)?;
        changed |= new != adjuster.name_offset;
        adjuster.name_offset = new;
    }
    list.adjusters_changed |= changed;
    Ok(())
}

/// Map the frame programs of the new FPO stream.
fn map_fpo(fpo: &mut NewFpo, map: &mut Map<'_>) -> Result<()> {
    let stream = FrameDataStream::parse(fpo.bytes.clone(), fpo.stream)?;
    let mut records: Vec<FrameData> = Vec::new();
    let mut changed = false;
    for mut record in &stream {
        let new = map(Role::Other, NameOffset(record.program))?;
        changed |= new.0 != record.program;
        record.program = new.0;
        push(&mut records, record)?;
    }
    if changed {
        fpo.bytes = FrameDataStream::from_records(&records)?.to_bytes()?;
        fpo.changed = true;
    }
    Ok(())
}

/// Map the 3 offsets of each `/src/headerblock` entry. A changed key needs
/// a new table, so the entries go into a new table in the order of their
/// source keys, with the insertion rules of the producer.
fn map_header_block(block: &mut HeaderBlock, map: &mut Map<'_>) -> Result<()> {
    let Some(decoded) = SourceHeaderBlock::parse(&block.bytes, block.stream)? else {
        return Ok(());
    };
    let mut entries: Vec<(u32, InjectedSource)> = Vec::new();
    let mut changed = false;
    for entry in decoded.entries() {
        let mut source = entry.value;
        for field in [
            &mut source.file_name_offset,
            &mut source.object_name_offset,
            &mut source.virtual_file_name_offset,
        ] {
            let new = map(Role::Path, *field)?;
            changed |= new != *field;
            *field = new;
        }
        push(&mut entries, (entry.key, source))?;
    }
    if !changed {
        return Ok(());
    }
    entries.sort_by_key(|(key, _)| *key);
    let header = decoded.header();
    let mut builder = SourceHeaderBlockBuilder::new(block.producer, header.file_time, header.age)?;
    for (_, source) in entries {
        if builder.insert(source)?.is_some() {
            return Err(invalid(
                "two injected source files get the same virtual file name",
            ));
        }
    }
    let mut bytes = Vec::new();
    builder.encode(&mut bytes)?;
    block.bytes = bytes;
    block.changed = true;
    Ok(())
}

/// Map the `LF_UDT_MOD_SRC_LINE` offsets of a module type stream. The other
/// records keep their bytes.
fn map_type_stream(stream: &mut TypeRecordStream, map: &mut Map<'_>) -> Result<()> {
    let mut out = Vec::new();
    let mut changed = false;
    for record in Records::new(&stream.bytes) {
        let record = record?;
        let start = record.offset as usize;
        let full = &stream.bytes[start..start + 4 + record.data.len()];
        if record.kind == LF_UDT_MOD_SRC_LINE.0
            && let IdRecord::UdtModuleSourceLine(mut line) = IdRecord::decode(record)?
        {
            let new = map(Role::Path, line.name_offset)?;
            if new != line.name_offset {
                line.name_offset = new;
                IdRecord::UdtModuleSourceLine(line).encode(&mut out)?;
                changed = true;
                continue;
            }
        }
        out.try_reserve(full.len())
            .map_err(|_| crate::native::output::no_memory())?;
        out.extend_from_slice(full);
    }
    if changed {
        stream.bytes = out;
        stream.changed = true;
    }
    Ok(())
}
