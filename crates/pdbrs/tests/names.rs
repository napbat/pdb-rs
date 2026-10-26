//! Tests for the string table of `/names` and the DBI EC substream.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::index::NameOffset;
use pdbrs::native::hash::{hash_string_v1, hash_string_v2};
use pdbrs::native::names::{HashVersion, SIGNATURE, StringEntry, StringTable, StringTableBuilder};

/// Serialize a string table from its parts.
fn table(version: u32, strings: &[u8], buckets: &[u32], name_count: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&SIGNATURE.to_le_bytes());
    bytes.extend_from_slice(&version.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(strings.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(strings);
    bytes.extend_from_slice(&u32::try_from(buckets.len()).unwrap().to_le_bytes());
    for bucket in buckets {
        bytes.extend_from_slice(&bucket.to_le_bytes());
    }
    bytes.extend_from_slice(&name_count.to_le_bytes());
    bytes
}

/// Lay out names after an empty string and place them with linear probing.
///
/// Return the string buffer, the buckets and the offset of each name.
fn place(names: &[&[u8]], bucket_count: usize) -> (Vec<u8>, Vec<u32>, Vec<u32>) {
    let mut strings = vec![0];
    let mut buckets = vec![0; bucket_count];
    let mut offsets = Vec::new();
    for name in names {
        let offset = u32::try_from(strings.len()).unwrap();
        strings.extend_from_slice(name);
        strings.push(0);
        let mut slot = home(name, bucket_count);
        while buckets[slot] != 0 {
            slot = (slot + 1) % bucket_count;
        }
        buckets[slot] = offset;
        offsets.push(offset);
    }
    (strings, buckets, offsets)
}

fn home(name: &[u8], bucket_count: usize) -> usize {
    usize::try_from(hash_string_v1(name)).unwrap() % bucket_count
}

/// Find a name with `prefix` and a decimal suffix that has home bucket `target`.
fn name_with_home(prefix: &str, bucket_count: usize, target: usize) -> Vec<u8> {
    (0..10_000)
        .map(|index| format!("{prefix}{index}").into_bytes())
        .find(|name| home(name, bucket_count) == target)
        .unwrap()
}

/// Find two distinct names with the same home bucket.
fn colliding_names(bucket_count: usize) -> (Vec<u8>, Vec<u8>) {
    let first = b"name".to_vec();
    let second = name_with_home("name", bucket_count, home(&first, bucket_count));
    (first, second)
}

fn assert_error<T: core::fmt::Debug>(result: pdbrs::Result<T>, kind: ErrorKind) {
    let error = result.unwrap_err();
    assert_eq!(error.kind(), kind);
    assert_eq!(error.offset(), None);
    assert_eq!(error.stream_location(), None);
}

#[test]
fn v1_table_resolves_offsets_and_names() {
    let (first, second) = colliding_names(7);
    let names: [&[u8]; 3] = [&first, &second, b"C:\\src\\\xff.rs"];
    let (strings, buckets, offsets) = place(&names, 7);
    let bytes = table(1, &strings, &buckets, 3);
    let table = StringTable::parse(&bytes).unwrap();

    assert_eq!(table.hash_version(), HashVersion::V1);
    assert_eq!(table.hash_version().raw(), 1);
    assert_eq!(table.strings(), strings.as_slice());
    assert_eq!(table.bucket_count(), 7);
    assert_eq!(table.name_count(), 3);
    assert_eq!(table.bucket_offsets().collect::<Vec<_>>(), buckets);
    for (name, &offset) in names.iter().zip(&offsets) {
        assert_eq!(table.get(NameOffset(offset)).unwrap(), *name);
        assert_eq!(table.find(name).unwrap(), Some(NameOffset(offset)));
    }
    // An offset in the middle of a name returns the rest of the name.
    assert_eq!(table.get(NameOffset(offsets[0] + 1)).unwrap(), &first[1..]);
    assert_eq!(table.get(NameOffset(0)).unwrap(), b"");
    assert_eq!(table.find(b"absent").unwrap(), None);

    let entries: Vec<StringEntry<'_>> = table.entries().collect();
    assert_eq!(entries.len(), 3);
    for entry in &entries {
        assert_eq!(
            buckets[usize::try_from(entry.bucket).unwrap()],
            entry.offset.0
        );
        assert_eq!(table.get(entry.offset).unwrap(), entry.name);
    }

    let placement = table.validate_buckets().unwrap();
    assert_eq!((placement.home, placement.probed), (2, 1));
}

#[test]
fn probing_wraps_past_the_last_bucket() {
    let count = 5;
    let first = name_with_home("w", count, count - 1);
    let second = name_with_home("x", count, count - 1);
    let (strings, buckets, offsets) = place(&[&first, &second], count);
    assert_eq!(buckets[0], offsets[1]);
    let bytes = table(1, &strings, &buckets, 2);
    let table = StringTable::parse(&bytes).unwrap();
    assert_eq!(table.find(&second).unwrap(), Some(NameOffset(offsets[1])));
    let placement = table.validate_buckets().unwrap();
    assert_eq!((placement.home, placement.probed), (1, 1));
}

#[test]
fn full_table_without_empty_buckets_verifies() {
    let (first, second) = colliding_names(2);
    let (strings, buckets, _) = place(&[&first, &second], 2);
    let bytes = table(1, &strings, &buckets, 2);
    let table = StringTable::parse(&bytes).unwrap();
    assert_eq!(table.find(b"absent").unwrap(), None);
    let placement = table.validate_buckets().unwrap();
    assert_eq!((placement.home, placement.probed), (1, 1));
}

#[test]
fn empty_table_has_no_entries() {
    let bytes = table(1, &[], &[], 0);
    let table = StringTable::parse(&bytes).unwrap();
    assert_eq!(table.bucket_count(), 0);
    assert_eq!(table.entries().count(), 0);
    assert_eq!(table.find(b"a").unwrap(), None);
    assert_eq!(table.validate_buckets().unwrap().home, 0);
    assert_error(table.get(NameOffset(0)), ErrorKind::InvalidReference);
}

#[test]
fn v2_table_uses_the_version_2_hash() {
    // Values from the `HasherV2::lhashPbCb` algorithm of Microsoft `misc.h`.
    for (name, hash) in [
        (&b""[..], 0xeb40_4412),
        (b"a", 0x42c5_f9e7),
        (b"abcd", 0x5bce_33cf),
        (b"/names", 0xf8ed_89fb),
        (b"main.cpp", 0x341b_5ab1),
    ] {
        assert_eq!(hash_string_v2(name), hash, "{name:?}");
        assert_eq!(HashVersion::V2.hash(name), hash);
    }
    // `hash_string_v2(b"a") % 2` is 1, so a version 2 lookup starts at bucket 1.
    let bytes = table(2, b"\0a\0", &[0, 1], 1);
    let table = StringTable::parse(&bytes).unwrap();
    assert_eq!(table.hash_version(), HashVersion::V2);
    assert_eq!(table.get(NameOffset(1)).unwrap(), b"a");
    assert_eq!(table.find(b"a").unwrap(), Some(NameOffset(1)));
    assert_eq!(table.find(b"b").unwrap(), None);
    let placement = table.validate_buckets().unwrap();
    assert_eq!((placement.home, placement.probed), (1, 0));
    let moved_bytes = self::table(2, b"\0a\0", &[1, 0], 1);
    let moved = StringTable::parse(&moved_bytes).unwrap();
    assert_error(moved.validate_buckets(), ErrorKind::InvalidFormat);
}

#[test]
fn unreachable_bucket_is_invalid() {
    let name = b"lost";
    let count = 4;
    let slot = (home(name, count) + 2) % count;
    let mut buckets = vec![0; count];
    buckets[slot] = 1;
    let bytes = table(1, b"\0lost\0", &buckets, 1);
    let table = StringTable::parse(&bytes).unwrap();
    assert_error(table.validate_buckets(), ErrorKind::InvalidFormat);
    assert_eq!(table.find(name).unwrap(), None);
}

#[test]
fn header_errors() {
    let valid = table(1, b"\0", &[0], 0);
    let mut signature = valid.clone();
    signature[0] ^= 1;
    assert_error(StringTable::parse(&signature), ErrorKind::InvalidSignature);
    assert_error(
        StringTable::parse(&table(3, b"\0", &[0], 0)),
        ErrorKind::UnsupportedVersion,
    );
    for end in 0..valid.len() {
        assert_error(StringTable::parse(&valid[..end]), ErrorKind::UnexpectedEof);
    }
    let mut trailing = valid;
    trailing.push(0);
    assert_error(StringTable::parse(&trailing), ErrorKind::InvalidFormat);
}

#[test]
fn huge_bucket_count_is_truncated_input() {
    let mut bytes = table(1, b"\0", &[], 0);
    // The bucket count follows the 12-byte header and the 1-byte buffer.
    bytes[13..17].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_error(StringTable::parse(&bytes), ErrorKind::UnexpectedEof);
}

#[test]
fn bucket_and_count_errors() {
    // The name count must equal the number of used buckets.
    assert_error(
        StringTable::parse(&table(1, b"\0a\0", &[1, 0], 2)),
        ErrorKind::InvalidFormat,
    );
    // A bucket past the string buffer.
    assert_error(
        StringTable::parse(&table(1, b"\0a\0", &[3, 0], 1)),
        ErrorKind::InvalidReference,
    );
    // A bucket with a name that has no terminating NUL.
    assert_error(
        StringTable::parse(&table(1, b"\0ab", &[1, 0], 1)),
        ErrorKind::InvalidReference,
    );
    let bytes = table(1, b"\0ab", &[0], 0);
    let table = StringTable::parse(&bytes).unwrap();
    assert_error(table.get(NameOffset(1)), ErrorKind::InvalidReference);
    assert_error(table.get(NameOffset(u32::MAX)), ErrorKind::InvalidReference);
}

#[test]
fn decoded_tables_encode_to_the_same_bytes() {
    let (strings, buckets, _) = place(&[b"one", b"two", b"three"], 7);
    for version in [1, 2] {
        let bytes = table(version, &strings, &buckets, 3);
        let parsed = StringTable::parse(&bytes).unwrap();
        let mut out = Vec::new();
        parsed.encode(&mut out).unwrap();
        assert_eq!(out, bytes);
        let mut copy = Vec::new();
        StringTableBuilder::from_table(&parsed)
            .unwrap()
            .encode(&mut copy)
            .unwrap();
        assert_eq!(copy, bytes);
    }
}

#[test]
fn builder_grows_like_the_microsoft_name_table() {
    for version in [HashVersion::V1, HashVersion::V2] {
        let mut builder = StringTableBuilder::new(version).unwrap();
        assert_eq!(builder.strings(), b"\0");
        let mut counts = Vec::new();
        let mut offsets = Vec::new();
        for index in 0..10 {
            offsets.push(builder.insert(format!("name{index}").as_bytes()).unwrap());
            counts.push(builder.bucket_count());
        }
        // The bucket counts of `NMT::grow`.
        assert_eq!(counts, [2, 4, 4, 7, 7, 11, 11, 11, 17, 17]);
        assert_eq!(offsets[1], NameOffset(7));
        assert_eq!(builder.insert(b"name1").unwrap(), NameOffset(7));
        assert_eq!(builder.name_count(), 10);
        assert_eq!(builder.get(NameOffset(7)), Some(&b"name1"[..]));
        let mut bytes = Vec::new();
        builder.encode(&mut bytes).unwrap();
        let parsed = StringTable::parse(&bytes).unwrap();
        assert_eq!(parsed.hash_version(), version);
        let placement = parsed.validate_buckets().unwrap();
        assert_eq!(placement.home + placement.probed, 10);
        for (index, offset) in offsets.iter().enumerate() {
            let name = format!("name{index}");
            assert_eq!(parsed.find(name.as_bytes()).unwrap(), Some(*offset));
        }
    }
    let mut builder = StringTableBuilder::new(HashVersion::V1).unwrap();
    assert_error(builder.insert(b"a\0b"), ErrorKind::InvalidFormat);
    // As in `NMT`, the empty name gets its own NUL byte; offset 0 marks an empty bucket.
    assert_eq!(builder.insert(b"").unwrap(), NameOffset(1));
    assert_eq!(builder.insert(b"").unwrap(), NameOffset(1));
    assert_eq!(builder.strings(), b"\0\0");
    assert_eq!(builder.name_count(), 1);
    assert_error(
        StringTableBuilder::from_table(&StringTable::parse(&table(1, b"\0", &[], 0)).unwrap()),
        ErrorKind::InvalidFormat,
    );
}
