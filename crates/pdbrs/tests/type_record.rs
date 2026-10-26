//! Tests for typed TPI leaf decoding.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::codeview::numeric::Integer;
use pdbrs::native::codeview::type_record::{
    BasedPointer, CallingConvention, ClassKind, ClassOptions, FunctionOptions, HfaKind, LF_ARGLIST,
    LF_ARRAY, LF_BITFIELD, LF_CLASS, LF_ENUM, LF_INTERFACE, LF_LABEL, LF_METHODLIST, LF_MFUNCTION,
    LF_MODIFIER, LF_POINTER, LF_PROCEDURE, LF_STRUCTURE, LF_UNION, LF_VFTABLE, LF_VTSHAPE,
    LabelMode, MemberAccess, MemberAttributes, MemberPointerRepresentation, MethodKind, MocomKind,
    PointerAttributes, PointerKind, PointerMode, TypeLeaf, VtableSlotKind,
};
use pdbrs::native::codeview::{LeafKind, Record};

/// Add `LF_PAD` bytes so that `payload` plus the 4-byte prefix is aligned to 4.
fn padded(mut payload: Vec<u8>) -> Vec<u8> {
    let count = (4 - payload.len() % 4) % 4;
    for left in (1..=count).rev() {
        payload.push(0xf0 + u8::try_from(left).unwrap());
    }
    payload
}

fn decode(kind: LeafKind, data: &[u8]) -> TypeLeaf<'_> {
    TypeLeaf::decode(Record::new(0, kind.0, data)).unwrap()
}

fn decode_error(kind: LeafKind, data: &[u8]) -> ErrorKind {
    TypeLeaf::decode(Record::new(0, kind.0, data))
        .unwrap_err()
        .kind()
}

/// Concatenate little-endian fields.
fn bytes(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

#[test]
fn decodes_modifier() {
    let data = padded(bytes(&[&0x1003_u32.to_le_bytes(), &3_u16.to_le_bytes()]));
    assert_eq!(data.len(), 8);
    let TypeLeaf::Modifier(modifier) = decode(LF_MODIFIER, &data) else {
        panic!("not a modifier");
    };
    assert_eq!(modifier.modified_type, TypeIndex(0x1003));
    assert!(modifier.options.is_const());
    assert!(modifier.options.is_volatile());
    assert!(!modifier.options.is_unaligned());
}

#[test]
fn decodes_pointer_attributes() {
    // Kind 0x0c (64-bit), mode 1 (reference), const, size 8, `&&` this pointer.
    let attributes = 0x0c | (1 << 5) | 0x400 | (8 << 13) | 0x0020_0000;
    let data = bytes(&[&0x0074_u32.to_le_bytes(), &u32::to_le_bytes(attributes)]);
    let TypeLeaf::Pointer(pointer) = decode(LF_POINTER, &data) else {
        panic!("not a pointer");
    };
    assert_eq!(pointer.referent_type, TypeIndex(0x74));
    let attributes = pointer.attributes;
    assert_eq!(attributes.kind(), PointerKind::Near64);
    assert_eq!(attributes.mode(), PointerMode::LValueReference);
    assert_eq!(attributes.size(), 8);
    assert!(attributes.contains(PointerAttributes::CONST));
    assert!(attributes.contains(PointerAttributes::RVALUE_REF_THIS_POINTER));
    assert!(!attributes.contains(PointerAttributes::VOLATILE));
    assert_eq!(pointer.member, None);
    assert_eq!(pointer.based, None);
}

#[test]
fn decodes_member_pointer_information() {
    let attributes: u32 = 0x0c | (3 << 5) | (8 << 13);
    let data = padded(bytes(&[
        &0x1001_u32.to_le_bytes(),
        &attributes.to_le_bytes(),
        &0x1002_u32.to_le_bytes(),
        &8_u16.to_le_bytes(),
    ]));
    let TypeLeaf::Pointer(pointer) = decode(LF_POINTER, &data) else {
        panic!("not a pointer");
    };
    assert_eq!(
        pointer.attributes.mode(),
        PointerMode::PointerToMemberFunction
    );
    let member = pointer.member.unwrap();
    assert_eq!(member.containing_type, TypeIndex(0x1002));
    assert_eq!(
        member.representation,
        MemberPointerRepresentation::GeneralFunction
    );
    // The member information is required for a member-pointer mode.
    assert_eq!(
        decode_error(LF_POINTER, &data[..10]),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn decodes_segment_based_pointer_data() {
    let attributes: u32 = 0x03 | (4 << 13);
    let data = bytes(&[
        &0x0074_u32.to_le_bytes(),
        &attributes.to_le_bytes(),
        &[0x12, 0x34, 0xf2, 0xf1],
    ]);
    let TypeLeaf::Pointer(pointer) = decode(LF_POINTER, &data) else {
        panic!("not a pointer");
    };
    assert_eq!(pointer.attributes.kind(), PointerKind::BasedOnSegment);
    assert_eq!(pointer.based, Some(BasedPointer::Segment(0x3412)));
}

#[test]
fn rejects_unknown_pointer_kind_and_mode() {
    for attributes in [0x0d_u32, 5 << 5] {
        let data = bytes(&[&0x0074_u32.to_le_bytes(), &attributes.to_le_bytes()]);
        assert_eq!(decode_error(LF_POINTER, &data), ErrorKind::InvalidFormat);
    }
}

#[test]
fn decodes_procedure_and_member_function() {
    let data = bytes(&[
        &0x0003_u32.to_le_bytes(),
        &[0x00, FunctionOptions::CXX_RETURN_UDT.0],
        &2_u16.to_le_bytes(),
        &0x1004_u32.to_le_bytes(),
    ]);
    let TypeLeaf::Procedure(procedure) = decode(LF_PROCEDURE, &data) else {
        panic!("not a procedure");
    };
    assert_eq!(procedure.return_type, TypeIndex(3));
    assert_eq!(procedure.calling_convention, CallingConvention::NearC);
    assert!(procedure.options.contains(FunctionOptions::CXX_RETURN_UDT));
    assert_eq!(procedure.parameter_count, 2);
    assert_eq!(procedure.argument_list, TypeIndex(0x1004));

    let data = bytes(&[
        &0x0003_u32.to_le_bytes(),
        &0x1005_u32.to_le_bytes(),
        &0x1006_u32.to_le_bytes(),
        &[0x0b, 0x00],
        &0_u16.to_le_bytes(),
        &0x1007_u32.to_le_bytes(),
        &(-8_i32).to_le_bytes(),
    ]);
    let TypeLeaf::MemberFunction(function) = decode(LF_MFUNCTION, &data) else {
        panic!("not a member function");
    };
    assert_eq!(function.class_type, TypeIndex(0x1005));
    assert_eq!(function.this_type, TypeIndex(0x1006));
    assert_eq!(function.calling_convention, CallingConvention::ThisCall);
    assert_eq!(function.argument_list, TypeIndex(0x1007));
    assert_eq!(function.this_adjustment, -8);
}

#[test]
fn rejects_unknown_calling_convention() {
    let data = bytes(&[
        &0x0003_u32.to_le_bytes(),
        &[0x1a, 0x00],
        &0_u16.to_le_bytes(),
        &0x1004_u32.to_le_bytes(),
    ]);
    assert_eq!(decode_error(LF_PROCEDURE, &data), ErrorKind::InvalidFormat);
}

#[test]
fn decodes_argument_list() {
    let data = bytes(&[
        &2_u32.to_le_bytes(),
        &0x0074_u32.to_le_bytes(),
        &0x1001_u32.to_le_bytes(),
    ]);
    let TypeLeaf::ArgumentList(list) = decode(LF_ARGLIST, &data) else {
        panic!("not an argument list");
    };
    assert_eq!(list.len(), 2);
    assert!(!list.is_empty());
    assert_eq!(
        list.iter().collect::<Vec<_>>(),
        [TypeIndex(0x74), TypeIndex(0x1001)]
    );
    // The count needs more bytes than the payload has.
    let data = bytes(&[&3_u32.to_le_bytes(), &[0; 8]]);
    assert_eq!(decode_error(LF_ARGLIST, &data), ErrorKind::InvalidFormat);
    let data = u32::MAX.to_le_bytes();
    assert_eq!(decode_error(LF_ARGLIST, &data), ErrorKind::InvalidFormat);
}

#[test]
fn decodes_bitfield() {
    let data = padded(bytes(&[&0x0075_u32.to_le_bytes(), &[3, 5]]));
    let TypeLeaf::Bitfield(bitfield) = decode(LF_BITFIELD, &data) else {
        panic!("not a bit field");
    };
    assert_eq!(bitfield.underlying_type, TypeIndex(0x75));
    assert_eq!((bitfield.bit_size, bitfield.bit_offset), (3, 5));
}

#[test]
fn decodes_method_list_with_virtual_offsets() {
    // Public introducing virtual method, then a public vanilla method.
    let data = bytes(&[
        &(3_u16 | (4 << 2)).to_le_bytes(),
        &0_u16.to_le_bytes(),
        &0x1010_u32.to_le_bytes(),
        &16_i32.to_le_bytes(),
        &3_u16.to_le_bytes(),
        &0_u16.to_le_bytes(),
        &0x1011_u32.to_le_bytes(),
    ]);
    let TypeLeaf::MethodList(list) = decode(LF_METHODLIST, &data) else {
        panic!("not a method list");
    };
    let entries: Vec<_> = list.entries().map(Result::unwrap).collect();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].attributes.method_kind(),
        MethodKind::IntroducingVirtual
    );
    assert_eq!(entries[0].method_type, TypeIndex(0x1010));
    assert_eq!(entries[0].vtable_offset, Some(16));
    assert_eq!(entries[1].attributes.access(), MemberAccess::Public);
    assert_eq!(entries[1].vtable_offset, None);
}

#[test]
fn rejects_malformed_method_list() {
    let mut data = bytes(&[
        &3_u16.to_le_bytes(),
        &1_u16.to_le_bytes(),
        &0x1011_u32.to_le_bytes(),
    ]);
    assert_eq!(decode_error(LF_METHODLIST, &data), ErrorKind::InvalidFormat);
    // An introducing virtual method without its offset.
    data[0] = 3 | (4 << 2);
    data[2] = 0;
    assert_eq!(decode_error(LF_METHODLIST, &data), ErrorKind::InvalidFormat);
    // Method property 7 has no meaning.
    data[0] = 3 | (7 << 2);
    assert_eq!(decode_error(LF_METHODLIST, &data), ErrorKind::InvalidFormat);
}

#[test]
fn decodes_array_with_numeric_leaf_size() {
    // `LF_ULONG` 0x0001_0000 as the size, then an empty name.
    let data = padded(bytes(&[
        &0x0070_u32.to_le_bytes(),
        &0x0023_u32.to_le_bytes(),
        &0x8004_u16.to_le_bytes(),
        &0x0001_0000_u32.to_le_bytes(),
        &[0],
    ]));
    let TypeLeaf::Array(array) = decode(LF_ARRAY, &data) else {
        panic!("not an array");
    };
    assert_eq!(array.element_type, TypeIndex(0x70));
    assert_eq!(array.index_type, TypeIndex(0x23));
    assert_eq!(array.size.integer(), Some(Integer::Unsigned(0x0001_0000)));
    assert_eq!(array.name, b"");
}

#[test]
fn decodes_real_size_and_rejects_unknown_numeric_tag() {
    // `LF_REAL32` decodes and keeps its bytes.
    let data = bytes(&[
        &0x0070_u32.to_le_bytes(),
        &0x0023_u32.to_le_bytes(),
        &0x8005_u16.to_le_bytes(),
        &1.5_f32.to_le_bytes(),
        &[0],
    ]);
    let TypeLeaf::Array(array) = decode(LF_ARRAY, &data) else {
        panic!("not an array");
    };
    assert_eq!(array.size.real32(), Some(1.5));
    // Tag 0x8011 is not defined.
    let data = bytes(&[
        &0x0070_u32.to_le_bytes(),
        &0x0023_u32.to_le_bytes(),
        &0x8011_u16.to_le_bytes(),
        &[0; 4],
        &[0],
    ]);
    assert_eq!(decode_error(LF_ARRAY, &data), ErrorKind::UnsupportedVersion);
}

fn class_payload(options: u16, names: &[u8]) -> Vec<u8> {
    padded(bytes(&[
        &2_u16.to_le_bytes(),
        &options.to_le_bytes(),
        &0x1002_u32.to_le_bytes(),
        &0_u32.to_le_bytes(),
        &0x1003_u32.to_le_bytes(),
        &24_u16.to_le_bytes(),
        names,
    ]))
}

#[test]
fn decodes_class_kinds_and_unique_names() {
    let options = ClassOptions::HAS_UNIQUE_NAME.0 | ClassOptions::NESTED.0;
    let data = class_payload(options, b"a::B\0.?AUB@a@@\0");
    for (kind, expected) in [
        (LF_CLASS, ClassKind::Class),
        (LF_STRUCTURE, ClassKind::Structure),
        (LF_INTERFACE, ClassKind::Interface),
    ] {
        let TypeLeaf::Class(class) = decode(kind, &data) else {
            panic!("not a class");
        };
        assert_eq!(class.kind, expected);
        assert_eq!(class.member_count, 2);
        assert!(class.options.has_unique_name());
        assert!(class.options.is_nested());
        assert!(!class.options.is_forward_reference());
        assert_eq!(class.field_list, TypeIndex(0x1002));
        assert_eq!(class.derivation_list, TypeIndex::NONE);
        assert_eq!(class.vtable_shape, TypeIndex(0x1003));
        assert_eq!(class.size.integer(), Some(Integer::Unsigned(24)));
        assert_eq!(class.name, b"a::B");
        assert_eq!(class.unique_name, Some(&b".?AUB@a@@"[..]));
        let tag = decode(kind, &data).tag().unwrap();
        assert_eq!(tag.name, b"a::B");
        assert_eq!(tag.options, ClassOptions(options));
    }
    // Without the unique-name flag, the second name is trailing data.
    let data = class_payload(0, b"a::B\0.?AUB@a@@\0");
    assert_eq!(decode_error(LF_STRUCTURE, &data), ErrorKind::InvalidFormat);
    // With the flag, the unique name is required.
    let data = class_payload(ClassOptions::HAS_UNIQUE_NAME.0, b"a::B\0");
    assert_eq!(decode_error(LF_STRUCTURE, &data), ErrorKind::InvalidFormat);
}

#[test]
fn decodes_union_and_enum() {
    let data = padded(bytes(&[
        &3_u16.to_le_bytes(),
        &ClassOptions::FORWARD_REFERENCE.0.to_le_bytes(),
        &0_u32.to_le_bytes(),
        &0_u16.to_le_bytes(),
        b"U\0",
    ]));
    let TypeLeaf::Union(union) = decode(LF_UNION, &data) else {
        panic!("not a union");
    };
    assert_eq!(union.member_count, 3);
    assert!(union.options.is_forward_reference());
    assert_eq!(union.size.integer(), Some(Integer::Unsigned(0)));
    assert_eq!(union.name, b"U");
    assert_eq!(union.unique_name, None);

    let data = padded(bytes(&[
        &2_u16.to_le_bytes(),
        &ClassOptions::SCOPED.0.to_le_bytes(),
        &0x0074_u32.to_le_bytes(),
        &0x1009_u32.to_le_bytes(),
        b"E\0",
    ]));
    let TypeLeaf::Enum(value) = decode(LF_ENUM, &data) else {
        panic!("not an enumeration");
    };
    assert_eq!(value.underlying_type, TypeIndex(0x74));
    assert_eq!(value.field_list, TypeIndex(0x1009));
    assert!(value.options.is_scoped());
    assert_eq!(value.name, b"E");
}

#[test]
fn decodes_class_option_fields() {
    let options = ClassOptions(0x2000 | (2 << 11) | (3 << 14) | ClassOptions::PACKED.0);
    assert_eq!(options.hfa(), HfaKind::Double);
    assert_eq!(options.mocom(), MocomKind::Interface);
    assert!(options.contains(ClassOptions::INTRINSIC));
    assert!(options.is_packed());
    assert_eq!(ClassOptions(0).hfa(), HfaKind::None);
    assert_eq!(ClassOptions(0).mocom(), MocomKind::None);
}

#[test]
fn decodes_member_attribute_fields() {
    let attributes = MemberAttributes::from_raw(
        2 | (6 << 2) | MemberAttributes::COMPILER_GENERATED.raw() | MemberAttributes::SEALED.raw(),
    )
    .unwrap();
    assert_eq!(attributes.access(), MemberAccess::Protected);
    assert_eq!(attributes.method_kind(), MethodKind::PureIntroducingVirtual);
    assert!(attributes.method_kind().is_introducing_virtual());
    assert!(attributes.contains(MemberAttributes::SEALED));
    assert!(!attributes.contains(MemberAttributes::PSEUDO));
    assert_eq!(
        MemberAttributes::from_raw(7 << 2).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn decodes_vtable_shape_descriptors() {
    let data = padded(bytes(&[&3_u16.to_le_bytes(), &[0x52, 0x60]]));
    let TypeLeaf::VtableShape(shape) = decode(LF_VTSHAPE, &data) else {
        panic!("not a vtable shape");
    };
    assert_eq!(shape.len(), 3);
    assert_eq!(
        shape.slots().collect::<Vec<_>>(),
        [
            VtableSlotKind::Near,
            VtableSlotKind::This,
            VtableSlotKind::Far
        ]
    );
    // The unused low 4 bits of the last byte must be zero.
    let data = padded(bytes(&[&3_u16.to_le_bytes(), &[0x52, 0x61]]));
    assert_eq!(decode_error(LF_VTSHAPE, &data), ErrorKind::InvalidFormat);
    // Slot kind 7 has no meaning.
    let data = padded(bytes(&[&1_u16.to_le_bytes(), &[0x70]]));
    assert_eq!(decode_error(LF_VTSHAPE, &data), ErrorKind::InvalidFormat);
}

#[test]
fn keeps_unsupported_kind_and_payload() {
    let data = [1, 2, 3];
    let leaf = decode(LeafKind(0x1601), &data);
    assert_eq!(leaf, TypeLeaf::Unsupported(Record::new(0, 0x1601, &data)));
    assert_eq!(leaf.tag(), None);
}

#[test]
fn rejects_truncated_leaves() {
    for (kind, length) in [
        (LF_MODIFIER, 5),
        (LF_POINTER, 7),
        (LF_PROCEDURE, 11),
        (LF_MFUNCTION, 23),
        (LF_ARGLIST, 3),
        (LF_BITFIELD, 5),
        (LF_ARRAY, 9),
        (LF_STRUCTURE, 17),
        (LF_UNION, 9),
        (LF_ENUM, 11),
        (LF_VTSHAPE, 1),
        (LF_VFTABLE, 15),
        (LF_LABEL, 1),
    ] {
        assert_eq!(
            decode_error(kind, &vec![0; length]),
            ErrorKind::InvalidFormat,
            "kind {kind:#x}"
        );
    }
}

#[test]
fn rejects_invalid_padding_and_missing_terminators() {
    // Padding must count down to 0xf1.
    let data = bytes(&[
        &0x1003_u32.to_le_bytes(),
        &0_u16.to_le_bytes(),
        &[0xf1, 0xf2],
    ]);
    assert_eq!(decode_error(LF_MODIFIER, &data), ErrorKind::InvalidFormat);
    // Four padding bytes are not valid.
    let data = bytes(&[
        &0x0003_u32.to_le_bytes(),
        &[0, 0],
        &0_u16.to_le_bytes(),
        &0x1004_u32.to_le_bytes(),
        &[0xf4, 0xf3, 0xf2, 0xf1],
    ]);
    assert_eq!(decode_error(LF_PROCEDURE, &data), ErrorKind::InvalidFormat);
    // A name without a terminator.
    let data = bytes(&[
        &0x0070_u32.to_le_bytes(),
        &0x0023_u32.to_le_bytes(),
        &4_u16.to_le_bytes(),
        b"abc",
    ]);
    assert_eq!(decode_error(LF_ARRAY, &data), ErrorKind::InvalidFormat);
}

fn vftable(overridden: u32, names: &[u8], length: u32) -> Vec<u8> {
    padded(bytes(&[
        &0x1895_u32.to_le_bytes(),
        &overridden.to_le_bytes(),
        &8_u32.to_le_bytes(),
        &length.to_le_bytes(),
        names,
    ]))
}

#[test]
fn decodes_virtual_function_table_names() {
    let names = b"??_7type_info@@6B@\0??_Etype_info@@UEAAPEAXI@Z\0";
    let data = vftable(0, names, u32::try_from(names.len()).unwrap());
    let TypeLeaf::VirtualFunctionTable(table) = decode(LF_VFTABLE, &data) else {
        panic!("not a virtual function table");
    };
    assert_eq!(table.complete_class, TypeIndex(0x1895));
    assert_eq!(table.overridden_table, None);
    assert_eq!(table.table_pointer_offset, 8);
    assert_eq!(table.table_name(), b"??_7type_info@@6B@");
    let methods: Vec<_> = table.method_names().collect();
    assert_eq!(methods, [&b"??_Etype_info@@UEAAPEAXI@Z"[..]]);

    let data = vftable(0x1001, b"t\0", 2);
    let TypeLeaf::VirtualFunctionTable(table) = decode(LF_VFTABLE, &data) else {
        panic!("not a virtual function table");
    };
    assert_eq!(table.overridden_table, Some(TypeIndex(0x1001)));
    assert_eq!(table.method_names().count(), 0);
}

#[test]
fn rejects_virtual_function_table_names_outside_their_length() {
    // The declared length excludes the terminator of the last name.
    assert_eq!(
        decode_error(LF_VFTABLE, &vftable(0, b"t\0", 1)),
        ErrorKind::InvalidFormat
    );
    // The declared length passes the end of the record.
    assert_eq!(
        decode_error(LF_VFTABLE, &vftable(0, b"t\0", 9)),
        ErrorKind::InvalidFormat
    );
    // No table name.
    assert_eq!(
        decode_error(LF_VFTABLE, &vftable(0, b"", 0)),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn decodes_label_modes_and_rejects_unknown_modes() {
    for (raw, mode) in [(0_u16, LabelMode::Near), (4, LabelMode::Far)] {
        let data = padded(raw.to_le_bytes().to_vec());
        assert_eq!(decode(LF_LABEL, &data), TypeLeaf::Label(mode));
    }
    let data = padded(2_u16.to_le_bytes().to_vec());
    assert_eq!(decode_error(LF_LABEL, &data), ErrorKind::InvalidFormat);
}
