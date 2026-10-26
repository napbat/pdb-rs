//! The qualified names of the inlinees of `S_INLINESITE` records.
//!
//! In a PDB with merged types, the inlinee IDs refer to the IPI stream. In a PDB without
//! type merging (`NOTM`, `link /DEBUG:LAZY`), the IPI stream is empty: the inlinee IDs of a
//! module refer to the ID records that its `S_MOD_TYPEREF` record names.

use alloc::vec::Vec;

use super::{InlineeName, push};
use crate::io::ReadAt;
use crate::native::Pdb;
use crate::native::codeview::id_record::{IdRecord, IdTable};
use crate::native::codeview::index::IdIndex;
use crate::native::codeview::symbol::ModuleTypeSource;
use crate::native::codeview::type_record::TypeLeaf;
use crate::native::types::{TypeStream, TypeTable};
use crate::{Error, ErrorKind, Result};

/// The ID scope of the inlinees of a PDB with merged types: the IPI stream.
pub(super) const MERGED_IDS: u32 = u32::MAX;

/// The error context for an index that cannot be allocated.
const LIMIT: &str = "symbol locator inlinee names cannot be allocated";

/// Resolve the qualified names of the inlinees in `inlinees`.
///
/// Each entry is an ID scope and an inlinee. The scope is [`MERGED_IDS`] for the IPI
/// stream, or the index of the module whose ID records hold the inlinee. `sources` gives
/// the `S_MOD_TYPEREF` source of each module that has one. The result is sorted by scope
/// and ID.
pub(super) fn inlinee_names<S: ReadAt>(
    pdb: &Pdb<S>,
    inlinees: &mut Vec<(u32, IdIndex)>,
    sources: &[(u32, ModuleTypeSource)],
) -> Result<Vec<InlineeName>> {
    inlinees.sort_unstable();
    inlinees.dedup();
    let mut names = Vec::new();
    let mut rest = inlinees.as_slice();
    while let Some(&(scope, _)) = rest.first() {
        let count = rest.iter().take_while(|entry| entry.0 == scope).count();
        let (group, tail) = rest.split_at(count);
        rest = tail;
        let (ids, types) = if scope == MERGED_IDS {
            let ids = pdb.ids()?.ok_or_else(|| missing("IPI"))?;
            (ids, pdb.types()?)
        } else {
            module_records(pdb, sources, scope)?
        };
        let id_table = IdTable::new(&ids)?;
        let type_table = match &types {
            Some(types) => Some(TypeTable::new(types)?),
            // `TypeRecords` sources hold the type and the ID records in one stream.
            None if scope != MERGED_IDS => Some(TypeTable::new(&ids)?),
            None => None,
        };
        for &(_, id) in group {
            let name = inlinee_name(&id_table, type_table.as_ref(), scope, id)?;
            push(&mut names, name, LIMIT)?;
        }
    }
    Ok(names)
}

/// Load the ID records of the module `module` of a `NOTM` PDB, and its type records if they
/// are in another stream.
fn module_records<S: ReadAt>(
    pdb: &Pdb<S>,
    sources: &[(u32, ModuleTypeSource)],
    module: u32,
) -> Result<(TypeStream, Option<TypeStream>)> {
    let source_of = |module: u32| {
        sources
            .iter()
            .find(|entry| entry.0 == module)
            .map(|entry| entry.1)
    };
    let mut source = source_of(module);
    if let Some(ModuleTypeSource::OtherModule { module: owner }) = source {
        source = source_of(u32::from(owner));
    }
    match source {
        Some(ModuleTypeSource::TypeRecords { stream }) => {
            Ok((TypeStream::from_records(&pdb.stream_bytes(stream)?)?, None))
        }
        Some(ModuleTypeSource::TypeServer { types, ids }) => Ok((
            TypeStream::from_records(&pdb.stream_bytes(ids)?)?,
            Some(TypeStream::from_records(&pdb.stream_bytes(types)?)?),
        )),
        _ => Err(Error::new(ErrorKind::InvalidReference, None).with_context(
            "inline site in a module without type merging names no ID records of the module",
        )),
    }
}

/// Resolve the qualified name of one inlinee.
fn inlinee_name(
    table: &IdTable<'_>,
    types: Option<&TypeTable<'_>>,
    scope: u32,
    id: IdIndex,
) -> Result<InlineeName> {
    let mut qualified = Vec::new();
    let name = match table.decode(id)? {
        IdRecord::Function(function) => {
            if let Some(scope) = function.scope {
                table.resolve_string(scope, &mut qualified)?;
                extend(&mut qualified, b"::")?;
            }
            function.name
        }
        IdRecord::MemberFunction(function) => {
            let types = types.ok_or_else(|| missing("TPI"))?;
            let class = match types.leaf(function.parent_type)? {
                TypeLeaf::Class(value) => value.name,
                TypeLeaf::Union(value) => value.name,
                TypeLeaf::Enum(value) => value.name,
                _ => {
                    return Err(Error::new(ErrorKind::InvalidReference, None)
                        .with_context("member function ID does not name a class, union or enum"));
                }
            };
            extend(&mut qualified, class)?;
            extend(&mut qualified, b"::")?;
            function.name
        }
        _ => {
            return Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("inline site inlinee is not a function ID"));
        }
    };
    let name_start = qualified.len();
    extend(&mut qualified, name)?;
    Ok(InlineeName {
        scope,
        id,
        qualified,
        name_start,
    })
}

/// Append bytes to a vector, or return [`ErrorKind::LimitExceeded`].
fn extend(target: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    target
        .try_reserve(bytes.len())
        .map_err(|_| Error::new(ErrorKind::LimitExceeded, None).with_context(LIMIT))?;
    target.extend_from_slice(bytes);
    Ok(())
}

/// Create an error for a missing stream.
fn missing(stream: &'static str) -> Error {
    Error::new(ErrorKind::MissingStream, None).with_context(stream)
}
