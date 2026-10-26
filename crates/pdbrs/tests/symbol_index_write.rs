//! Tests for the global and public symbol index builders.

use std::collections::HashMap;

use pdbrs::ErrorKind;
use pdbrs::native::msf::StreamId;
use pdbrs::native::symbols::{
    BucketCount, GlobalSymbolIndex, HashEntry, HashRecord, IndexProducer, PublicSymbolIndex,
    SectionStart, SymbolOffset, ThunkTable, indexed_name,
};

/// The stream ID of the rebuilt index streams.
const INDEX: StreamId = StreamId(9);
/// The kind of a user-defined type symbol (`S_UDT`).
const S_UDT: u16 = 0x1108;
/// The kind of a public symbol (`S_PUB32`).
const S_PUB32: u16 = 0x110e;
/// The size of the bucket bitmap for 4096 buckets.
const STANDARD_BITMAP: usize = 516;
/// The size of the bucket bitmap for 262143 buckets.
const MINIMAL_BITMAP: usize = 32_768;

/// Append a record with `kind` and `fields`, then a name, NUL and zero padding.
fn record(stream: &mut Vec<u8>, kind: u16, fields: &[u8], name: &[u8]) -> SymbolOffset {
    let offset = SymbolOffset(u32::try_from(stream.len()).unwrap());
    let mut body = kind.to_le_bytes().to_vec();
    body.extend_from_slice(fields);
    body.extend_from_slice(name);
    body.push(0);
    while !(body.len() + 2).is_multiple_of(4) {
        body.push(0);
    }
    stream.extend_from_slice(&u16::try_from(body.len()).unwrap().to_le_bytes());
    stream.extend_from_slice(&body);
    offset
}

/// Append an `S_UDT` record.
fn udt(stream: &mut Vec<u8>, name: &[u8]) -> SymbolOffset {
    record(stream, S_UDT, &0x1000_u32.to_le_bytes(), name)
}

/// Append an `S_PUB32` record.
fn public(stream: &mut Vec<u8>, name: &[u8], section: u16, offset: u32) -> SymbolOffset {
    let mut fields = 0_u32.to_le_bytes().to_vec();
    fields.extend_from_slice(&offset.to_le_bytes());
    fields.extend_from_slice(&section.to_le_bytes());
    record(stream, S_PUB32, &fields, name)
}

/// Return entries with a reference count of 1 in the bucket of each name.
fn entries(offsets: &[SymbolOffset]) -> Vec<HashEntry> {
    offsets.iter().copied().map(HashEntry::new).collect()
}

fn build_globals(stream: &[u8], entries: &[HashEntry], buckets: BucketCount) -> Vec<u8> {
    GlobalSymbolIndex::build(stream, entries, buckets, IndexProducer::Microsoft).unwrap()
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

/// Return the symbol offsets of a bucket in file order.
fn bucket(index: &GlobalSymbolIndex, bucket: u32) -> Vec<u32> {
    index
        .hash_table()
        .bucket(bucket)
        .map(|record| record.symbol.0)
        .collect()
}

/// Return names of `length` letters that share one standard bucket.
fn colliding_names(length: u32, count: usize) -> (u32, Vec<Vec<u8>>) {
    let mut groups: HashMap<u32, Vec<Vec<u8>>> = HashMap::new();
    for value in 0_u32.. {
        let name: Vec<u8> = (0..length)
            .map(|position| b'a' + u8::try_from(value / 26_u32.pow(position) % 26).unwrap())
            .collect();
        let group = groups
            .entry(BucketCount::Standard.bucket_of(&name))
            .or_default();
        group.push(name);
        if group.len() == count {
            let bucket = BucketCount::Standard.bucket_of(&group[0]);
            return (bucket, group.clone());
        }
    }
    unreachable!()
}

#[test]
fn empty_global_index_follows_the_producer() {
    let microsoft =
        GlobalSymbolIndex::build(&[], &[], BucketCount::Standard, IndexProducer::Microsoft)
            .unwrap();
    assert_eq!(microsoft.len(), 16);
    assert_eq!(u32_at(&microsoft, 0), u32::MAX);
    assert_eq!(u32_at(&microsoft, 4), 0xf12f_091a);
    assert_eq!(u32_at(&microsoft, 8), 0);
    assert_eq!(u32_at(&microsoft, 12), 0);
    let llvm =
        GlobalSymbolIndex::build(&[], &[], BucketCount::Standard, IndexProducer::Llvm).unwrap();
    assert_eq!(llvm.len(), 16 + STANDARD_BITMAP);
    assert_eq!(u32_at(&llvm, 12), 516);
    assert!(llvm[16..].iter().all(|&byte| byte == 0));
    for bytes in [microsoft, llvm] {
        let index = GlobalSymbolIndex::parse(bytes, INDEX, BucketCount::Standard).unwrap();
        assert!(index.hash_table().is_empty());
        assert_eq!(index.hash_table().buckets().len(), 0);
    }
}

#[test]
fn one_record_gives_one_bucket() {
    let mut stream = Vec::new();
    let skipped = udt(&mut stream, b"skipped");
    let offset = udt(&mut stream, b"Point");
    let bytes = build_globals(&stream, &entries(&[offset]), BucketCount::Standard);
    assert_eq!(bytes.len(), 16 + 8 + STANDARD_BITMAP + 4);
    assert_eq!(u32_at(&bytes, 8), 8);
    assert_eq!(u32_at(&bytes, 12), 520);
    assert_eq!(u32_at(&bytes, 16), offset.0 + 1);
    assert_eq!(u32_at(&bytes, 20), 1);
    assert_eq!(u32_at(&bytes, bytes.len() - 4), 0);
    let index = GlobalSymbolIndex::parse(bytes, INDEX, BucketCount::Standard).unwrap();
    let found: Vec<_> = index.hash_table().lookup(b"Point").collect();
    assert_eq!(
        found,
        [HashRecord {
            symbol: offset,
            reference_count: 1,
        }]
    );
    assert_ne!(skipped, offset);
    assert_eq!(index.hash_table().len(), 1);
}

#[test]
fn bucket_collisions_sort_by_length_then_name() {
    let (shared, names) = colliding_names(3, 3);
    let (_, long) = colliding_names(4, 1);
    let mut stream = Vec::new();
    let mut offsets: Vec<SymbolOffset> = names.iter().map(|name| udt(&mut stream, name)).collect();
    let other = udt(&mut stream, &long[0]);
    offsets.reverse();
    let mut input = entries(&offsets);
    input.push(HashEntry::new(other));
    let index = GlobalSymbolIndex::parse(
        build_globals(&stream, &input, BucketCount::Standard),
        INDEX,
        BucketCount::Standard,
    )
    .unwrap();
    let mut expected: Vec<(Vec<u8>, u32)> = names
        .iter()
        .zip(offsets.iter().rev())
        .map(|(name, offset)| (name.clone(), offset.0))
        .collect();
    expected.sort();
    let order: Vec<u32> = expected.iter().map(|(_, offset)| *offset).collect();
    assert_eq!(bucket(&index, shared), order);
    let buckets: Vec<u32> = index.hash_table().buckets().map(|b| b.index).collect();
    let mut sorted = buckets.clone();
    sorted.sort_unstable();
    assert_eq!(buckets, sorted);
    assert_eq!(index.hash_table().len(), 4);
}

#[test]
fn names_that_differ_in_case_keep_the_offset_order() {
    let mut stream = Vec::new();
    let upper = udt(&mut stream, b"VALUE");
    let shorter = udt(&mut stream, b"val");
    let lower = udt(&mut stream, b"value");
    let mixed = udt(&mut stream, b"Value");
    let bucket_index = BucketCount::Standard.bucket_of(b"value");
    assert_eq!(BucketCount::Standard.bucket_of(b"VALUE"), bucket_index);
    assert_eq!(BucketCount::Standard.bucket_of(b"Value"), bucket_index);
    let mut input = entries(&[mixed, lower, upper]);
    // Keep "val" in the same bucket to test the length rule.
    input.push(HashEntry {
        bucket: Some(bucket_index),
        ..HashEntry::new(shorter)
    });
    let bytes = build_globals(&stream, &input, BucketCount::Standard);
    let index = GlobalSymbolIndex::parse(bytes, INDEX, BucketCount::Standard).unwrap();
    assert_eq!(
        bucket(&index, bucket_index),
        [shorter.0, upper.0, lower.0, mixed.0]
    );
}

#[test]
fn ascii_letters_sort_as_lowercase() {
    let mut stream = Vec::new();
    let underscore = udt(&mut stream, b"_b");
    let upper = udt(&mut stream, b"AB");
    let lower = udt(&mut stream, b"aa");
    let high = udt(&mut stream, b"\xc3\xa9");
    let input: Vec<HashEntry> = [high, lower, upper, underscore]
        .into_iter()
        .map(|offset| HashEntry {
            bucket: Some(7),
            ..HashEntry::new(offset)
        })
        .collect();
    let bytes = build_globals(&stream, &input, BucketCount::Standard);
    let index = GlobalSymbolIndex::parse(bytes, INDEX, BucketCount::Standard).unwrap();
    // `_` (0x5f) sorts before `a`; the non-ASCII name compares by bytes.
    assert_eq!(bucket(&index, 7), [underscore.0, lower.0, upper.0, high.0]);
}

#[test]
fn minimal_bucket_count_uses_the_large_bitmap() {
    let mut stream = Vec::new();
    let offsets: Vec<SymbolOffset> = (0..50)
        .map(|value| udt(&mut stream, format!("name_{value}").as_bytes()))
        .collect();
    let bytes = build_globals(&stream, &entries(&offsets), BucketCount::Minimal);
    assert_eq!(u32_at(&bytes, 8), 400);
    let nonempty = (u32_at(&bytes, 12) as usize - MINIMAL_BITMAP) / 4;
    let index = GlobalSymbolIndex::parse(bytes, INDEX, BucketCount::Minimal).unwrap();
    let table = index.hash_table();
    assert_eq!(table.nonempty_bucket_count(), nonempty);
    assert!(table.buckets().any(|bucket| bucket.index >= 0x1000));
    for (value, offset) in offsets.iter().enumerate() {
        let name = format!("name_{value}");
        assert!(
            table
                .lookup(name.as_bytes())
                .any(|record| record.symbol == *offset)
        );
    }
}

#[test]
fn reference_counts_and_producer_buckets_round_trip() {
    let mut stream = Vec::new();
    let first = udt(&mut stream, b"first");
    let second = udt(&mut stream, b"second");
    let input = [
        HashEntry {
            reference_count: 23,
            ..HashEntry::new(first)
        },
        HashEntry {
            bucket: Some(4095),
            ..HashEntry::new(second)
        },
    ];
    let bytes = build_globals(&stream, &input, BucketCount::Standard);
    let index = GlobalSymbolIndex::parse(bytes.clone(), INDEX, BucketCount::Standard).unwrap();
    let table = index.hash_table();
    let lookup: Vec<_> = table.lookup(b"first").collect();
    assert_eq!(lookup[0].reference_count, 23);
    assert_eq!(bucket(&index, 4095), [second.0]);
    let decoded: Vec<HashEntry> = table
        .buckets()
        .flat_map(|bucket| {
            bucket.records.map(move |record| HashEntry {
                bucket: Some(bucket.index),
                ..HashEntry::from(record)
            })
        })
        .collect();
    assert_eq!(
        build_globals(&stream, &decoded, BucketCount::Standard),
        bytes
    );
}

#[test]
fn public_index_without_thunks_round_trips() {
    let mut stream = Vec::new();
    let late = public(&mut stream, b"late", 2, 0x10);
    let early = public(&mut stream, b"early", 1, 0x40);
    let bytes = PublicSymbolIndex::build(
        &stream,
        &entries(&[late, early]),
        BucketCount::Standard,
        IndexProducer::Llvm,
        ThunkTable::default(),
        &[],
    )
    .unwrap();
    let index = PublicSymbolIndex::parse(bytes.clone(), INDEX, BucketCount::Standard).unwrap();
    let header = index.header();
    assert_eq!(header.address_map_size, 8);
    assert_eq!(
        (
            header.thunk_count,
            header.thunk_size,
            header.thunk_table_section
        ),
        (0, 0, 0)
    );
    assert_eq!(
        (
            header.padding,
            header.thunk_table_offset,
            header.section_count
        ),
        ([0, 0], 0, 0)
    );
    assert_eq!(
        header.hash_size as usize,
        bytes.len() - 28 - 8,
        "the hash table fills the space before the address map"
    );
    let map: Vec<_> = index.address_map().collect();
    assert_eq!(map, [early, late]);
    assert_eq!(index.thunk_map().len(), 0);
    assert_eq!(index.section_starts().len(), 0);
    assert_eq!(index.hash_table().len(), 2);
}

#[test]
fn public_index_writes_thunks_and_section_starts() {
    let mut stream = Vec::new();
    let only = public(&mut stream, b"main", 1, 0);
    let starts = [SectionStart {
        offset: 0x11000,
        section: 2,
        padding: [0xff, 0xff],
    }];
    let thunks = ThunkTable {
        thunk_size: 5,
        section: 2,
        offset: 5,
        targets: &[0x1_0010, 0x1_0020, 0x1_0010],
    };
    let bytes = PublicSymbolIndex::build(
        &stream,
        &entries(&[only]),
        BucketCount::Standard,
        IndexProducer::Microsoft,
        thunks,
        &starts,
    )
    .unwrap();
    assert_eq!(
        &bytes[bytes.len() - 8..],
        b"\x00\x10\x01\x00\x02\x00\xff\xff"
    );
    let index = PublicSymbolIndex::parse(bytes, INDEX, BucketCount::Standard).unwrap();
    let header = index.header();
    assert_eq!(
        (
            header.thunk_count,
            header.thunk_size,
            header.thunk_table_section
        ),
        (3, 5, 2)
    );
    assert_eq!((header.thunk_table_offset, header.section_count), (5, 1));
    let targets: Vec<_> = index.thunk_map().collect();
    assert_eq!(targets, thunks.targets);
    let read: Vec<_> = index.section_starts().collect();
    assert_eq!(read, starts);
}

#[test]
fn address_map_ties_sort_by_name_then_offset() {
    let mut stream = Vec::new();
    let beta = public(&mut stream, b"beta", 1, 0x20);
    let alpha = public(&mut stream, b"alpha", 1, 0x20);
    let twin = public(&mut stream, b"alpha", 1, 0x20);
    let first = public(&mut stream, b"zeta", 1, 0x10);
    let bytes = PublicSymbolIndex::build(
        &stream,
        &entries(&[twin, beta, first, alpha]),
        BucketCount::Standard,
        IndexProducer::Microsoft,
        ThunkTable::default(),
        &[],
    )
    .unwrap();
    let index = PublicSymbolIndex::parse(bytes, INDEX, BucketCount::Standard).unwrap();
    let map: Vec<_> = index.address_map().collect();
    assert_eq!(map, [first, alpha, twin, beta]);
}

#[test]
fn address_order_follows_the_producer_signedness() {
    let mut stream = Vec::new();
    let low = public(&mut stream, b"low", 1, 0x10);
    let high = public(&mut stream, b"high", 1, 0x8000_0000);
    let first_section = public(&mut stream, b"first", 1, 0);
    let high_section = public(&mut stream, b"last", 0x8000, 0);
    let input = entries(&[low, high, first_section, high_section]);
    let map = |producer| {
        let bytes = PublicSymbolIndex::build(
            &stream,
            &input,
            BucketCount::Standard,
            producer,
            ThunkTable::default(),
            &[],
        )
        .unwrap();
        let index = PublicSymbolIndex::parse(bytes, INDEX, BucketCount::Standard).unwrap();
        index.address_map().collect::<Vec<_>>()
    };
    assert_eq!(
        map(IndexProducer::Llvm),
        [first_section, low, high, high_section]
    );
    assert_eq!(
        map(IndexProducer::Microsoft),
        [high_section, high, first_section, low]
    );
}

#[test]
fn kinds_without_a_decoder_give_their_names() {
    let mut stream = Vec::new();
    let fields = [0_u8; 10];
    // S_GMANDATA uses the `DATASYM32` layout.
    let managed = record(&mut stream, 0x111d, &fields, b"m_value");
    // S_DATAREF uses the `REFSYM2` layout.
    let reference = record(&mut stream, 0x1126, &fields, b"data");
    let decoded = udt(&mut stream, b"Point");
    // S_FRAMEPROC has no name.
    let nameless = record(&mut stream, 0x1012, &[0; 26], b"");
    assert_eq!(indexed_name(&stream, managed).unwrap(), b"m_value");
    assert_eq!(indexed_name(&stream, reference).unwrap(), b"data");
    assert_eq!(indexed_name(&stream, decoded).unwrap(), b"Point");
    assert_eq!(
        indexed_name(&stream, nameless).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    let bytes = build_globals(
        &stream,
        &entries(&[managed, reference]),
        BucketCount::Standard,
    );
    let index = GlobalSymbolIndex::parse(bytes, INDEX, BucketCount::Standard).unwrap();
    let found: Vec<_> = index.hash_table().lookup(b"m_value").collect();
    assert_eq!(found[0].symbol, managed);
    // A name without a NUL terminator is not valid.
    let mut truncated = Vec::new();
    truncated.extend_from_slice(&14_u16.to_le_bytes());
    truncated.extend_from_slice(&0x111d_u16.to_le_bytes());
    truncated.extend_from_slice(&[0; 10]);
    truncated.extend_from_slice(b"ab");
    assert_eq!(
        indexed_name(&truncated, SymbolOffset(0))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn invalid_entries_give_errors() {
    let mut stream = Vec::new();
    let offset = udt(&mut stream, b"type");
    let build = |entry: HashEntry| {
        GlobalSymbolIndex::build(
            &stream,
            &[entry],
            BucketCount::Standard,
            IndexProducer::Microsoft,
        )
        .unwrap_err()
        .kind()
    };
    assert_eq!(
        build(HashEntry::new(SymbolOffset(0x7fff_ffff))),
        ErrorKind::LimitExceeded
    );
    assert_eq!(
        build(HashEntry::new(SymbolOffset(u32::MAX))),
        ErrorKind::LimitExceeded
    );
    assert_eq!(
        build(HashEntry {
            reference_count: 0,
            ..HashEntry::new(offset)
        }),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        build(HashEntry {
            reference_count: 0x8000_0000,
            ..HashEntry::new(offset)
        }),
        ErrorKind::LimitExceeded
    );
    assert_eq!(
        build(HashEntry {
            bucket: Some(4096),
            ..HashEntry::new(offset)
        }),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        build(HashEntry::new(SymbolOffset(0x100))),
        ErrorKind::InvalidReference
    );
    let public = PublicSymbolIndex::build(
        &stream,
        &[HashEntry::new(offset)],
        BucketCount::Standard,
        IndexProducer::Microsoft,
        ThunkTable::default(),
        &[],
    )
    .unwrap_err();
    assert_eq!(public.kind(), ErrorKind::InvalidFormat);
}

#[test]
fn largest_symbol_offset_passes_the_width_check() {
    // The offset plus 1 is 0x7fffffff, the largest value that the decoder
    // accepts. The build fails only because no record starts there.
    let mut stream = Vec::new();
    udt(&mut stream, b"x");
    let entry = HashEntry::new(SymbolOffset(0x7fff_fffe));
    let error = GlobalSymbolIndex::build(
        &stream,
        &[entry],
        BucketCount::Standard,
        IndexProducer::Microsoft,
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
}
