//! Tests for `CodeView` record framing and symbol decoding.

use pdbrs::ErrorKind;
use pdbrs::native::address::SectionOffset;
use pdbrs::native::codeview::index::{IdIndex, ModuleSymbolOffset, TypeIndex};
use pdbrs::native::codeview::symbol::{
    ProcedureFlags, ProcedureType, PublicFlags, S_GDATA32, S_GPROC32_ID, S_LDATA32, S_LPROC32,
    S_PUB32, Symbol, SymbolData,
};
use pdbrs::native::codeview::{Record, Records, SymbolKind};

/// Serialize one record with `kind` and `payload`.
fn record(kind: u16, payload: &[u8]) -> Vec<u8> {
    let length = u16::try_from(payload.len() + 2).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&kind.to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn decode_kind(bytes: &[u8]) -> ErrorKind {
    let record = Records::new(bytes).next().unwrap().unwrap();
    Symbol::decode(record).unwrap_err().kind()
}

fn public_payload(name: &[u8]) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&2_u32.to_le_bytes());
    payload.extend_from_slice(&0x1234_u32.to_le_bytes());
    payload.extend_from_slice(&1_u16.to_le_bytes());
    payload.extend_from_slice(name);
    payload
}

#[test]
fn frames_records_of_all_kinds_with_their_padding() {
    let mut bytes = record(0x9999, &[0xaa, 0xbb, 0xf2, 0xf1]);
    bytes.extend(record(0x1504, &[]));
    bytes.extend(record(
        0x1606,
        &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14],
    ));

    let records: Vec<Record<'_>> = Records::new(&bytes).map(Result::unwrap).collect();

    assert_eq!(records.len(), 3);
    assert_eq!(
        (records[0].offset, records[0].kind, records[0].data),
        (0, 0x9999, &[0xaa, 0xbb, 0xf2, 0xf1][..])
    );
    assert_eq!((records[1].offset, records[1].kind), (8, 0x1504));
    assert_eq!(records[1].data, []);
    assert_eq!((records[2].offset, records[2].kind), (12, 0x1606));
    assert_eq!(records[2].data.len(), 14);
}

#[test]
fn empty_sequence_has_no_records() {
    assert!(Records::new(&[]).next().is_none());
}

#[test]
fn rejects_length_that_does_not_cover_the_kind() {
    for length in [0_u16, 1] {
        let mut bytes = record(0x1101, &[0; 4]);
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(&[0; 6]);
        let mut records = Records::new(&bytes);
        assert!(records.next().unwrap().is_ok());
        let error = records.next().unwrap().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat);
        assert_eq!(error.offset(), None);
        assert_eq!(records.position(), 8);
        assert!(records.next().is_none());
    }
}

#[test]
fn rejects_truncated_prefix_at_each_length() {
    for trailing in 1..4 {
        let mut bytes = record(0x1101, &[]);
        bytes.extend(std::iter::repeat_n(0x06, trailing));
        let mut records = Records::new(&bytes);
        assert!(records.next().unwrap().is_ok());
        let error = records.next().unwrap().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
        assert_eq!(records.position(), 4);
        assert!(records.next().is_none());
    }
}

#[test]
fn rejects_record_that_extends_past_the_end() {
    let mut bytes = record(0x1101, &[1, 2, 3, 4]);
    bytes.pop();
    let mut records = Records::new(&bytes);
    let error = records.next().unwrap().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(records.position(), 0);
    assert!(records.next().is_none());
}

#[test]
fn accepts_record_that_ends_exactly_at_the_end() {
    let bytes = record(0x1101, &[1, 2, 3]);
    let mut records = Records::new(&bytes);
    assert_eq!(records.next().unwrap().unwrap().data, &[1, 2, 3]);
    assert!(records.next().is_none());
    assert_eq!(records.position(), bytes.len());
}

#[test]
fn decodes_public_name_without_padding() {
    let bytes = record(S_PUB32.0, &public_payload(b"main\0\0\0"));
    let record = Records::new(&bytes).next().unwrap().unwrap();
    let symbol = Symbol::decode(record).unwrap();

    assert_eq!(symbol.kind(), S_PUB32);
    assert_eq!(
        symbol.name().map(pdbrs::Bytes::as_bytes),
        Some(&b"main"[..])
    );
    let address = SectionOffset {
        section: 1,
        offset: 0x1234,
    };
    assert_eq!(symbol.address(), Some(address));
    let SymbolData::Public(public) = symbol.data() else {
        panic!("expected a public symbol");
    };
    assert_eq!(public.flags, PublicFlags::FUNCTION);
    assert_eq!(symbol.type_index(), None);
    assert_eq!(symbol.id_index(), None);
    assert_eq!(symbol.record(), record);
}

#[test]
fn rejects_padding_that_is_not_zero() {
    let bytes = record(S_PUB32.0, &public_payload(b"_start\0\xf2\xf1"));
    assert_eq!(decode_kind(&bytes), ErrorKind::InvalidFormat);
}

#[test]
fn keeps_four_or_more_zero_padding_bytes() {
    // The HLSL compiler fxc writes more zero bytes than the alignment needs.
    let bytes = record(S_PUB32.0, &public_payload(b"main\0\0\0\0\0"));
    let symbol = Symbol::decode(Records::new(&bytes).next().unwrap().unwrap()).unwrap();
    assert_eq!(
        symbol.name().map(pdbrs::Bytes::as_bytes),
        Some(&b"main"[..])
    );
    assert!(symbol.trailing_zeros() >= 4);
    let mut out = Vec::new();
    symbol.encode(&mut out).unwrap();
    assert_eq!(out, bytes);
}

#[test]
fn keeps_the_length_of_an_unaligned_record() {
    // Regression (#35, fuzz target `native_leaf_encode`): a decoded record whose length is
    // not a multiple of 4 encoded with alignment padding, so the bytes differed from the
    // input record.
    for bytes in [
        record(0x1607, &[0x8e, 0, 0, 0, 0xfd, 0x47, 0, 0, 9, 0]),
        record(S_PUB32.0, &public_payload(b"main\0")),
    ] {
        assert!(!bytes.len().is_multiple_of(4));
        let symbol = Symbol::decode(Records::new(&bytes).next().unwrap().unwrap()).unwrap();
        let mut out = Vec::new();
        symbol.encode(&mut out).unwrap();
        assert_eq!(out, bytes);
    }
}

#[test]
fn encodes_a_record_of_the_largest_length_without_padding() {
    // Regression (#35, fuzz target `native_symbol_encode`): a decoded record with the length
    // field 0xffff did not encode, because the alignment padding made it too long.
    let mut name = vec![b'a'; 0xfff2];
    name.push(0);
    let bytes = record(S_PUB32.0, &public_payload(&name));
    assert_eq!(bytes[..2], [0xff, 0xff]);
    let symbol = Symbol::decode(Records::new(&bytes).next().unwrap().unwrap()).unwrap();
    let mut out = Vec::new();
    symbol.encode(&mut out).unwrap();
    assert_eq!(out, bytes);
}

#[test]
fn decodes_data_symbols() {
    for kind in [S_GDATA32, S_LDATA32] {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0x1003_u32.to_le_bytes());
        payload.extend_from_slice(&0x40_u32.to_le_bytes());
        payload.extend_from_slice(&3_u16.to_le_bytes());
        payload.extend_from_slice(b"GLOBAL\0\0");
        let bytes = record(kind.0, &payload);
        let record = Records::new(&bytes).next().unwrap().unwrap();
        let symbol = Symbol::decode(record).unwrap();

        assert_eq!(symbol.kind(), kind);
        assert_eq!(
            symbol.name().map(pdbrs::Bytes::as_bytes),
            Some(&b"GLOBAL"[..])
        );
        let address = SectionOffset {
            section: 3,
            offset: 0x40,
        };
        assert_eq!(symbol.address(), Some(address));
        assert_eq!(symbol.type_index(), Some(TypeIndex(0x1003)));
        assert_eq!(symbol.scope_links(), None);
    }
}

#[test]
fn decodes_procedure_fields() {
    for (kind, signature) in [
        (S_GPROC32_ID, ProcedureType::Id(IdIndex(0x1010))),
        (S_LPROC32, ProcedureType::Type(TypeIndex(0x1010))),
    ] {
        let mut payload = Vec::new();
        for value in [0_u32, 0x200, 0, 0x55, 4, 0x50, 0x1010, 0x30] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload.extend_from_slice(&1_u16.to_le_bytes());
        payload.push(0x80);
        payload.extend_from_slice(b"_ZN4core3fmt5write17h0123456789abcdefE\0");
        let bytes = record(kind.0, &payload);
        let record = Records::new(&bytes).next().unwrap().unwrap();
        let symbol = Symbol::decode(record).unwrap();

        assert_eq!(
            symbol.name().map(pdbrs::Bytes::as_bytes),
            Some(&b"_ZN4core3fmt5write17h0123456789abcdefE"[..])
        );
        let SymbolData::Procedure(procedure) = symbol.data() else {
            panic!("expected a procedure");
        };
        assert_eq!(procedure.address.section, 1);
        assert_eq!(procedure.address.offset, 0x30);
        assert_eq!(procedure.code_length, 0x55);
        assert_eq!(procedure.signature, signature);
        assert_eq!(
            (procedure.end, procedure.debug_start),
            (ModuleSymbolOffset(0x200), 4)
        );
        assert_eq!(procedure.debug_end, 0x50);
        assert!(
            procedure
                .flags
                .contains(ProcedureFlags::OPTIMIZED_DEBUG_INFO)
        );
        let links = symbol.scope_links().unwrap();
        assert_eq!(
            (links.parent, links.end),
            (ModuleSymbolOffset(0), ModuleSymbolOffset(0x200))
        );
        match signature {
            ProcedureType::Id(index) => {
                assert_eq!(symbol.id_index(), Some(index));
                assert_eq!(symbol.type_index(), None);
            }
            ProcedureType::Type(index) => {
                assert_eq!(symbol.type_index(), Some(index));
                assert_eq!(symbol.id_index(), None);
            }
        }
    }
}

#[test]
fn keeps_unsupported_kinds_raw() {
    // No reference gives the layout of S_ALTOBJNAME, so this crate keeps it raw.
    let bytes = record(0x1181, &[0x10, 0x10, 0, 0, 0xff, b'T', 0, 0]);
    let record = Records::new(&bytes).next().unwrap().unwrap();
    let symbol = Symbol::decode(record).unwrap();
    assert_eq!(symbol.kind(), SymbolKind(0x1181));
    assert_eq!(
        *symbol.data(),
        SymbolData::Unsupported(pdbrs::Bytes::new(&[0x10, 0x10, 0, 0, 0xff, b'T', 0, 0]))
    );
    assert_eq!(symbol.name(), None);
    assert_eq!(symbol.address(), None);
}

#[test]
fn rejects_symbol_name_without_terminator() {
    let bytes = record(S_PUB32.0, &public_payload(b"main"));
    assert_eq!(decode_kind(&bytes), ErrorKind::InvalidFormat);
}

#[test]
fn rejects_payload_shorter_than_fixed_fields() {
    let public = public_payload(b"");
    assert_eq!(
        decode_kind(&record(S_PUB32.0, &public[..9])),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        decode_kind(&record(S_GPROC32_ID.0, &[0; 34])),
        ErrorKind::InvalidFormat
    );
    // The fixed fields are complete, but the name terminator is absent.
    assert_eq!(
        decode_kind(&record(S_GPROC32_ID.0, &[1; 35])),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        decode_kind(&record(S_LDATA32.0, &[0; 10])),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn accepts_empty_name() {
    let bytes = record(S_GDATA32.0, &[0; 11]);
    let record = Records::new(&bytes).next().unwrap().unwrap();
    assert_eq!(
        Symbol::decode(record)
            .unwrap()
            .name()
            .map(pdbrs::Bytes::as_bytes),
        Some(&b""[..])
    );
}
