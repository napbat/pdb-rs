//! Each type leaf kind: decoding, exact re-encoding and boundaries.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::Record;
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::codeview::numeric::{Integer, Numeric};
use pdbrs::native::codeview::type_record::{
    BasedPointer, Class, ClassKind, ClassOptions, LF_ALIAS, LF_ARGLIST, LF_ARRAY, LF_BARRAY,
    LF_BITFIELD, LF_CLASS, LF_CLASS2, LF_COBOL0, LF_DEFARG, LF_DERIVED, LF_DIMARRAY, LF_DIMCONLU,
    LF_DIMCONU, LF_DIMVARLU, LF_DIMVARU, LF_ENDPRECOMP, LF_ENUM, LF_HLSL, LF_INTERFACE,
    LF_INTERFACE2, LF_LABEL, LF_MANAGED, LF_MATRIX, LF_METHODLIST, LF_MFUNCTION, LF_MODIFIER,
    LF_MODIFIER_EX, LF_OEM, LF_OEM2, LF_POINTER, LF_PRECOMP, LF_PROCEDURE, LF_SKIP,
    LF_STRIDED_ARRAY, LF_STRUCTURE, LF_STRUCTURE2, LF_TYPESERVER, LF_TYPESERVER2, LF_UNION,
    LF_UNION2, LF_VECTOR, LF_VFTABLE, LF_VFTPATH, LF_VTSHAPE, ModifierEx, Pointer,
    PointerAttributes, TypeLeaf, VirtualFunctionTable, VtableShape,
};

/// Concatenate fields.
fn bytes(fields: &[&[u8]]) -> Vec<u8> {
    fields.concat()
}

/// Return a `u32` field.
fn u32le(value: u32) -> [u8; 4] {
    value.to_le_bytes()
}

/// Frame `payload` as a record of `kind` with `LF_PAD` padding.
fn record(kind: u16, payload: &[u8]) -> Vec<u8> {
    let mut body = kind.to_le_bytes().to_vec();
    body.extend_from_slice(payload);
    let count = (4 - (body.len() + 2) % 4) % 4;
    for left in (1..=count).rev() {
        body.push(0xf0 + u8::try_from(left).unwrap());
    }
    let mut full = u16::try_from(body.len()).unwrap().to_le_bytes().to_vec();
    full.extend_from_slice(&body);
    full
}

/// Decode the record of `full`.
fn decode(full: &[u8]) -> TypeLeaf<'_> {
    TypeLeaf::decode(Record {
        offset: 0,
        kind: u16::from_le_bytes([full[2], full[3]]),
        data: &full[4..],
    })
    .unwrap()
}

/// Decode a padded record of `kind`, check that it encodes to the same bytes
/// and that each shorter payload is an error.
fn check(kind: u16, payload: &[u8]) -> Vec<u8> {
    let full = record(kind, payload);
    let leaf = decode(&full);
    assert_eq!(leaf.kind(), kind);
    assert!(!matches!(leaf, TypeLeaf::Unsupported(_)), "{kind:#x}");
    let mut out = vec![0xaa];
    leaf.encode(&mut out).unwrap();
    assert_eq!(out[1..], full[..], "kind {kind:#x}");
    for end in 0..payload.len() {
        let result = TypeLeaf::decode(Record {
            offset: 0,
            kind,
            data: &payload[..end],
        });
        assert!(result.is_err(), "kind {kind:#x} prefix {end} decodes");
    }
    full
}

/// Decode and re-encode a record whose payload keeps raw data at its end.
fn check_raw_tail(kind: u16, fixed: &[u8], tail: &[u8]) -> Vec<u8> {
    let payload = bytes(&[fixed, tail]);
    let mut full = (u16::try_from(payload.len() + 2).unwrap())
        .to_le_bytes()
        .to_vec();
    full.extend_from_slice(&kind.to_le_bytes());
    full.extend_from_slice(&payload);
    let leaf = decode(&full);
    let mut out = Vec::new();
    leaf.encode(&mut out).unwrap();
    assert_eq!(out, full, "kind {kind:#x}");
    for end in 0..fixed.len() {
        let result = TypeLeaf::decode(Record {
            offset: 0,
            kind,
            data: &fixed[..end],
        });
        assert!(result.is_err(), "kind {kind:#x} prefix {end} decodes");
    }
    full
}

#[test]
fn simple_records_round_trip() {
    check(LF_MODIFIER, &bytes(&[&u32le(0x74), &[3, 0]]));
    check(
        LF_PROCEDURE,
        &bytes(&[&u32le(3), &[0x0b, 2, 1, 0], &u32le(0x1000)]),
    );
    check(
        LF_MFUNCTION,
        &bytes(&[
            &u32le(3),
            &u32le(0x1001),
            &u32le(0x1002),
            &[0, 0, 0, 0],
            &u32le(0x1000),
            &u32le(0xffff_fff8),
        ]),
    );
    check(
        LF_ARGLIST,
        &bytes(&[&u32le(2), &u32le(0x74), &u32le(0x1000)]),
    );
    check(LF_BITFIELD, &bytes(&[&u32le(0x75), &[3, 5]]));
    check(
        LF_ARRAY,
        &bytes(&[
            &u32le(0x70),
            &u32le(0x23),
            &[0x02, 0x80, 0x00, 0x90],
            b"a\0",
        ]),
    );
    check(LF_LABEL, &[4, 0]);
    check(LF_VTSHAPE, &[3, 0, 0x55, 0x50]);
    check(
        LF_VFTABLE,
        &bytes(&[
            &u32le(0x1000),
            &u32le(0),
            &u32le(8),
            &u32le(6),
            b"t\0f\0g\0",
        ]),
    );
    // A prefix of a method list can be a shorter valid list, so check only
    // the round trip.
    let methods = bytes(&[
        &[0x13, 0],
        &[0, 0],
        &u32le(0x1003),
        &u32le(8),
        &[0x03, 0],
        &[0, 0],
        &u32le(0x1004),
    ]);
    check_raw_tail(LF_METHODLIST, &[], &methods);
}

#[test]
fn udt_records_round_trip() {
    let names = b"S\0.?AUS@@\0";
    let unique = ClassOptions::HAS_UNIQUE_NAME.to_le_bytes();
    for kind in [LF_CLASS, LF_STRUCTURE, LF_INTERFACE] {
        check(
            kind,
            &bytes(&[
                &[2, 0],
                &unique,
                &u32le(0x1000),
                &u32le(0),
                &u32le(0),
                &[8, 0],
                names,
            ]),
        );
    }
    for kind in [LF_CLASS2, LF_STRUCTURE2, LF_INTERFACE2] {
        let full = check(
            kind,
            &bytes(&[
                &u32le(0x0001_0200),
                &u32le(0x1000),
                &u32le(0),
                &u32le(0),
                &[0x04, 0x80, 0, 0, 1, 0],
                &[8, 0],
                names,
            ]),
        );
        let leaf = decode(&full);
        let TypeLeaf::WideClass(class) = leaf else {
            panic!("not a wide class");
        };
        assert_eq!(class.options, 0x0001_0200);
        assert_eq!(
            class.member_count.integer(),
            Some(Integer::Unsigned(0x1_0000))
        );
        assert_eq!(leaf.tag().unwrap().unique_name, Some(&b".?AUS@@"[..]));
    }
    check(
        LF_UNION,
        &bytes(&[&[1, 0], &[0, 0], &u32le(0x1000), &[4, 0], b"U\0"]),
    );
    check(
        LF_UNION2,
        &bytes(&[&u32le(0), &u32le(0x1000), &[1, 0], &[4, 0], b"U\0"]),
    );
    check(
        LF_ENUM,
        &bytes(&[&[2, 0], &unique, &u32le(0x74), &u32le(0x1000), names]),
    );
}

#[test]
fn pointer_records_round_trip_with_their_base_data() {
    let attributes = |kind: u32, mode: u32| u32le(kind | mode << 5 | 8 << 13);
    check(LF_POINTER, &bytes(&[&u32le(0x74), &attributes(0x0c, 0)]));
    check(
        LF_POINTER,
        &bytes(&[&u32le(0x74), &attributes(0x0c, 2), &u32le(0x1000), &[1, 0]]),
    );
    check(
        LF_POINTER,
        &bytes(&[&u32le(0x74), &attributes(0x03, 0), &[0x34, 0x12]]),
    );
    let symbol = bytes(&[&[6, 0], &[0x0c, 0x11], &[1, 2, 3, 4]]);
    for kind in 4..=7 {
        let full = check(
            LF_POINTER,
            &bytes(&[&u32le(0x74), &attributes(kind, 0), &symbol]),
        );
        let TypeLeaf::Pointer(pointer) = decode(&full) else {
            panic!("not a pointer");
        };
        let base = pointer.based.unwrap();
        assert_eq!(base, BasedPointer::Symbol(&symbol));
        assert_eq!(base.symbol_record().unwrap().kind, 0x110c);
    }
    let full = check(
        LF_POINTER,
        &bytes(&[&u32le(0x74), &attributes(8, 0), &u32le(0x1000), b"base\0"]),
    );
    let TypeLeaf::Pointer(pointer) = decode(&full) else {
        panic!("not a pointer");
    };
    assert_eq!(
        pointer.based,
        Some(BasedPointer::Type {
            base_type: TypeIndex(0x1000),
            name: b"base",
        })
    );
    let full = check(LF_POINTER, &bytes(&[&u32le(0x74), &attributes(9, 0)]));
    assert!(matches!(
        decode(&full),
        TypeLeaf::Pointer(Pointer {
            based: Some(BasedPointer::SelfBased),
            ..
        })
    ));
}

#[test]
fn pointer_encoder_rejects_data_that_does_not_match_the_kind() {
    let near64 = PointerAttributes::from_raw(0x0c | 8 << 13).unwrap();
    let segment = PointerAttributes::from_raw(0x03 | 8 << 13).unwrap();
    for pointer in [
        Pointer {
            referent_type: TypeIndex(0x74),
            attributes: near64,
            member: None,
            based: Some(BasedPointer::Segment(1)),
        },
        Pointer {
            referent_type: TypeIndex(0x74),
            attributes: segment,
            member: None,
            based: None,
        },
        Pointer {
            referent_type: TypeIndex(0x74),
            attributes: segment,
            member: None,
            based: Some(BasedPointer::SelfBased),
        },
        Pointer {
            referent_type: TypeIndex(0x74),
            attributes: segment,
            member: None,
            based: Some(BasedPointer::Symbol(&[2, 0])),
        },
    ] {
        let error = TypeLeaf::Pointer(pointer)
            .encode(&mut Vec::new())
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    }
}

#[test]
fn extended_records_round_trip() {
    check(LF_BARRAY, &u32le(0x74));
    check(
        LF_VFTPATH,
        &bytes(&[&u32le(2), &u32le(0x1000), &u32le(0x1001)]),
    );
    check(LF_DERIVED, &bytes(&[&u32le(1), &u32le(0x1002)]));
    check(
        LF_TYPESERVER,
        &bytes(&[&u32le(0x1234), &u32le(2), b"a.pdb\0"]),
    );
    check(LF_TYPESERVER2, &bytes(&[&[7; 16], &u32le(3), b"b.pdb\0"]));
    check(
        LF_PRECOMP,
        &bytes(&[&u32le(0x1000), &u32le(0x20), &u32le(0xabcd), b"p.pch\0"]),
    );
    check(LF_ENDPRECOMP, &u32le(0xabcd));
    check(LF_DIMARRAY, &bytes(&[&u32le(0x74), &u32le(0x1003), b"d\0"]));
    check(LF_ALIAS, &bytes(&[&u32le(0x74), b"word\0"]));
    check(LF_DEFARG, &bytes(&[&u32le(0x74), b"1 + 2\0"]));
    check(LF_MANAGED, b"System.String\0");
    check(
        LF_STRIDED_ARRAY,
        &bytes(&[&u32le(0x40), &u32le(0x23), &u32le(16), &[64, 0], b"\0"]),
    );
    check(
        LF_VECTOR,
        &bytes(&[&u32le(0x40), &u32le(4), &[16, 0], b"float4\0"]),
    );
    check(
        LF_MATRIX,
        &bytes(&[
            &u32le(0x40),
            &u32le(4),
            &u32le(4),
            &u32le(16),
            &[1],
            &[64, 0],
            b"float4x4\0",
        ]),
    );
    check(
        LF_MODIFIER_EX,
        &bytes(&[&u32le(0x40), &[2, 0], &[1, 0, 0x00, 0x02]]),
    );
    check(
        LF_HLSL,
        &bytes(&[&u32le(0), &[0x10, 0], &[2, 0], &[1, 0, 2, 0], &[16, 0]]),
    );
    let unsigned = bytes(&[&u32le(0x75), &[2, 0], &u32le(10), &u32le(20)]);
    let full = check(LF_DIMCONU, &unsigned);
    let TypeLeaf::ConstantDimensions(dimensions) = decode(&full) else {
        panic!("not dimensions");
    };
    let uppers: Vec<_> = dimensions
        .bounds()
        .map(|bound| (bound.lower, bound.upper))
        .collect();
    assert_eq!(
        uppers,
        [(None, Integer::Unsigned(10)), (None, Integer::Unsigned(20))]
    );
    let signed = bytes(&[
        &u32le(0x11),
        &[1, 0],
        &(-3_i16).to_le_bytes(),
        &5_i16.to_le_bytes(),
    ]);
    let full = check(LF_DIMCONLU, &signed);
    let TypeLeaf::ConstantDimensions(dimensions) = decode(&full) else {
        panic!("not dimensions");
    };
    let bound = dimensions.bounds().next().unwrap();
    assert_eq!(
        (bound.lower, bound.upper),
        (Some(Integer::Signed(-3)), Integer::Signed(5))
    );
    check(
        LF_DIMVARU,
        &bytes(&[&u32le(1), &u32le(0x74), &u32le(0x1004)]),
    );
    check(
        LF_DIMVARLU,
        &bytes(&[&u32le(1), &u32le(0x74), &u32le(0x1004), &u32le(0x1005)]),
    );
}

#[test]
fn raw_tail_records_keep_their_data() {
    check_raw_tail(LF_COBOL0, &u32le(0x1000), &[1, 2, 3]);
    check_raw_tail(LF_SKIP, &u32le(0x1010), &[0; 6]);
    check_raw_tail(
        LF_OEM,
        &bytes(&[&[1, 0], &[2, 0], &u32le(1), &u32le(0x74)]),
        b"oem data",
    );
    let full = check_raw_tail(
        LF_OEM2,
        &bytes(&[&[9; 16], &u32le(1), &u32le(0x1000)]),
        &[0xf3, 0xf2, 0xf1],
    );
    let TypeLeaf::Oem2(oem) = decode(&full) else {
        panic!("not an OEM record");
    };
    assert_eq!(oem.indices.iter().collect::<Vec<_>>(), [TypeIndex(0x1000)]);
    assert_eq!(oem.data, [0xf3, 0xf2, 0xf1]);
}

#[test]
fn dimension_bounds_need_a_primitive_integer_type() {
    for index_type in [0x40_u32, 0x1000, 0x0474] {
        let payload = bytes(&[&u32le(index_type), &[1, 0], &u32le(10)]);
        let error = TypeLeaf::decode(Record {
            offset: 0,
            kind: LF_DIMCONU,
            data: &record(LF_DIMCONU, &payload)[4..],
        })
        .unwrap_err();
        assert_eq!(
            error.kind(),
            ErrorKind::UnsupportedVersion,
            "{index_type:#x}"
        );
    }
}

#[test]
fn shader_records_check_their_counts() {
    // Bits 4 to 15 of the HLSL property word must be zero.
    let payload = bytes(&[&u32le(0), &[0x10, 0], &[0x10, 0], &[16, 0]]);
    let error = TypeLeaf::decode(Record {
        offset: 0,
        kind: LF_HLSL,
        data: &record(LF_HLSL, &payload)[4..],
    })
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    let error = ModifierEx::new(TypeIndex(0x40), &[1, 0, 2]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    let modifiers = ModifierEx::new(TypeIndex(0x40), &[1, 0, 2, 0]).unwrap();
    assert_eq!(modifiers.modifiers().collect::<Vec<_>>(), [1, 2]);
}

#[test]
fn list_constructors_validate_their_bytes() {
    assert_eq!(
        VtableShape::new(3, &[0x55]).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        VtableShape::new(1, &[0x51]).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        VirtualFunctionTable::new(TypeIndex(0x1000), None, 0, b"t")
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidFormat
    );
}

fn class<'a>(name: &'a [u8], unique_name: Option<&'a [u8]>, options: u16) -> TypeLeaf<'a> {
    TypeLeaf::Class(Class {
        kind: ClassKind::Structure,
        member_count: 0,
        options: ClassOptions(options),
        field_list: TypeIndex::NONE,
        derivation_list: TypeIndex::NONE,
        vtable_shape: TypeIndex::NONE,
        size: Numeric::from_integer(Integer::Unsigned(0)),
        name,
        unique_name,
    })
}

#[test]
fn encoders_reject_values_that_do_not_fit() {
    let unique = ClassOptions::HAS_UNIQUE_NAME;
    assert_eq!(
        class(b"S", None, unique)
            .encode(&mut Vec::new())
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        class(b"S", Some(b"u"), 0)
            .encode(&mut Vec::new())
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        class(b"S\0T", None, 0)
            .encode(&mut Vec::new())
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidFormat
    );
    // A record of more than 0xffff bytes after the length field is too long.
    let long = vec![b'x'; 0xfff0];
    let mut out = vec![1, 2, 3];
    let error = class(&long, None, 0).encode(&mut out).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    assert_eq!(out, [1, 2, 3]);
    // The largest aligned record fits: 23 bytes of fields and the name.
    let fits = vec![b'x'; 0x1_0000 - 23];
    let mut out = Vec::new();
    class(&fits, None, 0).encode(&mut out).unwrap();
    assert_eq!(out.len(), 0x1_0000);
    assert_eq!(out[..2], [0xfe, 0xff]);
}

#[test]
fn unsupported_kinds_keep_their_payload() {
    let full = record(0x1522, &[1, 2, 3, 4]);
    let leaf = decode(&full);
    assert!(matches!(leaf, TypeLeaf::Unsupported(_)));
    let mut out = Vec::new();
    leaf.encode(&mut out).unwrap();
    assert_eq!(out, full);
}
