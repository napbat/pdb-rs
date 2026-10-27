//! Rebuild the global and public symbol indexes and compare their bytes.

use pdbrs::io::ReadAt;
use pdbrs::native::Pdb;
use pdbrs::native::Producer;
use pdbrs::native::dbi::Dbi;
use pdbrs::native::symbols::{
    GlobalSymbolIndex, HashEntry, HashTable, PublicSymbolIndex, SectionStart, ThunkTable,
    indexed_name,
};
use pdbrs::{ErrorKind, Result as PdbResult};

use crate::{Outcome, Report};

/// Rebuild the GSI and the PSI from their decoded contents and compare the bytes.
///
/// The decoded contents are the hash records with their reference counts and
/// buckets, the thunk fields and the section starts. The builder computes the
/// order of the records in each bucket and the address map. LLD writes 0 as
/// the PDB DLL version in the DBI header, so the example selects the LLVM
/// conventions for such a PDB. The run fails if a rebuilt stream is different.
pub(crate) fn symbol_indexes(
    pdb: &Pdb<impl ReadAt>,
    dbi: &Dbi,
    symbol_records: &[u8],
    report: &mut Report,
) -> Outcome {
    let header = dbi.header();
    let producer = if header.pdb_dll_version == 0 {
        Producer::Llvm
    } else {
        Producer::Microsoft
    };
    let (mut encoded, mut different) = (0, 0);
    if let (Some(id), Some(globals)) = (header.global_stream, decoded(pdb.globals())?) {
        let table = globals.index().hash_table();
        let entries = entries(table, symbol_records, report)?;
        let mut bytes = Vec::new();
        GlobalSymbolIndex::encode_parts(
            symbol_records,
            &entries,
            table.bucket_count(),
            producer,
            &mut bytes,
        )?;
        encoded += 1;
        different += usize::from(bytes != pdb.stream_bytes(id)?);
    }
    if let (Some(id), Some(publics)) = (header.public_stream, decoded(pdb.publics())?) {
        let publics = publics.index();
        let table = publics.hash_table();
        let entries = entries(table, symbol_records, report)?;
        let fields = publics.header();
        let targets: Vec<u32> = publics.thunk_map().collect();
        let mut thunks = ThunkTable::default();
        thunks.thunk_size = fields.thunk_size;
        thunks.section = fields.thunk_table_section;
        thunks.offset = fields.thunk_table_offset;
        thunks.targets = &targets;
        let starts: Vec<SectionStart> = publics.section_starts().collect();
        let mut bytes = Vec::new();
        PublicSymbolIndex::encode_parts(
            symbol_records,
            &entries,
            table.bucket_count(),
            producer,
            thunks,
            &starts,
            &mut bytes,
        )?;
        encoded += 1;
        different += usize::from(bytes != pdb.stream_bytes(id)?);
    }
    println!("symbol indexes re-encoded: {encoded}, different: {different}");
    if different != 0 {
        return Err(format!("{different} rebuilt symbol indexes differ from the original").into());
    }
    Ok(())
}

/// Return a decoded index, or `None` for a format that the shared symbol pass
/// already counted as undecoded.
fn decoded<T>(result: PdbResult<Option<T>>) -> Outcome<Option<T>> {
    match result {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == ErrorKind::UnsupportedVersion => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Return the builder entries of a decoded hash table.
///
/// An entry keeps its bucket only if the bucket is not the bucket of its name.
fn entries(
    table: HashTable<'_>,
    symbol_records: &[u8],
    report: &mut Report,
) -> Outcome<Vec<HashEntry>> {
    let buckets = table.bucket_count();
    let mut entries = Vec::with_capacity(table.len());
    for bucket in table.buckets() {
        for record in bucket.records {
            let name = indexed_name(symbol_records, record.symbol)?;
            let mut entry = HashEntry::from(record);
            if buckets.bucket_of(name) != bucket.index {
                entry.bucket = Some(bucket.index);
                report.add("symbol index records kept in a producer bucket", 1);
            }
            entries.push(entry);
        }
    }
    Ok(entries)
}
