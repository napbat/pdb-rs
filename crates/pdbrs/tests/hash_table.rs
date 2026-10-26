//! Behavior tests for the serialized PDB hash table.

use pdbrs::native::hash_table::{HashTable, HashTableEntry, HashTableValue};
use pdbrs::native::msf::StreamId;
use pdbrs::{Error, ErrorKind, Result};

const STREAM: StreamId = StreamId(42);

/// A two-byte value whose high byte must be zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Small(u8);

impl HashTableValue<'_> for Small {
    const SIZE: usize = 2;

    fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes[1] == 0 {
            Ok(Self(bytes[0]))
        } else {
            Err(Error::new(ErrorKind::InvalidFormat, None).with_context("small value"))
        }
    }
}

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// Three entries in a ten-bucket table: buckets 2, 3 and 9. Bucket 5 is deleted.
fn table() -> Vec<u8> {
    words(&[
        3,
        10,
        1,
        0b10_0000_1100,
        1,
        0b10_0000,
        12,
        120,
        13,
        130,
        29,
        290,
    ])
}

/// Return a hash that places key `k` at bucket `k % 10`, except key 13 at bucket 2.
fn home(entry: &HashTableEntry<u32>) -> u32 {
    if entry.key == 13 { 2 } else { entry.key }
}

#[test]
fn entries_are_in_bucket_order_and_the_end_offset_is_returned() {
    let mut bytes = vec![0xee; 6];
    bytes.extend(table());
    bytes.extend([1, 2, 3]);
    let (table, end) = HashTable::<u32>::parse(&bytes, 6, STREAM).unwrap();
    assert_eq!(end, bytes.len() - 3);
    assert_eq!(table.capacity(), 10);
    assert_eq!(table.len(), 3);
    assert!(!table.is_empty());
    let entries: Vec<(u32, u32, u32)> = table
        .entries()
        .iter()
        .map(|entry| (entry.bucket, entry.key, entry.value))
        .collect();
    assert_eq!(entries, [(2, 12, 120), (3, 13, 130), (9, 29, 290)]);
    assert!(table.is_deleted(5));
    assert!(!table.is_deleted(2));
    assert_eq!(table.deleted_count(), 1);
    table.check_placement(|entry| Ok(home(entry))).unwrap();
}

#[test]
fn placement_follows_tombstones_and_wraps_at_the_capacity() {
    let bytes = table();
    let (table, _) = HashTable::<u32>::parse(&bytes, 0, STREAM).unwrap();
    // Key 29 from bucket 4 passes bucket 4 (empty): not found.
    let error = table
        .check_placement(|entry| Ok(if entry.key == 29 { 4 } else { home(entry) }))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(error.stream_location(), Some((STREAM.0, 24 + 2 * 8)));
    // Key 12 from bucket 9 stops at bucket 0, which is empty.
    assert!(
        table
            .check_placement(|entry| Ok(if entry.key == 12 { 19 } else { home(entry) }))
            .is_err()
    );
    // A hash larger than the capacity is reduced modulo the capacity.
    table
        .check_placement(|entry| Ok(home(entry) + 1000))
        .unwrap();

    // Buckets 9, 0 and 1 are occupied, so a key from bucket 9 reaches bucket 1.
    let wrapped = words(&[3, 10, 1, 0b10_0000_0011, 0, 0, 0, 1, 1, 9, 9]);
    let (table, _) = HashTable::<u32>::parse(&wrapped, 0, STREAM).unwrap();
    table.check_placement(|_| Ok(9)).unwrap();
}

#[test]
fn placement_errors_from_the_hash_are_returned() {
    let bytes = table();
    let (table, _) = HashTable::<u32>::parse(&bytes, 0, STREAM).unwrap();
    let error = table
        .check_placement(|_| Err(Error::new(ErrorKind::InvalidReference, None)))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
}

#[test]
fn placement_in_a_large_sparse_table_is_bounded_by_the_stored_words() {
    // A capacity of u32::MAX with one entry in bucket 0 and no stored words after word 0.
    let bytes = words(&[1, u32::MAX, 1, 1, 0, 7, 70]);
    let (table, _) = HashTable::<u32>::parse(&bytes, 0, STREAM).unwrap();
    table.check_placement(|_| Ok(0)).unwrap();
    assert!(table.check_placement(|_| Ok(u32::MAX - 1)).is_err());
}

#[test]
fn values_of_other_fixed_sizes_decode_and_report_their_location() {
    let mut bytes = words(&[2, 4, 1, 0b0101, 0, 5]);
    bytes.extend([0x11, 0]);
    bytes.extend(2_u32.to_le_bytes());
    bytes.extend([0x22, 0]);
    let (table, end) = HashTable::<Small>::parse(&bytes, 0, STREAM).unwrap();
    assert_eq!(end, bytes.len());
    let values: Vec<_> = table.entries().iter().map(|entry| entry.value).collect();
    assert_eq!(values, [Small(0x11), Small(0x22)]);

    let last = bytes.len() - 1;
    bytes[last] = 1;
    let error = HashTable::<Small>::parse(&bytes, 0, STREAM).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(error.context(), Some("small value"));
    assert_eq!(error.stream_location(), Some((STREAM.0, 30)));
    assert_eq!(error.offset(), None);
}

#[test]
fn structural_rules_are_enforced() {
    let cases: [(&[u32], ErrorKind); 9] = [
        // Zero capacity.
        (&[0, 0, 0, 0], ErrorKind::InvalidFormat),
        // Size above `capacity * 2 / 3 + 1`.
        (&[4, 4, 1, 0b1111, 0], ErrorKind::InvalidFormat),
        // Too many present words.
        (&[0, 10, 2, 0, 0, 0], ErrorKind::InvalidFormat),
        // A present bit after the capacity.
        (&[1, 10, 1, 1 << 10, 0, 0, 0], ErrorKind::InvalidFormat),
        // A deleted bit after the capacity.
        (&[0, 10, 0, 1, 1 << 12], ErrorKind::InvalidFormat),
        // A bucket that is present and deleted.
        (&[1, 10, 1, 1, 1, 1, 0, 0], ErrorKind::InvalidFormat),
        // A size that differs from the present bits.
        (&[2, 10, 1, 1, 0, 0, 0], ErrorKind::InvalidFormat),
        // Entries that extend past the end.
        (&[1, 10, 1, 1, 0, 0], ErrorKind::UnexpectedEof),
        // A truncated bit vector.
        (&[0, 10, 1], ErrorKind::UnexpectedEof),
    ];
    for (values, expected) in cases {
        let bytes = words(values);
        let error = HashTable::<u32>::parse(&bytes, 0, STREAM).unwrap_err();
        assert_eq!(error.kind(), expected, "{values:?}");
        assert_eq!(error.stream_location().map(|(stream, _)| stream), Some(42));
        assert_eq!(error.offset(), None);
    }
    let error = HashTable::<u32>::parse(&[], 4, STREAM).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
}

#[test]
fn an_empty_table_with_one_bucket_is_valid() {
    let bytes = words(&[0, 1, 0, 0]);
    let (table, end) = HashTable::<u32>::parse(&bytes, 0, STREAM).unwrap();
    assert_eq!(end, 16);
    assert!(table.is_empty());
    table.check_placement(|_| unreachable!()).unwrap();
}
