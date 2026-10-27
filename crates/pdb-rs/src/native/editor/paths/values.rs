//! The path fields of the modules, the IPI records, the module type streams
//! and the DBI values.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use super::table::{self, applies};
use super::{NameRoles, Parts, PathMap, TypeRecordStream, named};
use crate::native::codeview::Records;
use crate::native::codeview::id_record::{IdRecord, LF_STRING_ID};
use crate::native::codeview::index::NameOffset;
use crate::native::codeview::symbol::{StringList, SymbolData};
use crate::native::editor::module::{ModuleContents, Symbols};
use crate::native::editor::record::SymbolRecord;
use crate::native::editor::{DebugInfo, IdList, ModuleSlot, tree};
use crate::native::output::no_memory;
use crate::{Bytes, Result};

/// Add `value` to `values` if it is not empty.
fn add(values: &mut BTreeSet<Vec<u8>>, value: &[u8]) {
    if !value.is_empty() && !values.contains(value) {
        values.insert(value.to_vec());
    }
}

/// Collect the distinct values of the path fields of `parts`.
pub(super) fn collect(parts: &Parts, roles: &NameRoles) -> Result<BTreeSet<Vec<u8>>> {
    let mut values = BTreeSet::new();
    if let Some(names) = &parts.names {
        let offsets = names
            .name_offsets()
            .map(|offset| offset.0)
            .chain(roles.paths.iter().copied());
        for offset in offsets {
            if applies(roles, offset)
                && let Some(name) = names.get(NameOffset(offset))
            {
                add(&mut values, name);
            }
        }
    }
    for slot in &parts.slots {
        if let ModuleSlot::Decoded(module) = slot {
            collect_module(module, &mut values)?;
        }
    }
    if let Some(ids) = &parts.ids {
        for entry in &ids.list.entries {
            if let IdRecord::String(string) = &entry.value {
                add(&mut values, &string.string);
            }
        }
    }
    for stream in &parts.module_types {
        for record in Records::new(&stream.bytes) {
            let record = record?;
            if record.kind == LF_STRING_ID.0
                && let IdRecord::String(string) = IdRecord::decode(record)?
            {
                add(&mut values, &string.string);
            }
        }
    }
    if let Some(dbi) = &parts.dbi {
        if let Some(ec) = &dbi.dbi.ec_names {
            for offset in ec.name_offsets() {
                add(&mut values, ec.get(offset).unwrap_or_default());
            }
        }
        for server in &dbi.dbi.type_servers {
            add(&mut values, &server.name);
            add(&mut values, &server.path);
        }
    }
    named::collect(parts, &mut values)?;
    Ok(values)
}

/// Collect the path fields of one module.
fn collect_module(module: &ModuleContents, values: &mut BTreeSet<Vec<u8>>) -> Result<()> {
    for file in module.files() {
        add(values, file);
    }
    add(values, &module.record().name);
    add(values, &module.record().object_file);
    if let Symbols::Tree(nodes) = module.symbol_region() {
        tree::walk(nodes, &mut |record| {
            match record.data() {
                SymbolData::ObjectName(object) => add(values, &object.name),
                SymbolData::EnvironmentBlock(block) => {
                    for string in &block.strings {
                        add(values, string);
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
    }
    Ok(())
}

/// Return the new value of `value`, or `None` if it stays.
fn new_value<'m>(map: &'m PathMap, value: &[u8]) -> Option<&'m [u8]> {
    map.get(value).map(Vec::as_slice)
}

/// Give each path field of `parts` its new value.
pub(super) fn apply(parts: &mut Parts, map: &PathMap) -> Result<()> {
    for slot in &mut parts.slots {
        if let ModuleSlot::Decoded(module) = slot {
            apply_module(module, map)?;
        }
    }
    if let Some(ids) = &mut parts.ids {
        apply_ids(ids, map)?;
    }
    for stream in &mut parts.module_types {
        apply_type_stream(stream, map)?;
    }
    if let Some(dbi) = &mut parts.dbi {
        apply_dbi(dbi, &mut parts.slots, map)?;
    }
    Ok(())
}

/// Check that `record` still encodes: its length must fit in `u16`.
fn check_encoding(record: &SymbolRecord) -> Result<()> {
    let mut scratch = Vec::new();
    record.encode(&mut scratch)
}

/// Give the path fields of one module their new values.
fn apply_module(module: &mut ModuleContents, map: &PathMap) -> Result<()> {
    if module
        .files()
        .iter()
        .any(|file| new_value(map, file).is_some())
    {
        for file in module.files_mut() {
            if let Some(new) = new_value(map, file) {
                *file = Bytes::from(new.to_vec());
            }
        }
    }
    let record = module.record_mut();
    if let Some(new) = new_value(map, &record.name) {
        record.name = Bytes::from(new.to_vec());
    }
    if let Some(new) = new_value(map, &record.object_file) {
        record.object_file = Bytes::from(new.to_vec());
    }
    if let Symbols::Tree(nodes) = module.symbol_region_mut() {
        tree::walk_mut(nodes, &mut |record| apply_symbol(record, map))?;
    }
    Ok(())
}

/// Give the paths of an `S_OBJNAME` or `S_ENVBLOCK` record their new values.
fn apply_symbol(record: &mut SymbolRecord, map: &PathMap) -> Result<()> {
    match record.data() {
        SymbolData::ObjectName(object) => {
            let Some(new) = new_value(map, &object.name) else {
                return Ok(());
            };
            if let SymbolData::ObjectName(object) = record.data_mut() {
                object.name = Bytes::from(new.to_vec());
            }
        }
        SymbolData::EnvironmentBlock(block) => {
            if !block
                .strings
                .iter()
                .any(|string| new_value(map, string).is_some())
            {
                return Ok(());
            }
            let mut bytes = Vec::new();
            for string in &block.strings {
                let value = new_value(map, string).unwrap_or(string);
                bytes
                    .try_reserve(value.len() + 1)
                    .map_err(|_| no_memory())?;
                bytes.extend_from_slice(value);
                bytes.push(0);
            }
            let strings = StringList::parse(bytes)?;
            if let SymbolData::EnvironmentBlock(block) = record.data_mut() {
                block.strings = strings;
            }
        }
        _ => return Ok(()),
    }
    check_encoding(record)
}

/// Give the strings of the `LF_STRING_ID` records of the IPI their new
/// values.
fn apply_ids(ids: &mut IdList, map: &PathMap) -> Result<()> {
    let mut scratch = Vec::new();
    for entry in &mut ids.list.entries {
        if let IdRecord::String(string) = &mut entry.value
            && let Some(new) = new_value(map, &string.string)
        {
            string.string = Bytes::from(new.to_vec());
            entry.touched = true;
            scratch.clear();
            entry.value.encode(&mut scratch)?;
        }
    }
    Ok(())
}

/// Give the strings of the `LF_STRING_ID` records of a module type stream
/// their new values. The other records keep their bytes.
fn apply_type_stream(stream: &mut TypeRecordStream, map: &PathMap) -> Result<()> {
    let mut out = Vec::new();
    let mut changed = false;
    for record in Records::new(&stream.bytes) {
        let record = record?;
        let start = record.offset as usize;
        let full = &stream.bytes[start..start + 4 + record.data.len()];
        if record.kind == LF_STRING_ID.0
            && let IdRecord::String(mut string) = IdRecord::decode(record)?
            && let Some(new) = new_value(map, &string.string)
        {
            string.string = Bytes::from(new.to_vec());
            IdRecord::String(string).encode(&mut out)?;
            changed = true;
            continue;
        }
        out.try_reserve(full.len()).map_err(|_| no_memory())?;
        out.extend_from_slice(full);
    }
    if changed {
        stream.bytes = out;
        stream.changed = true;
    }
    Ok(())
}

/// Give the EC names and the type server names and paths their new values.
/// A changed EC name builds the EC name table again; the module records get
/// the new EC indices.
fn apply_dbi(dbi: &mut DebugInfo, slots: &mut [ModuleSlot], map: &PathMap) -> Result<()> {
    for server in &mut dbi.dbi.type_servers {
        if let Some(new) = new_value(map, &server.name) {
            server.name = new.to_vec();
        }
        if let Some(new) = new_value(map, &server.path) {
            server.path = new.to_vec();
        }
    }
    let Some(ec) = &dbi.dbi.ec_names else {
        return Ok(());
    };
    let mut roles = NameRoles::default();
    for slot in slots.iter() {
        if let ModuleSlot::Decoded(module) = slot {
            let record = module.record();
            roles.paths.insert(record.source_file_name_index);
            roles.paths.insert(record.pdb_file_path_index);
        }
    }
    let Some((table, moves, _)) = table::rebuild(ec, map, &roles)? else {
        return Ok(());
    };
    for slot in slots.iter_mut() {
        if let ModuleSlot::Decoded(module) = slot {
            let record = module.record_mut();
            for index in [
                &mut record.source_file_name_index,
                &mut record.pdb_file_path_index,
            ] {
                if let Some(&moved) = moves.get(index) {
                    *index = moved;
                }
            }
        }
    }
    dbi.dbi.ec_names = Some(table);
    Ok(())
}
