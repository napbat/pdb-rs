//! Tests for the embedded-source blob codec.

use std::borrow::Cow;

use pdbrs::ErrorKind;
use pdbrs::portable::embedded::{EmbeddedFormat, EmbeddedSource};

/// Encode `source` in `format` into a new buffer.
fn encode(source: &[u8], format: EmbeddedFormat) -> pdbrs::Result<Vec<u8>> {
    let mut out = Vec::new();
    EmbeddedSource::encode_parts(source, format, &mut out)?;
    Ok(out)
}

/// `abc` as a raw DEFLATE stream with one stored block.
const STORED_ABC: [u8; 8] = [0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c'];

/// `abc` as a raw DEFLATE stream with one fixed-Huffman block (Python `zlib`, `wbits=-15`).
const FIXED_ABC: [u8; 5] = [0x4b, 0x4c, 0x4a, 0x06, 0x00];

/// Return a blob with the format field `format`.
fn blob(format: u32, data: &[u8]) -> Vec<u8> {
    let mut bytes = format.to_le_bytes().to_vec();
    bytes.extend_from_slice(data);
    bytes
}

/// Parse `bytes` and return the decode error kind for `limit`.
fn decode_error(bytes: &[u8], limit: usize) -> ErrorKind {
    EmbeddedSource::parse(bytes)
        .unwrap()
        .decode(limit)
        .unwrap_err()
        .kind()
}

#[test]
fn parses_the_format_field() {
    let raw = blob(0, b"abc");
    assert_eq!(
        EmbeddedSource::parse(&raw).unwrap(),
        EmbeddedSource::Raw(b"abc".into())
    );
    let empty = blob(0, b"");
    assert_eq!(
        EmbeddedSource::parse(&empty).unwrap(),
        EmbeddedSource::Raw(b"".into())
    );
    // The largest positive format is a DEFLATE size.
    let deflate = blob(0x7fff_ffff, &FIXED_ABC);
    let parsed = EmbeddedSource::parse(&deflate).unwrap();
    assert_eq!(
        parsed,
        EmbeddedSource::Deflate {
            size: 0x7fff_ffff,
            data: pdbrs::Bytes::new(&FIXED_ABC)
        }
    );
    assert_eq!(parsed.decoded_size(), 0x7fff_ffff);
    assert_eq!(parsed.format(), EmbeddedFormat::Deflate);
}

#[test]
fn rejects_short_and_reserved_blobs() {
    for length in 0..4 {
        let error = EmbeddedSource::parse(&[0; 3][..length]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof, "length {length}");
        assert_eq!(error.offset(), None);
    }
    for format in [0x8000_0000, u32::MAX] {
        let error = EmbeddedSource::parse(&blob(format, &FIXED_ABC)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnsupportedVersion);
    }
}

#[test]
fn decodes_raw_sources_without_a_copy() {
    let raw = blob(0, b"abc");
    let parsed = EmbeddedSource::parse(&raw).unwrap();
    let source = parsed.decode(3).unwrap();
    assert!(matches!(source, Cow::Borrowed(b"abc")));
    assert_eq!(decode_error(&raw, 2), ErrorKind::LimitExceeded);
    let empty = blob(0, b"");
    assert_eq!(
        &*EmbeddedSource::parse(&empty)
            .unwrap()
            .decode(0)
            .unwrap()
            .into_owned(),
        b""
    );
}

#[test]
fn decodes_deflate_sources() {
    for data in [&STORED_ABC[..], &FIXED_ABC] {
        let bytes = blob(3, data);
        let parsed = EmbeddedSource::parse(&bytes).unwrap();
        let source = parsed.decode(3).unwrap();
        assert!(matches!(source, Cow::Owned(_)));
        assert_eq!(&*source, b"abc");
    }
}

#[test]
fn checks_the_limit_before_deflate_decoding() {
    // The limit applies to the declared size, even when the stream is corrupt.
    assert_eq!(
        decode_error(&blob(4, &[0xff; 4]), 3),
        ErrorKind::LimitExceeded
    );
    assert_eq!(
        decode_error(&blob(0x7fff_ffff, &FIXED_ABC), 1 << 20),
        ErrorKind::LimitExceeded
    );
}

#[test]
fn rejects_deflate_size_mismatches() {
    // The stream gives three bytes, not two or four.
    assert_eq!(
        decode_error(&blob(2, &FIXED_ABC), 16),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        decode_error(&blob(4, &FIXED_ABC), 16),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn rejects_corrupt_deflate_streams() {
    // Block type 3 is reserved.
    assert_eq!(
        decode_error(&blob(3, &[0x07, 0x00]), 16),
        ErrorKind::InvalidFormat
    );
    // The stored block length and its complement differ.
    let mut stored = STORED_ABC;
    stored[3] = 0;
    assert_eq!(
        decode_error(&blob(3, &stored), 16),
        ErrorKind::InvalidFormat
    );
    // Bytes after the final block.
    let mut longer = FIXED_ABC.to_vec();
    longer.push(0);
    assert_eq!(
        decode_error(&blob(3, &longer), 16),
        ErrorKind::InvalidFormat
    );
    // The stream ends before its final block.
    assert_eq!(
        decode_error(&blob(3, &STORED_ABC[..6]), 16),
        ErrorKind::UnexpectedEof
    );
    assert_eq!(decode_error(&blob(3, &[]), 16), ErrorKind::UnexpectedEof);
}

#[test]
fn encodes_raw_and_deflate_blobs() {
    let source = b"embedded source embedded source embedded source";
    assert_eq!(
        encode(source, EmbeddedFormat::Raw).unwrap(),
        blob(0, source)
    );

    let bytes = encode(source, EmbeddedFormat::Deflate).unwrap();
    let length = u32::try_from(source.len()).unwrap();
    assert_eq!(bytes[..4], length.to_le_bytes());
    assert!(bytes.len() < source.len() + 4);
    let parsed = EmbeddedSource::parse(&bytes).unwrap();
    assert_eq!(parsed.format(), EmbeddedFormat::Deflate);
    assert_eq!(&*parsed.decode(source.len()).unwrap(), source);
    // A parsed blob encodes to the same bytes, after the bytes that `out` holds.
    let mut out = vec![0xaa];
    parsed.encode(&mut out).unwrap();
    assert_eq!(out[0], 0xaa);
    assert_eq!(out[1..], bytes);
}

#[test]
fn encodes_an_empty_source_in_the_raw_form() {
    // The format 0 marks the raw form, so DEFLATE cannot store an empty source.
    for format in [EmbeddedFormat::Raw, EmbeddedFormat::Deflate] {
        assert_eq!(encode(b"", format).unwrap(), [0; 4]);
    }
}
