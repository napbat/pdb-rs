//! Tests for the managed symbol kinds that .NET symbol writers produce.

use pdbrs::native::address::SectionOffset;
use pdbrs::native::codeview::index::{ModuleIndex, ModuleSymbolOffset, TypeIndex};
use pdbrs::native::codeview::numeric::{Integer, Numeric};
use pdbrs::native::codeview::symbol::{
    LocalFlags, MANAGED_ATTRIBUTE_OEM_ID, ModuleSymbolTarget, ProcedureFlags, S_GMANPROC,
    S_LMANPROC, S_MANCONSTANT, S_MANSLOT, S_OEM, S_PROCREF, S_TOKENREF, Symbol, SymbolData,
};
use pdbrs::native::codeview::{Records, SymbolKind};
use pdbrs::portable::metadata::MetadataToken;
use pdbrs::{Bytes, ErrorKind};

/// The module symbol offset `value`.
const fn at(value: u32) -> ModuleSymbolOffset {
    ModuleSymbolOffset(value)
}

/// A builder for little-endian symbol payloads.
#[derive(Clone, Default)]
struct Payload(Vec<u8>);

impl Payload {
    fn u8(mut self, value: u8) -> Self {
        self.0.push(value);
        self
    }

    fn u16(mut self, value: u16) -> Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn u32(mut self, value: u32) -> Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn bytes(mut self, value: &[u8]) -> Self {
        self.0.extend_from_slice(value);
        self
    }
}

/// Serialize one record with `payload` exactly.
fn record(kind: SymbolKind, payload: &[u8]) -> Vec<u8> {
    let length = u16::try_from(payload.len() + 2).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&kind.0.to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

/// Add zero padding to a 4-byte record boundary.
fn padded(payload: &[u8]) -> Vec<u8> {
    let mut payload = payload.to_vec();
    while !(payload.len() + 4).is_multiple_of(4) {
        payload.push(0);
    }
    payload
}

/// Decode the record of `kind` with `payload` exactly.
fn decode(kind: SymbolKind, payload: &[u8], check: impl FnOnce(Symbol<'_>)) {
    let bytes = record(kind, payload);
    let record = Records::new(&bytes).next().unwrap().unwrap();
    check(Symbol::decode(record).unwrap());
}

/// Decode the record of `kind` with `payload` exactly and return the error kind.
fn error(kind: SymbolKind, payload: &[u8]) -> ErrorKind {
    let bytes = record(kind, payload);
    let record = Records::new(&bytes).next().unwrap().unwrap();
    Symbol::decode(record).unwrap_err().kind()
}

/// Make sure that each truncation of `payload` is an error of `expected`.
fn rejects_truncation(kind: SymbolKind, payload: &[u8], expected: ErrorKind) {
    for length in 0..payload.len() {
        assert_eq!(
            error(kind, &payload[..length]),
            expected,
            "kind {kind:#x}, length {length}"
        );
    }
}

/// Return an `S_GMANPROC` payload with boundary values.
fn managed_procedure(name: &[u8]) -> Payload {
    Payload::default()
        .u32(0x10)
        .u32(0xb0)
        .u32(u32::MAX)
        .u32(0x38)
        .u32(1)
        .u32(0x37)
        .u32(0x0600_0001)
        .u32(u32::MAX)
        .u16(1)
        .u8(0x81)
        .u16(0xffff)
        .bytes(name)
}

#[test]
fn decodes_managed_procedures() {
    for kind in [S_GMANPROC, S_LMANPROC] {
        let payload = padded(&managed_procedure(b"ComputeAsync\0").0);
        decode(kind, &payload, |symbol| {
            let SymbolData::ManagedProcedure(value) = symbol.data() else {
                panic!("expected a managed procedure");
            };
            assert_eq!(value.next, at(u32::MAX));
            assert_eq!(value.code_length, 0x38);
            assert_eq!((value.debug_start, value.debug_end), (1, 0x37));
            assert_eq!(value.token, MetadataToken::from_raw(0x0600_0001));
            let address = SectionOffset {
                section: 1,
                offset: u32::MAX,
            };
            assert_eq!(value.address, address);
            assert!(value.flags.contains(ProcedureFlags::HAS_FRAME_POINTER));
            assert!(value.flags.contains(ProcedureFlags::OPTIMIZED_DEBUG_INFO));
            assert_eq!(value.return_register, 0xffff);
            assert_eq!(
                symbol.name().map(Bytes::as_bytes),
                Some(&b"ComputeAsync"[..])
            );
            assert_eq!(symbol.address(), Some(address));
            assert_eq!(symbol.type_index(), None);
            let links = symbol.links().unwrap();
            assert_eq!((links.parent, links.end), (at(0x10), at(0xb0)));
        });
    }
}

#[test]
fn managed_procedure_name_can_be_empty() {
    // The packed fixed fields take 37 bytes, so the NUL ends the payload at 38.
    // Two zero padding bytes align the record.
    let payload = managed_procedure(b"\0").0;
    assert_eq!(payload.len(), 38);
    let padded = padded(&payload);
    assert_eq!(padded.len(), 40);
    decode(S_GMANPROC, &padded, |symbol| {
        assert_eq!(symbol.name().map(Bytes::as_bytes), Some(&b""[..]));
    });
    decode(S_GMANPROC, &payload, |symbol| {
        assert_eq!(symbol.record().data.len(), 38);
    });
}

#[test]
fn rejects_truncated_managed_procedures() {
    for kind in [S_GMANPROC, S_LMANPROC] {
        rejects_truncation(kind, &managed_procedure(b"f\0").0, ErrorKind::InvalidFormat);
    }
}

#[test]
fn rejects_managed_procedure_padding_that_is_not_zero() {
    let payload = managed_procedure(b"f\0").bytes(&[0, 1]).0;
    assert_eq!(error(S_GMANPROC, &payload), ErrorKind::InvalidFormat);
    // Four zero bytes are padding that the decoder keeps (#30).
    let payload = managed_procedure(b"f\0").bytes(&[0; 4]).0;
    decode(S_LMANPROC, &payload, |symbol| {
        assert!(symbol.trailing_zeros() >= 4);
    });
}

/// Return an `S_MANSLOT` payload.
fn managed_slot(slot: u32, name: &[u8]) -> Payload {
    Payload::default()
        .u32(slot)
        .u32(0x1100_0005)
        .u32(0x20)
        .u16(1)
        .u16(0x0004)
        .bytes(name)
}

#[test]
fn decodes_managed_slots() {
    for slot in [0, 5, u32::MAX] {
        let payload = padded(&managed_slot(slot, b"visible\0").0);
        decode(S_MANSLOT, &payload, |symbol| {
            let SymbolData::ManagedSlot(value) = symbol.data() else {
                panic!("expected S_MANSLOT");
            };
            assert_eq!(value.slot, slot);
            assert_eq!(value.type_or_token, 0x1100_0005);
            assert_eq!(
                value.address,
                SectionOffset {
                    section: 1,
                    offset: 0x20
                }
            );
            assert_eq!(value.flags, LocalFlags::COMPILER_GENERATED);
            assert_eq!(symbol.name().map(Bytes::as_bytes), Some(&b"visible"[..]));
            // The token is not a TPI index, and the live address is not a symbol address.
            assert_eq!(symbol.type_index(), None);
            assert_eq!(symbol.address(), None);
            assert_eq!(symbol.links(), None);
        });
    }
}

#[test]
fn rejects_truncated_managed_slots() {
    rejects_truncation(
        S_MANSLOT,
        &managed_slot(1, b"x\0").0,
        ErrorKind::InvalidFormat,
    );
}

/// Return an `S_MANCONSTANT` payload with the value bytes `value`.
fn managed_constant(value: &[u8], name: &[u8]) -> Payload {
    Payload::default().u32(0x1100_000e).bytes(value).bytes(name)
}

/// Decode a managed constant and return its value.
fn constant_value(value: &[u8], check: impl FnOnce(Numeric<'_>)) {
    let payload = padded(&managed_constant(value, b"Seed\0").0);
    decode(S_MANCONSTANT, &payload, |symbol| {
        let SymbolData::ManagedConstant(constant) = symbol.data() else {
            panic!("expected S_MANCONSTANT");
        };
        assert_eq!(constant.type_or_token, 0x1100_000e);
        assert_eq!(symbol.name().map(Bytes::as_bytes), Some(&b"Seed"[..]));
        assert_eq!(symbol.type_index(), None);
        check(constant.value.clone());
    });
}

#[test]
fn decodes_managed_integer_constants() {
    // An immediate value, the largest immediate value and an LF_LONG leaf.
    for (bytes, expected) in [
        (&[7, 0][..], Integer::Unsigned(7)),
        (&[0xff, 0x7f], Integer::Unsigned(0x7fff)),
        (&[0x03, 0x80, 0xff, 0xff, 0xff, 0xff], Integer::Signed(-1)),
    ] {
        constant_value(bytes, |value| {
            assert_eq!(value.integer(), Some(expected));
        });
    }
}

#[test]
fn decodes_managed_string_constants() {
    // Roslyn writes `const string Label = "csharp"` as LF_VARSTRING.
    constant_value(
        &[0x10, 0x80, 6, 0, b'c', b's', b'h', b'a', b'r', b'p'],
        |value| {
            assert_eq!(value, Numeric::VarString(Bytes::new(b"csharp")));
        },
    );
    constant_value(&[0x10, 0x80, 0, 0], |value| {
        assert_eq!(value, Numeric::VarString(Bytes::new(b"")));
    });
}

#[test]
fn decodes_managed_fixed_size_constants() {
    let real64 = 1.5_f64.to_le_bytes();
    let mut bytes = vec![0x06, 0x80];
    bytes.extend_from_slice(&real64);
    constant_value(&bytes, |value| {
        assert_eq!(value, Numeric::Real64(real64));
    });
    // LF_DECIMAL has 16 value bytes, LF_REAL16 has two.
    for (leaf, size) in [(0x8019_u16, 16), (0x801c, 2), (0x800f, 32)] {
        let mut bytes = leaf.to_le_bytes().to_vec();
        bytes.extend(core::iter::repeat_n(0xab, size));
        constant_value(&bytes, |value| {
            assert_eq!(value.value_bytes().map(<[u8]>::len), Some(size));
            assert_eq!(value.encoded_len(), size + 2);
        });
    }
}

#[test]
fn rejects_managed_constants_with_unknown_leaves() {
    // 0x80ff is not a numeric leaf.
    let payload = managed_constant(&[0xff, 0x80], b"a\0").0;
    assert_eq!(
        error(S_MANCONSTANT, &payload),
        ErrorKind::UnsupportedVersion
    );
}

#[test]
fn rejects_truncated_managed_constants() {
    for value in [
        &[7, 0][..],
        &[0x03, 0x80, 1, 0, 0, 0],
        &[0x10, 0x80, 2, 0, b'h', b'i'],
        &[0x05, 0x80, 0, 0, 0xc0, 0x3f],
    ] {
        let payload = managed_constant(value, b"a\0").0;
        rejects_truncation(S_MANCONSTANT, &payload, ErrorKind::InvalidFormat);
    }
    // A string length past the end of the record.
    let payload = managed_constant(&[0x10, 0x80, 0xff, 0xff, b'a', 0], b"").0;
    assert_eq!(error(S_MANCONSTANT, &payload), ErrorKind::InvalidFormat);
}

/// Return an `S_OEM` payload with `data` after the type index.
fn oem(data: &[u8]) -> Payload {
    Payload::default()
        .bytes(&MANAGED_ATTRIBUTE_OEM_ID)
        .u32(0)
        .bytes(data)
}

#[test]
fn decodes_oem_records_with_raw_data() {
    // The Roslyn `MD2` attribute: a UTF-16 name, then the value and padding.
    let data = [b'M', 0, b'D', 0, b'2', 0, 0, 0, 4, 1, 0, 0, 0, 0, 0, 0];
    decode(S_OEM, &oem(&data).0, |symbol| {
        let SymbolData::Oem(value) = symbol.data() else {
            panic!("expected S_OEM");
        };
        assert_eq!(value.id, MANAGED_ATTRIBUTE_OEM_ID);
        assert_eq!(uuid_text(&value.id), "c6ea3fc9-59b3-49d6-bc25-0902bbabb460");
        assert_eq!(value.type_index, TypeIndex(0));
        assert_eq!(value.data, data);
        assert_eq!(symbol.type_index(), Some(TypeIndex(0)));
        assert_eq!(symbol.name(), None);
    });
    // Data can be empty, and non-zero trailing bytes stay in the data.
    decode(S_OEM, &oem(&[]).0, |symbol| {
        let SymbolData::Oem(value) = symbol.data() else {
            panic!("expected S_OEM");
        };
        assert_eq!(value.data, []);
    });
    decode(S_OEM, &oem(&[1, 2, 3]).0, |symbol| {
        let SymbolData::Oem(value) = symbol.data() else {
            panic!("expected S_OEM");
        };
        assert_eq!(value.data, [1, 2, 3]);
    });
}

#[test]
fn rejects_truncated_oem_records() {
    rejects_truncation(S_OEM, &oem(&[]).0, ErrorKind::InvalidFormat);
}

/// Format GUID bytes in serialized order as UUID text.
fn uuid_text(id: &[u8; 16]) -> String {
    let data1 = u32::from_le_bytes([id[0], id[1], id[2], id[3]]);
    let data2 = u16::from_le_bytes([id[4], id[5]]);
    let data3 = u16::from_le_bytes([id[6], id[7]]);
    let tail = u64::from_be_bytes(id[8..].try_into().unwrap());
    format!(
        "{data1:08x}-{data2:04x}-{data3:04x}-{:04x}-{:012x}",
        tail >> 48,
        tail & 0xffff_ffff_ffff
    )
}

/// Return an `S_TOKENREF` payload.
fn token_reference(module: u16) -> Payload {
    Payload::default()
        .u32(0)
        .u32(0x34)
        .u16(module)
        .bytes(b"06000017\0")
}

#[test]
fn decodes_token_references_as_procedure_references() {
    let payload = padded(&token_reference(6).0);
    decode(S_TOKENREF, &payload, |symbol| {
        let reference = symbol.procedure_reference().unwrap();
        assert_eq!(
            reference.target,
            ModuleSymbolTarget::Position {
                module: Some(ModuleIndex(5)),
                offset: at(0x34),
            }
        );
        assert_eq!(reference.name, b"06000017");
        assert_eq!(symbol.name().map(Bytes::as_bytes), Some(&b"06000017"[..]));
        assert!(matches!(symbol.data(), SymbolData::ProcedureReference(_)));
        assert_ne!(symbol.kind(), S_PROCREF);
    });
    // The raw module number 0 has no module index.
    decode(S_TOKENREF, &token_reference(0).0, |symbol| {
        assert_eq!(
            symbol.procedure_reference().unwrap().target,
            ModuleSymbolTarget::Position {
                module: None,
                offset: at(0x34),
            }
        );
    });
}

#[test]
fn rejects_truncated_token_references() {
    rejects_truncation(S_TOKENREF, &token_reference(1).0, ErrorKind::InvalidFormat);
}
