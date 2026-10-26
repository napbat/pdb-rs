//! General numeric leaves: every tag, typed values, exact re-encoding and
//! boundaries.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::numeric::{
    Decimal, Integer, IntegerKind, IntegerLeaf, LF_CHAR, LF_COMPLEX32, LF_COMPLEX64, LF_COMPLEX80,
    LF_COMPLEX128, LF_DATE, LF_DECIMAL, LF_LONG, LF_OCTWORD, LF_QUADWORD, LF_REAL16, LF_REAL32,
    LF_REAL48, LF_REAL64, LF_REAL80, LF_REAL128, LF_SHORT, LF_ULONG, LF_UOCTWORD, LF_UQUADWORD,
    LF_USHORT, LF_UTF8STRING, LF_VARSTRING, Numeric,
};

/// Encode a tag and a value.
fn leaf(tag: u16, value: &[u8]) -> Vec<u8> {
    let mut bytes = tag.to_le_bytes().to_vec();
    bytes.extend_from_slice(value);
    bytes
}

/// Decode `encoded` with a suffix, check the suffix and the re-encoding,
/// and check that each shorter prefix is truncated.
fn round_trip(encoded: &[u8]) -> Numeric<'_> {
    let mut with_suffix = encoded.to_vec();
    with_suffix.extend_from_slice(b"name\0");
    let (value, rest) = Numeric::parse_prefix(&with_suffix).unwrap();
    assert_eq!(rest, b"name\0");
    let mut out = Vec::new();
    value.encode(&mut out).unwrap();
    assert_eq!(out, encoded);
    assert_eq!(value.encoded_len(), encoded.len());
    for end in 0..encoded.len() {
        let error = Numeric::parse_prefix(&encoded[..end]).unwrap_err();
        // A truncated UTF-8 string has no terminator.
        let utf8 = end >= 2 && encoded[..2] == LF_UTF8STRING.to_le_bytes();
        let expected = if utf8 {
            ErrorKind::InvalidFormat
        } else {
            ErrorKind::UnexpectedEof
        };
        assert_eq!(error.kind(), expected, "prefix {end}");
    }
    Numeric::parse_prefix(encoded).unwrap().0
}

#[test]
fn integer_leaves_keep_their_encoding_width() {
    for (encoded, kind, value) in [
        (
            vec![0x34, 0x12],
            IntegerKind::Immediate,
            Integer::Unsigned(0x1234),
        ),
        (
            leaf(LF_CHAR, &[0xff]),
            IntegerKind::Char,
            Integer::Signed(-1),
        ),
        (
            leaf(LF_SHORT, &[0x00, 0x80]),
            IntegerKind::Short,
            Integer::Signed(-0x8000),
        ),
        (
            leaf(LF_USHORT, &[0x05, 0x00]),
            IntegerKind::UShort,
            Integer::Unsigned(5),
        ),
        (
            leaf(LF_LONG, &[1, 0, 0, 0]),
            IntegerKind::Long,
            Integer::Signed(1),
        ),
        (
            leaf(LF_ULONG, &[0xff; 4]),
            IntegerKind::ULong,
            Integer::Unsigned(0xffff_ffff),
        ),
        (
            leaf(LF_QUADWORD, &[0xfe; 8]),
            IntegerKind::QuadWord,
            Integer::Signed(-0x0101_0101_0101_0102),
        ),
        (
            leaf(LF_UQUADWORD, &[0xff; 8]),
            IntegerKind::UQuadWord,
            Integer::Unsigned(u128::from(u64::MAX)),
        ),
        (
            leaf(LF_OCTWORD, &[0xff; 16]),
            IntegerKind::OctWord,
            Integer::Signed(-1),
        ),
        (
            leaf(LF_UOCTWORD, &[0xff; 16]),
            IntegerKind::UOctWord,
            Integer::Unsigned(u128::MAX),
        ),
    ] {
        let value_read = round_trip(&encoded);
        let Numeric::Integer(integer) = value_read else {
            panic!("not an integer: {value_read:?}");
        };
        assert_eq!(integer.kind(), kind);
        assert_eq!(integer.value(), value);
        assert_eq!(value_read.integer(), Some(value));
    }
}

#[test]
fn non_canonical_integer_widths_re_encode_exactly() {
    // A value of 1 in an `LF_ULONG` leaf keeps the 4-byte form.
    let encoded = leaf(LF_ULONG, &[1, 0, 0, 0]);
    let value = round_trip(&encoded);
    assert_eq!(value.integer(), Some(Integer::Unsigned(1)));
    assert_eq!(value.tag(), LF_ULONG);
}

#[test]
fn real_and_complex_leaves_keep_exact_bytes() {
    let encoded = leaf(LF_REAL32, &2.5_f32.to_le_bytes());
    assert_eq!(round_trip(&encoded).real32(), Some(2.5));
    let encoded = leaf(LF_REAL64, &(-0.125_f64).to_le_bytes());
    assert_eq!(round_trip(&encoded).real64(), Some(-0.125));
    let encoded = leaf(LF_REAL16, &[0x00, 0x3c]);
    assert_eq!(round_trip(&encoded).real16_bits(), Some(0x3c00));
    // A NaN payload is kept bit for bit.
    let nan = [0x01, 0x00, 0xc0, 0x7f];
    assert_eq!(
        round_trip(&leaf(LF_REAL32, &nan)).value_bytes(),
        Some(&nan[..])
    );
    for (tag, width) in [(LF_REAL48, 6), (LF_REAL80, 10), (LF_REAL128, 16)] {
        let bytes: Vec<u8> = (1..=width).collect();
        let encoded = leaf(tag, &bytes);
        let value = round_trip(&encoded);
        assert_eq!(value.value_bytes(), Some(&bytes[..]));
        assert_eq!(value.tag(), tag);
        assert_eq!(value.integer(), None);
    }
    let mut complex = 1.0_f32.to_le_bytes().to_vec();
    complex.extend_from_slice(&(-2.0_f32).to_le_bytes());
    assert_eq!(
        round_trip(&leaf(LF_COMPLEX32, &complex)).complex32(),
        Some((1.0, -2.0))
    );
    let mut complex = 3.0_f64.to_le_bytes().to_vec();
    complex.extend_from_slice(&4.0_f64.to_le_bytes());
    assert_eq!(
        round_trip(&leaf(LF_COMPLEX64, &complex)).complex64(),
        Some((3.0, 4.0))
    );
    for (tag, width) in [(LF_COMPLEX80, 20), (LF_COMPLEX128, 32)] {
        let bytes: Vec<u8> = (0..width).collect();
        assert_eq!(
            round_trip(&leaf(tag, &bytes)).value_bytes(),
            Some(&bytes[..])
        );
    }
}

#[test]
fn decimal_and_date_leaves_give_their_fields() {
    let mut decimal = 0_u16.to_le_bytes().to_vec();
    decimal.extend_from_slice(&[2, 0x80]);
    decimal.extend_from_slice(&7_u32.to_le_bytes());
    decimal.extend_from_slice(&12345_u64.to_le_bytes());
    let encoded = leaf(LF_DECIMAL, &decimal);
    let value = round_trip(&encoded);
    assert_eq!(
        value.decimal(),
        Some(Decimal {
            reserved: 0,
            scale: 2,
            sign: 0x80,
            high: 7,
            low: 12345,
        })
    );
    let encoded = leaf(LF_DATE, &36526.5_f64.to_le_bytes());
    assert_eq!(round_trip(&encoded).date(), Some(36526.5));
}

/// Roslyn writes the .NET tick count of a VB `Date` constant in `LF_DATE` (#29). The bytes come
/// from the `S_MANCONSTANT` record of `#2024-01-02 13:45:30#` in the `dotnet-vb-full` fixture.
#[test]
fn roslyn_date_leaf_gives_the_tick_count() {
    let bytes = [0x00, 0xb9, 0x98, 0x15, 0x99, 0x0b, 0xdc, 0x08];
    let encoded = leaf(LF_DATE, &bytes);
    let value = round_trip(&encoded);
    assert_eq!(value.date_ticks(), Some(638_397_999_300_000_000));
    assert_eq!(value.value_bytes(), Some(&bytes[..]));
    assert_eq!(Numeric::Real64(bytes).date_ticks(), None);
}

#[test]
fn string_leaves_keep_their_bytes() {
    let mut var = 3_u16.to_le_bytes().to_vec();
    var.extend_from_slice(b"a\0b");
    let encoded = leaf(LF_VARSTRING, &var);
    assert_eq!(round_trip(&encoded).string(), Some(&b"a\0b"[..]));
    let encoded = leaf(LF_UTF8STRING, "é\0".as_bytes());
    assert_eq!(round_trip(&encoded).string(), Some("é".as_bytes()));
    // A UTF-8 string without a terminator is malformed.
    let error = Numeric::parse_prefix(&leaf(LF_UTF8STRING, b"ab")).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
}

#[test]
fn string_encoders_check_their_limits() {
    let long = vec![b'x'; 0x1_0000];
    let error = Numeric::VarString(&long)
        .encode(&mut Vec::new())
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    let error = Numeric::Utf8String(b"a\0b")
        .encode(&mut Vec::new())
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    let mut out = Vec::new();
    Numeric::VarString(&long[..0xffff])
        .encode(&mut out)
        .unwrap();
    assert_eq!(out.len(), 4 + 0xffff);
}

#[test]
fn unknown_tags_are_unsupported() {
    for tag in [0x8011_u16, 0x8016, 0x801d, 0xffff] {
        let error = Numeric::parse_prefix(&leaf(tag, &[0; 16])).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnsupportedVersion, "tag {tag:#x}");
    }
}

#[test]
fn integers_get_the_smallest_encoding() {
    for (value, tag, length) in [
        (Integer::Unsigned(0), 0, 2),
        (Integer::Signed(0x7fff), 0x7fff, 2),
        (Integer::Unsigned(0x8000), LF_USHORT, 4),
        (Integer::Unsigned(0x1_0000), LF_ULONG, 6),
        (Integer::Unsigned(0x1_0000_0000), LF_UQUADWORD, 10),
        (Integer::Unsigned(u128::from(u64::MAX) + 1), LF_UOCTWORD, 18),
        (Integer::Signed(-1), LF_CHAR, 3),
        (Integer::Signed(-0x81), LF_SHORT, 4),
        (Integer::Signed(-0x8001), LF_LONG, 6),
        (Integer::Signed(-0x8000_0001), LF_QUADWORD, 10),
        (Integer::Signed(i128::MIN), LF_OCTWORD, 18),
    ] {
        let numeric = Numeric::from_integer(value);
        assert_eq!(numeric.tag(), tag, "{value:?}");
        assert_eq!(numeric.encoded_len(), length);
        let mut out = Vec::new();
        numeric.encode(&mut out).unwrap();
        let (decoded, _) = Numeric::parse_prefix(&out).unwrap();
        assert_eq!(
            decoded.integer().and_then(Integer::to_i128),
            value.to_i128()
        );
    }
}

#[test]
fn integer_leaf_checks_the_range_of_its_kind() {
    assert!(IntegerLeaf::new(IntegerKind::Immediate, Integer::Unsigned(0x7fff)).is_ok());
    for (kind, value) in [
        (IntegerKind::Immediate, Integer::Unsigned(0x8000)),
        (IntegerKind::Immediate, Integer::Signed(-1)),
        (IntegerKind::Char, Integer::Signed(128)),
        (IntegerKind::UShort, Integer::Unsigned(0x1_0000)),
        (IntegerKind::Long, Integer::Unsigned(0x8000_0000)),
        (IntegerKind::UQuadWord, Integer::Signed(-1)),
        (IntegerKind::OctWord, Integer::Unsigned(u128::MAX)),
    ] {
        let error = IntegerLeaf::new(kind, value).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat, "{kind:?} {value:?}");
    }
    // The value takes the signedness of the encoding.
    let leaf = IntegerLeaf::new(IntegerKind::Long, Integer::Unsigned(5)).unwrap();
    assert_eq!(leaf.value(), Integer::Signed(5));
    let leaf = IntegerLeaf::new(IntegerKind::ULong, Integer::Signed(5)).unwrap();
    assert_eq!(leaf.value(), Integer::Unsigned(5));
}

#[test]
fn integer_parser_rejects_non_integer_tags_without_reading_values() {
    for tag in [LF_REAL32, LF_DATE, LF_VARSTRING, LF_UTF8STRING] {
        let error = Integer::parse_prefix(&tag.to_le_bytes()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnsupportedVersion);
    }
}
