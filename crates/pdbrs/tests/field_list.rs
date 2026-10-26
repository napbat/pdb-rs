//! Tests for field-list member decoding.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::field_list::{
    FieldList, LF_BCLASS, LF_ENUMERATE, LF_INDEX, LF_IVBCLASS, LF_MEMBER, LF_METHOD, LF_NESTTYPE,
    LF_ONEMETHOD, LF_STMEMBER, LF_VBCLASS, LF_VFUNCTAB, Member,
};
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::codeview::numeric::Integer;
use pdbrs::native::codeview::type_record::{MemberAccess, MethodKind};

/// Serialize one member and pad it to 4 bytes with `LF_PAD` bytes.
fn member(kind: u16, fields: &[&[u8]]) -> Vec<u8> {
    let mut bytes = kind.to_le_bytes().to_vec();
    for field in fields {
        bytes.extend_from_slice(field);
    }
    let count = (4 - bytes.len() % 4) % 4;
    for left in (1..=count).rev() {
        bytes.push(0xf0 + u8::try_from(left).unwrap());
    }
    bytes
}

const PUBLIC: [u8; 2] = 3_u16.to_le_bytes();

fn members(data: &[u8]) -> Vec<Member<'_>> {
    FieldList::decode(data)
        .unwrap()
        .members()
        .map(Result::unwrap)
        .collect()
}

fn decode_error(data: &[u8]) -> ErrorKind {
    FieldList::decode(data).unwrap_err().kind()
}

#[test]
fn decodes_data_members_and_enumerators() {
    let data = [
        member(
            LF_MEMBER,
            &[
                &PUBLIC,
                &0x0074_u32.to_le_bytes(),
                &8_u16.to_le_bytes(),
                b"x\0",
            ],
        ),
        member(
            LF_STMEMBER,
            &[&1_u16.to_le_bytes(), &0x1001_u32.to_le_bytes(), b"s\0"],
        ),
        // A negative value as `LF_CHAR`.
        member(LF_ENUMERATE, &[&PUBLIC, &[0x00, 0x80, 0xff], b"Minus\0"]),
    ]
    .concat();
    let list = members(&data);
    let [
        Member::DataMember(data_member),
        Member::StaticMember(static_member),
        Member::Enumerator(enumerator),
    ] = list.as_slice()
    else {
        panic!("unexpected members {list:?}");
    };
    assert_eq!(data_member.attributes.access(), MemberAccess::Public);
    assert_eq!(data_member.member_type, TypeIndex(0x74));
    assert_eq!(data_member.offset.integer(), Some(Integer::Unsigned(8)));
    assert_eq!(data_member.name, b"x");
    assert_eq!(static_member.attributes.access(), MemberAccess::Private);
    assert_eq!(static_member.name, b"s");
    assert_eq!(enumerator.value.integer(), Some(Integer::Signed(-1)));
    assert_eq!(enumerator.name, b"Minus");
    assert_eq!(FieldList::decode(&data).unwrap().continuation(), None);
}

#[test]
fn decodes_base_classes_and_vtable_pointer() {
    let data = [
        member(
            LF_BCLASS,
            &[&PUBLIC, &0x1002_u32.to_le_bytes(), &0_u16.to_le_bytes()],
        ),
        member(
            LF_VBCLASS,
            &[
                &PUBLIC,
                &0x1003_u32.to_le_bytes(),
                &0x1004_u32.to_le_bytes(),
                &8_u16.to_le_bytes(),
                &1_u16.to_le_bytes(),
            ],
        ),
        member(
            LF_IVBCLASS,
            &[
                &PUBLIC,
                &0x1005_u32.to_le_bytes(),
                &0x1004_u32.to_le_bytes(),
                &8_u16.to_le_bytes(),
                &2_u16.to_le_bytes(),
            ],
        ),
        member(
            LF_VFUNCTAB,
            &[&0_u16.to_le_bytes(), &0x1006_u32.to_le_bytes()],
        ),
    ]
    .concat();
    let list = members(&data);
    let [
        Member::BaseClass(base),
        Member::VirtualBaseClass(direct),
        Member::VirtualBaseClass(indirect),
        Member::VirtualFunctionTable(table),
    ] = list.as_slice()
    else {
        panic!("unexpected members {list:?}");
    };
    assert_eq!(base.base_type, TypeIndex(0x1002));
    assert_eq!(base.offset.integer(), Some(Integer::Unsigned(0)));
    assert!(!direct.indirect);
    assert_eq!(direct.base_type, TypeIndex(0x1003));
    assert_eq!(direct.vbptr_type, TypeIndex(0x1004));
    assert_eq!(direct.vbptr_offset.integer(), Some(Integer::Unsigned(8)));
    assert_eq!(direct.vbtable_index.integer(), Some(Integer::Unsigned(1)));
    assert!(indirect.indirect);
    assert_eq!(indirect.vbtable_index.integer(), Some(Integer::Unsigned(2)));
    assert_eq!(*table, TypeIndex(0x1006));
}

#[test]
fn decodes_methods_and_nested_types() {
    let intro_virtual = (3_u16 | (4 << 2)).to_le_bytes();
    let data = [
        member(
            LF_ONEMETHOD,
            &[
                &intro_virtual,
                &0x1010_u32.to_le_bytes(),
                &24_i32.to_le_bytes(),
                b"f\0",
            ],
        ),
        member(LF_ONEMETHOD, &[&PUBLIC, &0x1011_u32.to_le_bytes(), b"g\0"]),
        member(
            LF_METHOD,
            &[&2_u16.to_le_bytes(), &0x1012_u32.to_le_bytes(), b"h\0"],
        ),
        member(
            LF_NESTTYPE,
            &[&0_u16.to_le_bytes(), &0x1013_u32.to_le_bytes(), b"Inner\0"],
        ),
    ]
    .concat();
    let list = members(&data);
    let [
        Member::OneMethod(virtual_method),
        Member::OneMethod(method),
        Member::OverloadedMethod(overloaded),
        Member::NestedType(nested),
    ] = list.as_slice()
    else {
        panic!("unexpected members {list:?}");
    };
    assert_eq!(
        virtual_method.attributes.method_kind(),
        MethodKind::IntroducingVirtual
    );
    assert_eq!(virtual_method.vtable_offset, Some(24));
    assert_eq!(virtual_method.name, b"f");
    assert_eq!(method.vtable_offset, None);
    assert_eq!(method.method_type, TypeIndex(0x1011));
    assert_eq!(overloaded.overload_count, 2);
    assert_eq!(overloaded.method_list, TypeIndex(0x1012));
    assert_eq!(nested.nested_type, TypeIndex(0x1013));
    assert_eq!(nested.name, b"Inner");
}

#[test]
fn reports_final_continuation() {
    let data = [
        member(LF_ENUMERATE, &[&PUBLIC, &1_u16.to_le_bytes(), b"A\0"]),
        member(LF_INDEX, &[&0_u16.to_le_bytes(), &0x1020_u32.to_le_bytes()]),
    ]
    .concat();
    let list = FieldList::decode(&data).unwrap();
    assert_eq!(list.continuation(), Some(TypeIndex(0x1020)));
    assert_eq!(list.data(), data.as_slice());
    assert_eq!(
        list.members().last().unwrap().unwrap(),
        Member::Continuation(TypeIndex(0x1020))
    );
}

#[test]
fn accepts_members_with_and_without_padding() {
    // The first member ends on a 4-byte boundary. The second member needs
    // three padding bytes.
    let data = [
        member(LF_ENUMERATE, &[&PUBLIC, &2_u16.to_le_bytes(), b"C\0"]),
        member(LF_ENUMERATE, &[&PUBLIC, &1_u16.to_le_bytes(), b"AB\0"]),
    ]
    .concat();
    assert_eq!(data.len(), 8 + 9 + 3);
    assert_eq!(&data[17..], &[0xf3, 0xf2, 0xf1]);
    assert_eq!(members(&data).len(), 2);
}

#[test]
fn rejects_member_after_continuation() {
    let data = [
        member(LF_INDEX, &[&0_u16.to_le_bytes(), &0x1020_u32.to_le_bytes()]),
        member(LF_ENUMERATE, &[&PUBLIC, &1_u16.to_le_bytes(), b"A\0"]),
    ]
    .concat();
    assert_eq!(decode_error(&data), ErrorKind::InvalidFormat);
}

#[test]
fn rejects_unknown_member_kind() {
    let data = member(0x1403, &[&[0; 6]]);
    assert_eq!(decode_error(&data), ErrorKind::UnsupportedVersion);
}

#[test]
fn rejects_invalid_padding() {
    let valid = member(LF_ENUMERATE, &[&PUBLIC, &1_u16.to_le_bytes(), b"AB\0"]);
    assert_eq!(&valid[9..], &[0xf3, 0xf2, 0xf1]);
    for padding in [[0xf3, 0xf3, 0xf1], [0xf3, 0xf2, 0xf2], [0xf0, 0xf0, 0xf0]] {
        let mut data = valid.clone();
        data[9..].copy_from_slice(&padding);
        assert_eq!(
            decode_error(&data),
            ErrorKind::InvalidFormat,
            "{padding:x?}"
        );
    }
    // The padding count is larger than the remaining bytes.
    let mut data = valid.clone();
    data[9] = 0xf4;
    assert_eq!(decode_error(&data), ErrorKind::InvalidFormat);
}

#[test]
fn rejects_truncated_members_and_reserved_fields() {
    let data = member(
        LF_MEMBER,
        &[&PUBLIC, &0x0074_u32.to_le_bytes(), &0_u16.to_le_bytes()],
    );
    // The name terminator is missing; the padding bytes are not a name.
    assert_eq!(decode_error(&data), ErrorKind::InvalidFormat);
    let data = LF_MEMBER.to_le_bytes();
    assert_eq!(decode_error(&data), ErrorKind::InvalidFormat);
    let data = member(
        LF_NESTTYPE,
        &[&1_u16.to_le_bytes(), &0x1013_u32.to_le_bytes(), b"I\0"],
    );
    assert_eq!(decode_error(&data), ErrorKind::InvalidFormat);
    let data = member(
        LF_VFUNCTAB,
        &[&1_u16.to_le_bytes(), &0x1006_u32.to_le_bytes()],
    );
    assert_eq!(decode_error(&data), ErrorKind::InvalidFormat);
    let data = member(LF_INDEX, &[&1_u16.to_le_bytes(), &0x1006_u32.to_le_bytes()]);
    assert_eq!(decode_error(&data), ErrorKind::InvalidFormat);
}
