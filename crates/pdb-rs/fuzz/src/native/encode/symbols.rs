//! Round trips of the symbol record encoders, the symbol stream writers and the GSI and PSI
//! builders.

use pdb_rs::native::Producer;
use pdb_rs::native::codeview::Records;
use pdb_rs::native::codeview::index::ModuleSymbolOffset;
use pdb_rs::native::codeview::symbol::{ModuleSymbolWriter, ModuleSymbols, Symbol};
use pdb_rs::native::msf::StreamId;
use pdb_rs::native::symbols::{
    BucketCount, GlobalSymbolIndex, HashEntry, HashTable, PublicSymbolIndex, SectionStart,
    SymbolRecordWriter, ThunkTable, indexed_name,
};

use crate::input::Input;
use crate::ok;

/// The most records that one input encodes, to bound the time of one run.
const MAX_RECORDS: usize = 1 << 14;

/// The stream number that the targets give to symbol data.
const STREAM: StreamId = StreamId(7);

/// Return the bytes of the record of a decoded symbol: the length field, the kind and the
/// payload.
fn record_bytes(symbol: &Symbol<'_>) -> Vec<u8> {
    let record = symbol.record();
    let length = u16::try_from(record.data.len() + 2).expect("a framed record has a length");
    let mut bytes = length.to_le_bytes().to_vec();
    bytes.extend_from_slice(&record.kind.to_le_bytes());
    bytes.extend_from_slice(record.data);
    bytes
}

/// Encode a decoded symbol, compare the bytes and decode the output.
pub(crate) fn symbol(symbol: &Symbol<'_>) {
    let mut out = Vec::new();
    symbol.encode(&mut out).expect("a decoded symbol encodes");
    assert_eq!(
        out,
        record_bytes(symbol),
        "a decoded symbol gives its bytes again"
    );
    let record = Records::new(&out)
        .next()
        .expect("an encoded symbol frames")
        .expect("an encoded symbol frames");
    let again = Symbol::decode(record).expect("an encoded symbol decodes");
    assert_eq!(again.data(), symbol.data());
}

/// Rewrite a shared symbol record stream and a module symbol region.
///
/// # Panics
///
/// Panics if a writer rejects a decoded symbol or writes other bytes.
pub fn stream(data: &[u8]) {
    let mut writer = SymbolRecordWriter::new();
    let mut used = 0;
    for record in Records::new(data).take(MAX_RECORDS) {
        let Some(record) = ok(record) else {
            break;
        };
        let Some(decoded) = ok(Symbol::decode(record)) else {
            break;
        };
        symbol(&decoded);
        let offset = writer
            .push_symbol(&decoded)
            .expect("a decoded symbol writes");
        assert_eq!(offset.0, record.offset);
        used = usize::try_from(record.offset).unwrap_or(0) + record.data.len() + 4;
    }
    let written = writer.finish();
    assert_eq!(
        written.bytes,
        &data[..used],
        "the record writer keeps the records"
    );

    let mut module = ModuleSymbolWriter::new(ModuleSymbolOffset(4));
    let mut used = 0;
    let mut complete = true;
    for scoped in ModuleSymbols::new(data, STREAM, ModuleSymbolOffset(4), 64).take(MAX_RECORDS) {
        let Some(scoped) = ok(scoped) else {
            complete = false;
            break;
        };
        let offset = module
            .push_symbol(&scoped.symbol)
            .expect("a decoded module symbol writes");
        assert_eq!(offset, scoped.offset);
        let length = scoped.symbol.record().data.len() + 4;
        used = usize::try_from(scoped.offset.0 - 4).unwrap_or(0) + length;
    }
    if complete && let Some(bytes) = ok(module.finish()) {
        assert_eq!(bytes, &data[..used], "the module writer keeps the records");
    }
}

/// Return the builder entries of a decoded hash table, or `None` if a record has no name.
fn entries(table: &HashTable<'_>, records: &[u8]) -> Option<Vec<HashEntry>> {
    let buckets = table.bucket_count();
    let mut entries = Vec::new();
    for bucket in table.buckets() {
        for record in bucket.records {
            let name = ok(indexed_name(records, record.symbol))?;
            let mut entry = HashEntry::from(record);
            if buckets.bucket_of(name) != bucket.index {
                entry.bucket = Some(bucket.index);
            }
            entries.push(entry);
        }
    }
    Some(entries)
}

/// Return the records of each bucket of a table, in bucket order.
fn buckets(table: &HashTable<'_>) -> Vec<(u32, Vec<(u32, u32)>)> {
    table
        .buckets()
        .map(|bucket| {
            let mut records: Vec<(u32, u32)> = bucket
                .records
                .map(|record| (record.symbol.0, record.reference_count))
                .collect();
            records.sort_unstable();
            (bucket.index, records)
        })
        .collect()
}

/// Rebuild a GSI and a PSI from their decoded contents and parse the output.
///
/// The input has the layout of the `native_indexes` target. The second selector bit selects
/// the LLVM conventions.
///
/// # Panics
///
/// Panics if a builder rejects decoded contents, or if the rebuilt index does not parse
/// with the same records in the same buckets.
pub fn indexes(data: &[u8]) {
    let mut input = Input::new(data);
    let selector = input.u8();
    let buckets_kind = if selector & 1 == 0 {
        BucketCount::Standard
    } else {
        BucketCount::Minimal
    };
    let producer = if selector & 2 == 0 {
        Producer::Microsoft
    } else {
        Producer::Llvm
    };
    let gsi = input.part().to_vec();
    let psi = input.part().to_vec();
    let records = input.rest();
    if let Some(globals) = ok(GlobalSymbolIndex::parse(gsi, StreamId(8), buckets_kind))
        && let Some(entries) = entries(&globals.hash_table(), records)
        && let Some(bytes) = ok({
            let mut bytes = Vec::new();
            GlobalSymbolIndex::encode_parts(records, &entries, buckets_kind, producer, &mut bytes)
                .map(|()| bytes)
        })
    {
        let again =
            GlobalSymbolIndex::parse(bytes, StreamId(8), buckets_kind).expect("a built GSI parses");
        assert_eq!(buckets(&again.hash_table()), buckets(&globals.hash_table()));
    }
    let Some(publics) = ok(PublicSymbolIndex::parse(psi, StreamId(9), buckets_kind)) else {
        return;
    };
    let Some(entries) = entries(&publics.hash_table(), records) else {
        return;
    };
    let fields = publics.header();
    let targets: Vec<u32> = publics.thunk_map().collect();
    let mut thunks = ThunkTable::default();
    thunks.thunk_size = fields.thunk_size;
    thunks.section = fields.thunk_table_section;
    thunks.offset = fields.thunk_table_offset;
    thunks.targets = &targets;
    let starts: Vec<SectionStart> = publics.section_starts().collect();
    let mut bytes = Vec::new();
    if ok(PublicSymbolIndex::encode_parts(
        records,
        &entries,
        buckets_kind,
        producer,
        thunks,
        &starts,
        &mut bytes,
    ))
    .is_none()
    {
        return;
    }
    let again =
        PublicSymbolIndex::parse(bytes, StreamId(9), buckets_kind).expect("a built PSI parses");
    assert_eq!(buckets(&again.hash_table()), buckets(&publics.hash_table()));
    assert!(again.thunk_map().eq(targets.iter().copied()));
    assert!(again.section_starts().eq(starts.iter().copied()));
}
