//! Identity and named-stream table validation.

use pdbrs::ErrorKind;
use pdbrs::native::Identity;
use pdbrs::native::info::Info;
use pdbrs::native::msf::StreamId;

fn word(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in [20_000_404, 123, 7] {
        word(&mut bytes, value);
    }
    bytes.extend_from_slice(&[0x42; 16]);
    let strings = b"/names\0/opaque\xff\0";
    word(&mut bytes, u32::try_from(strings.len()).unwrap());
    bytes.extend_from_slice(strings);
    // Two live buckets and one tombstone in an eight-bucket table.
    for value in [
        2,
        8,
        1,
        0b1000_0010,
        1,
        0b0000_0100,
        7,
        9,
        0,
        8,
        0xdead_beef,
    ] {
        word(&mut bytes, value);
    }
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
    assert_eq!(info.features().collect::<Vec<_>>(), [0xdead_beef]);
}

#[test]
fn validates_bitmaps_and_name_references() {
    let original = fixture();
    let table = 32 + b"/names\0/opaque\xff\0".len();
    let cases = [
        (table, 3, ErrorKind::InvalidFormat),
        (table + 12, 0b1_0000_0010, ErrorKind::InvalidFormat),
        (table + 20, 0b1000_0000, ErrorKind::InvalidFormat),
        (table + 24, 500, ErrorKind::InvalidReference),
        (table + 24, 0, ErrorKind::InvalidFormat),
    ];
    for (offset, value, expected) in cases {
        let mut bytes = original.clone();
        bytes[offset..offset + 4].copy_from_slice(&u32::to_le_bytes(value));
        let error = Info::parse(bytes).unwrap_err();
        assert_eq!(error.kind(), expected);
        assert_eq!(error.stream_location().unwrap().0, 1);
        assert_eq!(error.offset(), None);
    }
}

#[test]
fn rejects_unterminated_names_and_partial_features() {
    let mut bytes = fixture();
    let end_of_strings = 32 + b"/names\0/opaque\xff\0".len();
    bytes[end_of_strings - 1] = b'x';
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
