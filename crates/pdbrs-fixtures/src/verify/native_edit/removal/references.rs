//! The selection of the records that the removal cases remove.
//!
//! A type candidate is a TPI record that is not a UDT, not a modifier of a UDT, and that
//! nothing refers to. An ID candidate is an `LF_UDT_SRC_LINE` or `LF_UDT_MOD_SRC_LINE` record
//! of the IPI stream that nothing refers to. The references are an over-approximation: a value
//! that is not an index only makes the sets smaller.

use std::collections::BTreeSet;

use pdbrs::native::Pdb;
use pdbrs::native::auxiliary::{NamedStreamKind, TmCache, TypeMapStream, UdtSourceLineUndone};
use pdbrs::native::codeview::LeafKind;
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::id_record::IdRecord;
use pdbrs::native::codeview::symbol::{Symbol, SymbolData};
use pdbrs::native::codeview::type_record::{
    LF_CLASS, LF_CLASS2, LF_ENUM, LF_INTERFACE, LF_INTERFACE2, LF_STRUCTURE, LF_STRUCTURE2,
    LF_UNION, LF_UNION2, TypeLeaf,
};
use pdbrs::native::lines::{LineSubsection, ModuleLines};

use super::dump::reference_tokens;

/// The UDT record kinds. The case does not remove a UDT: DIA lists the UDTs.
const UDT_KINDS: [LeafKind; 9] = [
    LF_CLASS,
    LF_STRUCTURE,
    LF_INTERFACE,
    LF_UNION,
    LF_ENUM,
    LF_CLASS2,
    LF_STRUCTURE2,
    LF_INTERFACE2,
    LF_UNION2,
];

/// Bit 31 of the local ID of a cross-scope export: the global ID is an ID index.
const FUNCTION_ID_BIT: u32 = 0x8000_0000;

/// Return the text of a `pdbrs` error.
fn read(error: &pdbrs::Error) -> String {
    format!("cannot read the source PDB: {error}")
}

/// Add the type indices of the TPI records to `references`. Return the records that are not
/// UDTs and that `pdbrs` decodes.
///
/// A modifier of a UDT is not a candidate either: DIA lists it as a UDT with a `const` or
/// `volatile` property.
fn type_records(pdb: &Pdb<&[u8]>, references: &mut BTreeSet<u32>) -> Result<Vec<u32>, String> {
    let mut found = Vec::new();
    let mut udts = BTreeSet::new();
    let Some(tpi) = pdb.types().map_err(|error| read(&error))? else {
        return Ok(Vec::new());
    };
    for record in tpi.records() {
        let record = record.map_err(|error| read(&error))?;
        // A record that pdbrs does not decode has unknown references, so it is not a candidate.
        let Ok(leaf) = TypeLeaf::decode(record.record) else {
            continue;
        };
        leaf.for_each_reference(|reference| {
            references.insert(reference.index.0);
            Ok(())
        })
        .map_err(|error| read(&error))?;
        let modified = match &leaf {
            TypeLeaf::Modifier(modifier) => Some(modifier.modified_type.0),
            TypeLeaf::ModifierEx(modifier) => Some(modifier.modified_type.0),
            _ => None,
        };
        if UDT_KINDS.contains(&leaf.kind()) {
            udts.insert(record.index);
        } else if !matches!(leaf, TypeLeaf::Unsupported(_)) {
            found.push((record.index, modified));
        }
    }
    Ok(found
        .into_iter()
        .filter(|(_, modified)| modified.is_none_or(|index| !udts.contains(&index)))
        .map(|(index, _)| index)
        .collect())
}

/// Add the type indices of the symbols that `llvm-pdbutil` does not print (the HLSL data
/// symbols) to `references`. Read the symbol record stream and the module streams.
fn symbols(pdb: &Pdb<&[u8]>, references: &mut BTreeSet<u32>) -> Result<(), String> {
    let mut add = |records: Records<'_>| -> Result<(), String> {
        for record in records {
            let record = record.map_err(|error| read(&error))?;
            let Ok(symbol) = Symbol::decode(record) else {
                continue;
            };
            match symbol.data() {
                SymbolData::HlslData(data) => references.insert(data.type_index.0),
                SymbolData::HlslDataEx(data) => references.insert(data.type_index.0),
                SymbolData::DpcGroupSharedLocal(local) => references.insert(local.type_index.0),
                _ => false,
            };
        }
        Ok(())
    };
    let globals = pdb
        .symbol_records()
        .map_err(|error| read(&error))?
        .unwrap_or_default();
    add(Records::new(globals))?;
    let Some(dbi) = pdb.debug_info().map_err(|error| read(&error))? else {
        return Ok(());
    };
    for module in dbi.modules() {
        let (index, _) = module.map_err(|error| read(&error))?;
        if let Some(stream) = pdb.module(index).map_err(|error| read(&error))? {
            add(stream.symbols())?;
        }
    }
    Ok(())
}

/// The references of a PDB in both index spaces.
#[derive(Debug, Default)]
struct References {
    /// The type indices.
    types: BTreeSet<u32>,
    /// The ID indices.
    ids: BTreeSet<u32>,
}

/// Add the fields of the IPI records to `references`. Return the `LF_UDT_SRC_LINE` and
/// `LF_UDT_MOD_SRC_LINE` records.
fn id_records(pdb: &Pdb<&[u8]>, references: &mut References) -> Result<Vec<u32>, String> {
    let mut found = Vec::new();
    let Some(ipi) = pdb.ids().map_err(|error| read(&error))? else {
        return Ok(found);
    };
    let (types, ids) = (&mut references.types, &mut references.ids);
    for record in ipi.records() {
        let record = record.map_err(|error| read(&error))?;
        let Ok(decoded) = IdRecord::decode(record.record) else {
            continue;
        };
        match decoded {
            IdRecord::Function(function) => {
                types.insert(function.function_type.0);
                ids.extend(function.scope.map(|scope| scope.0));
            }
            IdRecord::MemberFunction(function) => {
                types.insert(function.parent_type.0);
                types.insert(function.function_type.0);
            }
            IdRecord::BuildInfo(info) => ids.extend(info.arguments.iter().map(|id| id.0)),
            IdRecord::SubstringList(list) => ids.extend(list.iter().map(|id| id.0)),
            IdRecord::String(string) => ids.extend(string.substrings.map(|id| id.0)),
            IdRecord::UdtSourceLine(line) => {
                types.insert(line.udt.0);
                ids.insert(line.source_file.0);
                found.push(record.index);
            }
            IdRecord::UdtModuleSourceLine(line) => {
                types.insert(line.udt.0);
                found.push(record.index);
            }
            _ => {}
        }
    }
    Ok(found)
}

/// Add the values of the C13 subsections that `llvm-pdbutil` does not print to
/// `references`. A value of an unknown space goes to both spaces.
fn module_subsections(pdb: &Pdb<&[u8]>, references: &mut References) -> Result<(), String> {
    let Some(dbi) = pdb.debug_info().map_err(|error| read(&error))? else {
        return Ok(());
    };
    let (types, ids) = (&mut references.types, &mut references.ids);
    for module in dbi.modules() {
        let (index, _) = module.map_err(|error| read(&error))?;
        let Some(stream) = pdb.module(index).map_err(|error| read(&error))? else {
            continue;
        };
        let Ok(lines) = ModuleLines::parse(&stream) else {
            continue;
        };
        for subsection in lines.subsections() {
            match &subsection.data {
                LineSubsection::XfgTypeHashes(hashes) => {
                    types.extend(hashes.into_iter().map(|entry| entry.function_type.0));
                }
                LineSubsection::XfgVirtualHashes(hashes) => {
                    types.extend(hashes.into_iter().map(|entry| entry.class.0));
                }
                LineSubsection::TypeTokenMap(map) | LineSubsection::FunctionTokenMap(map) => {
                    types.extend(map.entries().into_iter().map(|entry| entry.key));
                    ids.extend(map.entries().into_iter().map(|entry| entry.key));
                }
                LineSubsection::CrossScopeExports(exports) => {
                    for entry in exports {
                        if entry.local_id & FUNCTION_ID_BIT == 0 {
                            types.insert(entry.global_id);
                        } else {
                            ids.insert(entry.global_id);
                        }
                    }
                }
                LineSubsection::CrossScopeImports(imports) => {
                    for import in imports {
                        ids.extend(&import.ids);
                    }
                }
                LineSubsection::InlineeLines(lines) => {
                    ids.extend(lines.into_iter().map(|entry| entry.inlinee.0));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Add the values of the type map streams and of `/UDTSRCLINEUNDONE` to `references`.
fn named_streams(pdb: &Pdb<&[u8]>, references: &mut References) -> Result<(), String> {
    for named in pdb.info().named_streams() {
        match NamedStreamKind::classify(&named.name) {
            NamedStreamKind::TmCache => {
                let bytes = pdb
                    .stream_bytes(named.stream)
                    .map_err(|error| read(&error))?;
                let cache = TmCache::parse(&bytes, named.stream).map_err(|error| read(&error))?;
                for id in cache.streams().flatten() {
                    let bytes = pdb.stream_bytes(id).map_err(|error| read(&error))?;
                    let map = TypeMapStream::parse(&bytes, id).map_err(|error| read(&error))?;
                    references.types.extend(map.types());
                    references.ids.extend(map.ids());
                    for (first, second) in map.function_types() {
                        references.types.extend([first, second]);
                        references.ids.extend([first, second]);
                    }
                }
            }
            NamedStreamKind::UdtSourceLineUndone => {
                let bytes = pdb
                    .stream_bytes(named.stream)
                    .map_err(|error| read(&error))?;
                let list = UdtSourceLineUndone::parse(&bytes, named.stream)
                    .map_err(|error| read(&error))?;
                references
                    .types
                    .extend(list.type_indices().map(|index| index.0));
            }
            _ => {}
        }
    }
    Ok(())
}

/// The records that the cases remove, in index order.
#[derive(Debug, Default)]
pub(super) struct Candidates {
    /// The TPI records of `remove-types`.
    pub types: Vec<u32>,
    /// The IPI records of `remove-ids`.
    pub ids: Vec<u32>,
}

/// Return the records that the cases remove. `dump` is the
/// `llvm-pdbutil dump --types --ids --symbols --globals` text of the source PDB.
pub(super) fn candidates(pdb: &Pdb<&[u8]>, dump: &str) -> Result<Candidates, String> {
    let tokens = reference_tokens(dump);
    let mut references = References {
        types: tokens.all,
        ids: tokens.ids,
    };
    let types = type_records(pdb, &mut references.types)?;
    let ids = id_records(pdb, &mut references)?;
    module_subsections(pdb, &mut references)?;
    named_streams(pdb, &mut references)?;
    symbols(pdb, &mut references.types)?;
    let unreferenced = |found: Vec<u32>, set: &BTreeSet<u32>| -> Vec<u32> {
        found
            .into_iter()
            .filter(|index| !set.contains(index))
            .collect()
    };
    Ok(Candidates {
        types: unreferenced(types, &references.types),
        ids: unreferenced(ids, &references.ids),
    })
}
