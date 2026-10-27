//! Tests for the Roslyn symbol attributes in `S_OEM` records: `MD2` custom debug
//! information and `asyncMethodInfo`.

use pdb_rs::ErrorKind;
use pdb_rs::native::codeview::Records;
use pdb_rs::native::codeview::symbol::{
    ASYNC_METHOD_INFO_ATTRIBUTE, AsyncMethodInfo, AttributeValue, CustomDebugInfo,
    CustomDebugInfoData, CustomDebugInfoKind, MANAGED_ATTRIBUTE_OEM_ID, MD2_ATTRIBUTE, Symbol,
    SymbolAttribute, SymbolData,
};
use pdb_rs::portable::custom::{Lambda, LambdaClosure, LocalSlot, StateMachineState};
use pdb_rs::portable::metadata::{IlOffset, MetadataToken};

/// The `S_OEM` payload with the `MD2` attribute of `ComputeAsync` in the
/// `dotnet-csharp-full` fixture (module 0, offset 0x3c).
const FIXTURE_MD2: &str = "c93feac6b359d649bc250902bbabb460000000004d004400320000000403000004040000300000003c0043006f006d0070007500740065004100730079006e0063003e0064005f005f003000000000000406000210000000010f014c1d54000004090000100000000300002b02530478";

/// The `S_OEM` payload with the `asyncMethodInfo` attribute of the `MoveNext`
/// method of `ComputeAsync` in the same fixture (module 9, offset 0xd8).
const FIXTURE_ASYNC: &str = "c93feac6b359d649bc250902bbabb460000000006100730079006e0063004d006500740068006f00640049006e0066006f00000001000006ffffffff03000000560000004700000675000000bd00000047000006dd00000032010000470000064f010000";

/// Decode hexadecimal text.
fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

/// Return an `S_OEM` record with `payload`.
fn oem_record(payload: &[u8]) -> Vec<u8> {
    let length = u16::try_from(payload.len() + 2).unwrap();
    [
        &length.to_le_bytes()[..],
        &0x0404_u16.to_le_bytes(),
        payload,
    ]
    .concat()
}

/// Return the UTF-16LE bytes of `text`.
fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// Return a record with `payload` and `pad` zero bytes. The size includes the header.
fn record(version: u8, kind: u8, alignment: u8, payload: &[u8], pad: usize) -> Vec<u8> {
    let size = u32::try_from(8 + payload.len() + pad).unwrap();
    let mut bytes = vec![version, kind, 0, alignment];
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes.resize(bytes.len() + pad, 0);
    bytes
}

/// Return a record as the Roslyn encoder writes it: padded to four bytes, with
/// the alignment size only for kinds after 5.
fn encoded(kind: u8, payload: &[u8]) -> Vec<u8> {
    let pad = (4 - payload.len() % 4) % 4;
    let alignment = if kind > 5 {
        u8::try_from(pad).unwrap()
    } else {
        0
    };
    record(4, kind, alignment, payload, pad)
}

/// Return an `MD2` value with the global header and `records`.
fn md2(count: u8, records: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = vec![4, count, 0, 0];
    for record in records {
        bytes.extend_from_slice(record);
    }
    bytes
}

/// Parse an `MD2` value with one record and return its decoded data.
fn decode_one(kind: u8, payload: &[u8], check: impl FnOnce(CustomDebugInfoData<'_>)) {
    let value = md2(1, &[encoded(kind, payload)]);
    let info = CustomDebugInfo::parse(&value).unwrap();
    let record = info.records().next().unwrap();
    assert_eq!(record.kind, CustomDebugInfoKind(kind));
    check(record.decode().unwrap());
}

/// Return the decode error kind of a one-record `MD2` value without padding.
fn decode_error(kind: u8, payload: &[u8]) -> ErrorKind {
    let value = md2(1, &[record(4, kind, 0, payload, 0)]);
    let info = CustomDebugInfo::parse(&value).unwrap();
    info.records().next().unwrap().decode().unwrap_err().kind()
}

/// Return the parse error kind of an `MD2` value.
fn parse_error(value: &[u8]) -> ErrorKind {
    CustomDebugInfo::parse(value).unwrap_err().kind()
}

#[test]
fn decodes_the_fixture_md2_attribute() {
    let payload = unhex(FIXTURE_MD2);
    let bytes = oem_record(&payload);
    let symbol = Symbol::decode(Records::new(&bytes).next().unwrap().unwrap()).unwrap();
    let SymbolData::Oem(oem) = symbol.data() else {
        panic!("expected S_OEM");
    };
    let attribute = oem.attribute().unwrap().unwrap();
    assert_eq!(attribute.name, MD2_ATTRIBUTE);
    assert_eq!(
        String::from_utf16(&attribute.name_units().collect::<Vec<_>>()).unwrap(),
        "MD2"
    );
    let AttributeValue::CustomDebugInfo(info) = attribute.decode().unwrap() else {
        panic!("expected MD2");
    };
    assert_eq!((info.len(), info.reserved), (3, [0, 0]));
    let records: Vec<_> = info.records().collect();
    assert_eq!(records.len(), 3);
    let kinds: Vec<_> = records.iter().map(|record| record.kind).collect();
    assert_eq!(
        kinds,
        [
            CustomDebugInfoKind::STATE_MACHINE_TYPE_NAME,
            CustomDebugInfoKind::EDIT_AND_CONTINUE_LOCAL_SLOT_MAP,
            CustomDebugInfoKind::EDIT_AND_CONTINUE_STATE_MACHINE_STATE_MAP,
        ]
    );
    assert_eq!(
        records[0].decode().unwrap(),
        CustomDebugInfoData::StateMachineTypeName(pdb_rs::Bytes::from(utf16("<ComputeAsync>d__0")))
    );
    // The alignment size 2 removes the two padding bytes.
    assert_eq!(records[1].alignment, 2);
    let CustomDebugInfoData::EditAndContinueLocalSlotMap(slots) = records[1].decode().unwrap()
    else {
        panic!("expected a local slot map");
    };
    assert_eq!(slots.as_bytes(), unhex("010f014c1d54"));
    let variable = |kind, syntax_offset| LocalSlot::Variable {
        kind,
        syntax_offset,
        ordinal: None,
    };
    assert_eq!(
        slots.iter().collect::<Vec<_>>(),
        [variable(0, 14), variable(0, 75), variable(0x1c, 83)]
    );
    let CustomDebugInfoData::EditAndContinueStateMachineStateMap(states) =
        records[2].decode().unwrap()
    else {
        panic!("expected a state machine state map");
    };
    assert_eq!(states.as_bytes(), unhex("0300002b02530478"));
    let state = |state_number, syntax_offset| {
        let mut state = StateMachineState::default();
        state.state_number = state_number;
        state.syntax_offset = syntax_offset;
        state
    };
    assert_eq!(
        states.iter().collect::<Vec<_>>(),
        [state(0, 43), state(1, 83), state(2, 120)]
    );
    assert_eq!(states.trailing(), []);
}

#[test]
fn decodes_the_fixture_async_method_info() {
    let payload = unhex(FIXTURE_ASYNC);
    let attribute = SymbolAttribute::parse(&payload[20..]).unwrap();
    assert_eq!(attribute.name, ASYNC_METHOD_INFO_ATTRIBUTE);
    let AttributeValue::AsyncMethodInfo(info) = attribute.decode().unwrap() else {
        panic!("expected asyncMethodInfo");
    };
    assert_eq!(info.kickoff_method, MetadataToken::from_raw(0x0600_0001));
    assert_eq!(info.catch_handler_offset, None);
    let steps: Vec<_> = info
        .steps
        .iter()
        .map(|step| {
            (
                step.yield_offset.0,
                step.resume_method.raw(),
                step.resume_offset.0,
            )
        })
        .collect();
    assert_eq!(
        steps,
        [
            (0x56, 0x0600_0047, 0x75),
            (0xbd, 0x0600_0047, 0xdd),
            (0x132, 0x0600_0047, 0x14f)
        ]
    );
    assert_eq!(info.steps.len(), 3);
    assert_eq!((&info.steps).into_iter().len(), 3);
}

#[test]
fn splits_symbol_attributes_at_the_nul_code_unit() {
    let data = [utf16("abc"), vec![0, 0, 9, 8, 0]].concat();
    let attribute = SymbolAttribute::parse(&data).unwrap();
    assert_eq!(attribute.name, utf16("abc"));
    assert_eq!(attribute.value, [9, 8, 0]);
    assert_eq!(
        attribute.decode().unwrap(),
        AttributeValue::Other(pdb_rs::Bytes::new(&[9, 8, 0]))
    );
    // A zero byte pair across two code units is not a NUL code unit.
    let error = SymbolAttribute::parse(&[0x41, 0, 0, 0x42, 1]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    for data in [&[][..], &[0], &[0x41, 0]] {
        let error = SymbolAttribute::parse(data).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    }
    // An empty name is valid.
    assert_eq!(SymbolAttribute::parse(&[0, 0]).unwrap().name, b"");
}

#[test]
fn only_the_managed_oem_id_has_attributes() {
    let mut payload = unhex(FIXTURE_MD2);
    payload[0] ^= 1;
    let bytes = oem_record(&payload);
    let symbol = Symbol::decode(Records::new(&bytes).next().unwrap().unwrap()).unwrap();
    let SymbolData::Oem(oem) = symbol.data() else {
        panic!("expected S_OEM");
    };
    assert_ne!(oem.id, MANAGED_ATTRIBUTE_OEM_ID);
    assert!(oem.attribute().unwrap().is_none());
}

#[test]
fn checks_the_global_header() {
    let empty = md2(0, &[]);
    let info = CustomDebugInfo::parse(&empty).unwrap();
    assert!(info.is_empty());
    assert_eq!(info.records().count(), 0);
    let mut reserved = empty.clone();
    reserved[2..4].copy_from_slice(&[7, 8]);
    assert_eq!(CustomDebugInfo::parse(&reserved).unwrap().reserved, [7, 8]);
    for version in [0, 3, 5] {
        assert_eq!(
            parse_error(&[version, 0, 0, 0]),
            ErrorKind::UnsupportedVersion
        );
    }
    for length in 0..4 {
        assert_eq!(
            parse_error(&[4, 0, 0, 0][..length]),
            ErrorKind::InvalidFormat
        );
    }
}

#[test]
fn checks_the_record_count_and_the_padding() {
    let one = encoded(1, &[1, 0, 0, 6]);
    // The count names two records, but only one follows.
    assert_eq!(
        parse_error(&md2(2, std::slice::from_ref(&one))),
        ErrorKind::InvalidFormat
    );
    // A second record after the count is not padding.
    assert_eq!(
        parse_error(&md2(1, &[one.clone(), one.clone()])),
        ErrorKind::InvalidFormat
    );
    // At most three zero bytes can follow the records.
    for pad in 1..4 {
        let value = [md2(1, std::slice::from_ref(&one)), vec![0; pad]].concat();
        assert_eq!(CustomDebugInfo::parse(&value).unwrap().len(), 1);
    }
    for tail in [&[0, 0, 0, 0][..], &[1]] {
        let value = [md2(1, std::slice::from_ref(&one)), tail.to_vec()].concat();
        assert_eq!(parse_error(&value), ErrorKind::InvalidFormat);
    }
}

#[test]
fn rejects_truncated_values() {
    let value = md2(
        3,
        &[
            encoded(0, &[1, 0, 2, 0]),
            encoded(4, &utf16("<M>d__0\0")),
            encoded(6, &[1, 2, 3]),
        ],
    );
    assert_eq!(CustomDebugInfo::parse(&value).unwrap().len(), 3);
    for length in 0..value.len() {
        assert_eq!(
            parse_error(&value[..length]),
            ErrorKind::InvalidFormat,
            "length {length}"
        );
    }
}

#[test]
fn checks_record_sizes() {
    for size in [0_u32, 7] {
        let mut bad = record(4, 1, 0, &[1, 0, 0, 6], 0);
        bad[4..8].copy_from_slice(&size.to_le_bytes());
        assert_eq!(parse_error(&md2(1, &[bad])), ErrorKind::InvalidFormat);
    }
    let mut past = record(4, 1, 0, &[1, 0, 0, 6], 0);
    past[4..8].copy_from_slice(&13_u32.to_le_bytes());
    assert_eq!(parse_error(&md2(1, &[past])), ErrorKind::InvalidFormat);
    // A header without a body is a valid frame.
    let value = md2(1, &[record(4, 7, 0, &[], 0)]);
    let info = CustomDebugInfo::parse(&value).unwrap();
    let record = info.records().next().unwrap();
    assert_eq!(record.data, b"");
}

#[test]
fn applies_the_alignment_size_after_kind_5() {
    // Kinds after 5 remove the padding.
    for alignment in 0..4 {
        let value = md2(1, &[record(4, 8, alignment, &[0, 0, 0, 0], 0)]);
        let info = CustomDebugInfo::parse(&value).unwrap();
        let record = info.records().next().unwrap();
        assert_eq!(record.data.len(), 4 - usize::from(alignment));
    }
    // The alignment size must be at most 3 and at most the body size.
    assert_eq!(
        parse_error(&md2(1, &[record(4, 6, 4, &[0; 8], 0)])),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        parse_error(&md2(1, &[record(4, 6, 2, &[0], 0)])),
        ErrorKind::InvalidFormat
    );
    // The alignment bytes must be zero.
    assert_eq!(
        parse_error(&md2(1, &[record(4, 7, 1, &[1, 2, 3, 4], 0)])),
        ErrorKind::InvalidFormat
    );
    // Kinds 0 to 5 have no alignment size.
    assert_eq!(
        parse_error(&md2(1, &[record(4, 5, 1, &[0; 4], 0)])),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn keeps_unknown_kinds_and_versions_framed() {
    let value = md2(
        2,
        &[
            record(4, 0x20, 3, &[1, 2, 3, 0, 0, 0], 0),
            record(3, 1, 0, &[9; 4], 0),
        ],
    );
    let info = CustomDebugInfo::parse(&value).unwrap();
    let records: Vec<_> = info.records().collect();
    assert_eq!(
        records[0].decode().unwrap(),
        CustomDebugInfoData::Unknown(pdb_rs::Bytes::new(&[1, 2, 3]))
    );
    assert_eq!(records[1].version, 3);
    assert_eq!(records[1].data, [9; 4]);
    assert_eq!(
        records[1].decode().unwrap_err().kind(),
        ErrorKind::UnsupportedVersion
    );
}

#[test]
fn decodes_using_groups() {
    decode_one(0, &[2, 0, 1, 0, 3, 0], |data| {
        let CustomDebugInfoData::UsingGroups(groups) = data else {
            panic!("expected using groups");
        };
        assert_eq!(groups.len(), 2);
        assert_eq!(groups.iter().collect::<Vec<_>>(), [1, 3]);
        assert_eq!(groups.into_iter().len(), 2);
    });
    decode_one(0, &[0, 0], |data| {
        assert!(matches!(data, CustomDebugInfoData::UsingGroups(groups) if groups.is_empty()));
    });
    assert_eq!(
        decode_error(0, &[4, 0, 1, 0, 3, 0]),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        decode_error(0, &[1, 0, 1, 0, 9, 0, 0, 0]),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn decodes_forward_records() {
    decode_one(1, &[5, 0, 0, 6], |data| {
        assert_eq!(
            data,
            CustomDebugInfoData::ForwardMethod(MetadataToken::from_raw(0x0600_0005))
        );
    });
    decode_one(2, &[9, 0, 0, 6], |data| {
        assert_eq!(
            data,
            CustomDebugInfoData::ForwardModule(MetadataToken::from_raw(0x0600_0009))
        );
    });
    assert_eq!(decode_error(1, &[5, 0, 0]), ErrorKind::InvalidFormat);
    assert_eq!(
        decode_error(2, &[5, 0, 0, 6, 1, 0, 0, 0]),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn decodes_hoisted_local_scopes() {
    let payload = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 9, 0, 0, 0];
    decode_one(3, &payload, |data| {
        let CustomDebugInfoData::StateMachineHoistedLocalScopes(scopes) = data else {
            panic!("expected hoisted scopes");
        };
        let scopes: Vec<_> = scopes
            .iter()
            .map(|scope| (scope.start.0, scope.end.0))
            .collect();
        assert_eq!(scopes, [(0, 0), (5, 9)]);
    });
    assert_eq!(decode_error(3, &payload[..16]), ErrorKind::InvalidFormat);
    // A negative count.
    assert_eq!(decode_error(3, &[0, 0, 0, 0x80]), ErrorKind::InvalidFormat);
}

#[test]
fn decodes_state_machine_type_names() {
    decode_one(4, &utf16("<Evens>d__0\0"), |data| {
        assert_eq!(
            data,
            CustomDebugInfoData::StateMachineTypeName(pdb_rs::Bytes::from(utf16("<Evens>d__0")))
        );
    });
    // Old compilers omit the NUL code unit.
    let value = md2(1, &[record(4, 4, 0, &utf16("<A>d"), 0)]);
    let info = CustomDebugInfo::parse(&value).unwrap();
    let record = info.records().next().unwrap();
    assert_eq!(
        record.decode().unwrap(),
        CustomDebugInfoData::StateMachineTypeName(pdb_rs::Bytes::from(utf16("<A>d")))
    );
    let value = md2(1, &[record_of(4, &[0x41, 0, 0x42])]);
    let info = CustomDebugInfo::parse(&value).unwrap();
    let record = info.records().next().unwrap();
    assert_eq!(
        record.decode().unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
}

/// Return an unpadded version 4 record without an alignment size.
fn record_of(kind: u8, payload: &[u8]) -> Vec<u8> {
    record(4, kind, 0, payload, 0)
}

/// Return one dynamic local entry.
fn dynamic_local(flags: &[u8], flag_count: u32, slot: u32, name: &str) -> Vec<u8> {
    let mut bytes = flags.to_vec();
    bytes.resize(64, 0);
    bytes.extend_from_slice(&flag_count.to_le_bytes());
    bytes.extend_from_slice(&slot.to_le_bytes());
    let mut name = utf16(name);
    name.resize(128, 0);
    bytes.extend_from_slice(&name);
    bytes
}

#[test]
fn decodes_dynamic_locals() {
    let full = "x".repeat(64);
    let payload = [
        2_u32.to_le_bytes().to_vec(),
        dynamic_local(&[1, 0, 1], 3, 2, "d"),
        dynamic_local(&[1; 64], 70, 0, &full),
    ]
    .concat();
    decode_one(5, &payload, |data| {
        let CustomDebugInfoData::DynamicLocals(locals) = data else {
            panic!("expected dynamic locals");
        };
        let locals: Vec<_> = locals.iter().collect();
        assert_eq!(locals.len(), 2);
        assert_eq!(locals[0].flags, [1, 0, 1]);
        assert_eq!(locals[0].flag_count, 3);
        assert_eq!(locals[0].slot, 2);
        assert_eq!(locals[0].name, utf16("d"));
        // The flag field holds at most 64 flags, and a full name has no NUL code unit.
        assert_eq!((locals[1].flags.len(), locals[1].flag_count), (64, 70));
        assert_eq!(locals[1].name, utf16(&full));
    });
    assert_eq!(
        decode_error(5, &payload[..payload.len() - 1]),
        ErrorKind::InvalidFormat
    );
}

/// Return one tuple element names entry.
fn tuple_entry(names: &[&str], slot: i32, scope: (u32, u32), name: &str) -> Vec<u8> {
    let mut bytes = u32::try_from(names.len()).unwrap().to_le_bytes().to_vec();
    for element in names {
        bytes.extend_from_slice(element.as_bytes());
        bytes.push(0);
    }
    bytes.extend_from_slice(&slot.to_le_bytes());
    bytes.extend_from_slice(&scope.0.to_le_bytes());
    bytes.extend_from_slice(&scope.1.to_le_bytes());
    bytes.extend_from_slice(name.as_bytes());
    bytes.push(0);
    bytes
}

#[test]
fn decodes_tuple_element_names() {
    let payload = [
        2_u32.to_le_bytes().to_vec(),
        tuple_entry(&["a", ""], 1, (0, 0), "t"),
        tuple_entry(&["x", "y", "z"], -1, (2, 10), "c"),
    ]
    .concat();
    decode_one(8, &payload, |data| {
        let CustomDebugInfoData::TupleElementNames(names) = data else {
            panic!("expected tuple element names");
        };
        assert_eq!(names.len(), 2);
        let entries: Vec<_> = names.iter().collect();
        let first: Vec<&[u8]> = entries[0].element_names.iter().collect();
        assert_eq!(first, [&b"a"[..], b""]);
        assert_eq!(entries[0].slot, 1);
        assert_eq!(entries[0].name, b"t");
        assert_eq!(entries[1].element_names.len(), 3);
        assert_eq!(entries[1].slot, -1);
        assert_eq!(
            (entries[1].scope_start, entries[1].scope_end),
            (IlOffset(2), IlOffset(10))
        );
        assert_eq!(entries[1].name, b"c");
    });
    for length in 0..payload.len() {
        let value = md2(1, &[record_of(8, &payload[..length])]);
        let info = CustomDebugInfo::parse(&value).unwrap();
        let record = info.records().next().unwrap();
        assert_eq!(
            record.decode().unwrap_err().kind(),
            ErrorKind::InvalidFormat,
            "length {length}"
        );
    }
}

#[test]
fn decodes_edit_and_continue_records_through_the_shared_codecs() {
    decode_one(6, &[0x01, 0x0f, 0x00, 0x81, 0x05, 0x02], |data| {
        let CustomDebugInfoData::EditAndContinueLocalSlotMap(slots) = data else {
            panic!("expected a local slot map");
        };
        assert_eq!(
            slots.iter().collect::<Vec<_>>(),
            [
                LocalSlot::Variable {
                    kind: 0,
                    syntax_offset: 14,
                    ordinal: None
                },
                LocalSlot::Temporary,
                LocalSlot::Variable {
                    kind: 0,
                    syntax_offset: 4,
                    ordinal: Some(2)
                },
            ]
        );
    });
    decode_one(7, &[3, 1, 1, 0x0b, 0x15, 2], |data| {
        let CustomDebugInfoData::EditAndContinueLambdaMap(map) = data else {
            panic!("expected a lambda map");
        };
        assert_eq!(map.method_ordinal(), 2);
        assert_eq!(map.closures().collect::<Vec<_>>(), [10]);
        let mut lambda = Lambda::default();
        lambda.syntax_offset = 20;
        lambda.closure = LambdaClosure::Closure(0);
        assert_eq!(map.lambdas().collect::<Vec<_>>(), [lambda]);
    });
    // The shared codecs report their errors through the native record.
    assert_eq!(decode_error(6, &[0x01]), ErrorKind::UnexpectedEof);
    assert_eq!(
        decode_error(7, &[3, 1, 1, 0x0b, 0x15, 3]),
        ErrorKind::InvalidFormat
    );
    assert_eq!(decode_error(9, &[1, 0, 0, 1, 7]), ErrorKind::InvalidFormat);
}

/// Return an `asyncMethodInfo` value.
fn async_info(catch: u32, steps: &[[u32; 3]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for word in [0x0600_0001, catch, u32::try_from(steps.len()).unwrap()] {
        bytes.extend_from_slice(&u32::to_le_bytes(word));
    }
    for step in steps {
        for word in step {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
    }
    bytes
}

#[test]
fn decodes_async_method_info() {
    let value = async_info(0x298, &[[0x19a, 0x0600_004f, 0x1b8]]);
    let info = AsyncMethodInfo::parse(&value).unwrap();
    assert_eq!(info.catch_handler_offset, Some(IlOffset(0x298)));
    let step = info.steps.iter().next().unwrap();
    assert_eq!(
        (
            step.yield_offset.0,
            step.resume_method.raw(),
            step.resume_offset.0
        ),
        (0x19a, 0x0600_004f, 0x1b8)
    );
    // No steps, and zero record padding.
    let empty = [async_info(u32::MAX, &[]), vec![0; 3]].concat();
    let info = AsyncMethodInfo::parse(&empty).unwrap();
    assert!(info.steps.is_empty());
    assert_eq!(info.catch_handler_offset, None);
}

#[test]
fn rejects_malformed_async_method_info() {
    let value = async_info(0, &[[1, 2, 3], [4, 5, 6]]);
    for length in 0..value.len() {
        let error = AsyncMethodInfo::parse(&value[..length]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat, "length {length}");
    }
    for tail in [&[0, 0, 0, 0][..], &[1]] {
        let longer = [value.clone(), tail.to_vec()].concat();
        let error = AsyncMethodInfo::parse(&longer).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    }
    // A step count that does not fit the value.
    let mut huge = async_info(0, &[]);
    huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        AsyncMethodInfo::parse(&huge).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
}
