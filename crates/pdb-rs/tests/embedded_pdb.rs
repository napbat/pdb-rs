//! Tests for the embedded Portable PDB (`MPDB`) envelope.

use pdb_rs::ErrorKind;
use pdb_rs::portable::embedded::{EMBEDDED_PDB_SIGNATURE, decode_embedded, encode_embedded};

/// `abc` as a raw DEFLATE stream with one stored block.
const STORED_ABC: [u8; 8] = [0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c'];

/// `abc` as a raw DEFLATE stream with one fixed-Huffman block (Python `zlib`, `wbits=-15`).
const FIXED_ABC: [u8; 5] = [0x4b, 0x4c, 0x4a, 0x06, 0x00];

/// A large decode limit for the tests that do not test the limit.
const LIMIT: usize = 1 << 20;

/// Return a payload with the uncompressed size `size` and the DEFLATE data `data`.
fn payload(size: u32, data: &[u8]) -> Vec<u8> {
    let mut bytes = EMBEDDED_PDB_SIGNATURE.to_vec();
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(data);
    bytes
}

/// Decode `bytes`, expect an error and return its kind.
fn decode_error(bytes: &[u8], limit: usize) -> ErrorKind {
    let error = decode_embedded(bytes, limit).unwrap_err();
    assert_eq!(error.offset(), None);
    error.kind()
}

/// Return data that starts like a Portable PDB and compresses well.
fn sample_pdb() -> Vec<u8> {
    let mut pdb = b"BSJB".to_vec();
    for index in 0_u32..5_000 {
        pdb.extend_from_slice(&(index % 251).to_le_bytes());
    }
    pdb
}

#[test]
fn decodes_stored_and_fixed_blocks() {
    for data in [&STORED_ABC[..], &FIXED_ABC[..]] {
        assert_eq!(decode_embedded(&payload(3, data), 3).unwrap(), b"abc");
    }
}

#[test]
fn round_trips_encoded_payloads() {
    let pdb = sample_pdb();
    let encoded = encode_embedded(&pdb).unwrap();
    assert_eq!(encoded[..4], EMBEDDED_PDB_SIGNATURE);
    assert_eq!(
        encoded[4..8],
        u32::try_from(pdb.len()).unwrap().to_le_bytes()
    );
    assert!(encoded.len() < pdb.len(), "{} bytes", encoded.len());
    assert_eq!(decode_embedded(&encoded, pdb.len()).unwrap(), pdb);
    // A one-byte PDB is the smallest that the envelope can hold.
    let small = encode_embedded(b"x").unwrap();
    assert_eq!(decode_embedded(&small, 1).unwrap(), b"x");
}

#[test]
fn rejects_bad_signatures() {
    for bytes in [
        &b"MPDA\x03\x00\x00\x00"[..],
        b"BSJB\x03\x00\x00\x00",
        b"mpdb",
        b"MX",
        b"X",
    ] {
        assert_eq!(decode_error(bytes, LIMIT), ErrorKind::InvalidSignature);
    }
}

#[test]
fn rejects_payloads_shorter_than_the_header() {
    let header = payload(3, &[]);
    for length in 0..header.len() {
        assert_eq!(
            decode_error(&header[..length], LIMIT),
            ErrorKind::UnexpectedEof,
            "length {length}"
        );
    }
}

#[test]
fn rejects_reserved_sizes() {
    for size in [0, 0x8000_0000, u32::MAX] {
        assert_eq!(
            decode_error(&payload(size, &FIXED_ABC), usize::MAX),
            ErrorKind::UnsupportedVersion,
            "size {size:#x}"
        );
    }
}

#[test]
fn rejects_size_mismatches() {
    for data in [&STORED_ABC[..], &FIXED_ABC[..]] {
        // The stream gives more bytes than the declared size.
        assert_eq!(
            decode_error(&payload(2, data), LIMIT),
            ErrorKind::InvalidFormat
        );
        // The stream gives fewer bytes than the declared size.
        assert_eq!(
            decode_error(&payload(4, data), LIMIT),
            ErrorKind::InvalidFormat
        );
    }
}

#[test]
fn checks_the_limit_before_decoding() {
    assert_eq!(
        decode_error(&payload(3, &FIXED_ABC), 2),
        ErrorKind::LimitExceeded
    );
    // The largest size fails before the decoder allocates its buffer.
    assert_eq!(
        decode_error(&payload(0x7fff_ffff, &FIXED_ABC), LIMIT),
        ErrorKind::LimitExceeded
    );
    let pdb = sample_pdb();
    let encoded = encode_embedded(&pdb).unwrap();
    assert_eq!(
        decode_error(&encoded, pdb.len() - 1),
        ErrorKind::LimitExceeded
    );
}

#[test]
fn rejects_corrupt_deflate_data() {
    // Block type 3 is reserved.
    assert_eq!(
        decode_error(&payload(3, &[0x07, 0x00]), LIMIT),
        ErrorKind::InvalidFormat
    );
    // The stored block length does not match its one's complement.
    let mut stored = STORED_ABC;
    stored[3] = 0;
    assert_eq!(
        decode_error(&payload(3, &stored), LIMIT),
        ErrorKind::InvalidFormat
    );
    // Bytes follow the final block.
    let mut longer = FIXED_ABC.to_vec();
    longer.push(0);
    assert_eq!(
        decode_error(&payload(3, &longer), LIMIT),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn rejects_truncated_deflate_data() {
    // No DEFLATE data.
    assert_eq!(
        decode_error(&payload(3, &[]), LIMIT),
        ErrorKind::UnexpectedEof
    );
    assert_eq!(
        decode_error(&payload(3, &STORED_ABC[..6]), LIMIT),
        ErrorKind::UnexpectedEof
    );
    let pdb = sample_pdb();
    let encoded = encode_embedded(&pdb).unwrap();
    for length in [9, encoded.len() / 2, encoded.len() - 1] {
        assert_eq!(
            decode_error(&encoded[..length], LIMIT),
            ErrorKind::UnexpectedEof,
            "length {length}"
        );
    }
}

#[test]
fn rejects_empty_pdbs_for_encoding() {
    let error = encode_embedded(b"").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
}
