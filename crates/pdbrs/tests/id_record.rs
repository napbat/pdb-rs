//! Tests for IPI ID records, string resolution and reference validation.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::id_record::{
    BuildInfoArgument, IdRecord, IdTable, LF_BUILDINFO, LF_FUNC_ID, LF_MFUNC_ID, LF_STRING_ID,
    LF_SUBSTR_LIST, LF_UDT_MOD_SRC_LINE, LF_UDT_SRC_LINE, MAX_STRING_DEPTH,
};
use pdbrs::native::codeview::index::{IdIndex, ModuleIndex, NameOffset, TypeIndex};
use pdbrs::native::codeview::{LeafKind, Record};
use pdbrs::native::msf::StreamId;
use pdbrs::native::types::{HEADER_SIZE, TypeStream};

/// A leaf kind that is a type leaf, not an ID leaf (`LF_PROCEDURE`).
const LF_PROCEDURE: LeafKind = LeafKind(0x1008);

/// A record kind and its payload, before framing.
type RawRecord = (LeafKind, Vec<u8>);

fn record(kind: LeafKind, data: &[u8]) -> Record<'_> {
    Record::new(0, kind.0, data)
}

fn decode(kind: LeafKind, data: &[u8]) -> pdbrs::Result<IdRecord<'_>> {
    IdRecord::decode(record(kind, data))
}

fn u32s(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// Append `LF_PAD` bytes until the payload and the kind fill 4-byte units.
fn pad(mut payload: Vec<u8>) -> Vec<u8> {
    while !(payload.len() + 2).is_multiple_of(4) {
        let left = 4 - (payload.len() + 2) % 4;
        payload.push(0xf0 + u8::try_from(left).unwrap());
    }
    payload
}

fn func_id(scope: u32, function_type: u32, name: &[u8]) -> Vec<u8> {
    let mut payload = u32s(&[scope, function_type]);
    payload.extend_from_slice(name);
    payload.push(0);
    pad(payload)
}

fn string_id(substrings: u32, string: &[u8]) -> Vec<u8> {
    let mut payload = u32s(&[substrings]);
    payload.extend_from_slice(string);
    payload.push(0);
    pad(payload)
}

fn list(count_size: usize, ids: &[u32]) -> Vec<u8> {
    let count = u32::try_from(ids.len()).unwrap().to_le_bytes();
    let mut payload = count[..count_size].to_vec();
    payload.extend_from_slice(&u32s(ids));
    pad(payload)
}

/// Build an IPI stream with `records` at indices from 0x1000.
fn ipi(records: &[RawRecord]) -> TypeStream {
    let mut body = Vec::new();
    for (kind, payload) in records {
        let length = u16::try_from(payload.len() + 2).unwrap();
        body.extend_from_slice(&length.to_le_bytes());
        body.extend_from_slice(&kind.0.to_le_bytes());
        body.extend_from_slice(payload);
    }
    let count = u32::try_from(records.len()).unwrap();
    let mut bytes = u32s(&[
        20_040_203,
        56,
        0x1000,
        0x1000 + count,
        u32::try_from(body.len()).unwrap(),
    ]);
    bytes.extend_from_slice(&0xffff_u16.to_le_bytes());
    bytes.extend_from_slice(&0xffff_u16.to_le_bytes());
    bytes.extend_from_slice(&u32s(&[4, 0x3_ffff, 0, u32::MAX, 0, u32::MAX, 0, u32::MAX]));
    assert_eq!(bytes.len(), HEADER_SIZE);
    bytes.extend_from_slice(&body);
    TypeStream::parse(bytes).unwrap()
}

#[test]
fn function_ids_keep_scope_type_and_name() {
    let global = func_id(0, 0x1234, b"main");
    let IdRecord::Function(function) = decode(LF_FUNC_ID, &global).unwrap() else {
        panic!("expected a function ID");
    };
    assert_eq!(function.scope, None);
    assert_eq!(function.function_type, TypeIndex(0x1234));
    assert_eq!(function.name, b"main");
    assert_eq!(function.decorated_name_hash, None);

    let scoped = func_id(0x1005, 0x1000, b"\xffx");
    let IdRecord::Function(function) = decode(LF_FUNC_ID, &scoped).unwrap() else {
        panic!("expected a function ID");
    };
    assert_eq!(function.scope, Some(IdIndex(0x1005)));
    assert_eq!(function.name, b"\xffx");
}

#[test]
fn function_ids_read_an_optional_name_hash() {
    // The example from the Microsoft `pdb-rs` documentation of `LF_FUNC_ID`.
    let mut payload = u32s(&[0x0610, 0x1378]);
    payload.extend_from_slice(b"CreateCacheContext\0");
    payload.extend_from_slice(&0x21cd_852b_2710_56dd_u64.to_le_bytes());
    payload.push(0xf1);
    let IdRecord::Function(function) = decode(LF_FUNC_ID, &payload).unwrap() else {
        panic!("expected a function ID");
    };
    assert_eq!(function.name, b"CreateCacheContext");
    assert_eq!(function.decorated_name_hash, Some(0x21cd_852b_2710_56dd));

    let mut member = u32s(&[0x107f, 0x10a3]);
    member.extend_from_slice(b"Get\0");
    member.extend_from_slice(&7_u64.to_le_bytes());
    member.extend_from_slice(&[0xf2, 0xf1]);
    let IdRecord::MemberFunction(function) = decode(LF_MFUNC_ID, &member).unwrap() else {
        panic!("expected a member function ID");
    };
    assert_eq!(function.parent_type, TypeIndex(0x107f));
    assert_eq!(function.function_type, TypeIndex(0x10a3));
    assert_eq!(function.name, b"Get");
    assert_eq!(function.decorated_name_hash, Some(7));
}

#[test]
fn lists_keep_id_indices_in_order() {
    let payload = list(2, &[0x1000, 0x1001, 0x1002, 0, 0x1003]);
    let IdRecord::BuildInfo(info) = decode(LF_BUILDINFO, &payload).unwrap() else {
        panic!("expected build information");
    };
    assert_eq!(info.arguments.len(), 5);
    assert_eq!(
        info.argument(BuildInfoArgument::SourceFile),
        Some(IdIndex(0x1002))
    );
    assert_eq!(
        info.argument(BuildInfoArgument::ProgramDatabase),
        Some(IdIndex::NONE)
    );
    assert_eq!(info.arguments.iter().next_back(), Some(IdIndex(0x1003)));

    let short = list(2, &[0x1000]);
    let IdRecord::BuildInfo(info) = decode(LF_BUILDINFO, &short).unwrap() else {
        panic!("expected build information");
    };
    assert_eq!(info.argument(BuildInfoArgument::CommandLine), None);

    let payload = list(4, &[0x1001, 0x1002]);
    let IdRecord::SubstringList(parts) = decode(LF_SUBSTR_LIST, &payload).unwrap() else {
        panic!("expected a substring list");
    };
    let parts: Vec<_> = parts.into_iter().collect();
    assert_eq!(parts, [IdIndex(0x1001), IdIndex(0x1002)]);
}

#[test]
fn source_line_records_keep_reference_kinds_apart() {
    let payload = pad(u32s(&[0x1100, 0x1005, 42]));
    let IdRecord::UdtSourceLine(line) = decode(LF_UDT_SRC_LINE, &payload).unwrap() else {
        panic!("expected a UDT source line");
    };
    assert_eq!(line.udt, TypeIndex(0x1100));
    assert_eq!(line.source_file, IdIndex(0x1005));
    assert_eq!(line.line, 42);

    // Link.exe writes this record without padding: 2 + 14 bytes.
    let mut payload = u32s(&[0x1004, 967, 610]);
    payload.extend_from_slice(&22_u16.to_le_bytes());
    let IdRecord::UdtModuleSourceLine(line) = decode(LF_UDT_MOD_SRC_LINE, &payload).unwrap() else {
        panic!("expected a UDT module source line");
    };
    assert_eq!(line.udt, TypeIndex(0x1004));
    assert_eq!(line.name_offset, NameOffset(967));
    assert_eq!(line.line, 610);
    // The record stores the 1-based module number 22.
    assert_eq!(line.module, Some(ModuleIndex(21)));
}

#[test]
fn padding_must_be_lf_pad_bytes() {
    let mut payload = u32s(&[0, 0x1000]);
    payload.extend_from_slice(b"ab\0");
    for (tail, valid) in [
        (&[][..], true),
        (&[0xf1][..], true),
        (&[0xf3, 0xf2, 0xf1][..], true),
        (&[0x00][..], false),
        (&[0xf2, 0xf2][..], false),
        (&[0xf4, 0xf3, 0xf2, 0xf1][..], false),
        (&[0xf1, 0, 0, 0, 0][..], false),
    ] {
        let data = [payload.as_slice(), tail].concat();
        let result = decode(LF_FUNC_ID, &data);
        if valid {
            result.unwrap();
        } else {
            assert_eq!(
                result.unwrap_err().kind(),
                ErrorKind::InvalidFormat,
                "{tail:?}"
            );
        }
    }
}

#[test]
fn malformed_payloads_fail_explicitly() {
    let truncated = u32s(&[0]);
    assert_eq!(
        decode(LF_FUNC_ID, &truncated).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    let unterminated = [u32s(&[0, 0x1000]).as_slice(), b"name"].concat();
    let error = decode(LF_FUNC_ID, &unterminated).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(error.offset(), None);
    // The count promises two indices, but only one follows.
    let mut short_list = 2_u16.to_le_bytes().to_vec();
    short_list.extend_from_slice(&u32s(&[0x1000]));
    assert_eq!(
        decode(LF_BUILDINFO, &short_list).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    let short_line = u32s(&[0x1004, 1, 2]);
    assert_eq!(
        decode(LF_UDT_MOD_SRC_LINE, &short_line).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn other_kinds_stay_raw() {
    let data = [1, 2, 3];
    let decoded = decode(LF_PROCEDURE, &data).unwrap();
    assert_eq!(decoded, IdRecord::Unsupported(record(LF_PROCEDURE, &data)));
}

#[test]
fn table_resolves_substrings_before_the_string() {
    let stream = ipi(&[
        (LF_STRING_ID, string_id(0, b"-c ")),
        (LF_STRING_ID, string_id(0, b"-O2")),
        (LF_SUBSTR_LIST, list(4, &[0x1000, 0x1001])),
        (LF_STRING_ID, string_id(0x1002, b" -X")),
        (LF_STRING_ID, string_id(0, b"")),
    ]);
    let table = IdTable::new(&stream).unwrap();
    assert_eq!(table.len(), 5);
    assert_eq!(table.index_begin(), IdIndex(0x1000));
    assert_eq!(table.index_end(), IdIndex(0x1005));
    assert_eq!(table.record(IdIndex(0x1005)), None);
    assert_eq!(table.record(IdIndex(0xfff)), None);

    let mut out = b"prefix:".to_vec();
    table.resolve_string(IdIndex(0x1003), &mut out).unwrap();
    assert_eq!(out, b"prefix:-c -O2 -X");
    out.clear();
    table.resolve_string(IdIndex(0x1004), &mut out).unwrap();
    assert_eq!(out, b"");

    let error = table.resolve_string(IdIndex(0x1002), &mut out).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
    let error = table.resolve_string(IdIndex(0x1005), &mut out).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
}

#[test]
fn table_resolution_is_bounded() {
    // The string refers to a list that contains the string again.
    let cycle = ipi(&[
        (LF_SUBSTR_LIST, list(4, &[0x1001])),
        (LF_STRING_ID, string_id(0x1000, b"x")),
    ]);
    let table = IdTable::new(&cycle).unwrap();
    let mut out = Vec::new();
    let error = table.resolve_string(IdIndex(0x1001), &mut out).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);

    // A chain one level deeper than the limit, with distinct records.
    let mut records = vec![(LF_STRING_ID, string_id(0, b"leaf"))];
    for level in 0..=MAX_STRING_DEPTH {
        let below = 0x1000 + u32::try_from(level * 2).unwrap();
        records.push((LF_SUBSTR_LIST, list(4, &[below])));
        records.push((LF_STRING_ID, string_id(below + 1, b"")));
    }
    let deep = ipi(&records);
    let table = IdTable::new(&deep).unwrap();
    let top = table.index_end().0 - 1;
    let error = table.resolve_string(IdIndex(top), &mut out).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    // One level less resolves.
    out.clear();
    table.resolve_string(IdIndex(top - 2), &mut out).unwrap();
    assert_eq!(out, b"leaf");
}

#[test]
fn table_validates_references() {
    let valid = ipi(&[
        (LF_STRING_ID, string_id(0, b"ns")),
        (LF_FUNC_ID, func_id(0x1000, 0x1010, b"f")),
        (LF_FUNC_ID, func_id(0, 0x0074, b"g")),
        (LF_BUILDINFO, list(2, &[0x1000, 0, 0x1000])),
        (LF_UDT_SRC_LINE, pad(u32s(&[0x1010, 0x1000, 3]))),
    ]);
    IdTable::new(&valid)
        .unwrap()
        .check(TypeIndex(0x1011))
        .unwrap();

    let cases: [(&[RawRecord], ErrorKind); 6] = [
        // The type index is past the TPI records.
        (
            &[(LF_FUNC_ID, func_id(0, 0x1011, b"f"))],
            ErrorKind::InvalidReference,
        ),
        // The primitive type index is not a known primitive.
        (
            &[(LF_FUNC_ID, func_id(0, 0x00ff, b"f"))],
            ErrorKind::InvalidReference,
        ),
        // The scope is past the IPI records.
        (
            &[(LF_FUNC_ID, func_id(0x1001, 0x1000, b"f"))],
            ErrorKind::InvalidReference,
        ),
        // The scope refers to a function, not a string.
        (
            &[(LF_FUNC_ID, func_id(0x1000, 0x1000, b"f"))],
            ErrorKind::InvalidReference,
        ),
        // The substring reference refers to a string, not a list.
        (
            &[(LF_STRING_ID, string_id(0x1000, b"s"))],
            ErrorKind::InvalidReference,
        ),
        // A type leaf in the IPI stream.
        (&[(LF_PROCEDURE, vec![0; 2])], ErrorKind::UnsupportedVersion),
    ];
    for (records, kind) in cases {
        let stream = ipi(records);
        let error = IdTable::new(&stream)
            .unwrap()
            .check(TypeIndex(0x1011))
            .unwrap_err();
        assert_eq!(error.kind(), kind);
        assert_eq!(error.stream_location(), Some((StreamId(4), 56)));
    }
}

#[test]
fn table_adds_the_record_location_to_decode_errors() {
    let stream = ipi(&[
        (LF_STRING_ID, string_id(0, b"ok")),
        (LF_FUNC_ID, vec![0, 0]),
    ]);
    let table = IdTable::new(&stream).unwrap();
    let error = table.leaf(IdIndex(0x1001)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    // The first record has a 4-byte prefix and a 10-byte payload.
    assert_eq!(error.stream_location(), Some((StreamId(4), 56 + 14)));
    let error = table.leaf(IdIndex(0x1002)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
}
