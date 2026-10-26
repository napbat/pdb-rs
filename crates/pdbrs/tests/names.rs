//! Tests for the string table of `/names` and the DBI EC substream.

use pdbrs::ErrorKind;
use pdbrs::native::hash::hash_string_v1;
use pdbrs::native::names::{HashVersion, SIGNATURE, StringEntry, StringTable};

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
    assert_eq!(table.hash_version().value(), 1);
    assert_eq!(table.strings(), strings.as_slice());
    assert_eq!(table.bucket_count(), 7);
    assert_eq!(table.name_count(), 3);
    assert_eq!(table.bucket_offsets().collect::<Vec<_>>(), buckets);
    for (name, &offset) in names.iter().zip(&offsets) {
        assert_eq!(table.get(offset).unwrap(), *name);
        assert_eq!(table.find(name).unwrap(), Some(offset));
    }
    // An offset in the middle of a name returns the rest of the name.
    assert_eq!(table.get(offsets[0] + 1).unwrap(), &first[1..]);
    assert_eq!(table.get(0).unwrap(), b"");
    assert_eq!(table.find(b"absent").unwrap(), None);

    let entries: Vec<StringEntry<'_>> = table.entries().collect();
    assert_eq!(entries.len(), 3);
    for entry in &entries {
        assert_eq!(
            buckets[usize::try_from(entry.bucket).unwrap()],
            entry.offset
        );
        assert_eq!(table.get(entry.offset).unwrap(), entry.name);
    }

    let placement = table.verify_buckets().unwrap();
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
    assert_eq!(table.find(&second).unwrap(), Some(offsets[1]));
    let placement = table.verify_buckets().unwrap();
    assert_eq!((placement.home, placement.probed), (1, 1));
}

#[test]
fn full_table_without_empty_buckets_verifies() {
    let (first, second) = colliding_names(2);
    let (strings, buckets, _) = place(&[&first, &second], 2);
    let bytes = table(1, &strings, &buckets, 2);
    let table = StringTable::parse(&bytes).unwrap();
    assert_eq!(table.find(b"absent").unwrap(), None);
    let placement = table.verify_buckets().unwrap();
    assert_eq!((placement.home, placement.probed), (1, 1));
}

#[test]
fn empty_table_has_no_entries() {
    let bytes = table(1, &[], &[], 0);
    let table = StringTable::parse(&bytes).unwrap();
    assert_eq!(table.bucket_count(), 0);
    assert_eq!(table.entries().count(), 0);
    assert_eq!(table.find(b"a").unwrap(), None);
    assert_eq!(table.verify_buckets().unwrap().home, 0);
    assert_error(table.get(0), ErrorKind::InvalidReference);
}

#[test]
fn v2_table_resolves_offsets_but_not_hashes() {
    let bytes = table(2, b"\0a\0", &[1, 0], 1);
    let table = StringTable::parse(&bytes).unwrap();
    assert_eq!(table.hash_version(), HashVersion::V2);
    assert_eq!(table.get(1).unwrap(), b"a");
    assert_eq!(table.entries().count(), 1);
    assert_error(table.find(b"a"), ErrorKind::UnsupportedVersion);
    assert_error(table.verify_buckets(), ErrorKind::UnsupportedVersion);
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
    assert_error(table.verify_buckets(), ErrorKind::InvalidFormat);
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
    assert_error(table.get(1), ErrorKind::InvalidReference);
    assert_error(table.get(u32::MAX), ErrorKind::InvalidReference);
}
