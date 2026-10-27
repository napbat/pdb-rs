//! The public and global symbol edits.
//!
//! The plan copies each record of the symbol record stream. It encodes a
//! renamed record again with its new name, and it appends the added records.
//! Then it builds the global and public indexes again from the source hash
//! records, with their buckets and reference counts, and the added records.
//! If an offset moves, it writes the new offsets into the global references
//! of the module streams.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::opaque::{self, Scope};
use super::plan::{Changes, Context, missing};
use super::types::Ends;
use super::{AddedSymbol, Rename, SymbolIndex, push, unsupported};
use crate::io::ReadAt;
use crate::native::Pdb;
use crate::native::codeview::Records;
use crate::native::codeview::symbol::validate::{
    Rule, ValidationContext, ValidationReport, validate_shared_symbols,
};
use crate::native::codeview::symbol::{
    Constant, Data, Public, Symbol, SymbolData, UserDefinedType,
};
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::native::symbols::{
    GlobalSymbolIndex, HashEntry, HashTable, PublicSymbolIndex, RecordOffsets, SymbolOffset,
    ThunkTable, indexed_name,
};
use crate::{Bytes, Error, ErrorKind, Result};

/// The new offset of each source record, in source offset order.
pub(super) struct OffsetMap {
    /// Pairs of a source offset and a new offset.
    pairs: Vec<(u32, u32)>,
}

impl OffsetMap {
    /// Return the new offset of the source record at `offset`.
    pub(super) fn get(&self, offset: SymbolOffset) -> Result<SymbolOffset> {
        self.pairs
            .binary_search_by_key(&offset.0, |&(old, _)| old)
            .map(|position| SymbolOffset(self.pairs[position].1))
            .map_err(|_| {
                Error::new(ErrorKind::InvalidReference, None).with_context(
                    "symbol offset does not identify a record of the symbol record stream",
                )
            })
    }

    /// Return `true` if a record has a new offset.
    pub(super) fn moved(&self) -> bool {
        self.pairs.iter().any(|(old, new)| old != new)
    }
}

/// Return `true` if the plan rejects a violation of `rule`.
///
/// The rules are the rules of [`PdbBuilder::plan`](crate::native::PdbBuilder::plan):
/// broken framing and references that do not resolve. The other rules
/// describe producer conventions; the plan keeps such records.
const fn rejected(rule: Rule) -> bool {
    matches!(
        rule,
        Rule::Walk
            | Rule::TypeIndexRange
            | Rule::IdIndexRange
            | Rule::ModuleIndex
            | Rule::ModuleRecordMissing
    )
}

/// Return the first rejected violation of `report` as an error.
pub(super) fn check(report: &ValidationReport) -> Result<()> {
    match report
        .violations
        .iter()
        .find(|violation| rejected(violation.rule))
    {
        Some(violation) => Err(violation.error()),
        None => Ok(()),
    }
}

/// Return the data of a record with the name `name`, for the kinds that a
/// rename of `index` supports.
fn renamed<'a>(
    data: &SymbolData<'a>,
    index: SymbolIndex,
    name: Bytes<'a>,
) -> Option<SymbolData<'a>> {
    match (index, data) {
        (SymbolIndex::Public, SymbolData::Public(public)) => Some(SymbolData::Public(Public {
            name,
            ..public.clone()
        })),
        (SymbolIndex::Global, SymbolData::Data(data)) => Some(SymbolData::Data(Data {
            name,
            ..data.clone()
        })),
        (SymbolIndex::Global, SymbolData::ThreadData(data)) => Some(SymbolData::ThreadData(Data {
            name,
            ..data.clone()
        })),
        (SymbolIndex::Global, SymbolData::Constant(constant)) => {
            Some(SymbolData::Constant(Constant {
                name,
                ..constant.clone()
            }))
        }
        (SymbolIndex::Global, SymbolData::UserDefinedType(udt)) => {
            Some(SymbolData::UserDefinedType(UserDefinedType {
                name,
                ..udt.clone()
            }))
        }
        _ => None,
    }
}

/// Return the offset of the only record of `table` with the name `name`.
fn find(records: &[u8], table: HashTable<'_>, name: &[u8]) -> Result<SymbolOffset> {
    let mut found = None;
    for record in table.records() {
        if indexed_name(records, record.symbol)? == name {
            if found.is_some() {
                return Err(Error::new(ErrorKind::InvalidReference, None)
                    .with_context("the renamed name identifies more than one symbol record"));
            }
            found = Some(record.symbol);
        }
    }
    found.ok_or_else(|| {
        Error::new(ErrorKind::InvalidReference, None)
            .with_context("the symbol index has no record with the renamed name")
    })
}

/// The source indexes of the symbol record stream.
struct Indexes {
    /// The global index and its stream.
    globals: Option<(StreamId, GlobalSymbolIndex)>,
    /// The public index and its stream.
    publics: Option<(StreamId, PublicSymbolIndex)>,
}

impl Indexes {
    /// Return the hash table of `index`.
    fn table(&self, index: SymbolIndex) -> Result<HashTable<'_>> {
        match index {
            SymbolIndex::Global => self
                .globals
                .as_ref()
                .map(|(_, globals)| globals.hash_table())
                .ok_or_else(|| missing("PDB has no global symbol index")),
            SymbolIndex::Public => self
                .publics
                .as_ref()
                .map(|(_, publics)| publics.hash_table())
                .ok_or_else(|| missing("PDB has no public symbol index")),
        }
    }
}

/// Find the record of each rename. Return the new name by source offset.
fn resolve<'r>(
    records: &[u8],
    indexes: &Indexes,
    renames: &'r [Rename],
) -> Result<BTreeMap<u32, (SymbolIndex, &'r [u8])>> {
    let mut targets = BTreeMap::new();
    for rename in renames {
        let offset = find(records, indexes.table(rename.index)?, &rename.old)?;
        let symbol = Symbol::decode(offset.record(records)?)?;
        if renamed(symbol.data(), rename.index, Bytes::new(&rename.new)).is_none() {
            return Err(unsupported(
                "the editor renames only public, data, thread data, constant and UDT records",
            ));
        }
        if targets
            .insert(offset.0, (rename.index, rename.new.as_slice()))
            .is_some()
        {
            return Err(unsupported("two renames change the same symbol record"));
        }
    }
    Ok(targets)
}

/// Copy the records of `records`, encode the renamed records again and
/// return the new stream and the offset map.
fn rewrite(
    records: &[u8],
    targets: &BTreeMap<u32, (SymbolIndex, &[u8])>,
) -> Result<(Vec<u8>, OffsetMap)> {
    let mut out = Vec::new();
    out.try_reserve(records.len()).map_err(|_| no_memory())?;
    let mut pairs = Vec::new();
    for record in Records::new(records) {
        let record = record?;
        let start = record.offset as usize;
        let new = u32::try_from(out.len()).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("symbol record stream does not fit in 32 bits")
        })?;
        push(&mut pairs, (record.offset, new))?;
        match targets.get(&record.offset) {
            Some(&(index, name)) => {
                let symbol = Symbol::decode(record)?;
                let data = renamed(symbol.data(), index, Bytes::new(name))
                    .ok_or_else(|| unsupported("renamed record has no name field"))?;
                data.encode(symbol.kind(), &mut out)?;
            }
            // The record has a 2-byte length field and the kind and data.
            None => out.extend_from_slice(&records[start..start + 4 + record.data.len()]),
        }
    }
    Ok((out, OffsetMap { pairs }))
}

/// Return the hash entries of `table` at their new offsets.
///
/// A source record keeps its bucket and its reference count. A renamed
/// record goes into the bucket of its new name.
fn entries(
    table: HashTable<'_>,
    map: &OffsetMap,
    targets: &BTreeMap<u32, (SymbolIndex, &[u8])>,
) -> Result<Vec<HashEntry>> {
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(table.len())
        .map_err(|_| no_memory())?;
    for bucket in table.buckets() {
        for record in bucket.records {
            entries.push(HashEntry {
                symbol: map.get(record.symbol)?,
                reference_count: record.reference_count,
                bucket: (!targets.contains_key(&record.symbol.0)).then_some(bucket.index),
            });
        }
    }
    Ok(entries)
}

/// Apply the symbol edits.
pub(super) fn plan<S: ReadAt>(
    pdb: &Pdb<S>,
    context: &Context,
    added: &[AddedSymbol],
    renames: &[Rename],
    ends: Ends,
    changes: &mut Changes,
) -> Result<()> {
    let header = context.dbi.header();
    let stream = header
        .symbol_stream
        .ok_or_else(|| missing("PDB has no symbol record stream"))?;
    let records = pdb.stream_bytes(stream)?;
    let indexes = Indexes {
        globals: header.global_stream.zip(pdb.global_symbols(&context.dbi)?),
        publics: header.public_stream.zip(pdb.public_symbols(&context.dbi)?),
    };
    let targets = resolve(&records, &indexes, renames)?;
    let (mut out, map) = rewrite(&records, &targets)?;
    if map.moved() {
        opaque::check(pdb, context, Scope::Records)?;
    }
    let mut new_entries: [Vec<HashEntry>; 2] = Default::default();
    for symbol in added {
        let offset = SymbolOffset(u32::try_from(out.len()).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("symbol record stream does not fit in 32 bits")
        })?);
        out.try_reserve(symbol.record.len())
            .map_err(|_| no_memory())?;
        out.extend_from_slice(&symbol.record);
        let list = match symbol.index {
            SymbolIndex::Global => &mut new_entries[0],
            SymbolIndex::Public => &mut new_entries[1],
        };
        push(list, HashEntry::new(offset))?;
    }
    let module_count = context.dbi.modules().count();
    check(&validate_shared_symbols(
        &out,
        stream,
        &ValidationContext::new(ends.types, ends.ids).with_module_count(module_count),
    )?)?;
    let offsets = RecordOffsets::frame(&out, stream)?;
    let buckets = pdb.symbol_bucket_count();
    let [global_added, public_added] = new_entries;
    if !global_added.is_empty()
        || targets
            .values()
            .any(|(index, _)| *index == SymbolIndex::Global)
        || map.moved()
    {
        let (id, globals) = indexes
            .globals
            .as_ref()
            .ok_or_else(|| missing("PDB has no global symbol index"))?;
        let mut list = entries(globals.hash_table(), &map, &targets)?;
        list.try_reserve(global_added.len())
            .map_err(|_| no_memory())?;
        list.extend(global_added);
        let mut bytes = Vec::new();
        GlobalSymbolIndex::encode_parts(&out, &list, buckets, context.index_producer, &mut bytes)?;
        let parsed = GlobalSymbolIndex::parse(bytes, *id, buckets)?;
        parsed.hash_table().check_records(&offsets)?;
        changes.replace(*id, parsed.into_bytes())?;
    }
    if !public_added.is_empty()
        || targets
            .values()
            .any(|(index, _)| *index == SymbolIndex::Public)
        || map.moved()
    {
        let (id, publics) = indexes
            .publics
            .as_ref()
            .ok_or_else(|| missing("PDB has no public symbol index"))?;
        let mut list = entries(publics.hash_table(), &map, &targets)?;
        list.try_reserve(public_added.len())
            .map_err(|_| no_memory())?;
        list.extend(public_added);
        let targets: Vec<u32> = publics.thunk_map().collect();
        let starts: Vec<_> = publics.section_starts().collect();
        let header = publics.header();
        let thunks = ThunkTable {
            thunk_size: header.thunk_size,
            section: header.thunk_table_section,
            offset: header.thunk_table_offset,
            targets: &targets,
        };
        let mut bytes = Vec::new();
        PublicSymbolIndex::encode_parts(
            &out,
            &list,
            buckets,
            context.index_producer,
            thunks,
            &starts,
            &mut bytes,
        )?;
        let parsed = PublicSymbolIndex::parse(bytes, *id, buckets)?;
        parsed.check_records(&offsets)?;
        changes.replace(*id, parsed.into_bytes())?;
    }
    if map.moved() {
        move_global_refs(pdb, context, &map, changes)?;
    }
    changes.replace(stream, out)
}

/// Write the new offsets into the global references of each module stream
/// whose references change.
fn move_global_refs<S: ReadAt>(
    pdb: &Pdb<S>,
    context: &Context,
    map: &OffsetMap,
    changes: &mut Changes,
) -> Result<()> {
    for module in context.dbi.modules() {
        let (_, module) = module?;
        let Some(stream) = pdb.module_stream(&module)? else {
            continue;
        };
        let mut moved = Vec::new();
        moved
            .try_reserve_exact(stream.global_ref_data().len() / 4)
            .map_err(|_| no_memory())?;
        let refs = stream.global_refs()?;
        let mut differs = false;
        for offset in &refs {
            let new = map.get(offset)?;
            differs |= new != offset;
            moved.push(new);
        }
        if !differs {
            continue;
        }
        let id = stream.stream();
        let start = stream.symbol_offset()
            + stream.symbol_data().len()
            + stream.c11_data().len()
            + stream.c13_data().len()
            + 4;
        let mut bytes = stream.into_bytes();
        for (position, offset) in moved.iter().enumerate() {
            let at = start + position * 4;
            bytes[at..at + 4].copy_from_slice(&offset.0.to_le_bytes());
        }
        changes.replace(id, bytes)?;
    }
    Ok(())
}
