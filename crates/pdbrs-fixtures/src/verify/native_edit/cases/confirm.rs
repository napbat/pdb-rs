//! The read of the output of a change and removal case with `pdbrs`.
//!
//! [`confirm`] opens the output PDB and reads the place of the [`Target`]. It finds the new
//! value there, or no record for a removal.

use pdbrs::native::codeview::SymbolKind;
use pdbrs::native::codeview::field_list::Member;
use pdbrs::native::codeview::index::{ModuleIndex, TypeIndex};
use pdbrs::native::codeview::numeric::Integer;
use pdbrs::native::codeview::symbol::SymbolData;
use pdbrs::native::codeview::type_record::TypeLeaf;
use pdbrs::native::editor::SymbolRecord;
use pdbrs::native::lines::LineSubsection;
use pdbrs::native::{IdentityPolicy, Pdb, PdbEditor};

use super::change::{Target, preorder, type_of};

/// Return the text of a name.
fn text(name: &[u8]) -> String {
    String::from_utf8_lossy(name).into_owned()
}

/// Return the error text of a read error.
fn failed(what: &str, error: &pdbrs::Error) -> String {
    format!("pdbrs cannot read {what} of the output: {error}")
}

/// Return a copy of the symbol record of `name` in the publics (`public`) or the globals of
/// `editor`.
fn symbol(
    editor: &mut PdbEditor<'_, &[u8]>,
    name: &[u8],
    public: bool,
) -> Result<Option<SymbolRecord>, String> {
    let what = if public { "the publics" } else { "the globals" };
    let fail = |error: pdbrs::Error| failed(what, &error);
    Ok(if public {
        let publics = editor.publics_mut().map_err(fail)?;
        let handle = publics.find(name).map_err(fail)?;
        handle.and_then(|handle| publics.get(handle).cloned())
    } else {
        let globals = editor.globals_mut().map_err(fail)?;
        let handle = globals.find(name).map_err(fail)?;
        handle.and_then(|handle| globals.get(handle).cloned())
    })
}

/// Return the records of a module of the output in pre-order, as kinds and type indices.
fn module_records(
    editor: &mut PdbEditor<'_, &[u8]>,
    module: ModuleIndex,
) -> Result<Vec<(SymbolKind, Option<TypeIndex>)>, String> {
    let what = format!("module {}", module.0);
    let contents = editor
        .module_mut(module)
        .map_err(|error| failed(&what, &error))?;
    let nodes = contents.symbols().map_err(|error| failed(&what, &error))?;
    let mut records = Vec::new();
    preorder(nodes, &mut records);
    Ok(records
        .iter()
        .map(|record| (record.kind(), type_of(record.data())))
        .collect())
}

/// Read the output `bytes` and check the place of `target`. Return a problem text if the
/// output does not have the new value there.
pub(super) fn confirm(bytes: &[u8], target: &Target) -> Result<(), String> {
    let pdb = Pdb::parse(bytes).map_err(|error| failed("the PDB", &error))?;
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    match target {
        Target::Public { name, address } => {
            match symbol(&mut editor, name, true)?
                .as_ref()
                .map(SymbolRecord::data)
            {
                Some(SymbolData::Public(public)) if public.address == *address => Ok(()),
                Some(SymbolData::Public(public)) => Err(format!(
                    "pdbrs reads the address {:?} of the public symbol {}, expected {address:?}",
                    public.address,
                    text(name)
                )),
                _ => Err(format!("pdbrs finds no public symbol {}", text(name))),
            }
        }
        Target::Global { name, type_index } => {
            match symbol(&mut editor, name, false)?.map(|record| type_of(record.data())) {
                Some(Some(found)) if found == *type_index => Ok(()),
                Some(found) => Err(format!(
                    "pdbrs reads the type {found:?} of the global symbol {}, expected {type_index:?}",
                    text(name)
                )),
                None => Err(format!("pdbrs finds no global symbol {}", text(name))),
            }
        }
        Target::PublicRemoved { name } => match symbol(&mut editor, name, true)? {
            None => Ok(()),
            Some(_) => Err(format!(
                "pdbrs still finds the public symbol {}",
                text(name)
            )),
        },
        Target::GlobalRemoved { name } => match symbol(&mut editor, name, false)? {
            None => Ok(()),
            Some(_) => Err(format!(
                "pdbrs still finds the global symbol {}",
                text(name)
            )),
        },
        Target::ModuleSymbol {
            module,
            position,
            kind,
            type_index,
        } => {
            let records = module_records(&mut editor, *module)?;
            match records.get(*position) {
                Some(&(found, Some(found_type))) if found == *kind && found_type == *type_index => {
                    Ok(())
                }
                found => Err(format!(
                    "pdbrs reads {found:?} at record {position} of module {}, expected {kind:?} with the type {type_index:?}",
                    module.0
                )),
            }
        }
        Target::ModuleSymbolRemoved { module, kinds } => {
            let records = module_records(&mut editor, *module)?;
            let found: Vec<_> = records.iter().map(|&(kind, _)| kind).collect();
            if found == *kinds {
                Ok(())
            } else {
                Err(format!(
                    "pdbrs reads {} records in module {}, expected {} records without the removed record",
                    found.len(),
                    module.0,
                    kinds.len()
                ))
            }
        }
        Target::Line {
            module,
            subsection,
            block,
            entry,
            line,
        } => {
            let what = format!("module {}", module.0);
            let contents = editor
                .module_mut(*module)
                .map_err(|error| failed(&what, &error))?;
            let found = match contents.lines().get(*subsection) {
                Some(LineSubsection::Lines(data)) => data
                    .blocks()
                    .nth(*block)
                    .and_then(|found| found.lines.get(*entry))
                    .map(|found| found.line_start),
                _ => None,
            };
            if found == Some(*line) {
                Ok(())
            } else {
                Err(format!(
                    "pdbrs reads the line {found:?} in module {}, subsection {subsection}, block {block}, entry {entry}, expected {line}",
                    module.0
                ))
            }
        }
        Target::Member {
            field_list,
            member,
            offset,
        } => {
            let types = editor
                .types_mut()
                .map_err(|error| failed("the TPI stream", &error))?;
            let found = match types.get(*field_list) {
                Some(TypeLeaf::FieldList(list)) => match list.members().nth(*member) {
                    Some(Ok(Member::DataMember(data))) => {
                        data.offset.integer().and_then(Integer::to_u64)
                    }
                    _ => None,
                },
                _ => None,
            };
            if found == Some(*offset) {
                Ok(())
            } else {
                Err(format!(
                    "pdbrs reads the offset {found:?} of member {member} of the field list {:#x}, expected {offset}",
                    field_list.0
                ))
            }
        }
    }
}
