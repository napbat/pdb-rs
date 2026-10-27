//! The signatures of the procedures in the module streams and of the function IDs in the IPI stream.

use pdbrs::io::ReadAt;
use pdbrs::native::Pdb;
use pdbrs::native::codeview::id_record::{IdRecord, IdTable};
use pdbrs::native::codeview::index::{IdIndex, TypeIndex};
use pdbrs::native::codeview::symbol::{
    ProcedureType, S_GPROC32, S_GPROC32_ID, S_GPROC32EX, S_GPROC32EX_ID, Symbol, SymbolData,
};
use pdbrs::native::types::TypeStream;

use crate::Outcome;
use crate::names::{Types, text};

/// Return the function type and the qualified name of an `LF_FUNC_ID` or `LF_MFUNC_ID` record.
///
/// Returns `None` for other ID records.
fn function_id(
    types: &Types<'_, '_>,
    ids: &IdTable<'_>,
    record: &IdRecord<'_>,
) -> Outcome<Option<(TypeIndex, String)>> {
    Ok(match record {
        IdRecord::Function(function) => {
            let mut name = Vec::new();
            if let Some(scope) = function.scope {
                ids.resolve_string(scope, &mut name)?;
                name.extend_from_slice(b"::");
            }
            name.extend_from_slice(&function.name);
            Some((function.function_type, text(&name)))
        }
        IdRecord::MemberFunction(function) => {
            let parent = types.name(function.parent_type)?;
            let name = format!("{parent}::{}", text(&function.name));
            Some((function.function_type, name))
        }
        _ => None,
    })
}

/// Print the signature of each procedure symbol in each module stream.
///
/// Each line has the scope (`global` or `local`), the section and offset of
/// the code, and the signature. An `_ID` procedure gets its type from its
/// `LF_FUNC_ID` or `LF_MFUNC_ID` record. A procedure without a type, for
/// example an MSVC unwind funclet `dtor$0`, gives only its name. Return the
/// number of procedures.
pub fn procedures(
    pdb: &Pdb<impl ReadAt>,
    types: &Types<'_, '_>,
    ids: Option<&IdTable<'_>>,
) -> Outcome<usize> {
    let Some(dbi) = pdb.debug_info()? else {
        return Ok(0);
    };
    let mut count = 0;
    for module in dbi.modules() {
        let Some(stream) = pdb.module_stream(&module?.1)? else {
            continue;
        };
        for record in stream.symbols() {
            let symbol = Symbol::decode(record?)?;
            let SymbolData::Procedure(procedure) = symbol.data() else {
                continue;
            };
            let scope = match symbol.kind() {
                S_GPROC32 | S_GPROC32_ID | S_GPROC32EX | S_GPROC32EX_ID => "global",
                _ => "local",
            };
            let name = text(&procedure.name);
            let function_type = match procedure.signature {
                ProcedureType::Type(index) => Some(index),
                ProcedureType::Id(IdIndex::NONE) => None,
                ProcedureType::Id(id) => {
                    let ids = ids.ok_or("an _ID procedure needs the IPI stream")?;
                    function_id(types, ids, &ids.leaf(id)?)?.map(|(index, _)| index)
                }
            };
            let signature = match function_type {
                Some(index) if index != TypeIndex::NONE => types.signature(index, &name)?,
                _ => name,
            };
            println!(
                "{scope} {:04x}:{:08x} {signature}",
                procedure.address.section, procedure.address.offset
            );
            count += 1;
        }
    }
    Ok(count)
}

/// Print the signature of each `LF_FUNC_ID` and `LF_MFUNC_ID` record. Return the number of records.
pub fn function_ids(types: &Types<'_, '_>, ipi: &TypeStream, ids: &IdTable<'_>) -> Outcome<usize> {
    let mut count = 0;
    for record in ipi.records() {
        let record = record?;
        let decoded = IdRecord::decode(record.record)?;
        if let Some((function_type, name)) = function_id(types, ids, &decoded)? {
            println!(
                "id {:#x} {}",
                record.index,
                types.signature(function_type, &name)?
            );
            count += 1;
        }
    }
    Ok(count)
}
