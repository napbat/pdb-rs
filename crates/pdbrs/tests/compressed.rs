//! Tests for ECMA-335 compressed integers.

use pdbrs::ErrorKind;
use pdbrs::portable::compressed::{
    MAX_SIGNED, MAX_UNSIGNED, MIN_SIGNED, decode_signed, decode_unsigned, encode_signed,
    encode_unsigned,
};

/// The unsigned examples of ECMA-335 partition II, section 23.2.
const UNSIGNED: &[(u32, &[u8])] = &[
    (0x03, &[0x03]),
    (0x7f, &[0x7f]),
    (0x80, &[0x80, 0x80]),
    (0x2e57, &[0xae, 0x57]),
    (0x3fff, &[0xbf, 0xff]),
    (0x4000, &[0xc0, 0x00, 0x40, 0x00]),
    (0x1fff_ffff, &[0xdf, 0xff, 0xff, 0xff]),
];

/// The signed examples of ECMA-335 partition II, section 23.2.
const SIGNED: &[(i32, &[u8])] = &[
    (3, &[0x06]),
    (-3, &[0x7b]),
    (64, &[0x80, 0x80]),
    (-64, &[0x01]),
    (8192, &[0xc0, 0x00, 0x40, 0x00]),
    (-8192, &[0x80, 0x01]),
    (268_435_455, &[0xdf, 0xff, 0xff, 0xfe]),
    (-268_435_456, &[0xc0, 0x00, 0x00, 0x01]),
];

#[test]
fn unsigned_examples_round_trip() {
    for &(value, bytes) in UNSIGNED {
        assert_eq!(
            encode_unsigned(value).unwrap().as_bytes(),
            bytes,
            "{value:#x}"
        );
        let mut input = bytes.to_vec();
        input.push(0xaa);
        let (decoded, rest) = decode_unsigned(&input).unwrap();
        assert_eq!(decoded, value);
        assert_eq!(rest, [0xaa]);
    }
}

#[test]
fn signed_examples_round_trip() {
    for &(value, bytes) in SIGNED {
        assert_eq!(encode_signed(value).unwrap().as_bytes(), bytes, "{value}");
        let (decoded, rest) = decode_signed(bytes).unwrap();
        assert_eq!(decoded, value);
        assert_eq!(rest, []);
    }
}

#[test]
fn signed_length_boundaries_round_trip() {
    let cases = [
        (63, 1),
        (-64, 1),
        (64, 2),
        (-65, 2),
        (8191, 2),
        (-8192, 2),
        (8192, 4),
        (-8193, 4),
        (MAX_SIGNED, 4),
        (MIN_SIGNED, 4),
        (0, 1),
        (-1, 1),
    ];
    for (value, len) in cases {
        let encoded = encode_signed(value).unwrap();
        assert_eq!(encoded.as_bytes().len(), len, "{value}");
        assert_eq!(decode_signed(encoded.as_bytes()).unwrap().0, value);
    }
}

#[test]
fn values_outside_the_range_are_rejected() {
    assert_eq!(
        encode_unsigned(MAX_UNSIGNED + 1).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    assert_eq!(
        encode_signed(MAX_SIGNED + 1).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    assert_eq!(
        encode_signed(MIN_SIGNED - 1).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
}

#[test]
fn truncated_and_invalid_input_is_rejected() {
    for bytes in [&[][..], &[0x80], &[0xc0, 0x00, 0x00]] {
        assert_eq!(
            decode_unsigned(bytes).unwrap_err().kind(),
            ErrorKind::UnexpectedEof
        );
        assert_eq!(
            decode_signed(bytes).unwrap_err().kind(),
            ErrorKind::UnexpectedEof
        );
    }
    for first in [0xe0, 0xff] {
        assert_eq!(
            decode_unsigned(&[first, 0, 0, 0]).unwrap_err().kind(),
            ErrorKind::InvalidFormat
        );
    }
}

#[test]
fn longer_encodings_are_accepted() {
    assert_eq!(decode_unsigned(&[0x80, 0x05]).unwrap().0, 5);
    assert_eq!(decode_unsigned(&[0xc0, 0x00, 0x00, 0x05]).unwrap().0, 5);
    assert_eq!(decode_signed(&[0x80, 0x06]).unwrap().0, 3);
}
