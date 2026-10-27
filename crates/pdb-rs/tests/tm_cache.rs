//! Tests for the `/TMCache` stream and the type map streams that it names.

use pdb_rs::ErrorKind;
use pdb_rs::native::auxiliary::{
    TM_CACHE_VERSION, TmCache, TmCacheBuilder, TypeMapKind, TypeMapStream,
};
use pdb_rs::native::msf::StreamId;

/// The stream ID of the synthetic streams.
const STREAM: StreamId = StreamId(6);

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn put(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Build one `/TMCache` entry.
fn entry(slot: u16, checksum: u64) -> Vec<u8> {
    let mut bytes = slot.to_le_bytes().to_vec();
    bytes.extend_from_slice(&[0; 6]);
    bytes.extend_from_slice(&checksum.to_le_bytes());
    bytes
}

/// Build a `/TMCache` stream like the MSVC 14.51 fixture: the stream table has 2 bytes of
/// padding after it. Slot 2 has no stream.
fn cache() -> Vec<u8> {
    let slots: [u16; 3] = [14, 0xffff, 17];
    let table: Vec<u8> = slots.iter().flat_map(|slot| slot.to_le_bytes()).collect();
    let entries = [
        entry(0, 0xde36_d0b8_1ade_e5a9),
        entry(0, 0xab8e_5077_b708_a0e0),
        entry(2, 1),
        entry(1, u64::MAX),
    ]
    .concat();
    let entry_offset = 20 + table.len() + 2;
    let mut bytes = words(&[
        TM_CACHE_VERSION,
        20,
        u32::try_from(table.len()).unwrap(),
        u32::try_from(entry_offset).unwrap(),
        u32::try_from(entries.len()).unwrap(),
    ]);
    bytes.extend_from_slice(&table);
    bytes.extend_from_slice(&[0, 0]);
    bytes.extend_from_slice(&entries);
    bytes
}

/// The offset of the entry table in [`cache`].
const ENTRIES: usize = 28;

fn assert_error<T: core::fmt::Debug>(result: pdb_rs::Result<T>, kind: ErrorKind, offset: usize) {
    let error = result.unwrap_err();
    assert_eq!(error.kind(), kind, "{error}");
    assert_eq!(error.offset(), None);
    assert_eq!(
        error.stream_location(),
        Some((STREAM, u64::try_from(offset).unwrap())),
        "{error}"
    );
}

#[test]
fn empty_tm_cache_has_no_slots_or_entries() {
    let cache = TmCache::parse(&[], STREAM).unwrap();
    assert!(cache.is_empty());
    assert_eq!(cache.streams().len(), 0);
    assert_eq!(cache.entries().len(), 0);
}

#[test]
fn tm_cache_maps_entries_to_slot_streams() {
    let bytes = cache();
    let cache = TmCache::parse(&bytes, STREAM).unwrap();
    assert!(!cache.is_empty());
    let streams: Vec<_> = cache.streams().collect();
    assert_eq!(streams, [Some(StreamId(14)), None, Some(StreamId(17))]);
    let entries: Vec<_> = cache
        .entries()
        .map(|entry| (entry.index, entry.slot, entry.stream, entry.checksum))
        .collect();
    assert_eq!(
        entries,
        [
            (0, 0, Some(StreamId(14)), 0xde36_d0b8_1ade_e5a9),
            (1, 0, Some(StreamId(14)), 0xab8e_5077_b708_a0e0),
            (2, 2, Some(StreamId(17)), 1),
            (3, 1, None, u64::MAX),
        ]
    );
}

#[test]
fn tm_cache_header_errors_have_stream_locations() {
    let bytes = cache();
    assert_error(
        TmCache::parse(&bytes[..19], STREAM),
        ErrorKind::UnexpectedEof,
        19,
    );

    let mut version = bytes.clone();
    put(&mut version, 0, 0x2020_0228);
    assert_error(
        TmCache::parse(&version, STREAM),
        ErrorKind::UnsupportedVersion,
        0,
    );

    let mut table = bytes.clone();
    put(&mut table, 8, u32::MAX);
    assert_error(TmCache::parse(&table, STREAM), ErrorKind::UnexpectedEof, 4);

    let mut list = bytes.clone();
    put(&mut list, 16, 16 * 5);
    assert_error(TmCache::parse(&list, STREAM), ErrorKind::UnexpectedEof, 12);
    // The entry table ends at the end of the stream; one more byte is truncated.
    assert_error(
        TmCache::parse(&bytes[..bytes.len() - 1], STREAM),
        ErrorKind::UnexpectedEof,
        12,
    );
}

#[test]
fn tm_cache_layout_rules_are_enforced() {
    let bytes = cache();

    let mut odd = bytes.clone();
    put(&mut odd, 8, 5);
    assert_error(TmCache::parse(&odd, STREAM), ErrorKind::InvalidFormat, 8);

    let mut partial = bytes.clone();
    put(&mut partial, 16, 16 * 4 - 1);
    assert_error(
        TmCache::parse(&partial, STREAM),
        ErrorKind::InvalidFormat,
        8,
    );

    let mut inside_header = bytes.clone();
    put(&mut inside_header, 4, 16);
    assert_error(
        TmCache::parse(&inside_header, STREAM),
        ErrorKind::InvalidFormat,
        4,
    );

    let mut overlap = bytes.clone();
    put(&mut overlap, 12, 24);
    put(&mut overlap, 16, 16);
    assert_error(
        TmCache::parse(&overlap, STREAM),
        ErrorKind::InvalidFormat,
        4,
    );

    let mut padding = bytes.clone();
    padding[27] = 1;
    assert_error(
        TmCache::parse(&padding, STREAM),
        ErrorKind::InvalidFormat,
        27,
    );

    let mut trailing = bytes.clone();
    trailing.push(0);
    TmCache::parse(&trailing, STREAM).unwrap();
    trailing.push(9);
    let last = trailing.len() - 1;
    assert_error(
        TmCache::parse(&trailing, STREAM),
        ErrorKind::InvalidFormat,
        last,
    );
}

#[test]
fn tm_cache_entry_rules_are_enforced() {
    let bytes = cache();
    for reserved in 2..8 {
        let mut changed = bytes.clone();
        changed[ENTRIES + 16 + reserved] = 1;
        assert_error(
            TmCache::parse(&changed, STREAM),
            ErrorKind::UnsupportedVersion,
            ENTRIES + 16 + 2,
        );
    }
    let mut slot = bytes.clone();
    slot[ENTRIES + 32] = 3;
    assert_error(
        TmCache::parse(&slot, STREAM),
        ErrorKind::InvalidReference,
        ENTRIES + 32,
    );
}

/// Build a version 2 type map stream.
fn object_map(precompiled: u16, map: &[u32], pairs: &[u32]) -> Vec<u8> {
    let mut bytes = words(&[2]);
    bytes.extend_from_slice(&precompiled.to_le_bytes());
    bytes.extend_from_slice(&[0, 0]);
    let count = u32::try_from(map.len()).unwrap();
    bytes.extend(words(&[7, 0x1000, count]));
    bytes.extend(words(map));
    let offsets: Vec<u32> = (0..=count).map(|index| index * 8).collect();
    bytes.extend(words(&offsets));
    bytes.extend(words(pairs));
    bytes
}

#[test]
fn type_server_maps_have_type_and_id_maps() {
    let bytes = words(&[1, 2, 3, 0x10ac, 0, 0x1003, 0x1004, 0, 0x1002, 0x1238]);
    let map = TypeMapStream::parse(&bytes, STREAM).unwrap();
    assert_eq!(map.kind(), TypeMapKind::TypeServer);
    assert_eq!(map.first_index(), 0x1000);
    assert_eq!(map.types().collect::<Vec<_>>(), [0x10ac, 0]);
    assert_eq!(map.ids().collect::<Vec<_>>(), [0x1003, 0x1004, 0]);
    assert_eq!(map.record_offsets().len(), 0);
    assert_eq!(map.function_types().collect::<Vec<_>>(), [(0x1002, 0x1238)]);

    let empty_bytes = words(&[1, 0, 0]);
    let empty = TypeMapStream::parse(&empty_bytes, STREAM).unwrap();
    assert_eq!((empty.types().len(), empty.ids().len()), (0, 0));
    assert_eq!(empty.function_types().len(), 0);
}

#[test]
fn object_maps_have_offsets_and_a_precompiled_reference() {
    let bytes = object_map(0xffff, &[0x1000, 0x1003, 0x1108], &[0x1002, 0x1003]);
    let map = TypeMapStream::parse(&bytes, STREAM).unwrap();
    assert_eq!(
        map.kind(),
        TypeMapKind::Object {
            precompiled: None,
            signature: 7
        }
    );
    assert_eq!(map.first_index(), 0x1000);
    assert_eq!(map.types().collect::<Vec<_>>(), [0x1000, 0x1003, 0x1108]);
    assert_eq!(map.ids().len(), 0);
    assert_eq!(map.record_offsets().collect::<Vec<_>>(), [0, 8, 16, 24]);
    assert_eq!(map.function_types().collect::<Vec<_>>(), [(0x1002, 0x1003)]);

    let bytes = object_map(66, &[], &[]);
    let map = TypeMapStream::parse(&bytes, STREAM).unwrap();
    assert_eq!(
        map.kind(),
        TypeMapKind::Object {
            precompiled: Some(StreamId(66)),
            signature: 7
        }
    );
    assert_eq!(map.types().len(), 0);
    assert_eq!(map.record_offsets().collect::<Vec<_>>(), [0]);
}

#[test]
fn type_map_errors_have_stream_locations() {
    assert_error(
        TypeMapStream::parse(&[], STREAM),
        ErrorKind::UnexpectedEof,
        0,
    );
    assert_error(
        TypeMapStream::parse(&words(&[3, 0, 0]), STREAM),
        ErrorKind::UnsupportedVersion,
        0,
    );
    assert_error(
        TypeMapStream::parse(&words(&[1, 0]), STREAM),
        ErrorKind::UnexpectedEof,
        8,
    );
    assert_error(
        TypeMapStream::parse(&words(&[2, 0xffff, 0, 0x1000]), STREAM),
        ErrorKind::UnexpectedEof,
        16,
    );
    // The ID map is one word short.
    assert_error(
        TypeMapStream::parse(&words(&[1, 1, 2, 0x1000, 0x1000]), STREAM),
        ErrorKind::UnexpectedEof,
        20,
    );
    // The record offsets are one word short.
    let bytes = object_map(0xffff, &[0x1000], &[]);
    let short = &bytes[..bytes.len() - 4];
    assert_error(
        TypeMapStream::parse(short, STREAM),
        ErrorKind::UnexpectedEof,
        short.len(),
    );
    let mut huge = object_map(0xffff, &[], &[]);
    put(&mut huge, 16, u32::MAX);
    assert_error(
        TypeMapStream::parse(&huge, STREAM),
        ErrorKind::UnexpectedEof,
        16,
    );

    // Function ID pairs must fill the rest of the stream.
    let mut tail = object_map(0xffff, &[0x1000], &[0x1000, 0x1001]);
    tail.extend_from_slice(&[1, 2, 3, 4]);
    let offset = tail.len() - 4;
    assert_error(
        TypeMapStream::parse(&tail, STREAM),
        ErrorKind::InvalidFormat,
        offset,
    );
    let odd = words(&[1, 0, 0, 0x1000]);
    assert_error(
        TypeMapStream::parse(&odd, STREAM),
        ErrorKind::InvalidFormat,
        12,
    );
}

fn encode_cache(cache: &TmCache<'_>) -> Vec<u8> {
    let mut out = Vec::new();
    cache.encode(&mut out).unwrap();
    out
}

#[test]
fn tm_cache_encodes_the_decoded_layout_again() {
    let bytes = cache();
    let decoded = TmCache::parse(&bytes, STREAM).unwrap();
    assert_eq!(decoded.table_offsets(), (20, ENTRIES));
    assert_eq!(decoded.size(), bytes.len());
    assert_eq!(encode_cache(&decoded), bytes);
    assert_eq!(encode_cache(&TmCache::parse(&[], STREAM).unwrap()), b"");

    // Zero gaps before, between and after the tables stay as decoded.
    let mut moved = words(&[TM_CACHE_VERSION, 24, 2, 32, 16, 0]);
    moved.extend_from_slice(&[17, 0, 0, 0, 0, 0, 0, 0]);
    moved.extend_from_slice(&entry(0, 5));
    moved.extend_from_slice(&[0; 3]);
    let decoded = TmCache::parse(&moved, STREAM).unwrap();
    assert_eq!(decoded.table_offsets(), (24, 32));
    assert_eq!(encode_cache(&decoded), moved);

    // A header without tables is not an empty stream.
    let header = words(&[TM_CACHE_VERSION, 20, 0, 20, 0]);
    let decoded = TmCache::parse(&header, STREAM).unwrap();
    assert!(!decoded.is_empty());
    assert_eq!(encode_cache(&decoded), header);
}

#[test]
fn tm_cache_builder_uses_the_link_exe_layout() {
    let mut builder = TmCacheBuilder::new();
    assert_eq!(builder.push_stream(Some(StreamId(14))).unwrap(), 0);
    assert_eq!(builder.push_stream(None).unwrap(), 1);
    assert_eq!(builder.push_stream(Some(StreamId(17))).unwrap(), 2);
    builder.push_entry(0, 0xde36_d0b8_1ade_e5a9).unwrap();
    builder.push_entry(0, 0xab8e_5077_b708_a0e0).unwrap();
    builder.push_entry(2, 1).unwrap();
    builder.push_entry(1, u64::MAX).unwrap();
    let mut out = Vec::new();
    builder.encode(&mut out).unwrap();
    assert_eq!(out, cache());

    let bytes = cache();
    let copy = TmCacheBuilder::from_cache(&TmCache::parse(&bytes, STREAM).unwrap()).unwrap();
    assert_eq!(copy, builder);

    // An even slot count needs no padding.
    let mut even = TmCacheBuilder::new();
    even.push_stream(Some(StreamId(3))).unwrap();
    even.push_stream(Some(StreamId(4))).unwrap();
    even.push_entry(1, 9).unwrap();
    let mut out = Vec::new();
    even.encode(&mut out).unwrap();
    let decoded = TmCache::parse(&out, STREAM).unwrap();
    assert_eq!(decoded.table_offsets(), (20, 24));
    assert_eq!(decoded.entries().next().unwrap().stream, Some(StreamId(4)));

    // A builder without slots writes the header only.
    let mut out = Vec::new();
    TmCacheBuilder::new().encode(&mut out).unwrap();
    assert_eq!(out, words(&[TM_CACHE_VERSION, 20, 0, 20, 0]));
}

#[test]
fn tm_cache_builder_rejects_wide_values_and_bad_slots() {
    let mut builder = TmCacheBuilder::new();
    for id in [0xffff, 0x1_0000] {
        let error = builder.push_stream(Some(StreamId(id))).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    }
    let error = builder.push_entry(0, 1).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
    builder.push_stream(Some(StreamId(0xfffe))).unwrap();
    builder.push_entry(0, 1).unwrap();
    assert_eq!(
        builder.push_entry(1, 1).unwrap_err().kind(),
        ErrorKind::InvalidReference
    );
    for _ in 1..0xffff {
        builder.push_stream(None).unwrap();
    }
    let error = builder.push_stream(None).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
}

fn encode_map(map: &TypeMapStream<'_>) -> Vec<u8> {
    let mut out = Vec::new();
    map.encode(&mut out).unwrap();
    out
}

#[test]
fn type_maps_encode_the_decoded_bytes_again() {
    let server = words(&[1, 2, 3, 0x10ac, 0, 0x1003, 0x1004, 0, 0x1002, 0x1238]);
    let empty = words(&[1, 0, 0]);
    let object = object_map(0xffff, &[0x1000, 0x1003, 0x1108], &[0x1002, 0x1003]);
    let precompiled = object_map(66, &[], &[]);
    let mut padded = object.clone();
    padded[6] = 0xab;
    padded[7] = 0xcd;
    for bytes in [server, empty, object, precompiled, padded] {
        let map = TypeMapStream::parse(&bytes, STREAM).unwrap();
        assert_eq!(encode_map(&map), bytes);
    }
}
