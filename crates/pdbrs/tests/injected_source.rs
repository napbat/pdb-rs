//! Behavior tests for the `/src/headerblock` stream and injected source contents.

use pdbrs::ErrorKind;
use pdbrs::native::hash::hash_buffer_v8;
use pdbrs::native::injected_source::{
    HEADER_BLOCK_STREAM_NAME, SOURCE_FILE_STREAM_PREFIX, SOURCE_VERSION_ONE, SourceCompression,
    SourceHeaderBlock, content_stream_name,
};
use pdbrs::native::msf::StreamId;

const STREAM: StreamId = StreamId(84);
const CONTENTS: [&[u8]; 4] = [b"<a/>", b"<AutoVisualizer/>", b"", b"<b>\r\n</b>"];

/// The keys of the four entries and their buckets in a six-bucket table.
///
/// The keys hash to buckets 1, 3, 4 and 4. The last entry moves to bucket 5.
const ENTRIES: [(u32, u32); 4] = [(1, 44_953), (3, 44_859), (4, 44_764), (5, 45_046)];

fn put(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn entry(content: &[u8], key: u32) -> [u8; 40] {
    let mut bytes = [0; 40];
    let size = u32::try_from(content.len()).unwrap();
    for (offset, value) in [
        (0, 40),
        (4, SOURCE_VERSION_ONE),
        (8, hash_buffer_v8(content)),
        (12, size),
        (16, key - 400),
        (20, 2139),
        (24, key),
    ] {
        put(&mut bytes, offset, value);
    }
    bytes
}

/// Build a header block with `entries` (bucket, key, entry) in a table of `capacity` buckets.
fn block(capacity: u32, entries: &[(u32, u32, [u8; 40])]) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    put(&mut bytes, 0, SOURCE_VERSION_ONE);
    bytes[8..16].copy_from_slice(&0x01dd_64e6_eab4_4902_u64.to_le_bytes());
    put(&mut bytes, 16, 1);
    let mut present = vec![0_u32; usize::try_from(capacity.div_ceil(32)).unwrap()];
    for (bucket, _, _) in entries {
        present[usize::try_from(bucket / 32).unwrap()] |= 1 << (bucket % 32);
    }
    let mut words = vec![u32::try_from(entries.len()).unwrap(), capacity];
    words.push(u32::try_from(present.len()).unwrap());
    words.extend(&present);
    words.push(0);
    for word in words {
        bytes.extend(word.to_le_bytes());
    }
    for (_, key, value) in entries {
        bytes.extend(key.to_le_bytes());
        bytes.extend(value);
    }
    let size = u32::try_from(bytes.len()).unwrap();
    put(&mut bytes, 4, size);
    bytes
}

fn sample() -> Vec<u8> {
    let entries: Vec<_> = ENTRIES
        .iter()
        .zip(CONTENTS)
        .map(|(&(bucket, key), content)| (bucket, key, entry(content, key)))
        .collect();
    block(6, &entries)
}

/// Return the offset of byte `offset` of the entry in table position `index` of [`sample`].
///
/// The 64-byte header and 20 bytes of table header precede the 44-byte keys and entries.
fn entry_offset(index: usize, offset: usize) -> usize {
    64 + 20 + index * 44 + 4 + offset
}

#[test]
fn decodes_the_header_and_verifies_each_source() {
    let bytes = sample();
    let block = SourceHeaderBlock::parse(&bytes, STREAM).unwrap();
    let header = block.header();
    assert_eq!(header.size, 260);
    assert_eq!(header.file_time, 0x01dd_64e6_eab4_4902);
    assert_eq!(header.age, 1);
    assert_eq!(block.entries().len(), 4);
    assert_eq!(block.table().capacity(), 6);
    for ((entry, &(bucket, key)), content) in block.entries().iter().zip(&ENTRIES).zip(CONTENTS) {
        assert_eq!((entry.bucket, entry.key), (bucket, key));
        let source = entry.value;
        assert_eq!(source.virtual_file_name_offset, key);
        assert_eq!(source.file_name_offset, key - 400);
        assert_eq!(source.object_name_offset, 2139);
        assert_eq!(source.compression, SourceCompression::None);
        assert!(!source.is_virtual);
        assert_eq!(source.reserved, [0; 8]);
        assert_eq!(
            source.verify_content(content, StreamId(85)).unwrap(),
            content
        );
    }
}

#[test]
fn content_checks_reject_other_sizes_crcs_and_compression() {
    let bytes = sample();
    let block = SourceHeaderBlock::parse(&bytes, STREAM).unwrap();
    let source = block.entries()[1].value;
    for content in [
        &b"<AutoVisualizer/"[..],
        b"<AutoVisualizer/>\n",
        b"<autoVisualizer/>",
    ] {
        let error = source.verify_content(content, StreamId(86)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat);
        assert_eq!(error.stream_location(), Some((86, 0)));
        assert_eq!(error.offset(), None);
    }
    let mut bytes = sample();
    bytes[entry_offset(1, 28)] = 3;
    let block = SourceHeaderBlock::parse(&bytes, STREAM).unwrap();
    let source = block.entries()[1].value;
    assert_eq!(source.compression, SourceCompression::Lz);
    let error = source
        .verify_content(CONTENTS[1], StreamId(86))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedVersion);
}

#[test]
fn compression_bytes_map_to_the_known_algorithms() {
    let expected = [
        SourceCompression::None,
        SourceCompression::RunLength,
        SourceCompression::Huffman,
        SourceCompression::Lz,
        SourceCompression::Unknown(4),
    ];
    for (raw, compression) in (0..).zip(expected) {
        assert_eq!(SourceCompression::from_raw(raw), compression);
    }
}

#[test]
fn virtual_flag_is_decoded_and_reserved_flag_bits_are_rejected() {
    let mut bytes = sample();
    bytes[entry_offset(0, 29)] = 1;
    let block = SourceHeaderBlock::parse(&bytes, STREAM).unwrap();
    assert!(block.entries()[0].value.is_virtual);

    bytes[entry_offset(0, 29)] = 2;
    let error = SourceHeaderBlock::parse(&bytes, STREAM).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    let location = u64::try_from(entry_offset(0, 0)).unwrap();
    assert_eq!(error.stream_location(), Some((STREAM.0, location)));
}

#[test]
fn header_rules_are_enforced() {
    let cases = [
        (0, SOURCE_VERSION_ONE + 1, ErrorKind::UnsupportedVersion),
        (4, 261, ErrorKind::InvalidFormat),
        (60, 1, ErrorKind::InvalidFormat),
    ];
    for (offset, value, expected) in cases {
        let mut bytes = sample();
        put(&mut bytes, offset, value);
        let error = SourceHeaderBlock::parse(&bytes, STREAM).unwrap_err();
        assert_eq!(error.kind(), expected, "offset {offset}");
        assert_eq!(error.stream_location().unwrap().0, STREAM.0);
        assert_eq!(error.offset(), None);
    }
    let mut bytes = sample();
    bytes.push(0);
    put(&mut bytes, 4, 261);
    let error = SourceHeaderBlock::parse(&bytes, STREAM).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(error.stream_location(), Some((STREAM.0, 260)));

    let error = SourceHeaderBlock::parse(&sample()[..63], STREAM).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
}

#[test]
fn entry_rules_are_enforced() {
    let cases = [
        (0, 41, ErrorKind::InvalidFormat),
        (4, 19_980_828, ErrorKind::UnsupportedVersion),
        (30, 1, ErrorKind::InvalidFormat),
        (24, 44_954, ErrorKind::InvalidFormat),
    ];
    for (offset, value, expected) in cases {
        let mut bytes = sample();
        let at = entry_offset(2, offset);
        if offset == 30 {
            bytes[at] = u8::try_from(value).unwrap();
        } else {
            put(&mut bytes, at, value);
        }
        let error = SourceHeaderBlock::parse(&bytes, STREAM).unwrap_err();
        assert_eq!(error.kind(), expected, "offset {offset}");
    }
}

#[test]
fn entries_must_be_in_their_probe_sequence() {
    // Without the entry in bucket 4, the key that hashes to bucket 4 is not reachable in 5.
    let entries = [
        (1, 44_953, entry(b"", 44_953)),
        (5, 45_046, entry(b"", 45_046)),
    ];
    let error = SourceHeaderBlock::parse(&block(6, &entries), STREAM).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(
        error.context(),
        Some("hash table entry is not in its probe sequence")
    );
}

#[test]
fn keys_hash_as_sixteen_bit_values() {
    // 0x1_0001 truncates to 1, so its home bucket is 1, not 65537 % 6 = 5.
    let key = 0x1_0001;
    let entries = [(1, key, entry(b"", key))];
    SourceHeaderBlock::parse(&block(6, &entries), STREAM).unwrap();
    let entries = [(5, key, entry(b"", key))];
    assert!(SourceHeaderBlock::parse(&block(6, &entries), STREAM).is_err());
}

#[test]
fn content_stream_names_use_the_exact_virtual_name_bytes() {
    assert_eq!(HEADER_BLOCK_STREAM_NAME, b"/src/headerblock");
    let name = content_stream_name(b"c:\\x\\Lib.natvis\xff").unwrap();
    assert_eq!(name, b"/src/files/c:\\x\\Lib.natvis\xff");
    assert!(name.starts_with(SOURCE_FILE_STREAM_PREFIX));
}
