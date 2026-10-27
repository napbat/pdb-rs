//! Field-list members: each kind, exact re-encoding, and the split of long
//! member sequences with `LF_INDEX` continuations.

use pdb_rs::ErrorKind;
use pdb_rs::native::codeview::field_list::{
    BaseClass, Enumerator, FieldList, LF_BCLASS, LF_BINTERFACE, LF_ENUMERATE, LF_FRIENDCLS,
    LF_FRIENDFCN, LF_INDEX, LF_IVBCLASS, LF_MEMBER, LF_MEMBERMODIFY, LF_METHOD, LF_NESTTYPE,
    LF_NESTTYPEEX, LF_ONEMETHOD, LF_STMEMBER, LF_VBCLASS, LF_VFUNCOFF, LF_VFUNCTAB,
    LLVM_FIELD_LIST_LIMIT, Member, OneMethod, encode_field_list,
};
use pdb_rs::native::codeview::index::TypeIndex;
use pdb_rs::native::codeview::numeric::{Integer, Numeric};
use pdb_rs::native::codeview::type_record::{LF_FIELDLIST, MemberAttributes, TypeLeaf};
use pdb_rs::native::codeview::{LeafKind, Record, Records};

/// Serialize one member and pad it to 4 bytes with `LF_PAD` bytes.
fn member(kind: LeafKind, fields: &[&[u8]]) -> Vec<u8> {
    let mut bytes = kind.0.to_le_bytes().to_vec();
    for field in fields {
        bytes.extend_from_slice(field);
    }
    let count = (4 - bytes.len() % 4) % 4;
    for left in (1..=count).rev() {
        bytes.push(0xf0 + u8::try_from(left).unwrap());
    }
    bytes
}

fn u32le(value: u32) -> [u8; 4] {
    value.to_le_bytes()
}

/// Every member kind once.
fn all_members() -> Vec<(LeafKind, Vec<u8>)> {
    vec![
        (
            LF_BCLASS,
            member(LF_BCLASS, &[&[3, 0], &u32le(0x1000), &[0, 0]]),
        ),
        (
            LF_BINTERFACE,
            member(
                LF_BINTERFACE,
                &[&[3, 0], &u32le(0x1001), &[0x02, 0x80, 0, 0x90]],
            ),
        ),
        (
            LF_VBCLASS,
            member(
                LF_VBCLASS,
                &[&[3, 0], &u32le(0x1002), &u32le(0x1003), &[8, 0], &[1, 0]],
            ),
        ),
        (
            LF_IVBCLASS,
            member(
                LF_IVBCLASS,
                &[&[3, 0], &u32le(0x1002), &u32le(0x1003), &[8, 0], &[2, 0]],
            ),
        ),
        (LF_VFUNCTAB, member(LF_VFUNCTAB, &[&[0, 0], &u32le(0x1004)])),
        (
            LF_VFUNCOFF,
            member(LF_VFUNCOFF, &[&[0, 0], &u32le(0x1004), &u32le(16)]),
        ),
        (
            LF_FRIENDCLS,
            member(LF_FRIENDCLS, &[&[0, 0], &u32le(0x1005)]),
        ),
        (
            LF_FRIENDFCN,
            member(LF_FRIENDFCN, &[&[0, 0], &u32le(0x1006), b"f\0"]),
        ),
        (
            LF_ENUMERATE,
            member(LF_ENUMERATE, &[&[3, 0], &[0x00, 0x80, 0xff], b"Minus\0"]),
        ),
        (
            LF_MEMBER,
            member(LF_MEMBER, &[&[3, 0], &u32le(0x74), &[8, 0], b"x\0"]),
        ),
        (
            LF_STMEMBER,
            member(LF_STMEMBER, &[&[1, 0], &u32le(0x74), b"s\0"]),
        ),
        (
            LF_METHOD,
            member(LF_METHOD, &[&[2, 0], &u32le(0x1007), b"m\0"]),
        ),
        (
            LF_NESTTYPE,
            member(LF_NESTTYPE, &[&[0, 0], &u32le(0x1008), b"N\0"]),
        ),
        (
            LF_NESTTYPEEX,
            member(LF_NESTTYPEEX, &[&[1, 0], &u32le(0x1008), b"N\0"]),
        ),
        (
            LF_MEMBERMODIFY,
            member(LF_MEMBERMODIFY, &[&[2, 0], &u32le(0x1000), b"x\0"]),
        ),
        (
            LF_ONEMETHOD,
            member(
                LF_ONEMETHOD,
                &[&[0x13, 0], &u32le(0x1009), &u32le(8), b"v\0"],
            ),
        ),
        (
            LF_ONEMETHOD,
            member(LF_ONEMETHOD, &[&[0x03, 0], &u32le(0x1009), b"w\0"]),
        ),
        (LF_INDEX, member(LF_INDEX, &[&[0, 0], &u32le(0x100a)])),
    ]
}

#[test]
fn every_member_kind_decodes_and_re_encodes() {
    let members = all_members();
    let data: Vec<u8> = members
        .iter()
        .flat_map(|(_, bytes)| bytes.clone())
        .collect();
    let list = FieldList::parse(data.as_slice()).unwrap();
    assert_eq!(list.continuation(), Some(TypeIndex(0x100a)));
    let decoded: Vec<Member<'_>> = list.members().collect::<Result<_, _>>().unwrap();
    assert_eq!(decoded.len(), members.len());
    for (member, (kind, bytes)) in decoded.iter().zip(&members) {
        assert_eq!(member.kind(), *kind);
        let mut out = Vec::new();
        member.encode(&mut out).unwrap();
        assert_eq!(&out, bytes, "kind {kind:#x}");
    }
    // The whole record re-encodes to the same bytes.
    let mut full = u16::try_from(data.len() + 2)
        .unwrap()
        .to_le_bytes()
        .to_vec();
    full.extend_from_slice(&LF_FIELDLIST.0.to_le_bytes());
    full.extend_from_slice(&data);
    let leaf = TypeLeaf::decode(Record::new(0, LF_FIELDLIST.0, &data)).unwrap();
    let mut out = Vec::new();
    leaf.encode(&mut out).unwrap();
    assert_eq!(out, full);
}

#[test]
fn each_truncated_member_is_an_error() {
    for (kind, bytes) in all_members() {
        let unpadded = bytes.iter().rposition(|&byte| byte < 0xf0).unwrap() + 1;
        for end in 2..unpadded {
            let error = FieldList::parse(&bytes[..end]).unwrap_err();
            assert_eq!(
                error.kind(),
                ErrorKind::InvalidFormat,
                "kind {kind:#x} prefix {end}"
            );
        }
    }
}

#[test]
fn reserved_fields_must_be_zero() {
    for kind in [LF_FRIENDCLS, LF_VFUNCOFF, LF_FRIENDFCN] {
        let bytes = member(kind, &[&[1, 0], &u32le(0x1000), &u32le(0), b"\0"]);
        let error = FieldList::parse(bytes.as_slice()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat, "kind {kind:#x}");
    }
}

#[test]
fn one_method_offset_must_match_the_method_property() {
    let introducing = MemberAttributes::from_raw(0x13).unwrap();
    let mut method = OneMethod::default();
    method.attributes = introducing;
    method.method_type = TypeIndex(0x1000);
    method.vtable_offset = None;
    method.name = b"f".into();
    let error = Member::OneMethod(method)
        .encode(&mut Vec::new())
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
}

/// An enumerator of 16 bytes: kind, attributes, an immediate value and a
/// name of 9 bytes with its terminator.
fn enumerator(value: u16) -> Member<'static> {
    let mut enumerator = Enumerator::default();
    enumerator.attributes = MemberAttributes::from_raw(3).unwrap();
    enumerator.value = Numeric::from_integer(Integer::Unsigned(u128::from(value)));
    enumerator.name = b"Item_000".into();
    Member::Enumerator(enumerator)
}

/// Split `members` and return the records.
fn split(members: &[Member<'_>], limit: usize) -> (Vec<u8>, TypeIndex, u32) {
    let mut out = Vec::new();
    let records = encode_field_list(members, TypeIndex(0x2000), limit, &mut out).unwrap();
    (out, records.head, records.count)
}

#[test]
fn short_lists_give_one_record() {
    let members: Vec<_> = (0..3).map(enumerator).collect();
    let (out, head, count) = split(&members, LLVM_FIELD_LIST_LIMIT);
    assert_eq!((head, count), (TypeIndex(0x2000), 1));
    assert_eq!(out.len(), 4 + 3 * 16);
    let empty: Vec<Member<'_>> = Vec::new();
    let (out, _, count) = split(&empty, LLVM_FIELD_LIST_LIMIT);
    assert_eq!((out, count), (vec![2, 0, 0x03, 0x12], 1));
}

#[test]
fn long_lists_split_at_the_limit_with_continuations() {
    // A limit of 4 + 3 * 16 + 8 bytes fits three members and a continuation.
    let limit = 4 + 3 * 16 + 8;
    let members: Vec<_> = (0..7).map(enumerator).collect();
    let (out, head, count) = split(&members, limit);
    assert_eq!((head, count), (TypeIndex(0x2002), 3));
    let records: Vec<Record<'_>> = Records::new(&out).collect::<Result<_, _>>().unwrap();
    // The last members come first, the head comes last.
    let values: Vec<Vec<Option<u128>>> = records
        .iter()
        .map(|record| {
            FieldList::parse(record.data)
                .unwrap()
                .members()
                .filter_map(|member| match member.unwrap() {
                    Member::Enumerator(value) => {
                        Some(value.value.integer().and_then(Integer::to_u128))
                    }
                    _ => None,
                })
                .collect()
        })
        .collect();
    assert_eq!(
        values,
        [
            vec![Some(6)],
            vec![Some(3), Some(4), Some(5)],
            vec![Some(0), Some(1), Some(2)]
        ]
    );
    let continuations: Vec<_> = records
        .iter()
        .map(|record| FieldList::parse(record.data).unwrap().continuation())
        .collect();
    assert_eq!(
        continuations,
        [None, Some(TypeIndex(0x2000)), Some(TypeIndex(0x2001))]
    );
    assert!(records.iter().all(|record| record.data.len() + 4 <= limit));
    // One byte less moves one member to the next record.
    let (_, _, count) = split(&members, limit - 1);
    assert_eq!(count, 4);
}

#[test]
fn split_rejects_invalid_input() {
    let continuation = [Member::Continuation(TypeIndex(0x1000))];
    let error = encode_field_list(
        &continuation,
        TypeIndex(0x1000),
        LLVM_FIELD_LIST_LIMIT,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    let members = [enumerator(1)];
    for limit in [11, 0x1_0002] {
        let error =
            encode_field_list(&members, TypeIndex(0x1000), limit, &mut Vec::new()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::LimitExceeded, "limit {limit}");
    }
    // A member that does not fit one record.
    let error =
        encode_field_list(&members, TypeIndex(0x1000), 4 + 8 + 8, &mut Vec::new()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    let error = encode_field_list(
        &[enumerator(1), enumerator(2)],
        TypeIndex(u32::MAX),
        4 + 16 + 8,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
}

#[test]
fn base_interface_is_a_base_class_layout() {
    let mut base = BaseClass::default();
    base.attributes = MemberAttributes::from_raw(3).unwrap();
    base.base_type = TypeIndex(0x1000);
    base.offset = Numeric::from_integer(Integer::Unsigned(0));
    let mut class = Vec::new();
    Member::BaseClass(base.clone()).encode(&mut class).unwrap();
    let mut interface = Vec::new();
    Member::BaseInterface(base).encode(&mut interface).unwrap();
    assert_eq!(class[2..], interface[2..]);
    assert_eq!(interface[..2], LF_BINTERFACE.0.to_le_bytes());
}
