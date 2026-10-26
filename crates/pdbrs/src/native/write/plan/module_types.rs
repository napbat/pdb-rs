//! The type records that the symbols of a module refer to.
//!
//! In a PDB without type merging (feature `NOTM`, MSVC `/DEBUG:LAZY`), the
//! `S_MOD_TYPEREF` record of a module names the streams with the type and
//! ID records of the module. The type and ID indices of the module symbols
//! refer to these records, not to TPI and IPI. The plan checks the indices
//! against the records that the record names:
//!
//! - [`ModuleTypeSource::TypeRecords`]: types and IDs share one stream.
//! - [`ModuleTypeSource::TypeServer`]: a type stream and an ID stream.
//! - [`ModuleTypeSource::OtherModule`]: the records of the named module.
//! - [`ModuleTypeSource::NoTypes`]: no records, so only primitive type
//!   indices are valid.
//! - [`ModuleTypeSource::PrecompiledTypeOwner`] and
//!   [`ModuleTypeSource::PrecompiledTypeUser`]: the precompiled types are in
//!   an object file outside the PDB and the first index of the own records
//!   is not known, so the plan does not check the type and ID index ranges.
//!   The builder still writes the stream of the own records.
//! - [`ModuleTypeSource::Other`]: the meaning of the words is not known, so
//!   the plan does not check the type and ID index ranges.
//!
//! A named stream must be a stream without a name of the builder
//! ([`PdbBuilder::add_stream`]).

use alloc::collections::BTreeMap;
use alloc::collections::btree_map;
use alloc::vec::Vec;

use super::super::PdbBuilder;
use super::super::entry::Entry;
use super::StreamIds;
use crate::native::codeview::id_record::IdTable;
use crate::native::codeview::index::{FIRST_RECORD_INDEX, IdIndex, TypeIndex};
use crate::native::codeview::symbol::validate::ValidationContext;
use crate::native::codeview::symbol::{ModuleTypeReference, ModuleTypeSource};
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::native::types::TypeStream;
use crate::{Error, ErrorKind, Result};

/// Where the type and ID indices of the symbols of one module resolve.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ModuleTypes {
    /// TPI and IPI: the module has no `S_MOD_TYPEREF` record.
    Pdb,
    /// No type records ([`ModuleTypeSource::NoTypes`]).
    Empty,
    /// The records of a type stream and an ID stream. Both can be the same
    /// stream.
    Streams {
        /// The stream with the type records.
        types: StreamId,
        /// The stream with the ID records.
        ids: StreamId,
    },
    /// Flags without a known meaning: no index range check.
    Unchecked,
}

/// Return the error for an `S_MOD_TYPEREF` record that does not resolve.
fn unresolved(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidReference, None).with_context(context)
}

/// Return where the indices of a module resolve, from the source of its
/// `S_MOD_TYPEREF` record. `other` gives the source of another module.
fn resolve(
    source: Option<ModuleTypeSource>,
    other: impl Fn(u16) -> Result<Option<ModuleTypeSource>>,
) -> Result<ModuleTypes> {
    let source = match source {
        Some(ModuleTypeSource::OtherModule { module }) => match other(module)? {
            Some(ModuleTypeSource::OtherModule { .. }) | None => {
                return Err(unresolved(
                    "S_MOD_TYPEREF names a module without its own type records",
                ));
            }
            source => source,
        },
        source => source,
    };
    Ok(match source {
        None => ModuleTypes::Pdb,
        Some(ModuleTypeSource::NoTypes) => ModuleTypes::Empty,
        Some(ModuleTypeSource::TypeRecords { stream }) => ModuleTypes::Streams {
            types: stream,
            ids: stream,
        },
        Some(ModuleTypeSource::TypeServer { types, ids }) => ModuleTypes::Streams { types, ids },
        Some(_) => ModuleTypes::Unchecked,
    })
}

/// The parsed module type streams of a plan, by stream ID.
pub(super) struct ModuleTypeStreams {
    /// The type streams.
    streams: BTreeMap<StreamId, TypeStream>,
}

impl ModuleTypeStreams {
    /// Return the validation context for the symbols of a module.
    ///
    /// `base` is the context with the TPI and IPI ends. The function calls
    /// `check` with the context of the module.
    ///
    /// # Errors
    ///
    /// Returns the errors of `check` and of [`IdTable::new`].
    pub(super) fn with_context<R>(
        &self,
        types: ModuleTypes,
        base: &ValidationContext<'_>,
        module_count: usize,
        check: impl FnOnce(&ValidationContext<'_>) -> Result<R>,
    ) -> Result<R> {
        let first = TypeIndex(FIRST_RECORD_INDEX);
        match types {
            ModuleTypes::Pdb => check(base),
            ModuleTypes::Empty => check(
                &ValidationContext::new(first, IdIndex(FIRST_RECORD_INDEX))
                    .with_module_count(module_count),
            ),
            ModuleTypes::Unchecked => check(
                &ValidationContext::new(TypeIndex(u32::MAX), IdIndex(u32::MAX))
                    .with_module_count(module_count),
            ),
            ModuleTypes::Streams { types, ids } => {
                let missing = || unresolved("module type stream was not read");
                let types = self.streams.get(&types).ok_or_else(missing)?;
                let ids = self.streams.get(&ids).ok_or_else(missing)?;
                let table = IdTable::new(ids)?;
                let id_kind = |id: IdIndex| table.get(id).map(|record| record.kind);
                check(
                    &ValidationContext::new(
                        TypeIndex(types.header().index_end),
                        IdIndex(ids.header().index_end),
                    )
                    .with_id_kinds(&id_kind)
                    .with_module_count(module_count),
                )
            }
        }
    }
}

impl PdbBuilder<'_> {
    /// Return where the indices of each module resolve, and the parsed
    /// module type streams.
    ///
    /// `symbols` holds the encoded symbol records of each module without
    /// the signature, or `None`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if an `S_MOD_TYPEREF` record names
    ///   a missing module, a module without its own type records, or a
    ///   stream that is not a stream without a name of the builder with
    ///   bytes.
    /// - The errors of the record decoders and of [`TypeStream::from_records`].
    pub(super) fn module_types(
        &self,
        ids: &StreamIds,
        symbols: &[Option<&[u8]>],
    ) -> Result<(Vec<ModuleTypes>, ModuleTypeStreams)> {
        let mut sources = Vec::new();
        sources
            .try_reserve_exact(symbols.len())
            .map_err(|_| no_memory())?;
        for records in symbols {
            let reference = match records {
                Some(records) => ModuleTypeReference::find(records)?,
                None => None,
            };
            sources.push(reference.map(|reference| reference.source()));
        }
        let mut types = Vec::new();
        types
            .try_reserve_exact(sources.len())
            .map_err(|_| no_memory())?;
        let mut streams = BTreeMap::new();
        for source in &sources {
            let resolved = resolve(*source, |module| {
                sources
                    .get(usize::from(module))
                    .copied()
                    .ok_or_else(|| unresolved("S_MOD_TYPEREF names a missing module"))
            })?;
            if let ModuleTypes::Streams {
                types: type_stream,
                ids: id_stream,
            } = resolved
            {
                for stream in [type_stream, id_stream] {
                    if let btree_map::Entry::Vacant(slot) = streams.entry(stream) {
                        slot.insert(self.type_stream_of(ids, stream)?);
                    }
                }
            }
            types.push(resolved);
        }
        Ok((types, ModuleTypeStreams { streams }))
    }

    /// Parse the stream without a name with the ID `stream` as type records.
    fn type_stream_of(&self, ids: &StreamIds, stream: StreamId) -> Result<TypeStream> {
        let position = ids
            .unnamed
            .iter()
            .position(|&id| id == stream)
            .ok_or_else(|| {
                unresolved("S_MOD_TYPEREF names a stream that is not a stream without a name")
            })?;
        match self.streams.get(position) {
            Some(Entry::Value(bytes)) => TypeStream::from_records(bytes),
            _ => Err(unresolved(
                "S_MOD_TYPEREF names a deferred stream; the plan cannot check its records",
            )),
        }
    }
}
