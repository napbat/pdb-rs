//! Behavior tests for container signature identification.

use core::cell::Cell;

use pdbrs::ErrorKind;
use pdbrs::format::{ContainerKind, PROBE_LEN, probe};
use pdbrs::io::ReadAt;

const NATIVE_MSF7: &[u8] = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0";
const LEGACY_MSF2: &[u8] = b"Microsoft C/C++ program database 2.00\r\n\x1aJG\0\0";
const CLI_METADATA: &[u8] = b"BSJB";
const EMBEDDED_PORTABLE_PDB: &[u8] = b"MPDB";

const SIGNATURES: [(&[u8], ContainerKind); 4] = [
    (NATIVE_MSF7, ContainerKind::NativeMsf7),
    (LEGACY_MSF2, ContainerKind::LegacyMsf2),
    (CLI_METADATA, ContainerKind::CliMetadata),
    (EMBEDDED_PORTABLE_PDB, ContainerKind::EmbeddedPortablePdb),
];

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn expect_kind(source: &(impl ReadAt + ?Sized), expected: ContainerKind, case: &str) {
    match probe(source) {
        Ok(kind) => assert_eq!(kind, expected, "{case}: wrong container kind"),
        Err(error) => panic!("{case}: expected {expected:?}, got error {error}"),
    }
}

fn expect_error(input: &[u8], kind: ErrorKind, offset: u64, case: &str) {
    match probe(input) {
        Ok(found) => panic!("{case}: expected {kind:?}, got {found:?}"),
        Err(error) => {
            assert_eq!(error.kind(), kind, "{case}: wrong error kind");
            assert_eq!(error.offset(), Some(offset), "{case}: wrong error offset");
        }
    }
}

fn length_u64(input: &[u8]) -> u64 {
    u64::try_from(input.len()).expect("test input length fits in u64")
}

#[test]
fn full_signature_alone_is_identified() {
    for (signature, kind) in SIGNATURES {
        expect_kind(
            signature,
            kind,
            &format!("{kind:?} signature without payload"),
        );
    }
}

#[test]
fn arbitrary_bytes_after_signature_do_not_change_the_kind() {
    for (signature, kind) in SIGNATURES {
        let input = concat(&[signature, &[0xA5; 200]]);
        expect_kind(
            input.as_slice(),
            kind,
            &format!("{kind:?} with trailing bytes"),
        );
    }
}

#[test]
fn every_proper_prefix_is_unexpected_eof_at_input_length() {
    let mut seen: Vec<&[u8]> = Vec::new();
    for (signature, _) in SIGNATURES {
        for length in 0..signature.len() {
            let prefix = &signature[..length];
            if seen.contains(&prefix) {
                continue;
            }
            seen.push(prefix);
            let case = format!("prefix b\"{}\"", prefix.escape_ascii());
            expect_error(prefix, ErrorKind::UnexpectedEof, length_u64(prefix), &case);
        }
    }
}

#[test]
fn every_single_byte_mutation_is_invalid_signature() {
    for (signature, kind) in SIGNATURES {
        for index in 0..signature.len() {
            for flip in [0x01_u8, 0x80] {
                let mut input = concat(&[signature, &[0x5A; PROBE_LEN]]);
                input[index] ^= flip;
                let case = format!("{kind:?} byte {index} xor {flip:#04x}");
                expect_error(&input, ErrorKind::InvalidSignature, 0, &case);
            }
        }
    }
}

#[test]
fn unknown_input_is_invalid_signature_at_zero() {
    let cases: [(&str, &[u8]); 7] = [
        (
            "PE image header",
            b"MZ\x90\0\x03\0\0\0\x04\0\0\0\xFF\xFF\0\0",
        ),
        ("short non-prefix", b"X"),
        ("ELF header", b"\x7FELF\x02\x01\x01\0"),
        ("zero bytes", &[0; 64]),
        ("byte-swapped metadata signature", b"BJSB\x01\0\x01\0"),
        (
            "lowercase MSF 7.00 text",
            b"microsoft c/c++ msf 7.00\r\n\x1aDS\0\0\0",
        ),
        ("signature at nonzero offset", b"\0BSJB"),
    ];
    for (case, input) in cases {
        expect_error(input, ErrorKind::InvalidSignature, 0, case);
    }
}

/// A source that reports a large length and records each read.
struct RecordingSource {
    length: u64,
    largest_read_end: Cell<u64>,
}

impl ReadAt for RecordingSource {
    fn len(&self) -> pdbrs::Result<u64> {
        Ok(self.length)
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> pdbrs::Result<()> {
        let end = offset + length_u64(buffer);
        self.largest_read_end
            .set(self.largest_read_end.get().max(end));
        for (position, byte) in buffer.iter_mut().enumerate() {
            let index = usize::try_from(offset).expect("small offset") + position;
            *byte = NATIVE_MSF7.get(index).copied().unwrap_or(0xEE);
        }
        Ok(())
    }
}

#[test]
fn probe_reads_a_bounded_prefix_of_a_large_source() {
    let source = RecordingSource {
        length: u64::MAX,
        largest_read_end: Cell::new(0),
    };
    expect_kind(&source, ContainerKind::NativeMsf7, "large MSF 7.00 source");
    assert!(source.largest_read_end.get() <= u64::try_from(PROBE_LEN).unwrap());
}
