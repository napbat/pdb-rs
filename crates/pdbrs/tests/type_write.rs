//! TPI and IPI stream writers: headers, hash values, index offsets, hash
//! adjusters and the two producer layouts.

use pdbrs::ErrorKind;
use pdbrs::native::Producer;
use pdbrs::native::codeview::index::NameOffset;
use pdbrs::native::msf::StreamId;
use pdbrs::native::types::{
    DEFAULT_HASH_BUCKETS, HashAdjuster, TypeHashes, TypeStream, TypeStreamOptions,
    encode_hash_adjusters, encode_type_stream, type_record_hash,
};

/// An `LF_ARGLIST` record of exactly `size` bytes (a multiple of 4, at least 8).
fn argument_list(size: usize) -> Vec<u8> {
    let count = (size - 8) / 4;
    let mut record = u16::try_from(size - 2).unwrap().to_le_bytes().to_vec();
    record.extend_from_slice(&0x1201_u16.to_le_bytes());
    record.extend_from_slice(&u32::try_from(count).unwrap().to_le_bytes());
    for _ in 0..count {
        record.extend_from_slice(&0x74_u32.to_le_bytes());
    }
    record
}

/// Ten records of 1000 bytes: record 8 crosses the 8 KiB boundary and
/// record 9 starts after it.
fn records() -> Vec<u8> {
    (0..10).flat_map(|_| argument_list(1000)).collect()
}

fn options(producer: Producer, adjusters: &[HashAdjuster]) -> TypeStreamOptions<'_> {
    TypeStreamOptions::default()
        .with_index_begin(0x1000)
        .with_hash_bucket_count(DEFAULT_HASH_BUCKETS)
        .with_hash_stream(Some(StreamId(9)))
        .with_hash_aux_stream(None)
        .with_producer(producer)
        .with_adjusters(adjusters)
}

/// Return an adjuster of the name at `name_offset` and the record `index`.
fn adjuster(name_offset: u32, index: u32) -> HashAdjuster {
    let mut adjuster = HashAdjuster::default();
    adjuster.name_offset = NameOffset(name_offset);
    adjuster.index = index;
    adjuster
}

/// Return the index offset pairs of `hashes` as `(index, offset)` tuples.
fn pairs(hashes: &TypeHashes<'_>) -> Vec<(u32, u32)> {
    hashes
        .index_offsets()
        .map(|pair| (pair.index, pair.offset))
        .collect()
}

/// Write the records, parse the result and check the hash stream.
fn write_and_check(producer: Producer, adjusters: &[HashAdjuster]) -> (TypeStream, Vec<u8>) {
    let records = records();
    let written = encode_type_stream(&records, &options(producer, adjusters)).unwrap();
    let stream = TypeStream::parse(written.stream).unwrap();
    let hash = written.hash_stream.unwrap();
    {
        let hashes = TypeHashes::parse(stream.header(), &hash).unwrap();
        hashes.check(&stream).unwrap();
        let expected: Vec<u32> = records
            .chunks(1000)
            .map(|record| type_record_hash(record).unwrap() % DEFAULT_HASH_BUCKETS)
            .collect();
        assert_eq!(hashes.hash_values().collect::<Vec<_>>(), expected);
    }
    assert_eq!(stream.record_bytes(), records);
    assert_eq!(stream.header().producer(), producer);
    (stream, hash)
}

#[test]
fn default_options_name_the_first_index_and_the_default_buckets() {
    let options = TypeStreamOptions::default();
    assert_eq!(options.index_begin(), 0x1000);
    assert_eq!(options.hash_bucket_count(), DEFAULT_HASH_BUCKETS);
    assert_eq!(options.hash_stream(), None);
    assert_eq!(options.hash_aux_stream(), None);
    assert_eq!(options.producer(), Producer::Microsoft);
    assert_eq!(options.adjusters(), &[] as &[HashAdjuster]);
}

#[test]
fn microsoft_layout_orders_values_offsets_and_adjusters() {
    let (stream, hash) = write_and_check(Producer::Microsoft, &[]);
    let header = stream.header();
    assert_eq!((header.index_begin, header.index_end), (0x1000, 0x100a));
    assert_eq!(header.hash_stream, Some(StreamId(9)));
    assert_eq!(header.hash_aux_stream, None);
    assert_eq!(
        (header.hash_values.offset, header.hash_values.length),
        (0, 40)
    );
    assert_eq!(
        (header.index_offsets.offset, header.index_offsets.length),
        (40, 16)
    );
    assert_eq!(
        (
            header.hash_adjustments.offset,
            header.hash_adjustments.length
        ),
        (56, 0)
    );
    let hashes = TypeHashes::parse(header, &hash).unwrap();
    // A pair starts at the first record that starts in a new 8 KiB block.
    assert_eq!(pairs(&hashes), [(0x1000, 0), (0x1009, 9000)]);
}

#[test]
fn llvm_layout_puts_offsets_after_values() {
    let (stream, hash) = write_and_check(Producer::Llvm, &[]);
    let header = stream.header();
    assert_eq!(
        (header.index_offsets.offset, header.index_offsets.length),
        (40, 16)
    );
    assert_eq!(
        (
            header.hash_adjustments.offset,
            header.hash_adjustments.length
        ),
        (40, 0)
    );
    let hashes = TypeHashes::parse(header, &hash).unwrap();
    // A pair starts at the record that ends in a new 8 KiB block.
    assert_eq!(pairs(&hashes), [(0x1000, 0), (0x1008, 8000)]);
}

#[test]
fn streams_without_a_hash_stream_follow_the_producer() {
    let records = records();
    for (producer, length) in [(Producer::Microsoft, u32::MAX), (Producer::Llvm, 0)] {
        let options = options(producer, &[]).with_hash_stream(None);
        let written = encode_type_stream(&records, &options).unwrap();
        assert_eq!(written.hash_stream, None);
        let stream = TypeStream::parse(written.stream).unwrap();
        let header = stream.header();
        assert_eq!(header.hash_stream, None);
        for buffer in [
            header.hash_values,
            header.index_offsets,
            header.hash_adjustments,
        ] {
            assert_eq!((buffer.offset, buffer.length), (0, length));
        }
        assert_eq!(header.producer(), producer);
    }
}

#[test]
fn empty_streams_have_empty_buffers() {
    let written = encode_type_stream(&[], &options(Producer::Microsoft, &[])).unwrap();
    assert_eq!(written.hash_stream, Some(Vec::new()));
    let stream = TypeStream::parse(written.stream).unwrap();
    assert_eq!(stream.header().record_count(), 0);
    assert_eq!(stream.bytes().len(), 56);
}

#[test]
fn adjusters_follow_the_microsoft_map_layout() {
    let adjusters = [
        adjuster(5, 0x1000),
        adjuster(7, 0x1001),
        adjuster(0x1_0003, 0x1002),
        // The same key again replaces the value.
        adjuster(5, 0x1003),
    ];
    let bytes = encode_hash_adjusters(&adjusters).unwrap();
    let words: Vec<u32> = bytes
        .chunks(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
        .collect();
    // Size 3, 6 slots, one present word with slots 1, 3 and 5, no deleted
    // words, then the entries in slot order.
    assert_eq!(
        words,
        [
            3, 6, 1, 0b10_1010, 0, 7, 0x1001, 0x1_0003, 0x1002, 5, 0x1003
        ]
    );
    let (stream, hash) = write_and_check(Producer::Microsoft, &adjusters[..3]);
    let header = stream.header();
    assert_eq!(header.hash_adjustments.offset, 56);
    let hashes = TypeHashes::parse(header, &hash).unwrap();
    assert_eq!(
        hashes.adjusters(),
        [
            adjuster(7, 0x1001),
            adjuster(0x1_0003, 0x1002),
            adjuster(5, 0x1000),
        ]
    );
}

#[test]
fn many_adjusters_grow_the_map() {
    let adjusters: Vec<HashAdjuster> = (0..100)
        .map(|value| adjuster(value * 37, 0x1000 + value % 10))
        .collect();
    let (stream, hash) = write_and_check(Producer::Microsoft, &adjusters);
    let hashes = TypeHashes::parse(stream.header(), &hash).unwrap();
    let mut found: Vec<_> = hashes.adjusters().to_vec();
    found.sort_by_key(|adjuster| adjuster.name_offset);
    assert_eq!(found, adjusters);
}

#[test]
fn rejects_invalid_options_and_records() {
    let records = records();
    let adjusters = [adjuster(1, 0x1000)];
    let error = encode_type_stream(&records, &options(Producer::Llvm, &adjusters)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    let low = options(Producer::Microsoft, &[]).with_index_begin(0xfff);
    assert_eq!(
        encode_type_stream(&records, &low).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    let no_buckets = options(Producer::Microsoft, &[]).with_hash_bucket_count(0);
    assert_eq!(
        encode_type_stream(&records, &no_buckets)
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidFormat
    );
    let nil = options(Producer::Microsoft, &[]).with_hash_stream(Some(StreamId(0xffff)));
    assert_eq!(
        encode_type_stream(&records, &nil).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    let truncated =
        encode_type_stream(&records[..999], &options(Producer::Microsoft, &[])).unwrap_err();
    assert_eq!(truncated.kind(), ErrorKind::UnexpectedEof);
    let overflow = options(Producer::Microsoft, &[]).with_index_begin(u32::MAX - 5);
    assert_eq!(
        encode_type_stream(&records, &overflow).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
}
