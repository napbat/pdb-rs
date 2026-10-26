//! Behavior tests for the serialized PDB hash table.

use pdbrs::native::hash_table::{
    HashTable, HashTableEntry, HashTableKeys, HashTableProducer, HashTableValue, OwnedHashTable,
};
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

    fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        out.extend([self.0, 0]);
        Ok(())
    }
}

/// Keys that hash to themselves.
struct Identity;

impl HashTableKeys for Identity {
    fn hash(&self, key: u32) -> Result<u32> {
        Ok(key)
    }
}

/// Return the bit vector words and the buckets of an encoded `u32` table.
fn layout(table: &OwnedHashTable<u32>) -> (u32, Vec<u8>, Vec<(u32, u32)>) {
    let mut bytes = Vec::new();
    table.encode(&mut bytes).unwrap();
    let (parsed, end) = HashTable::<u32>::parse(&bytes, 0, STREAM).unwrap();
    assert_eq!(end, bytes.len());
    let entries = parsed
        .entries()
        .iter()
        .map(|entry| (entry.bucket, entry.key))
        .collect();
    let bits = bytes[8..bytes.len() - parsed.len() * 8].to_vec();
    (parsed.capacity(), bits, entries)
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
    assert_eq!(error.stream_location(), Some((STREAM, 24 + 2 * 8)));
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
    assert_eq!(error.stream_location(), Some((STREAM, 30)));
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
        assert_eq!(
            error.stream_location().map(|(stream, _)| stream),
            Some(StreamId(42))
        );
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

#[test]
fn a_decoded_table_encodes_to_the_same_bytes() {
    let bytes = table();
    let (parsed, _) = HashTable::<u32>::parse(&bytes, 0, STREAM).unwrap();
    let mut out = Vec::new();
    parsed.encode(&mut out).unwrap();
    assert_eq!(out, bytes);
    for producer in [HashTableProducer::Microsoft, HashTableProducer::Llvm] {
        let owned = OwnedHashTable::from_table(&parsed, producer).unwrap();
        let mut out = Vec::new();
        owned.encode(&mut out).unwrap();
        assert_eq!(out, bytes);
    }
    let mut small = Vec::new();
    let input = words(&[1, 1, 1, 1, 0, 7, 0x0009]);
    let (parsed, _) = HashTable::<Small>::parse(&input, 0, STREAM).unwrap();
    parsed.encode(&mut small).unwrap();
    assert_eq!(small, [&words(&[1, 1, 1, 1, 0, 7])[..], &[9, 0]].concat());
}

#[test]
fn microsoft_tables_start_with_one_bucket_and_grow_at_two_thirds() {
    let mut table = OwnedHashTable::<u32>::new(HashTableProducer::Microsoft).unwrap();
    assert_eq!(table.capacity(), 1);
    let mut capacities = Vec::new();
    for key in 1..=6 {
        assert_eq!(table.insert(key, key * 10, &Identity).unwrap(), None);
        capacities.push(table.capacity());
    }
    // Growth to `2 * (capacity * 2 / 3 + 1)` when the count reaches `capacity * 2 / 3 + 1`.
    assert_eq!(capacities, [2, 4, 6, 6, 10, 10]);
    let (capacity, bits, entries) = layout(&table);
    assert_eq!(capacity, 10);
    // One present word and no deleted word, as `ISet` after a growth.
    assert_eq!(bits, words(&[1, 0b111_1110, 0]));
    assert_eq!(entries, [(1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (6, 6)]);
    assert_eq!(table.get(4, &Identity).unwrap(), Some(&40));
    assert_eq!(table.insert(4, 41, &Identity).unwrap(), Some(40));
    assert_eq!(table.len(), 6);
}

#[test]
fn llvm_tables_start_with_eight_buckets_and_trim_bit_vectors() {
    let mut table = OwnedHashTable::<u32>::new(HashTableProducer::Llvm).unwrap();
    assert_eq!(table.capacity(), 8);
    let (_, bits, _) = layout(&table);
    assert_eq!(bits, words(&[0, 0]));
    for key in [3, 11] {
        table.insert(key, 0, &Identity).unwrap();
    }
    let (capacity, bits, entries) = layout(&table);
    assert_eq!(capacity, 8);
    // Key 11 collides with key 3 and moves to the next bucket.
    assert_eq!(entries, [(3, 3), (4, 11)]);
    assert_eq!(bits, words(&[1, 0b1_1000, 0]));
    for key in 20..24 {
        table.insert(key, 0, &Identity).unwrap();
    }
    assert_eq!(table.capacity(), 12);
}

#[test]
fn removal_marks_a_deleted_bucket_that_lookups_pass_and_insertions_reuse() {
    for producer in [HashTableProducer::Microsoft, HashTableProducer::Llvm] {
        let mut table = OwnedHashTable::<u32>::with_capacity(producer, 40).unwrap();
        for key in [33, 73] {
            table.insert(key, key, &Identity).unwrap();
        }
        assert_eq!(table.remove(33, &Identity).unwrap(), Some(33));
        assert_eq!(table.remove(33, &Identity).unwrap(), None);
        // Key 73 is after the deleted bucket 33 in its probe sequence.
        assert_eq!(table.get(73, &Identity).unwrap(), Some(&73));
        let (_, bits, entries) = layout(&table);
        assert_eq!(entries, [(34, 73)]);
        assert_eq!(bits, words(&[2, 0, 1 << 2, 2, 0, 1 << 1]));
        table.insert(113, 1, &Identity).unwrap();
        let (_, bits, entries) = layout(&table);
        assert_eq!(entries, [(33, 113), (34, 73)]);
        let deleted = match producer {
            // `ISet::remove` does not shrink the word vector.
            HashTableProducer::Microsoft => words(&[2, 0, 0]),
            HashTableProducer::Llvm => words(&[0]),
        };
        assert_eq!(bits, [&words(&[2, 0, 0b110])[..], &deleted].concat());
    }
    let error = OwnedHashTable::<u32>::with_capacity(HashTableProducer::Llvm, 0).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
}
