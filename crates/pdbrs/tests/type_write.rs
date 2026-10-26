//! TPI and IPI stream writers: headers, hash values, index offsets, hash
//! adjusters and the two producer layouts.

use pdbrs::ErrorKind;
use pdbrs::native::msf::StreamId;
use pdbrs::native::types::{
    DEFAULT_HASH_BUCKETS, HashAdjuster, HashLayout, IndexOffset, TypeHashes, TypeStream,
    TypeStreamOptions, serialize_hash_adjusters, type_record_hash, write_type_stream,
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

fn options(layout: HashLayout, adjusters: &[HashAdjuster]) -> TypeStreamOptions<'_> {
    TypeStreamOptions {
        index_begin: 0x1000,
        hash_bucket_count: DEFAULT_HASH_BUCKETS,
        hash_stream: Some(StreamId(9)),
        hash_aux_stream: None,
        layout,
        adjusters,
    }
}

/// Write the records, parse the result and verify the hash stream.
fn write_and_verify(layout: HashLayout, adjusters: &[HashAdjuster]) -> (TypeStream, Vec<u8>) {
    let records = records();
    let written = write_type_stream(&records, &options(layout, adjusters)).unwrap();
    let stream = TypeStream::parse(written.stream).unwrap();
    let hash = written.hash_stream.unwrap();
    {
        let hashes = TypeHashes::parse(stream.header(), &hash).unwrap();
        hashes.verify(&stream).unwrap();
        let expected: Vec<u32> = records
            .chunks(1000)
            .map(|record| type_record_hash(record).unwrap() % DEFAULT_HASH_BUCKETS)
            .collect();
        assert_eq!(hashes.hash_values().collect::<Vec<_>>(), expected);
    }
    assert_eq!(stream.record_bytes(), records);
    assert_eq!(HashLayout::of(stream.header()), layout);
    (stream, hash)
}

#[test]
fn microsoft_layout_orders_values_offsets_and_adjusters() {
    let (stream, hash) = write_and_verify(HashLayout::Microsoft, &[]);
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
    assert_eq!(
        hashes.index_offsets().collect::<Vec<_>>(),
        [
            IndexOffset {
                index: 0x1000,
                offset: 0
            },
            IndexOffset {
                index: 0x1009,
                offset: 9000
            },
        ]
    );
}

#[test]
fn llvm_layout_puts_offsets_after_values() {
    let (stream, hash) = write_and_verify(HashLayout::Llvm, &[]);
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
    assert_eq!(
        hashes.index_offsets().collect::<Vec<_>>(),
        [
            IndexOffset {
                index: 0x1000,
                offset: 0
            },
            IndexOffset {
                index: 0x1008,
                offset: 8000
            },
        ]
    );
}

#[test]
fn streams_without_a_hash_stream_follow_the_producer() {
    let records = records();
    for (layout, length) in [(HashLayout::Microsoft, u32::MAX), (HashLayout::Llvm, 0)] {
        let mut options = options(layout, &[]);
        options.hash_stream = None;
        let written = write_type_stream(&records, &options).unwrap();
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
        assert_eq!(HashLayout::of(header), layout);
    }
}

#[test]
fn empty_streams_have_empty_buffers() {
    let written = write_type_stream(&[], &options(HashLayout::Microsoft, &[])).unwrap();
    assert_eq!(written.hash_stream, Some(Vec::new()));
    let stream = TypeStream::parse(written.stream).unwrap();
    assert_eq!(stream.header().record_count(), 0);
    assert_eq!(stream.bytes().len(), 56);
}

#[test]
fn adjusters_follow_the_microsoft_map_layout() {
    let adjusters = [
        HashAdjuster {
            name_offset: 5,
            index: 0x1000,
        },
        HashAdjuster {
            name_offset: 7,
            index: 0x1001,
        },
        HashAdjuster {
            name_offset: 0x1_0003,
            index: 0x1002,
        },
        // The same key again replaces the value.
        HashAdjuster {
            name_offset: 5,
            index: 0x1003,
        },
    ];
    let bytes = serialize_hash_adjusters(&adjusters).unwrap();
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
    let (stream, hash) = write_and_verify(HashLayout::Microsoft, &adjusters[..3]);
    let header = stream.header();
    assert_eq!(header.hash_adjustments.offset, 56);
    let hashes = TypeHashes::parse(header, &hash).unwrap();
    assert_eq!(
        hashes.adjusters(),
        [
            HashAdjuster {
                name_offset: 7,
                index: 0x1001
            },
            HashAdjuster {
                name_offset: 0x1_0003,
                index: 0x1002
            },
            HashAdjuster {
                name_offset: 5,
                index: 0x1000
            },
        ]
    );
}

#[test]
fn many_adjusters_grow_the_map() {
    let adjusters: Vec<HashAdjuster> = (0..100)
        .map(|value| HashAdjuster {
            name_offset: value * 37,
            index: 0x1000 + value % 10,
        })
        .collect();
    let (stream, hash) = write_and_verify(HashLayout::Microsoft, &adjusters);
    let hashes = TypeHashes::parse(stream.header(), &hash).unwrap();
    let mut found: Vec<_> = hashes.adjusters().to_vec();
    found.sort_by_key(|adjuster| adjuster.name_offset);
    assert_eq!(found, adjusters);
}

#[test]
fn rejects_invalid_options_and_records() {
    let records = records();
    let adjusters = [HashAdjuster {
        name_offset: 1,
        index: 0x1000,
    }];
    let error = write_type_stream(&records, &options(HashLayout::Llvm, &adjusters)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    let mut low = options(HashLayout::Microsoft, &[]);
    low.index_begin = 0xfff;
    assert_eq!(
        write_type_stream(&records, &low).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    let mut no_buckets = options(HashLayout::Microsoft, &[]);
    no_buckets.hash_bucket_count = 0;
    assert_eq!(
        write_type_stream(&records, &no_buckets).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    let mut nil = options(HashLayout::Microsoft, &[]);
    nil.hash_stream = Some(StreamId(0xffff));
    assert_eq!(
        write_type_stream(&records, &nil).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    let truncated =
        write_type_stream(&records[..999], &options(HashLayout::Microsoft, &[])).unwrap_err();
    assert_eq!(truncated.kind(), ErrorKind::UnexpectedEof);
    let mut overflow = options(HashLayout::Microsoft, &[]);
    overflow.index_begin = u32::MAX - 5;
    assert_eq!(
        write_type_stream(&records, &overflow).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
}
