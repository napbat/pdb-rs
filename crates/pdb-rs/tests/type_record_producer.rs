//! Tests for the producer forms of type records that real consumers accept.
//!
//! Microsoft Macro Assembler (`ml64` 14.50) writes `f1` and two stale bytes after the name
//! of some `LF_STRUCTURE` records, instead of the `LF_PAD` run `f3 f2 f1`. The static runtime
//! libraries (`libvcruntime.lib`, `libcmt.lib`) contain these records, and `link.exe` and
//! `lld-link` copy them to the PDB without a change.

use pdb_rs::native::codeview::index::TypeIndex;
use pdb_rs::native::codeview::numeric::{Integer, Numeric};
use pdb_rs::native::codeview::type_record::{
    ClassKind, LF_ARRAY, LF_STRUCTURE, LF_UNION, TypeLeaf, Union,
};
use pdb_rs::native::codeview::{LeafKind, Record};
use pdb_rs::{Bytes, ErrorKind};

/// The `CsFrameEncoded` record of `handlers.obj` in `libvcruntimed.lib` (MSVC 14.51.36231,
/// MASM 14.50.35721), with the length field and the kind.
const CS_FRAME_ENCODED: [u8; 40] = [
    0x26, 0x00, 0x05, 0x15, 0x0a, 0x00, 0x01, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x4c, 0x00, b'C', b's', b'F', b'r', b'a', b'm', b'e', b'E', b'n', b'c',
    b'o', b'd', b'e', b'd', 0x00, 0xf1, 0x6d, 0x65,
];

fn decode(kind: LeafKind, data: &[u8]) -> pdb_rs::Result<TypeLeaf<'_>> {
    TypeLeaf::decode(Record::new(0, kind.0, data))
}

#[test]
fn masm_structure_padding_decodes_and_encodes_to_the_same_bytes() {
    let leaf = decode(LF_STRUCTURE, &CS_FRAME_ENCODED[4..]).unwrap();
    let TypeLeaf::Class(class) = &leaf else {
        panic!("not a structure");
    };
    assert_eq!(class.kind, ClassKind::Structure);
    assert_eq!(class.member_count, 10);
    assert_eq!(class.field_list, TypeIndex(0x1000));
    assert_eq!(class.name, b"CsFrameEncoded");
    assert_eq!(class.size.integer(), Some(Integer::Unsigned(76)));
    assert_eq!(class.trailing, [0xf1, 0x6d, 0x65]);
    assert_eq!(leaf.trailing(), [0xf1, 0x6d, 0x65]);
    let mut out = Vec::new();
    leaf.encode(&mut out).unwrap();
    assert_eq!(out, CS_FRAME_ENCODED);
}

#[test]
fn standard_padding_gives_no_producer_padding() {
    let mut record = CS_FRAME_ENCODED;
    record[37..].copy_from_slice(&[0xf3, 0xf2, 0xf1]);
    let leaf = decode(LF_STRUCTURE, &record[4..]).unwrap();
    assert_eq!(leaf.trailing(), &[] as &[u8]);
    let mut out = Vec::new();
    leaf.encode(&mut out).unwrap();
    assert_eq!(out, record);
}

/// A union with the name `U` and `tail` after it.
fn union(tail: &[u8]) -> Vec<u8> {
    let mut data = vec![
        0x02, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x04, 0x00, b'U', 0,
    ];
    data.extend_from_slice(tail);
    data
}

#[test]
fn every_udt_kind_keeps_the_masm_form() {
    let data = union(&[0xf1, 0x55, 0x00, 0x00]);
    // 12 bytes of fields and the 4-byte prefix are aligned, so 4 more bytes are not padding.
    assert_eq!(
        decode(LF_UNION, &data).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    let mut data = vec![0x02, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x04, 0x00];
    data.extend_from_slice(b"Un\0\xf1\x6d\x65");
    let leaf = decode(LF_UNION, &data).unwrap();
    let TypeLeaf::Union(value) = &leaf else {
        panic!("not a union");
    };
    assert_eq!(value.trailing, [0xf1, 0x6d, 0x65]);
    let mut out = Vec::new();
    leaf.encode(&mut out).unwrap();
    assert_eq!(out[4..], data);
}

#[test]
fn other_trailing_bytes_stay_errors() {
    for tail in [
        &[0x6d, 0x65, 0xf1][..],
        &[0x00, 0xf2, 0xf1],
        &[0xf4, 0x00, 0x00],
    ] {
        let mut record = CS_FRAME_ENCODED;
        record[37..].copy_from_slice(tail);
        assert_eq!(
            decode(LF_STRUCTURE, &record[4..]).unwrap_err().kind(),
            ErrorKind::InvalidFormat,
            "tail {tail:02x?}"
        );
    }
    // The form is accepted only after the names of a UDT record.
    let array = [
        0x74, 0x00, 0x00, 0x00, 0x23, 0x00, 0x00, 0x00, 0x08, 0x00, b'a', b'b', 0x00, 0xf1, 0x6d,
        0x65,
    ];
    assert_eq!(
        decode(LF_ARRAY, &array).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
}

fn union_leaf(producer_padding: &[u8]) -> TypeLeaf<'_> {
    let mut union = Union::default();
    union.member_count = 2;
    union.field_list = TypeIndex(0x1000);
    union.size = Numeric::from_integer(Integer::Unsigned(4));
    union.name = Bytes::new(b"Un");
    union.trailing = Bytes::new(producer_padding);
    TypeLeaf::Union(union)
}

#[test]
fn encoder_rejects_producer_padding_outside_the_masm_form() {
    // `Un\0` ends 17 bytes into the record, so a valid form has 3 bytes.
    for producer_padding in [
        &[0x41, 0x00, 0x00][..],
        &[0xf1, 0x00],
        &[0xf1, 0x00, 0x00, 0x00],
    ] {
        assert_eq!(
            union_leaf(producer_padding)
                .encode(&mut Vec::new())
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidFormat,
            "producer padding {producer_padding:02x?}"
        );
    }
    let mut out = Vec::new();
    union_leaf(&[0xf1, 0x6d, 0x65]).encode(&mut out).unwrap();
    assert_eq!(out.len(), 20);
    assert_eq!(out[17..], [0xf1, 0x6d, 0x65]);
}

#[test]
fn masm_padding_must_end_the_record_on_a_4_byte_boundary() {
    // Without the last stale byte, the record has 39 bytes. The encoder cannot write such a
    // record, so the decoder rejects it (fuzz finding).
    let error = decode(LF_STRUCTURE, &CS_FRAME_ENCODED[4..39]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
}
