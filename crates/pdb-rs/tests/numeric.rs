//! Integer numeric leaf values, boundaries and suffix preservation.

use pdb_rs::ErrorKind;
use pdb_rs::native::codeview::numeric::Integer;

fn check(encoded: &[u8], expected: Integer) {
    let mut bytes = encoded.to_vec();
    bytes.extend_from_slice(b"field\0\xf2\xf1");
    let (value, rest) = Integer::parse_prefix(&bytes).unwrap();
    assert_eq!(value, expected);
    assert_eq!(rest, b"field\0\xf2\xf1");
    assert_eq!(rest.as_ptr(), bytes[encoded.len()..].as_ptr());
    for end in 0..encoded.len() {
        let error = Integer::parse_prefix(&encoded[..end]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof, "prefix {end}");
    }
}

#[test]
fn immediate_values_stop_at_the_tag_boundary() {
    for value in [0_u16, 255, 256, 0x7fff] {
        check(&value.to_le_bytes(), Integer::Unsigned(u128::from(value)));
    }
}

#[test]
fn signed_leaves_preserve_negative_values_and_extremes() {
    for (tag, width) in [
        (0x8000_u16, 1),
        (0x8001, 2),
        (0x8003, 4),
        (0x8009, 8),
        (0x8017, 16),
    ] {
        let bits = width * 8;
        let min = i128::MIN >> (128 - bits);
        let max = i128::MAX >> (128 - bits);
        for value in [min, -1, 0, max] {
            let mut encoded = tag.to_le_bytes().to_vec();
            encoded.extend_from_slice(&value.to_le_bytes()[..width]);
            check(&encoded, Integer::Signed(value));
        }
    }
}

#[test]
fn unsigned_leaves_do_not_overflow_signed_storage() {
    for (tag, width) in [(0x8002_u16, 2), (0x8004, 4), (0x800a, 8), (0x8018, 16)] {
        let max = u128::MAX >> (128 - width * 8);
        for value in [0, 0x8000, max] {
            let mut encoded = tag.to_le_bytes().to_vec();
            encoded.extend_from_slice(&value.to_le_bytes()[..width]);
            check(&encoded, Integer::Unsigned(value));
        }
    }
}

#[test]
fn decodes_asymmetric_little_endian_payloads() {
    check(&[0x03, 0x80, 0x11, 0x22, 0, 0], Integer::Signed(0x2211));
    check(
        &[0x04, 0x80, 0xfe, 0xff, 0xff, 0xff],
        Integer::Unsigned(0xffff_fffe),
    );
    check(&[0x01, 0x80, 0xfe, 0xff], Integer::Signed(-2));
}

#[test]
fn non_integer_and_unknown_tags_are_not_guessed() {
    for tag in [
        0x8005_u16, 0x8006, 0x8007, 0x8008, 0x800b, 0x800c, 0x800d, 0x800e, 0x800f, 0x8010, 0x8019,
        0x801a, 0x801b, 0xffff,
    ] {
        let error = Integer::parse_prefix(&tag.to_le_bytes()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnsupportedVersion);
    }
}
