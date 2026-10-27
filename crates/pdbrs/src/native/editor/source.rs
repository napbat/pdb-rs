//! The decoders of the copy-on-write accessors: each one reads one part of
//! the source PDB.

use alloc::vec::Vec;

use super::module::ModuleContents;
use super::symbols::SharedSymbols;
use super::types::{ListRecord, RecordList};
use super::{ModuleSlot, Names, missing};
use crate::io::ReadAt;
use crate::native::Pdb;
use crate::native::codeview::id_record::IdRecord;
use crate::native::codeview::index::ModuleIndex;
use crate::native::codeview::type_record::TypeLeaf;
use crate::native::names::StringTableBuilder;
use crate::native::output::no_memory;
use crate::native::types::{HashAdjuster, TypeHashes, TypeStream};
use crate::{Bytes, Error, ErrorKind, Result};

/// Decode the symbol record stream and its global and public indexes.
pub(super) fn shared_symbols<S: ReadAt>(pdb: &Pdb<S>) -> Result<SharedSymbols> {
    let stream = pdb
        .debug_info()?
        .ok_or_else(|| missing("PDB has no DBI stream"))?
        .header()
        .symbol_stream
        .ok_or_else(|| missing("PDB has no symbol record stream"))?;
    let records = pdb
        .symbol_records()?
        .ok_or_else(|| missing("PDB has no symbol record stream"))?;
    let globals = pdb.globals()?;
    let publics = pdb.publics()?;
    SharedSymbols::decode(
        stream,
        records,
        globals.as_ref().map(crate::native::symbols::Globals::index),
        publics.as_ref().map(crate::native::symbols::Publics::index),
    )
}

/// Make a slot for each module of the source DBI stream.
pub(super) fn module_slots<S: ReadAt>(pdb: &Pdb<S>) -> Result<Vec<ModuleSlot>> {
    let dbi = pdb
        .debug_info()?
        .ok_or_else(|| missing("PDB has no DBI stream"))?;
    let mut slots = Vec::new();
    for module in dbi.modules() {
        module?;
        slots.try_reserve(1).map_err(|_| no_memory())?;
        slots.push(ModuleSlot::Source);
    }
    Ok(slots)
}

/// Decode the source module `index`.
pub(super) fn module<S: ReadAt>(pdb: &Pdb<S>, index: ModuleIndex) -> Result<ModuleContents> {
    let dbi = pdb
        .debug_info()?
        .ok_or_else(|| missing("PDB has no DBI stream"))?;
    let outside = || {
        Error::new(ErrorKind::InvalidReference, None)
            .with_context("module index is outside the module list")
    };
    let (_, record) = dbi.modules().nth(index.0 as usize).ok_or_else(outside)??;
    let mut files = Vec::new();
    if let Some(info) = dbi.source_info()?
        && let Some(list) = info.modules.get(index.0 as usize)
    {
        for name in info.file_names(list) {
            files.try_reserve(1).map_err(|_| no_memory())?;
            files.push(Bytes::from(name?).into_owned());
        }
    }
    let stream = pdb.load_module(&record)?;
    ModuleContents::decode(index, record.into_owned(), files, stream)
}

/// Read the hash adjusters of a type stream.
pub(super) fn adjusters<S: ReadAt>(pdb: &Pdb<S>, stream: &TypeStream) -> Result<Vec<HashAdjuster>> {
    let Some(id) = stream.header().hash_stream else {
        return Ok(Vec::new());
    };
    let bytes = pdb.stream_bytes(id)?;
    let hashes = TypeHashes::parse(stream.header(), &bytes)?;
    let mut list = Vec::new();
    list.try_reserve_exact(hashes.adjusters().len())
        .map_err(|_| no_memory())?;
    list.extend_from_slice(hashes.adjusters());
    Ok(list)
}

/// Decode the records of a type stream.
fn list<S: ReadAt, T: ListRecord>(
    pdb: &Pdb<S>,
    stream: Option<TypeStream>,
    context: &'static str,
) -> Result<RecordList<T>> {
    let stream = stream.ok_or_else(|| missing(context))?;
    let adjusters = adjusters(pdb, &stream)?;
    RecordList::decode(stream, adjusters)
}

/// Decode the records of the TPI stream.
pub(super) fn type_list<S: ReadAt>(pdb: &Pdb<S>) -> Result<RecordList<TypeLeaf<'static>>> {
    list(pdb, pdb.types()?, "PDB has no TPI stream")
}

/// Decode the records of the IPI stream.
pub(super) fn id_list<S: ReadAt>(pdb: &Pdb<S>) -> Result<RecordList<IdRecord<'static>>> {
    list(pdb, pdb.ids()?, "PDB has no IPI stream")
}

/// Decode the `/names` stream.
pub(super) fn names<S: ReadAt>(pdb: &Pdb<S>) -> Result<Names> {
    match pdb.names()? {
        Some(table) => Ok(Names::Table(StringTableBuilder::from_table(&table)?)),
        None => Err(missing("the PDB has no /names table")),
    }
}
