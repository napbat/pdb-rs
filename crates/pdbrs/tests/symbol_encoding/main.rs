//! Tests that every decoded symbol kind encodes to its original bytes, and
//! the boundary and error cases of the encoders.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::id_record::IdList;
use pdbrs::native::codeview::numeric::{Integer, Numeric};
use pdbrs::native::codeview::symbol::{
    Annotation, BinaryAnnotations, FunctionList, RegisterList, Symbol, SymbolData,
};
use pdbrs::native::codeview::{Records, SymbolKind};

/// `S_ALTOBJNAME`: no reference gives its layout, so it stays raw.
const S_ALTOBJNAME: u16 = 0x1181;

mod symbol_kinds;

use symbol_kinds::{Payload, all_kinds};

/// Frame one record without padding.
fn unpadded(kind: u16, payload: &[u8]) -> Vec<u8> {
    let length = u16::try_from(payload.len() + 2).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&kind.to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

/// Frame one record with zero padding to a 4-byte boundary.
fn padded(kind: u16, payload: &[u8]) -> Vec<u8> {
    let mut payload = payload.to_vec();
    while !(payload.len() + 4).is_multiple_of(4) {
        payload.push(0);
    }
    unpadded(kind, &payload)
}

/// Decode the single record in `bytes`.
fn decode(bytes: &[u8]) -> pdbrs::Result<Symbol<'_>> {
    Symbol::decode(Records::new(bytes).next().unwrap()?)
}

/// Encode `symbol` and return the bytes.
fn encode(symbol: &Symbol<'_>) -> pdbrs::Result<Vec<u8>> {
    let mut out = Vec::new();
    symbol.encode(&mut out)?;
    Ok(out)
}

/// Return the error kind of encoding `data` as `kind`.
fn encode_error(data: &SymbolData<'_>, kind: u16) -> ErrorKind {
    let mut out = Vec::new();
    data.encode(SymbolKind(kind), &mut out).unwrap_err().kind()
}

#[test]
fn every_kind_decodes_and_encodes_to_the_same_bytes() {
    let kinds = all_kinds();
    assert!(kinds.len() > 100, "{} kinds", kinds.len());
    for case in &kinds {
        let bytes = padded(case.kind, &case.payload);
        let symbol =
            decode(&bytes).unwrap_or_else(|error| panic!("kind {:#06x}: {error:?}", case.kind));
        assert!(
            !matches!(symbol.data(), SymbolData::Unsupported(_)),
            "kind {:#06x} has no decoder",
            case.kind
        );
        assert_eq!(encode(&symbol).unwrap(), bytes, "kind {:#06x}", case.kind);
        // A foreign kind does not accept the fields.
        let foreign = if matches!(symbol.data(), SymbolData::Public(_)) {
            0x1108
        } else {
            0x110e
        };
        assert_eq!(
            encode_error(symbol.data(), foreign),
            ErrorKind::InvalidFormat,
            "kind {:#06x}",
            case.kind
        );
    }
}

#[test]
fn every_kind_rejects_truncated_fixed_fields() {
    for case in &all_kinds() {
        for cut in 0..case.fixed {
            let bytes = unpadded(case.kind, &case.payload[..cut]);
            assert!(
                decode(&bytes).is_err(),
                "kind {:#06x} accepts {cut} of {} fixed bytes",
                case.kind,
                case.fixed
            );
        }
    }
}

#[test]
fn every_kind_has_one_case() {
    let kinds = all_kinds();
    let mut seen: Vec<u16> = kinds.iter().map(|case| case.kind).collect();
    seen.sort_unstable();
    let count = seen.len();
    seen.dedup();
    assert_eq!(seen.len(), count, "a kind has two cases");
}

#[test]
fn every_decoded_kind_has_a_case() {
    let kinds: Vec<u16> = all_kinds().iter().map(|case| case.kind).collect();
    let mut decoded = 0;
    for kind in 0..=u16::MAX {
        let bytes = unpadded(kind, &[]);
        if matches!(
            decode(&bytes).map(|symbol| *symbol.data()),
            Ok(SymbolData::Unsupported(_))
        ) {
            continue;
        }
        decoded += 1;
        assert!(kinds.contains(&kind), "kind {kind:#06x} has no case");
    }
    assert_eq!(decoded, kinds.len());
}

#[test]
fn unsupported_kinds_stay_raw_and_encode_raw() {
    let payload = [1, 2, 3, 4];
    let bytes = padded(S_ALTOBJNAME, &payload);
    let symbol = decode(&bytes).unwrap();
    assert_eq!(*symbol.data(), SymbolData::Unsupported(&payload));
    assert_eq!(encode(&symbol).unwrap(), bytes);
    // Raw bytes are not valid for a kind with a decoder.
    assert_eq!(
        encode_error(&SymbolData::Unsupported(&payload), 0x110e),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn names_with_nul_bytes_are_rejected() {
    let payload = Payload::default().u32(0).u32(0x10).u16(1).name(b"main");
    let bytes = padded(0x110e, &payload.0);
    let symbol = decode(&bytes).unwrap();
    let SymbolData::Public(mut public) = *symbol.data() else {
        panic!("expected a public");
    };
    public.name = b"ma\0in";
    assert_eq!(
        encode_error(&SymbolData::Public(public), 0x110e),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn records_longer_than_the_length_field_are_rejected() {
    let payload = Payload::default().u32(0).u32(0x10).u16(1).name(b"main");
    let bytes = padded(0x110e, &payload.0);
    let symbol = decode(&bytes).unwrap();
    let SymbolData::Public(mut public) = *symbol.data() else {
        panic!("expected a public");
    };
    // 10 bytes of fields, the name and its NUL: 65524 + 10 + 1 + 2 = 65537.
    let long = vec![b'x'; 0xfff4];
    public.name = &long;
    let mut out = vec![0xaa];
    let error = SymbolData::Public(public).encode(SymbolKind(0x110e), &mut out);
    assert_eq!(error.unwrap_err().kind(), ErrorKind::LimitExceeded);
    // The output is unchanged after the error.
    assert_eq!(out, [0xaa]);
    // One byte shorter fits: the length field is 0xfffe after padding.
    let fits = vec![b'x'; 0xfff1];
    public.name = &fits;
    out.clear();
    SymbolData::Public(public)
        .encode(SymbolKind(0x110e), &mut out)
        .unwrap();
    assert_eq!(out.len(), 0x1_0000);
    assert_eq!(decode(&out).unwrap().name(), Some(&fits[..]));
}

#[test]
fn procedure_signature_must_match_the_kind() {
    let case = all_kinds()
        .into_iter()
        .find(|case| case.kind == 0x1110)
        .unwrap();
    let bytes = padded(case.kind, &case.payload);
    let symbol = decode(&bytes).unwrap();
    // S_GPROC32 has a type index; S_GPROC32_ID and S_GPROC32EX need other fields.
    for kind in [0x1147, 0x1172] {
        assert_eq!(encode_error(symbol.data(), kind), ErrorKind::InvalidFormat);
    }
    let mut out = Vec::new();
    symbol.data().encode(SymbolKind(0x110f), &mut out).unwrap();
    assert_eq!(out[2..4], 0x110f_u16.to_le_bytes());
}

#[test]
fn constant_keeps_its_stored_leaf_and_encodes_new_values_canonically() {
    // LF_ULONG 7 is wider than necessary.
    let payload = Payload::default()
        .u32(0x74)
        .u16(0x8004)
        .u32(7)
        .name(b"seven");
    let bytes = padded(0x1107, &payload.0);
    let symbol = decode(&bytes).unwrap();
    assert_eq!(encode(&symbol).unwrap(), bytes);
    let SymbolData::Constant(mut constant) = *symbol.data() else {
        panic!("expected a constant");
    };
    constant.value = Numeric::from_integer(Integer::Unsigned(8));
    let mut out = Vec::new();
    SymbolData::Constant(constant)
        .encode(SymbolKind(0x1107), &mut out)
        .unwrap();
    let expected = Payload::default().u32(0x74).u16(8).name(b"seven");
    assert_eq!(out, padded(0x1107, &expected.0));
}

#[test]
fn thunk_variant_data_needs_a_variant_kind() {
    let payload = Payload::default()
        .u32(0)
        .u32(0)
        .u32(0)
        .u32(0x10)
        .u16(1)
        .u16(5)
        .u8(0)
        .name(b"t");
    let bytes = padded(0x1102, &payload.0);
    let symbol = decode(&bytes).unwrap();
    let SymbolData::Thunk(mut thunk) = *symbol.data() else {
        panic!("expected a thunk");
    };
    thunk.variant = &[1, 2];
    assert_eq!(
        encode_error(&SymbolData::Thunk(thunk), 0x1102),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn register_lists_check_their_count_width() {
    assert_eq!(
        RegisterList::parse(&[1, 2, 3], true).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        RegisterList::parse(&[0; 256], false).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    let list = RegisterList::parse(&[0; 255], false).unwrap();
    assert_eq!(list.len(), 255);
    let wide = RegisterList::parse(&[0x4c, 0x01, 0x4d, 0x01], true).unwrap();
    assert_eq!(wide.iter().collect::<Vec<_>>(), [0x14c, 0x14d]);
}

#[test]
fn function_lists_check_their_counts() {
    let functions = IdList::parse(&[1, 0, 0, 0]).unwrap();
    assert!(FunctionList::new(functions, &[5, 0, 0, 0]).is_ok());
    assert_eq!(
        FunctionList::new(functions, &[5, 0, 0, 0, 6, 0, 0, 0])
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        IdList::parse(&[1, 0, 0]).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn annotations_encode_to_their_compressed_form() {
    let annotations = [
        Annotation::CodeOffset(0x7f),
        Annotation::ChangeCodeOffset(0x80),
        Annotation::ChangeCodeLength(0x3fff),
        Annotation::ChangeFile(0x4000),
        Annotation::ChangeLineOffset(-3),
        Annotation::ChangeLineOffset(5),
        Annotation::ChangeColumnEndDelta(-1),
        Annotation::ChangeCodeOffsetAndLineOffset {
            code_delta: 0xf,
            line_delta: -2,
        },
        Annotation::ChangeCodeLengthAndCodeOffset {
            code_length: 3,
            code_delta: 0x1fff_ffff,
        },
        Annotation::ChangeCodeOffsetBase(0),
        Annotation::ChangeLineEndDelta(1),
        Annotation::ChangeRangeKind(1),
        Annotation::ChangeColumnStart(2),
        Annotation::ChangeColumnEnd(9),
    ];
    let mut bytes = Vec::new();
    for annotation in &annotations {
        annotation.encode(&mut bytes).unwrap();
    }
    let decoded: Vec<Annotation> = BinaryAnnotations::parse(&bytes).unwrap().iter().collect();
    assert_eq!(decoded, annotations);
    assert_eq!(bytes[..2], [1, 0x7f]);
    assert_eq!(bytes[2..5], [3, 0x80, 0x80]);
    for annotation in [
        Annotation::CodeOffset(0x2000_0000),
        Annotation::ChangeLineOffset(0x1000_0000),
        Annotation::ChangeCodeOffsetAndLineOffset {
            code_delta: 0x10,
            line_delta: 0,
        },
        Annotation::ChangeCodeOffsetAndLineOffset {
            code_delta: 0,
            line_delta: 0x0100_0000,
        },
    ] {
        let mut out = Vec::new();
        assert_eq!(
            annotation.encode(&mut out).unwrap_err().kind(),
            ErrorKind::LimitExceeded,
            "{annotation:?}"
        );
    }
}

#[test]
fn hlsl_def_range_offsets_follow_the_flags() {
    let case = all_kinds()
        .into_iter()
        .find(|case| case.kind == 0x1150)
        .unwrap();
    let bytes = padded(case.kind, &case.payload);
    let symbol = decode(&bytes).unwrap();
    let SymbolData::DefRangeHlsl(mut range) = *symbol.data() else {
        panic!("expected an HLSL def-range");
    };
    assert_eq!(range.register_index_count(), 1);
    assert_eq!(range.offsets().collect::<Vec<_>>(), [0x40]);
    assert_eq!(range.gaps.len(), 1);
    range.flags = 2;
    assert_eq!(
        encode_error(&SymbolData::DefRangeHlsl(range), 0x1150),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn narrow_hlsl_slots_must_fit_in_16_bits() {
    let case = all_kinds()
        .into_iter()
        .find(|case| case.kind == 0x1151)
        .unwrap();
    let bytes = padded(case.kind, &case.payload);
    let symbol = decode(&bytes).unwrap();
    let SymbolData::HlslData(mut data) = *symbol.data() else {
        panic!("expected HLSL data");
    };
    data.uav_slot = 0x1_0000;
    assert_eq!(
        encode_error(&SymbolData::HlslData(data), 0x1151),
        ErrorKind::LimitExceeded
    );
    // The 32-bit kind keeps the wide slot.
    let mut out = Vec::new();
    SymbolData::HlslData(data)
        .encode(SymbolKind(0x1162), &mut out)
        .unwrap();
    let SymbolData::HlslData(wide) = *decode(&out).unwrap().data() else {
        panic!("expected HLSL data");
    };
    assert_eq!(wide.uav_slot, 0x1_0000);
}

#[test]
fn pdb_map_names_are_utf16_without_nul() {
    let case = all_kinds()
        .into_iter()
        .find(|case| case.kind == 0x1161)
        .unwrap();
    let bytes = padded(case.kind, &case.payload);
    let symbol = decode(&bytes).unwrap();
    let SymbolData::PdbMap(mut map) = *symbol.data() else {
        panic!("expected a PDB map");
    };
    assert_eq!(map.from, b"a\0");
    map.to = b"b\0\0\0";
    assert_eq!(
        encode_error(&SymbolData::PdbMap(map), 0x1161),
        ErrorKind::InvalidFormat
    );
    map.to = b"b";
    assert_eq!(
        encode_error(&SymbolData::PdbMap(map), 0x1161),
        ErrorKind::InvalidFormat
    );
}
