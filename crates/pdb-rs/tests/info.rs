//! Identity and named-stream table validation.

use pdb_rs::ErrorKind;
use pdb_rs::native::Identity;
use pdb_rs::native::info::{Feature, Info};
use pdb_rs::native::msf::StreamId;

const STRINGS: &[u8] = b"/names\0/opaque\xff\0";

/// The offset of the hash table size in [`fixture`].
const TABLE: usize = 32 + STRINGS.len();

fn word(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

/// Build an Info stream with `strings` and the table and trailer words in `table`.
fn info_stream(strings: &[u8], table: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in [20_000_404, 123, 7] {
        word(&mut bytes, value);
    }
    bytes.extend_from_slice(&[0x42; 16]);
    word(&mut bytes, u32::try_from(strings.len()).unwrap());
    bytes.extend_from_slice(strings);
    for &value in table {
        word(&mut bytes, value);
    }
    bytes
}

/// An eight-bucket table. `/names` hashes to bucket 1. `/opaque\xff` hashes to bucket 3,
/// which is deleted, so it is in bucket 4.
fn fixture() -> Vec<u8> {
    let table = [
        2,
        8,
        1,
        0b1_0010,
        1,
        0b1000,
        0,
        8,
        7,
        9,
        // `niMac`, then one feature code.
        3,
        0xdead_beef,
    ];
    info_stream(STRINGS, &table)
}

fn with(offset: usize, value: u32) -> Vec<u8> {
    let mut bytes = fixture();
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    bytes
}

#[test]
fn preserves_identity_feature_codes_and_byte_names() {
    let info = Info::parse(fixture()).unwrap();
    assert_eq!(info.signature(), 123);
    assert_eq!(
        info.identity(),
        Identity::Modern {
            guid: [0x42; 16],
            age: 7
        }
    );
    assert_eq!(info.named_stream(b"/names"), Some(StreamId(8)));
    assert_eq!(info.named_stream(b"/opaque\xff"), Some(StreamId(9)));
    assert_eq!(info.named_stream(b"/absent"), None);
    assert_eq!(info.named_streams().len(), 2);
    assert_eq!(info.named_stream_capacity(), 8);
    assert_eq!(info.next_name_index(), 3);
    assert_eq!(info.features().collect::<Vec<_>>(), [Feature(0xdead_beef)]);
}

#[test]
fn the_name_index_word_is_not_a_feature_code() {
    // Regression: the `niMac` word after the table was reported as feature code 0.
    let mut bytes = fixture();
    bytes.truncate(bytes.len() - 8);
    word(&mut bytes, 0);
    word(&mut bytes, 20_140_508);
    let info = Info::parse(bytes).unwrap();
    assert_eq!(info.next_name_index(), 0);
    assert_eq!(info.features().collect::<Vec<_>>(), [Feature::VC140]);

    let mut bytes = fixture();
    bytes.truncate(bytes.len() - 8);
    let error = Info::parse(bytes).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(
        error.stream_location(),
        Some((StreamId(1), u64::try_from(TABLE + 40).unwrap()))
    );
}

#[test]
fn validates_bitmaps_capacity_and_name_references() {
    let cases = [
        (TABLE, 3, ErrorKind::InvalidFormat),
        (TABLE, 7, ErrorKind::InvalidFormat),
        (TABLE + 4, 0, ErrorKind::InvalidFormat),
        (TABLE + 8, 2, ErrorKind::InvalidFormat),
        (TABLE + 12, 0b1_0001_0010, ErrorKind::InvalidFormat),
        (TABLE + 20, 0b1_1000, ErrorKind::InvalidFormat),
        (TABLE + 24, 500, ErrorKind::InvalidReference),
        (TABLE + 32, 0, ErrorKind::InvalidFormat),
    ];
    for (offset, value, expected) in cases {
        let error = Info::parse(with(offset, value)).unwrap_err();
        assert_eq!(error.kind(), expected, "offset {offset} value {value}");
        assert_eq!(error.stream_location().unwrap().0, StreamId(1));
        assert_eq!(error.offset(), None);
    }
}

#[test]
fn names_outside_their_probe_sequence_are_rejected() {
    // Without the tombstone in bucket 3, a lookup of `/opaque\xff` stops at bucket 3.
    let error = Info::parse(with(TABLE + 20, 0)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(
        error.context(),
        Some("hash table entry is not in its probe sequence")
    );
    assert_eq!(
        error.stream_location(),
        Some((StreamId(1), u64::try_from(TABLE + 32).unwrap()))
    );
}

#[test]
fn probe_sequences_wrap_at_the_capacity() {
    // `/a` and `/b` both hash to bucket 7 of 8. `/b` wraps to bucket 0.
    let table = [2, 8, 1, 0b1000_0001, 0, 3, 5, 0, 4, 0];
    let info = Info::parse(info_stream(b"/a\0/b\0", &table)).unwrap();
    assert_eq!(info.named_stream(b"/a"), Some(StreamId(4)));
    assert_eq!(info.named_stream(b"/b"), Some(StreamId(5)));

    let swapped = [2, 8, 1, 0b0100_0001, 0, 3, 5, 0, 4, 0];
    let error = Info::parse(info_stream(b"/a\0/b\0", &swapped)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
}

#[test]
fn rejects_unterminated_names_and_partial_features() {
    let mut bytes = fixture();
    bytes[TABLE - 1] = b'x';
    assert_eq!(
        Info::parse(bytes).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    let mut bytes = fixture();
    bytes.pop();
    assert_eq!(
        Info::parse(bytes).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn distinguishes_unsupported_layout_from_truncated_header() {
    let mut bytes = fixture();
    bytes[..4].copy_from_slice(&19_950_410_u32.to_le_bytes());
    assert_eq!(
        Info::parse(bytes).unwrap_err().kind(),
        ErrorKind::UnsupportedVersion
    );
    assert_eq!(
        Info::parse(vec![0; 3]).unwrap_err().kind(),
        ErrorKind::UnexpectedEof
    );
}
