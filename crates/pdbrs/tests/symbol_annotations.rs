//! Tests for the binary annotations of inline sites.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::symbol::{Annotation, BinaryAnnotations};

fn decode(bytes: &[u8]) -> Vec<Annotation> {
    BinaryAnnotations::new(bytes).unwrap().iter().collect()
}

fn error(bytes: &[u8]) -> ErrorKind {
    BinaryAnnotations::new(bytes).unwrap_err().kind()
}

#[test]
fn decodes_annotations_that_llvm_reports() {
    // These bytes and results come from `llvm-pdbutil dump --symbols`.
    let annotations = decode(&[
        0x0b, 0x0c, 0x06, 0x21, 0x03, 0x08, 0x06, 0x22, 0x03, 0x77, 0x04, 0x0a,
    ]);
    assert_eq!(
        annotations,
        [
            Annotation::ChangeCodeOffsetAndLineOffset {
                code_delta: 0xc,
                line_delta: 0
            },
            Annotation::ChangeLineOffset(-16),
            Annotation::ChangeCodeOffset(8),
            Annotation::ChangeLineOffset(17),
            Annotation::ChangeCodeOffset(0x77),
            Annotation::ChangeCodeLength(0xa),
        ]
    );
}

#[test]
fn decodes_each_opcode() {
    let annotations = decode(&[
        1, 5, 2, 1, 3, 2, 4, 3, 5, 0x18, 6, 3, 7, 1, 8, 0, 9, 4, 10, 5, 11, 0x33, 12, 6, 7, 13, 9,
    ]);
    assert_eq!(
        annotations,
        [
            Annotation::CodeOffset(5),
            Annotation::ChangeCodeOffsetBase(1),
            Annotation::ChangeCodeOffset(2),
            Annotation::ChangeCodeLength(3),
            Annotation::ChangeFile(0x18),
            Annotation::ChangeLineOffset(-1),
            Annotation::ChangeLineEndDelta(1),
            Annotation::ChangeRangeKind(0),
            Annotation::ChangeColumnStart(4),
            Annotation::ChangeColumnEndDelta(-2),
            Annotation::ChangeCodeOffsetAndLineOffset {
                code_delta: 3,
                line_delta: -1
            },
            Annotation::ChangeCodeLengthAndCodeOffset {
                code_length: 6,
                code_delta: 7
            },
            Annotation::ChangeColumnEnd(9),
        ]
    );
}

#[test]
fn decodes_compressed_integer_lengths() {
    assert_eq!(decode(&[3, 0x7f]), [Annotation::ChangeCodeOffset(0x7f)]);
    assert_eq!(
        decode(&[3, 0x80, 0x8b]),
        [Annotation::ChangeCodeOffset(0x8b)]
    );
    assert_eq!(
        decode(&[3, 0xbf, 0xff]),
        [Annotation::ChangeCodeOffset(0x3fff)]
    );
    assert_eq!(
        decode(&[3, 0xc0, 0x01, 0x02, 0x03]),
        [Annotation::ChangeCodeOffset(0x0001_0203)]
    );
    assert_eq!(
        decode(&[3, 0xdf, 0xff, 0xff, 0xff]),
        [Annotation::ChangeCodeOffset(0x1fff_ffff)]
    );
    assert_eq!(
        decode(&[6, 0xdf, 0xff, 0xff, 0xff]),
        [Annotation::ChangeLineOffset(-0x0fff_ffff)]
    );
}

#[test]
fn accepts_trailing_zero_padding() {
    for bytes in [&[6, 2][..], &[6, 2, 0], &[6, 2, 0, 0], &[6, 2, 0, 0, 0]] {
        let annotations = BinaryAnnotations::new(bytes).unwrap();
        assert_eq!(annotations.bytes(), [6, 2]);
        assert_eq!(annotations.iter().count(), 1);
    }
    for bytes in [&[0, 0][..], &[]] {
        assert_eq!(BinaryAnnotations::new(bytes).unwrap().bytes(), b"");
    }
}

#[test]
fn rejects_invalid_padding() {
    assert_eq!(error(&[6, 2, 0, 0, 0, 0]), ErrorKind::InvalidFormat);
    assert_eq!(error(&[6, 2, 0, 6, 2]), ErrorKind::InvalidFormat);
    // A two-byte encoding of opcode 0 is not padding.
    assert_eq!(error(&[0x80, 0x00]), ErrorKind::InvalidFormat);
}

#[test]
fn rejects_unknown_opcodes() {
    assert_eq!(error(&[14, 1]), ErrorKind::InvalidFormat);
    assert_eq!(error(&[0x80, 0x20, 1]), ErrorKind::InvalidFormat);
}

#[test]
fn rejects_invalid_and_truncated_integers() {
    assert_eq!(error(&[3, 0xe0, 0, 0, 0]), ErrorKind::InvalidFormat);
    assert_eq!(error(&[3]), ErrorKind::InvalidFormat);
    assert_eq!(error(&[3, 0x80]), ErrorKind::InvalidFormat);
    assert_eq!(error(&[3, 0xc0, 1, 2]), ErrorKind::InvalidFormat);
    assert_eq!(error(&[12, 1]), ErrorKind::InvalidFormat);
}

#[test]
fn iterator_works_by_value_and_by_reference() {
    let annotations = BinaryAnnotations::new(&[3, 1, 3, 2]).unwrap();
    assert_eq!((&annotations).into_iter().count(), 2);
    assert_eq!(
        annotations.into_iter().last(),
        Some(Annotation::ChangeCodeOffset(2))
    );
}
