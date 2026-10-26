//! Tests for the symbol kinds that MSVC, Clang and rustc fixtures add (#13):
//! frame cookies, heap allocation sites, thread storage, frame-relative and
//! register variables, file statics, subfield def-ranges, frame registers,
//! trampolines, FASTLINK references and Edit and Continue temporaries.

use pdbrs::ErrorKind;
use pdbrs::native::address::SectionOffset;
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::codeview::symbol::{
    CookieKind, FastLinkFlags, LocalFlags, S_BPREL32, S_DEFRANGE_SUBFIELD_REGISTER, S_FASTLINK,
    S_FILESTATIC, S_FRAMECOOKIE, S_FRAMEREG, S_GTHREAD32, S_HEAPALLOCSITE, S_LTHREAD32, S_REGISTER,
    S_REGREL32, S_REGREL32_ENCTMP, S_TRAMPOLINE, Symbol, SymbolData, TrampolineKind,
};

/// A builder for little-endian symbol payloads.
#[derive(Default)]
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

    fn i32(mut self, value: i32) -> Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn bytes(mut self, value: &[u8]) -> Self {
        self.0.extend_from_slice(value);
        self
    }

    /// Remove the last `count` bytes.
    fn truncated(mut self, count: usize) -> Self {
        self.0.truncate(self.0.len() - count);
        self
    }
}

/// Serialize one record and add zero padding to a 4-byte boundary.
fn padded(kind: u16, payload: &[u8]) -> Vec<u8> {
    let mut payload = payload.to_vec();
    while !(payload.len() + 4).is_multiple_of(4) {
        payload.push(0);
    }
    unpadded(kind, &payload)
}

/// Serialize one record without padding.
fn unpadded(kind: u16, payload: &[u8]) -> Vec<u8> {
    let length = u16::try_from(payload.len() + 2).unwrap();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&kind.to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

/// Decode the padded record and give the symbol to `check`.
fn decode(kind: u16, payload: &Payload, check: impl FnOnce(Symbol<'_>)) {
    let bytes = padded(kind, &payload.0);
    let record = Records::new(&bytes).next().unwrap().unwrap();
    check(record.symbol().unwrap());
}

/// Decode the unpadded record and return the error kind.
fn error(kind: u16, payload: &Payload) -> ErrorKind {
    let bytes = unpadded(kind, &payload.0);
    let record = Records::new(&bytes).next().unwrap().unwrap();
    record.symbol().unwrap_err().kind()
}

fn heap_allocation_site() -> Payload {
    Payload::default().u32(8574).u16(1).u16(5).u32(0x1671)
}

#[test]
fn decodes_heap_allocation_site() {
    decode(S_HEAPALLOCSITE, &heap_allocation_site(), |symbol| {
        let SymbolData::HeapAllocationSite(value) = *symbol.data() else {
            panic!("expected S_HEAPALLOCSITE");
        };
        assert_eq!(value.instruction_length, 5);
        let address = SectionOffset {
            section: 1,
            offset: 8574,
        };
        assert_eq!(symbol.address(), Some(address));
        assert_eq!(symbol.type_index(), Some(TypeIndex(0x1671)));
        assert_eq!(symbol.name(), None);
    });
    // The fixed fields fill 12 bytes, so the record has no padding.
    let short = heap_allocation_site().truncated(1);
    assert_eq!(error(S_HEAPALLOCSITE, &short), ErrorKind::InvalidFormat);
    // Trailing bytes must be zero; zero bytes are kept as padding (#30).
    let trailing = heap_allocation_site().bytes(&[0, 0, 0, 1]);
    assert_eq!(error(S_HEAPALLOCSITE, &trailing), ErrorKind::InvalidFormat);
}

fn frame_cookie(kind: u8) -> Payload {
    Payload::default().i32(-40).u16(23).u8(kind).u8(0)
}

#[test]
fn decodes_frame_cookie_with_signed_offset() {
    decode(S_FRAMECOOKIE, &frame_cookie(1), |symbol| {
        let SymbolData::FrameCookie(value) = *symbol.data() else {
            panic!("expected S_FRAMECOOKIE");
        };
        assert_eq!((value.offset, value.register), (-40, 23));
        assert_eq!(value.kind, CookieKind::XOR_STACK_POINTER);
        assert_eq!(value.flags, 0);
        assert_eq!(symbol.address(), None);
    });
    // An unknown cookie kind stays raw.
    decode(S_FRAMECOOKIE, &frame_cookie(7), |symbol| {
        let SymbolData::FrameCookie(value) = *symbol.data() else {
            panic!("expected S_FRAMECOOKIE");
        };
        assert_eq!(value.kind, CookieKind(7));
    });
    assert_eq!(
        error(S_FRAMECOOKIE, &frame_cookie(3).truncated(1)),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn decodes_thread_storage_of_both_kinds() {
    for kind in [S_GTHREAD32, S_LTHREAD32] {
        let payload = Payload::default()
            .u32(0x23)
            .u32(16)
            .u16(2)
            .bytes(b"tls_wide\0");
        decode(kind, &payload, |symbol| {
            let SymbolData::ThreadData(value) = *symbol.data() else {
                panic!("expected thread storage");
            };
            assert_eq!(value.name, b"tls_wide");
            assert_eq!(symbol.kind(), kind);
            assert_eq!(symbol.name(), Some(&b"tls_wide"[..]));
            assert_eq!(symbol.type_index(), Some(TypeIndex(0x23)));
            let address = SectionOffset {
                section: 2,
                offset: 16,
            };
            assert_eq!(symbol.address(), Some(address));
        });
        let unterminated = Payload::default().u32(0x23).u32(16).u16(2).bytes(b"tls");
        assert_eq!(error(kind, &unterminated), ErrorKind::InvalidFormat);
    }
}

#[test]
fn decodes_frame_relative_variable_at_offset_limits() {
    for offset in [i32::MIN, -28, 0, i32::MAX] {
        let payload = Payload::default()
            .i32(offset)
            .u32(0x1110)
            .bytes(b"record\0");
        decode(S_BPREL32, &payload, |symbol| {
            let SymbolData::BasePointerRelative(value) = *symbol.data() else {
                panic!("expected S_BPREL32");
            };
            assert_eq!(value.offset, offset);
            assert_eq!(symbol.name(), Some(&b"record"[..]));
            assert_eq!(symbol.type_index(), Some(TypeIndex(0x1110)));
        });
    }
    let short = Payload::default().i32(-28).u16(0x1110);
    assert_eq!(error(S_BPREL32, &short), ErrorKind::InvalidFormat);
}

#[test]
fn decodes_register_variable() {
    let payload = Payload::default().u32(0x17d7).u16(18).bytes(b"this\0");
    decode(S_REGISTER, &payload, |symbol| {
        let SymbolData::Register(value) = *symbol.data() else {
            panic!("expected S_REGISTER");
        };
        assert_eq!(value.register, 18);
        assert_eq!(symbol.name(), Some(&b"this"[..]));
        assert_eq!(symbol.type_index(), Some(TypeIndex(0x17d7)));
    });
    // A non-zero byte after the name is not padding.
    let bad_padding = Payload::default().u32(0x17d7).u16(18).bytes(b"this\0\x01");
    assert_eq!(error(S_REGISTER, &bad_padding), ErrorKind::InvalidFormat);
}

#[test]
fn decodes_file_static_with_name_offset() {
    let payload = Payload::default()
        .u32(0x74)
        .u32(u32::MAX)
        .u16(0x600)
        .bytes(b"s_c_calls\0");
    decode(S_FILESTATIC, &payload, |symbol| {
        let SymbolData::FileStatic(value) = *symbol.data() else {
            panic!("expected S_FILESTATIC");
        };
        assert_eq!(value.module_name_offset, u32::MAX);
        assert!(value.flags.contains(LocalFlags::ENREGISTERED_GLOBAL));
        assert!(value.flags.contains(LocalFlags::ENREGISTERED_STATIC));
        assert_eq!(symbol.name(), Some(&b"s_c_calls"[..]));
        assert_eq!(symbol.type_index(), Some(TypeIndex(0x74)));
    });
    let short = Payload::default().u32(0x74).u32(5297).u8(0);
    assert_eq!(error(S_FILESTATIC, &short), ErrorKind::InvalidFormat);
}

fn subfield(parent_offset: u32, gaps: &[u8]) -> Payload {
    Payload::default()
        .u16(330)
        .u16(0)
        .u32(parent_offset)
        .u32(0x120)
        .u16(1)
        .u16(31)
        .bytes(gaps)
}

#[test]
fn decodes_subfield_register_def_range_with_gaps() {
    decode(
        S_DEFRANGE_SUBFIELD_REGISTER,
        &subfield(8, &[4, 0, 2, 0, 9, 0, 1, 0]),
        |symbol| {
            let SymbolData::DefRangeSubfieldRegister(value) = *symbol.data() else {
                panic!("expected S_DEFRANGE_SUBFIELD_REGISTER");
            };
            assert_eq!(value.register, 330);
            assert!(!value.may_have_no_name());
            assert_eq!(value.offset_in_parent(), 8);
            assert_eq!(value.range.start.offset, 0x120);
            assert_eq!((value.range.start.section, value.range.length), (1, 31));
            let gaps: Vec<_> = value
                .gaps
                .iter()
                .map(|gap| (gap.start, gap.length))
                .collect();
            assert_eq!(gaps, [(4, 2), (9, 1)]);
        },
    );
}

#[test]
fn subfield_offset_keeps_twelve_bits_and_the_raw_field() {
    decode(
        S_DEFRANGE_SUBFIELD_REGISTER,
        &subfield(0xffff_f123, &[]),
        |symbol| {
            let SymbolData::DefRangeSubfieldRegister(value) = *symbol.data() else {
                panic!("expected S_DEFRANGE_SUBFIELD_REGISTER");
            };
            assert_eq!(value.offset_in_parent(), 0x123);
            assert_eq!(value.parent_offset, 0xffff_f123);
            assert!(value.gaps.is_empty());
        },
    );
}

#[test]
fn rejects_truncated_subfield_def_ranges() {
    // A gap array must be a multiple of 4 bytes.
    let odd = subfield(0, &[4, 0]);
    assert_eq!(
        error(S_DEFRANGE_SUBFIELD_REGISTER, &odd),
        ErrorKind::InvalidFormat
    );
    let short = subfield(0, &[]).truncated(1);
    assert_eq!(
        error(S_DEFRANGE_SUBFIELD_REGISTER, &short),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn decodes_frame_register_and_checks_the_upper_bytes() {
    let payload = Payload::default().u16(69).u16(0);
    decode(S_FRAMEREG, &payload, |symbol| {
        assert_eq!(*symbol.data(), SymbolData::FrameRegister(69));
        assert_eq!(symbol.name(), None);
    });
    // A 32-bit register field with a non-zero upper half is not accepted.
    let wide = Payload::default().u16(69).u16(1);
    assert_eq!(error(S_FRAMEREG, &wide), ErrorKind::InvalidFormat);
    assert_eq!(
        error(S_FRAMEREG, &Payload::default().u8(69)),
        ErrorKind::InvalidFormat
    );
}

fn trampoline(kind: u16) -> Payload {
    Payload::default()
        .u16(kind)
        .u16(5)
        .u32(10)
        .u32(0xde10)
        .u16(2)
        .u16(3)
}

#[test]
fn decodes_trampoline_with_offsets_before_sections() {
    for kind in [TrampolineKind::INCREMENTAL, TrampolineKind::BRANCH_ISLAND] {
        decode(S_TRAMPOLINE, &trampoline(kind.0), |symbol| {
            let SymbolData::Trampoline(value) = *symbol.data() else {
                panic!("expected S_TRAMPOLINE");
            };
            assert_eq!(value.kind, kind);
            assert_eq!(value.thunk_size, 5);
            let thunk = SectionOffset {
                section: 2,
                offset: 10,
            };
            let target = SectionOffset {
                section: 3,
                offset: 0xde10,
            };
            assert_eq!((value.thunk, value.target), (thunk, target));
            assert_eq!(symbol.address(), Some(thunk));
        });
    }
    assert_eq!(
        error(S_TRAMPOLINE, &trampoline(0).truncated(1)),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn decodes_fastlink_references() {
    let udt = Payload::default().u32(0x1087).u16(0x4).bytes(b"c_mode\0");
    decode(S_FASTLINK, &udt, |symbol| {
        let SymbolData::FastLink(value) = *symbol.data() else {
            panic!("expected S_FASTLINK");
        };
        assert!(value.flags.contains(FastLinkFlags::UDT));
        assert_eq!(value.type_index(), Some(TypeIndex(0x1087)));
        assert_eq!(value.section(), None);
        assert_eq!(symbol.name(), Some(&b"c_mode"[..]));
        // The type index refers to the object file, not to the TPI stream.
        assert_eq!(symbol.type_index(), None);
    });
    let local_data = Payload::default().u32(2).u16(0x3).bytes(b"s_c_calls\0");
    decode(S_FASTLINK, &local_data, |symbol| {
        let SymbolData::FastLink(value) = *symbol.data() else {
            panic!("expected S_FASTLINK");
        };
        let flags = FastLinkFlags(FastLinkFlags::LOCAL.0 | FastLinkFlags::DATA.0);
        assert_eq!(value.flags, flags);
        assert_eq!((value.type_index(), value.section()), (None, Some(2)));
    });
    let namespace = Payload::default().u32(2).u16(0x40).bytes(b"std\0");
    decode(S_FASTLINK, &namespace, |symbol| {
        let SymbolData::FastLink(value) = *symbol.data() else {
            panic!("expected S_FASTLINK");
        };
        assert!(value.flags.contains(FastLinkFlags::NAMESPACE));
    });
}

#[test]
fn decodes_empty_fastlink_record_and_rejects_bad_ones() {
    // Each module of the fixture starts with an 8-byte record: zero fields,
    // an empty name and one padding byte.
    let empty = Payload::default().u32(0).u16(0).bytes(b"\0\0");
    let bytes = unpadded(S_FASTLINK, &empty.0);
    let record = Records::new(&bytes).next().unwrap().unwrap();
    let SymbolData::FastLink(value) = *record.symbol().unwrap().data() else {
        panic!("expected S_FASTLINK");
    };
    assert_eq!((value.value, value.flags.0, value.name), (0, 0, &b""[..]));
    let unterminated = Payload::default().u32(2).u16(0).bytes(b"std");
    assert_eq!(error(S_FASTLINK, &unterminated), ErrorKind::InvalidFormat);
    let short = Payload::default().u32(2).u8(0);
    assert_eq!(error(S_FASTLINK, &short), ErrorKind::InvalidFormat);
}

#[test]
fn decodes_edit_and_continue_temporary_like_register_relative() {
    let payload = Payload::default()
        .u32(104)
        .u32(0x75)
        .u16(334)
        .bytes(b"$initVBases\0");
    for kind in [S_REGREL32, S_REGREL32_ENCTMP] {
        decode(kind, &payload, |symbol| {
            let SymbolData::RegisterRelative(value) = *symbol.data() else {
                panic!("expected a register-relative variable");
            };
            assert_eq!(symbol.kind(), kind);
            assert_eq!((value.offset, value.register), (104, 334));
            assert_eq!(symbol.name(), Some(&b"$initVBases"[..]));
            assert_eq!(symbol.type_index(), Some(TypeIndex(0x75)));
        });
    }
    let short = Payload::default().u32(104).u32(0x75).u8(0x4e);
    assert_eq!(error(S_REGREL32_ENCTMP, &short), ErrorKind::InvalidFormat);
}
