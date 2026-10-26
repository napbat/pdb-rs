//! Tests for the global and public symbol indexes.

use pdbrs::ErrorKind;
use pdbrs::native::hash::hash_string_v1;
use pdbrs::native::msf::StreamId;
use pdbrs::native::symbols::{
    BucketCount, GlobalSymbolIndex, MINIMAL_DEBUG_INFO_FEATURE, PublicSymbolIndex, RecordOffsets,
    SectionStart, SymbolOffset,
};

/// The stream ID of the synthetic index streams.
const INDEX: StreamId = StreamId(9);
/// The stream ID of the synthetic symbol record stream.
const SYMBOLS: StreamId = StreamId(11);
/// The kind of a user-defined type symbol (`S_UDT`).
const S_UDT: u16 = 0x1108;
/// The version of the compressed-bucket hash header.
const VERSION: u32 = 0xf12f_091a;
/// The size of the bucket bitmap for 4096 buckets.
const STANDARD_BITMAP: usize = 516;

fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
    bytes[offset..offset + value.len()].copy_from_slice(value);
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    put(bytes, offset, &value.to_le_bytes());
}

/// Build an `S_UDT` record with zero padding to a multiple of 4.
fn udt(name: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&S_UDT.to_le_bytes());
    body.extend_from_slice(&0x1000_u32.to_le_bytes());
    body.extend_from_slice(name);
    body.push(0);
    while !(body.len() + 2).is_multiple_of(4) {
        body.push(0);
    }
    let mut record = u16::try_from(body.len()).unwrap().to_le_bytes().to_vec();
    record.extend_from_slice(&body);
    record
}

/// Build a symbol record stream; return it and the record offsets.
fn symbols(names: &[&[u8]]) -> (Vec<u8>, Vec<u32>) {
    let mut bytes = Vec::new();
    let mut offsets = Vec::new();
    for name in names {
        offsets.push(u32::try_from(bytes.len()).unwrap());
        bytes.extend_from_slice(&udt(name));
    }
    (bytes, offsets)
}

/// Build a hash table.
///
/// `records` holds `(serialized offset, reference count)` pairs.
/// `buckets` holds `(bucket index, first record index)` pairs in bucket order.
fn table(records: &[(u32, u32)], buckets: &[(u32, u32)], count: BucketCount) -> Vec<u8> {
    let words = (count.get() as usize + 1).div_ceil(32);
    let mut bitmap = vec![0_u8; words * 4];
    let mut offsets = Vec::new();
    for &(index, first) in buckets {
        let index = index as usize;
        bitmap[index / 8] |= 1 << (index % 8);
        offsets.extend_from_slice(&(first * 12).to_le_bytes());
    }
    let mut bucket_bytes = bitmap;
    bucket_bytes.extend_from_slice(&offsets);
    if records.is_empty() && buckets.is_empty() {
        bucket_bytes.clear();
    }
    raw_table(records, &bucket_bytes)
}

/// Build a hash table with the given bucket bytes.
fn raw_table(records: &[(u32, u32)], buckets: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(records.len() * 8).unwrap().to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(buckets.len()).unwrap().to_le_bytes());
    for &(offset, references) in records {
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(&references.to_le_bytes());
    }
    bytes.extend_from_slice(buckets);
    bytes
}

/// Build a hash table that indexes records at `offsets` by `names`.
fn index_for(names: &[&[u8]], offsets: &[u32]) -> Vec<u8> {
    let count = BucketCount::Standard;
    let mut entries: Vec<(u32, u32)> = names
        .iter()
        .zip(offsets)
        .map(|(name, &offset)| (count.bucket_of(name), offset))
        .collect();
    entries.sort_unstable();
    let records: Vec<(u32, u32)> = entries.iter().map(|&(_, offset)| (offset + 1, 1)).collect();
    let mut buckets: Vec<(u32, u32)> = Vec::new();
    for (index, &(bucket, _)) in (0_u32..).zip(&entries) {
        if buckets.last().is_none_or(|&(last, _)| last != bucket) {
            buckets.push((bucket, index));
        }
    }
    table(&records, &buckets, count)
}

fn gsi(bytes: Vec<u8>) -> pdbrs::Result<GlobalSymbolIndex> {
    GlobalSymbolIndex::parse(bytes, INDEX, BucketCount::Standard)
}

fn gsi_error(bytes: Vec<u8>) -> pdbrs::Error {
    gsi(bytes).unwrap_err()
}

fn assert_error(error: &pdbrs::Error, kind: ErrorKind, offset: usize) {
    assert_eq!(error.kind(), kind, "{error}");
    assert_eq!(error.offset(), None);
    assert_eq!(
        error.stream_location(),
        Some((INDEX.0, u64::try_from(offset).unwrap())),
        "{error}"
    );
}

#[test]
fn bucket_count_follows_the_minimal_debug_info_feature() {
    assert_eq!(BucketCount::from_features([]), BucketCount::Standard);
    assert_eq!(
        BucketCount::from_features([20_140_508, MINIMAL_DEBUG_INFO_FEATURE]),
        BucketCount::Minimal
    );
    assert_eq!(BucketCount::Standard.get(), 4096);
    assert_eq!(BucketCount::Minimal.get(), 0x3ffff);
    let hash = hash_string_v1(b"main");
    assert_eq!(BucketCount::Standard.bucket_of(b"main"), hash % 4096);
    assert_eq!(BucketCount::Minimal.bucket_of(b"main"), hash % 0x3ffff);
    // The producer truncates the bucket to 16 bits (`HASH` is `USHORT`).
    let hash = hash_string_v1(b"__chkstk");
    assert_eq!(hash % 0x3ffff, 0x372ae);
    assert_eq!(BucketCount::Minimal.bucket_of(b"__chkstk"), 0x72ae);
    assert_eq!(BucketCount::Standard.bucket_of(b"__chkstk"), hash % 4096);
}

#[test]
fn minimal_lookup_uses_the_16_bit_bucket() {
    let names: [&[u8]; 2] = [b"main", b"__chkstk"];
    let (stream, offsets) = symbols(&names);
    // `main` is in bucket 0xddbe; `__chkstk` is in bucket 0x72ae, not 0x372ae.
    let records = [(offsets[1] + 1, 1), (offsets[0] + 1, 1)];
    let bytes = table(&records, &[(0x72ae, 0), (0xddbe, 1)], BucketCount::Minimal);
    let index = GlobalSymbolIndex::parse(bytes, INDEX, BucketCount::Minimal).unwrap();
    let table = index.hash_table();
    for (name, &offset) in names.iter().zip(&offsets) {
        let found: Vec<_> = table.lookup(name).map(|record| record.symbol).collect();
        assert_eq!(found, [SymbolOffset(offset)]);
        assert_eq!(SymbolOffset(offset).record(&stream).unwrap().offset, offset);
    }
    assert_eq!(table.bucket(0x372ae).count(), 0);
}

#[test]
fn global_index_finds_every_name_in_its_bucket() {
    let names: [&[u8]; 6] = [b"main", b"Point", b"u32", b"Vec<u8>", b"x", b"Point2"];
    let (stream, offsets) = symbols(&names);
    let index = gsi(index_for(&names, &offsets)).unwrap();
    let table = index.hash_table();
    assert_eq!(table.len(), names.len());
    assert!(!table.is_empty());
    assert_eq!(table.bucket_count(), BucketCount::Standard);
    assert_eq!(table.header().signature, u32::MAX);
    assert_eq!(table.header().version, VERSION);
    assert_eq!(table.header().records_size, 48);

    for (name, &offset) in names.iter().zip(&offsets) {
        let candidates: Vec<_> = table.lookup(name).map(|record| record.symbol).collect();
        assert!(candidates.contains(&SymbolOffset(offset)));
        let record = SymbolOffset(offset).record(&stream).unwrap();
        assert_eq!(record.offset, offset);
        assert_eq!(record.kind, S_UDT);
    }
    let mut total = 0;
    for bucket in table.buckets() {
        for record in bucket.records {
            let position = offsets.iter().position(|&o| o == record.symbol.0).unwrap();
            assert_eq!(
                BucketCount::Standard.bucket_of(names[position]),
                bucket.index
            );
            assert_eq!(record.reference_count, 1);
            total += 1;
        }
    }
    assert_eq!(total, names.len());
    let records = RecordOffsets::frame(&stream, SYMBOLS).unwrap();
    assert_eq!(records.len(), names.len());
    assert_eq!(records.stream_length(), stream.len());
    table.check_records(&records).unwrap();
}

#[test]
fn buckets_report_index_first_record_and_records() {
    let records = [(1, 1), (9, 2), (17, 1), (25, 1), (33, 1)];
    let bytes = table(
        &records,
        &[(0, 0), (31, 2), (32, 3), (4095, 4)],
        BucketCount::Standard,
    );
    let index = gsi(bytes).unwrap();
    let table = index.hash_table();
    let buckets: Vec<_> = table.buckets().collect();
    assert_eq!(table.buckets().len(), 4);
    assert_eq!(table.nonempty_bucket_count(), 4);
    let summary: Vec<_> = buckets
        .iter()
        .map(|bucket| (bucket.index, bucket.first_record, bucket.records.len()))
        .collect();
    assert_eq!(summary, [(0, 0, 2), (31, 2, 1), (32, 3, 1), (4095, 4, 1)]);
    let first: Vec<_> = table.bucket(0).map(|record| record.symbol).collect();
    assert_eq!(first, [SymbolOffset(0), SymbolOffset(8)]);
    assert_eq!(table.bucket(0).next_back().unwrap().reference_count, 2);
    assert_eq!(table.bucket(4095).next().unwrap().symbol, SymbolOffset(32));
    assert_eq!(table.bucket(1).len(), 0);
    assert_eq!(table.bucket(4096).len(), 0);
    assert_eq!(table.bucket(u32::MAX).len(), 0);
    let all: Vec<_> = table.records().map(|record| record.symbol.0).collect();
    assert_eq!(all, [0, 8, 16, 24, 32]);
}

#[test]
fn empty_tables_with_and_without_a_bitmap_are_valid() {
    let without = raw_table(&[], &[]);
    let with = raw_table(&[], &[0; STANDARD_BITMAP]);
    for bytes in [without, with] {
        let index = gsi(bytes).unwrap();
        let table = index.hash_table();
        assert!(table.is_empty());
        assert_eq!(table.buckets().count(), 0);
        assert_eq!(table.lookup(b"main").count(), 0);
        let empty = RecordOffsets::frame(&[], SYMBOLS).unwrap();
        assert!(empty.is_empty());
        table.check_records(&empty).unwrap();
    }
}

#[test]
fn minimal_tables_use_the_large_bitmap() {
    let mut bytes = table(&[(1, 1)], &[(0x3fffe, 0)], BucketCount::Minimal);
    assert_eq!(bytes.len(), 16 + 8 + 0x8000 + 4);
    let index = GlobalSymbolIndex::parse(bytes.clone(), INDEX, BucketCount::Minimal).unwrap();
    let table = index.hash_table();
    assert_eq!(table.bucket(0x3fffe).count(), 1);
    assert_eq!(table.buckets().next().unwrap().index, 0x3fffe);

    // The bit of the free-list bucket 0x3ffff must be clear.
    let last_word = 16 + 8 + 0x8000 - 4;
    bytes[last_word + 3] |= 0x80;
    let error = GlobalSymbolIndex::parse(bytes, INDEX, BucketCount::Minimal).unwrap_err();
    assert_error(&error, ErrorKind::InvalidFormat, last_word);

    let standard = table_bytes_standard();
    let error = GlobalSymbolIndex::parse(standard, INDEX, BucketCount::Minimal).unwrap_err();
    assert_error(
        &error,
        ErrorKind::UnexpectedEof,
        16 + 8 + STANDARD_BITMAP + 4,
    );
}

/// A valid table with one record in bucket 7.
fn table_bytes_standard() -> Vec<u8> {
    table(&[(1, 1)], &[(7, 0)], BucketCount::Standard)
}

#[test]
fn header_errors_have_stream_locations() {
    assert_error(&gsi_error(Vec::new()), ErrorKind::UnexpectedEof, 0);
    assert_error(&gsi_error(vec![0xff; 15]), ErrorKind::UnexpectedEof, 15);

    let mut bytes = table_bytes_standard();
    put_u32(&mut bytes, 0, 0);
    assert_error(&gsi_error(bytes), ErrorKind::UnsupportedVersion, 0);

    let mut bytes = table_bytes_standard();
    put_u32(&mut bytes, 4, VERSION + 1);
    assert_error(&gsi_error(bytes), ErrorKind::UnsupportedVersion, 4);

    let mut bytes = table_bytes_standard();
    put_u32(&mut bytes, 8, 12);
    assert_error(&gsi_error(bytes), ErrorKind::InvalidFormat, 8);

    let mut bytes = table_bytes_standard();
    put_u32(&mut bytes, 8, 0x1000_0000);
    assert_error(&gsi_error(bytes), ErrorKind::UnexpectedEof, 8);

    let mut bytes = table_bytes_standard();
    put_u32(&mut bytes, 12, 0x1000_0000);
    assert_error(&gsi_error(bytes), ErrorKind::UnexpectedEof, 12);

    let mut bytes = table_bytes_standard();
    let end = bytes.len();
    bytes.push(0);
    assert_error(&gsi_error(bytes), ErrorKind::InvalidFormat, end);
}

#[test]
fn hash_record_fields_are_checked() {
    let error = gsi_error(table(&[(0, 1)], &[(7, 0)], BucketCount::Standard));
    assert_error(&error, ErrorKind::InvalidReference, 16);
    let error = gsi_error(table(&[(0x8000_0000, 1)], &[(7, 0)], BucketCount::Standard));
    assert_error(&error, ErrorKind::InvalidReference, 16);
    let error = gsi_error(table(&[(1, 1), (5, 0)], &[(7, 0)], BucketCount::Standard));
    assert_error(&error, ErrorKind::InvalidFormat, 28);
    let error = gsi_error(table(&[(1, 0x8000_0000)], &[(7, 0)], BucketCount::Standard));
    assert_error(&error, ErrorKind::InvalidFormat, 20);
}

#[test]
fn bucket_errors_have_stream_locations() {
    let bitmap = 16 + 8;
    let offsets = bitmap + STANDARD_BITMAP;

    // Hash records without bucket data.
    let error = gsi_error(raw_table(&[(1, 1)], &[]));
    assert_error(&error, ErrorKind::InvalidFormat, bitmap);
    // Hash records with an empty bitmap.
    let error = gsi_error(raw_table(&[(1, 1)], &[0; STANDARD_BITMAP]));
    assert_error(&error, ErrorKind::InvalidFormat, bitmap);
    // A truncated bitmap.
    let error = gsi_error(raw_table(&[(1, 1)], &[0; 8]));
    assert_error(&error, ErrorKind::UnexpectedEof, bitmap + 8);

    // The bit of the free-list bucket 4096.
    let mut bytes = table_bytes_standard();
    bytes[bitmap + 512] = 1;
    assert_error(&gsi_error(bytes), ErrorKind::InvalidFormat, bitmap + 512);

    // More set bits than bucket offsets.
    let mut bytes = table_bytes_standard();
    bytes[bitmap] |= 1;
    assert_error(&gsi_error(bytes), ErrorKind::UnexpectedEof, offsets);

    // More bucket offsets than set bits.
    let mut bytes = table_bytes_standard();
    bytes.extend_from_slice(&[0; 4]);
    put_u32(&mut bytes, 12, u32::try_from(STANDARD_BITMAP + 8).unwrap());
    assert_error(&gsi_error(bytes), ErrorKind::InvalidFormat, offsets);

    let mut bytes = table_bytes_standard();
    put_u32(&mut bytes, offsets, 6);
    assert_error(&gsi_error(bytes), ErrorKind::InvalidFormat, offsets);

    let mut bytes = table_bytes_standard();
    put_u32(&mut bytes, offsets, 12);
    assert_error(&gsi_error(bytes), ErrorKind::InvalidFormat, offsets);
}

#[test]
fn bucket_offsets_start_at_zero_and_increase() {
    let records = [(1, 1), (9, 1)];
    let base = 16 + 16 + STANDARD_BITMAP;

    let bytes = table(&records, &[(1, 1), (2, 1)], BucketCount::Standard);
    assert_error(&gsi_error(bytes), ErrorKind::InvalidFormat, base);

    let bytes = table(&records, &[(1, 0), (2, 0)], BucketCount::Standard);
    assert_error(&gsi_error(bytes), ErrorKind::InvalidFormat, base + 4);

    let bytes = table(&records, &[(1, 0), (2, 2)], BucketCount::Standard);
    assert_error(&gsi_error(bytes), ErrorKind::InvalidFormat, base + 4);
}

#[test]
fn record_checks_require_record_boundaries() {
    let (stream, offsets) = symbols(&[b"alpha", b"beta"]);
    let records = RecordOffsets::frame(&stream, SYMBOLS).unwrap();
    assert!(records.contains(SymbolOffset(offsets[1])));
    assert!(!records.contains(SymbolOffset(offsets[1] + 4)));
    let listed: Vec<_> = records.iter().map(|offset| offset.0).collect();
    assert_eq!(listed, offsets);

    let inside = offsets[1] + 4;
    let bytes = table(&[(1, 1), (inside + 1, 1)], &[(3, 0)], BucketCount::Standard);
    let index = gsi(bytes).unwrap();
    let error = index.hash_table().check_records(&records).unwrap_err();
    assert_error(&error, ErrorKind::InvalidReference, 24);

    let past = u32::try_from(stream.len()).unwrap();
    let bytes = table(&[(past + 1, 1)], &[(3, 0)], BucketCount::Standard);
    let index = gsi(bytes).unwrap();
    let error = index.hash_table().check_records(&records).unwrap_err();
    assert_error(&error, ErrorKind::InvalidReference, 16);
}

#[test]
fn record_framing_errors_identify_the_symbol_stream() {
    let (mut stream, offsets) = symbols(&[b"alpha", b"beta"]);
    stream.truncate(stream.len() - 1);
    let error = RecordOffsets::frame(&stream, SYMBOLS).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(
        error.stream_location(),
        Some((SYMBOLS.0, u64::from(offsets[1])))
    );

    let error = SymbolOffset(offsets[1]).record(&stream).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    let length = u32::try_from(stream.len()).unwrap();
    let error = SymbolOffset(length).record(&stream).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
}

/// The parts of a synthetic public symbol index.
struct Public {
    header: [u32; 7],
    table: Vec<u8>,
    tail: Vec<u8>,
}

impl Public {
    /// Build an index over a table, with an address map, thunks and sections.
    fn new(table: Vec<u8>, address_map: &[u32], thunks: &[u32], sections: &[(u32, u16)]) -> Self {
        let mut tail = Vec::new();
        for value in address_map.iter().chain(thunks) {
            tail.extend_from_slice(&value.to_le_bytes());
        }
        for &(offset, section) in sections {
            tail.extend_from_slice(&offset.to_le_bytes());
            tail.extend_from_slice(&section.to_le_bytes());
            tail.extend_from_slice(&[0xcd, 0xab]);
        }
        let header = [
            u32::try_from(table.len()).unwrap(),
            u32::try_from(address_map.len() * 4).unwrap(),
            u32::try_from(thunks.len()).unwrap(),
            5,
            0xbeef_0002,
            0x1000,
            u32::try_from(sections.len()).unwrap(),
        ];
        Self {
            header,
            table,
            tail,
        }
    }

    fn bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in self.header {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&self.table);
        bytes.extend_from_slice(&self.tail);
        bytes
    }

    fn parse(&self) -> pdbrs::Result<PublicSymbolIndex> {
        PublicSymbolIndex::parse(self.bytes(), INDEX, BucketCount::Standard)
    }
}

/// A public index over the records at offsets 0, 12 and 24.
fn public(thunks: &[u32], sections: &[(u32, u16)]) -> Public {
    let table = table(
        &[(1, 1), (13, 1), (25, 1)],
        &[(9, 0)],
        BucketCount::Standard,
    );
    Public::new(table, &[24, 0, 12], thunks, sections)
}

#[test]
fn public_index_decodes_all_parts() {
    let parts = public(&[0x1010, 0x2020], &[(0x1000, 1), (0x5000, 2)]);
    let index = parts.parse().unwrap();
    let header = index.header();
    assert_eq!(header.hash_size, u32::try_from(parts.table.len()).unwrap());
    assert_eq!(header.address_map_size, 12);
    assert_eq!(header.thunk_count, 2);
    assert_eq!(header.thunk_size, 5);
    assert_eq!(header.thunk_table_section, 2);
    assert_eq!(header.padding, [0xef, 0xbe]);
    assert_eq!(header.thunk_table_offset, 0x1000);
    assert_eq!(header.section_count, 2);
    assert_eq!(index.hash_table().bucket(9).len(), 3);
    let map: Vec<_> = index.address_map().map(|offset| offset.0).collect();
    assert_eq!(map, [24, 0, 12]);
    assert_eq!(index.address_map().next_back(), Some(SymbolOffset(12)));
    let thunks: Vec<_> = index.thunk_map().collect();
    assert_eq!(thunks, [0x1010, 0x2020]);
    let sections: Vec<_> = index.section_starts().collect();
    assert_eq!(
        sections,
        [
            SectionStart {
                offset: 0x1000,
                section: 1,
                padding: [0xcd, 0xab],
            },
            SectionStart {
                offset: 0x5000,
                section: 2,
                padding: [0xcd, 0xab],
            },
        ]
    );
}

#[test]
fn absent_section_starts_ignore_the_section_count() {
    let mut parts = public(&[], &[]);
    parts.header[6] = 0xcccc_cccc;
    let index = parts.parse().unwrap();
    assert_eq!(index.header().section_count, 0xcccc_cccc);
    assert_eq!(index.section_starts().len(), 0);
    assert_eq!(index.thunk_map().len(), 0);
}

#[test]
fn public_size_errors_have_stream_locations() {
    let error = PublicSymbolIndex::parse(vec![0; 27], INDEX, BucketCount::Standard).unwrap_err();
    assert_error(&error, ErrorKind::UnexpectedEof, 27);

    let mut parts = public(&[], &[]);
    parts.header[0] = u32::try_from(parts.bytes().len()).unwrap();
    assert_error(&parts.parse().unwrap_err(), ErrorKind::UnexpectedEof, 0);

    // A hash size that cuts the buckets of the table.
    let mut parts = public(&[], &[]);
    parts.header[0] -= 4;
    assert_error(
        &parts.parse().unwrap_err(),
        ErrorKind::UnexpectedEof,
        28 + 12,
    );

    // A hash size with bytes after the buckets of the table.
    let mut parts = public(&[], &[]);
    let table_end = 28 + parts.table.len();
    parts.table.extend_from_slice(&[0; 4]);
    parts.header[0] += 4;
    assert_error(
        &parts.parse().unwrap_err(),
        ErrorKind::InvalidFormat,
        table_end,
    );

    let mut parts = public(&[], &[]);
    parts.header[1] = 6;
    assert_error(&parts.parse().unwrap_err(), ErrorKind::InvalidFormat, 4);

    let mut parts = public(&[], &[]);
    parts.header[1] = 16;
    assert_error(&parts.parse().unwrap_err(), ErrorKind::UnexpectedEof, 4);

    let mut parts = public(&[7], &[]);
    parts.header[2] = 2;
    assert_error(&parts.parse().unwrap_err(), ErrorKind::UnexpectedEof, 8);

    let mut parts = public(&[7], &[]);
    parts.header[2] = 0x4000_0000;
    assert_error(&parts.parse().unwrap_err(), ErrorKind::OffsetOverflow, 8);

    let mut parts = public(&[7], &[(0x1000, 1)]);
    parts.header[6] = 2;
    let sections = 28 + parts.table.len() + 12 + 4;
    assert_error(
        &parts.parse().unwrap_err(),
        ErrorKind::InvalidFormat,
        sections,
    );

    let mut parts = public(&[7], &[(0x1000, 1)]);
    parts.tail.push(0);
    assert_error(
        &parts.parse().unwrap_err(),
        ErrorKind::InvalidFormat,
        sections,
    );
}

#[test]
fn public_hash_errors_have_stream_locations() {
    let mut parts = public(&[], &[]);
    put_u32(&mut parts.table, 4, 0);
    assert_error(
        &parts.parse().unwrap_err(),
        ErrorKind::UnsupportedVersion,
        32,
    );
}

#[test]
fn public_record_checks_cover_the_address_map() {
    let names: [&[u8]; 3] = [b"a", b"bb", b"ccc"];
    let (stream, offsets) = symbols(&names);
    assert_eq!(offsets, [0, 12, 24]);
    let records = RecordOffsets::frame(&stream, SYMBOLS).unwrap();
    public(&[], &[])
        .parse()
        .unwrap()
        .check_records(&records)
        .unwrap();

    let table = table(
        &[(1, 1), (13, 1), (25, 1)],
        &[(9, 0)],
        BucketCount::Standard,
    );
    let table_len = table.len();
    let index = Public::new(table, &[24, 4, 12], &[], &[]).parse().unwrap();
    let error = index.check_records(&records).unwrap_err();
    assert_error(&error, ErrorKind::InvalidReference, 28 + table_len + 4);
}

#[test]
fn address_order_uses_caller_addresses() {
    let index = public(&[], &[]).parse().unwrap();
    let address = |offset: SymbolOffset| -> pdbrs::Result<(u16, u32)> {
        // Equal addresses are valid.
        Ok(match offset.0 {
            0 | 24 => (1, 0x10),
            12 => (2, 0),
            _ => unreachable!(),
        })
    };
    index.check_address_order(address).unwrap();

    let unsorted = |offset: SymbolOffset| -> pdbrs::Result<(u16, u32)> {
        Ok(match offset.0 {
            24 => (1, 0x20),
            0 => (1, 0x10),
            _ => (2, 0),
        })
    };
    let error = index.check_address_order(unsorted).unwrap_err();
    let table_len = public(&[], &[]).table.len();
    assert_error(&error, ErrorKind::InvalidFormat, 28 + table_len + 4);

    let failing =
        |_| -> pdbrs::Result<(u16, u32)> { Err(pdbrs::Error::new(ErrorKind::InvalidFormat, None)) };
    let error = index.check_address_order(failing).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(error.stream_location(), None);
}
