//! Tests for the typed decoding of each symbol record kind.

use pdbrs::ErrorKind;
use pdbrs::native::address::SectionOffset;
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::index::{IdIndex, TypeIndex};
use pdbrs::native::codeview::numeric::Integer;
use pdbrs::native::codeview::symbol::{
    AddressGap, CompileFlags, FramePointer, FrameProcedureFlags, IdList, JumpTableEntry,
    LocalFlags, ProcedureFlags, S_ARMSWITCHTABLE, S_BLOCK32, S_BUILDINFO, S_CALLEES, S_CALLERS,
    S_CALLSITEINFO, S_COFFGROUP, S_COMPILE2, S_COMPILE3, S_CONSTANT, S_DEFRANGE_FRAMEPOINTER_REL,
    S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE, S_DEFRANGE_REGISTER, S_DEFRANGE_REGISTER_REL,
    S_DEFRANGE_REGISTER_REL_INDIR, S_END, S_ENVBLOCK, S_FRAMEPROC, S_INLINEES, S_INLINESITE,
    S_INLINESITE_END, S_LABEL32, S_LOCAL, S_LPROCREF, S_OBJNAME, S_PROC_ID_END, S_PROCREF,
    S_REGREL32, S_SECTION, S_SKIP, S_THUNK32, S_UDT, S_UNAMESPACE, SourceLanguage, Symbol,
    SymbolData, ThunkOrdinal,
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

#[test]
fn decodes_object_name() {
    let payload = Payload::default().u32(7).bytes(b"C:\\a.o\0");
    decode(S_OBJNAME, &payload, |symbol| {
        let SymbolData::ObjectName(value) = symbol.data() else {
            panic!("expected S_OBJNAME");
        };
        assert_eq!((value.signature, value.name), (7, &b"C:\\a.o"[..]));
        assert_eq!(symbol.name(), Some(&b"C:\\a.o"[..]));
    });
}

#[test]
fn decodes_compile3_flags_language_and_versions() {
    let payload = Payload::default()
        .u32(0x15 | 1 << 17 | 1 << 10)
        .u16(0xd0)
        .bytes(&[1, 0, 99, 0, 0, 0, 7, 0])
        .bytes(&[0x0b, 0x5a, 0, 0, 0, 0, 3, 0])
        .bytes(b"rustc\0");
    decode(S_COMPILE3, &payload, |symbol| {
        let SymbolData::Compile3(value) = symbol.data() else {
            panic!("expected S_COMPILE3");
        };
        assert_eq!(value.flags.language(), SourceLanguage::RUST);
        assert!(value.flags.contains(CompileFlags::SDL));
        assert!(value.flags.contains(CompileFlags::LTCG));
        assert!(!value.flags.contains(CompileFlags::PGO));
        assert_eq!(value.machine, 0xd0);
        assert_eq!(value.frontend, [1, 99, 0, 7]);
        assert_eq!(value.backend, [23_051, 0, 0, 3]);
        assert_eq!(value.version, b"rustc");
        assert_eq!(symbol.name(), None);
    });
}

fn compile3(tail: &[u8]) -> Payload {
    Payload::default()
        .u32(u32::from(SourceLanguage::MASM.0))
        .u16(0xd0)
        .bytes(&[0; 16])
        .bytes(b"ML\0\0\0")
        .bytes(tail)
}

#[test]
fn compile3_accepts_zero_bytes_after_the_version() {
    // ml64 writes an extra NUL, so the zero bytes can fill four bytes.
    for tail in [&b""[..], b"\0", b"\0\0\0\0\0"] {
        let bytes = unpadded(S_COMPILE3, &compile3(tail).0);
        let record = Records::new(&bytes).next().unwrap().unwrap();
        let SymbolData::Compile3(value) = *record.symbol().unwrap().data() else {
            panic!("expected S_COMPILE3");
        };
        assert_eq!(value.version, b"ML");
        assert_eq!(value.flags.language(), SourceLanguage::MASM);
    }
    assert_eq!(
        error(S_COMPILE3, &compile3(b"\0\x01")),
        ErrorKind::InvalidFormat
    );
}

fn compile2(tail: &[u8]) -> Payload {
    Payload::default()
        .u32(u32::from(SourceLanguage::LINK.0))
        .u16(0xd0)
        .bytes(&[0; 6])
        .bytes(&[14, 0, 38, 0, 0x79, 0x81])
        .bytes(b"LINK\0")
        .bytes(tail)
}

#[test]
fn decodes_compile2_with_extra_strings() {
    decode(S_COMPILE2, &compile2(b"-a\0-bc\0\0"), |symbol| {
        let SymbolData::Compile2(value) = symbol.data() else {
            panic!("expected S_COMPILE2");
        };
        assert_eq!(value.flags.language(), SourceLanguage::LINK);
        assert_eq!(value.backend, [14, 38, 33_145]);
        assert_eq!(value.version, b"LINK");
        let strings: Vec<&[u8]> = value.extra_strings.iter().collect();
        assert_eq!(strings, [&b"-a"[..], &b"-bc"[..]]);
    });
}

#[test]
fn compile2_extra_strings_can_be_absent_or_empty() {
    // The terminating empty string can be explicit or come from the padding.
    for (tail, pad) in [(&b"\0"[..], false), (b"", true)] {
        let payload = compile2(tail);
        let bytes = if pad {
            padded(S_COMPILE2, &payload.0)
        } else {
            unpadded(S_COMPILE2, &payload.0)
        };
        let record = Records::new(&bytes).next().unwrap().unwrap();
        let SymbolData::Compile2(value) = *record.symbol().unwrap().data() else {
            panic!("expected S_COMPILE2");
        };
        assert!(value.extra_strings.is_empty());
    }
    // No bytes after the version string: the block is absent.
    let mut payload = compile2(b"");
    payload.0.truncate(payload.0.len() - 2);
    let bytes = unpadded(S_COMPILE2, &payload.bytes(b"\0").0);
    let record = Records::new(&bytes).next().unwrap().unwrap();
    let SymbolData::Compile2(value) = *record.symbol().unwrap().data() else {
        panic!("expected S_COMPILE2");
    };
    assert_eq!(value.version, b"LIN");
    assert!(value.extra_strings.is_empty());
}

#[test]
fn rejects_extra_strings_without_empty_terminator() {
    assert_eq!(
        error(S_COMPILE2, &compile2(b"-a\0")),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        error(S_COMPILE2, &compile2(b"-a\0-b")),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn decodes_environment_block() {
    let payload = Payload::default().u8(0).bytes(b"cwd\0C:\\b\0\0");
    decode(S_ENVBLOCK, &payload, |symbol| {
        let SymbolData::EnvironmentBlock(value) = symbol.data() else {
            panic!("expected S_ENVBLOCK");
        };
        assert_eq!(value.flags, 0);
        let strings: Vec<&[u8]> = value.strings.into_iter().collect();
        assert_eq!(strings, [&b"cwd"[..], &b"C:\\b"[..]]);
    });
}

#[test]
fn decodes_build_info_and_using_namespace() {
    decode(S_BUILDINFO, &Payload::default().u32(0x1234), |symbol| {
        assert_eq!(*symbol.data(), SymbolData::BuildInfo(IdIndex(0x1234)));
        assert_eq!(symbol.id_index(), Some(IdIndex(0x1234)));
    });
    decode(
        S_UNAMESPACE,
        &Payload::default().bytes(b"std\0"),
        |symbol| {
            assert_eq!(*symbol.data(), SymbolData::UsingNamespace(b"std"));
            assert_eq!(symbol.name(), Some(&b"std"[..]));
        },
    );
}

#[test]
fn decodes_frame_procedure_and_encoded_registers() {
    let flags = 1 << 5 | 1 << 13 | 1 << 14 | 2 << 16;
    let payload = Payload::default()
        .u32(88)
        .u32(4)
        .u32(8)
        .u32(16)
        .u32(0x40)
        .u16(2)
        .u32(flags);
    decode(S_FRAMEPROC, &payload, |symbol| {
        let SymbolData::FrameProcedure(value) = symbol.data() else {
            panic!("expected S_FRAMEPROC");
        };
        assert_eq!(
            (value.frame_size, value.padding_size, value.padding_offset),
            (88, 4, 8)
        );
        assert_eq!(value.callee_saved_size, 16);
        let handler = SectionOffset {
            section: 2,
            offset: 0x40,
        };
        assert_eq!(value.exception_handler, handler);
        assert!(value.flags.contains(FrameProcedureFlags::MARKED_INLINE));
        assert!(value.flags.contains(FrameProcedureFlags::SAFE_BUFFERS));
        assert!(!value.flags.contains(FrameProcedureFlags::HAS_ALLOCA));
        assert_eq!(value.flags.local_frame_pointer(), FramePointer::STACK);
        assert_eq!(value.flags.parameter_frame_pointer(), FramePointer::FRAME);
        assert_eq!(symbol.address(), None);
    });
}

#[test]
fn decodes_block_and_label() {
    let block = Payload::default()
        .u32(4)
        .u32(96)
        .u32(0x20)
        .u32(0x100)
        .u16(1)
        .bytes(b"\0");
    decode(S_BLOCK32, &block, |symbol| {
        let SymbolData::Block(value) = symbol.data() else {
            panic!("expected S_BLOCK32");
        };
        assert_eq!((value.parent, value.end, value.code_length), (4, 96, 0x20));
        assert_eq!(symbol.name(), Some(&b""[..]));
        let scope = symbol.scope().unwrap();
        assert_eq!((scope.parent, scope.end), (4, 96));
    });
    let label = Payload::default()
        .u32(0x30)
        .u16(1)
        .u8(0x10)
        .bytes(b"cs10\0");
    decode(S_LABEL32, &label, |symbol| {
        let SymbolData::Label(value) = symbol.data() else {
            panic!("expected S_LABEL32");
        };
        assert_eq!(value.flags, ProcedureFlags::UNREACHABLE);
        let address = SectionOffset {
            section: 1,
            offset: 0x30,
        };
        assert_eq!(symbol.address(), Some(address));
        assert_eq!(symbol.name(), Some(&b"cs10"[..]));
    });
}

fn thunk(ordinal: u8, tail: &[u8]) -> Payload {
    Payload::default()
        .u32(0)
        .u32(112)
        .u32(0)
        .u32(0x1234)
        .u16(1)
        .u16(6)
        .u8(ordinal)
        .bytes(b"memset\0")
        .bytes(tail)
}

#[test]
fn decodes_standard_thunk() {
    decode(S_THUNK32, &thunk(0, b""), |symbol| {
        let SymbolData::Thunk(value) = symbol.data() else {
            panic!("expected S_THUNK32");
        };
        assert_eq!(value.ordinal, ThunkOrdinal::STANDARD);
        assert_eq!((value.end, value.length), (112, 6));
        assert_eq!(value.variant, b"");
        assert_eq!(symbol.name(), Some(&b"memset"[..]));
        assert_eq!(symbol.address().unwrap().offset, 0x1234);
    });
    assert_eq!(
        error(S_THUNK32, &thunk(0, b"\x01\x02")),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn keeps_thunk_variant_raw_for_other_ordinals() {
    decode(S_THUNK32, &thunk(1, b"\x08\x00"), |symbol| {
        let SymbolData::Thunk(value) = symbol.data() else {
            panic!("expected S_THUNK32");
        };
        assert_eq!(value.ordinal, ThunkOrdinal::THIS_ADJUSTOR);
        // The variant keeps the two bytes and the two padding bytes.
        assert_eq!(value.variant, b"\x08\x00\x00\x00");
    });
}

#[test]
fn decodes_scope_ends_and_rejects_their_payload() {
    for kind in [S_END, S_PROC_ID_END, S_INLINESITE_END] {
        decode(kind, &Payload::default(), |symbol| {
            let expected = if kind == S_INLINESITE_END {
                SymbolData::InlineSiteEnd
            } else {
                SymbolData::ScopeEnd
            };
            assert_eq!(*symbol.data(), expected);
        });
        assert_eq!(
            error(kind, &Payload::default().u32(1)),
            ErrorKind::InvalidFormat
        );
    }
}

#[test]
fn decodes_inline_site() {
    let payload = Payload::default()
        .u32(4)
        .u32(80)
        .u32(0x11fd)
        .bytes(&[0x0b, 0x0c, 0x06, 0x21]);
    decode(S_INLINESITE, &payload, |symbol| {
        let SymbolData::InlineSite(value) = symbol.data() else {
            panic!("expected S_INLINESITE");
        };
        assert_eq!((value.parent, value.end), (4, 80));
        assert_eq!(symbol.id_index(), Some(IdIndex(0x11fd)));
        assert_eq!(value.annotations.bytes(), [0x0b, 0x0c, 0x06, 0x21]);
        assert_eq!(value.annotations.iter().count(), 2);
    });
    let broken = Payload::default()
        .u32(4)
        .u32(80)
        .u32(0x11fd)
        .bytes(&[0x0e, 1]);
    assert_eq!(error(S_INLINESITE, &broken), ErrorKind::InvalidFormat);
}

/// Return the ID list of an `S_INLINEES`, `S_CALLEES` or `S_CALLERS` record.
fn id_list(symbol: Symbol<'_>) -> IdList<'_> {
    match *symbol.data() {
        SymbolData::Inlinees(list) => list,
        SymbolData::Callees(list) | SymbolData::Callers(list) => list.functions(),
        _ => panic!("expected an ID list"),
    }
}

#[test]
fn decodes_inlinee_and_callee_lists() {
    for kind in [S_INLINEES, S_CALLEES, S_CALLERS] {
        let payload = Payload::default().u32(2).u32(0x11fd).u32(0x1237);
        decode(kind, &payload, |symbol| {
            let list = id_list(symbol);
            assert_eq!(list.len(), 2);
            let ids: Vec<IdIndex> = list.iter().collect();
            assert_eq!(ids, [IdIndex(0x11fd), IdIndex(0x1237)]);
        });
        decode(kind, &Payload::default().u32(0), |symbol| {
            assert!(id_list(symbol).is_empty());
        });
        let short = Payload::default().u32(3).u32(1).u32(2);
        assert_eq!(error(kind, &short), ErrorKind::InvalidFormat);
    }
    let extra = Payload::default().u32(1).u32(1).u32(2);
    assert_eq!(error(S_INLINEES, &extra), ErrorKind::InvalidFormat);
    // A function list can have one invocation count for each function.
    for kind in [S_CALLEES, S_CALLERS] {
        decode(kind, &extra, |symbol| {
            let (SymbolData::Callees(list) | SymbolData::Callers(list)) = *symbol.data() else {
                panic!("expected a function list");
            };
            assert_eq!(list.invocations().collect::<Vec<_>>(), [2]);
        });
        let too_many = Payload::default().u32(1).u32(1).u32(2).u32(3);
        assert_eq!(error(kind, &too_many), ErrorKind::InvalidFormat);
    }
}

#[test]
fn decodes_call_site_and_checks_reserved_field() {
    let payload = Payload::default().u32(0x50).u16(1).u16(0).u32(0x1003);
    decode(S_CALLSITEINFO, &payload, |symbol| {
        assert_eq!(symbol.type_index(), Some(TypeIndex(0x1003)));
        assert_eq!(symbol.address().unwrap().offset, 0x50);
    });
    let reserved = Payload::default().u32(0x50).u16(1).u16(1).u32(0x1003);
    assert_eq!(error(S_CALLSITEINFO, &reserved), ErrorKind::InvalidFormat);
}

#[test]
fn decodes_jump_table_field_order() {
    let payload = Payload::default()
        .u32(0x1000)
        .u16(2)
        .u16(4)
        .u32(0x2000)
        .u32(0x3000)
        .u16(1)
        .u16(3)
        .u32(11);
    decode(S_ARMSWITCHTABLE, &payload, |symbol| {
        let SymbolData::JumpTable(value) = symbol.data() else {
            panic!("expected S_ARMSWITCHTABLE");
        };
        let at = |section, offset| SectionOffset { section, offset };
        assert_eq!(value.base, at(2, 0x1000));
        assert_eq!(value.entry, JumpTableEntry::INT32);
        assert_eq!(value.branch, at(1, 0x2000));
        assert_eq!(value.table, at(3, 0x3000));
        assert_eq!(value.entry_count, 11);
        assert_eq!(symbol.address(), None);
    });
}

#[test]
fn decodes_local() {
    let payload = Payload::default().u32(0x1326).u16(0x101).bytes(b"self\0");
    decode(S_LOCAL, &payload, |symbol| {
        let SymbolData::Local(value) = symbol.data() else {
            panic!("expected S_LOCAL");
        };
        assert!(value.flags.contains(LocalFlags::PARAMETER));
        assert!(value.flags.contains(LocalFlags::OPTIMIZED_OUT));
        assert!(!value.flags.contains(LocalFlags::ALIAS));
        assert_eq!(symbol.type_index(), Some(TypeIndex(0x1326)));
        assert_eq!(symbol.name(), Some(&b"self"[..]));
    });
}

fn range(tail: &[u8]) -> Payload {
    Payload::default().u32(0x124).u16(1).u16(31).bytes(tail)
}

#[test]
fn decodes_frame_pointer_def_range_with_gaps() {
    let gaps = [8_u8, 0, 119, 0, 0x80, 0, 2, 0];
    let payload = Payload::default().i32(-24).bytes(&range(&gaps).0);
    decode(S_DEFRANGE_FRAMEPOINTER_REL, &payload, |symbol| {
        let SymbolData::DefRangeFramePointerRelative(value) = symbol.data() else {
            panic!("expected S_DEFRANGE_FRAMEPOINTER_REL");
        };
        assert_eq!(value.offset, -24);
        assert_eq!(value.range.start.offset, 0x124);
        assert_eq!((value.range.start.section, value.range.length), (1, 31));
        assert_eq!(value.gaps.len(), 2);
        let gaps: Vec<AddressGap> = value.gaps.iter().collect();
        assert_eq!((gaps[0].start, gaps[0].length), (8, 119));
        assert_eq!((gaps[1].start, gaps[1].length), (0x80, 2));
    });
    let odd = Payload::default().i32(-24).bytes(&range(&[1, 0]).0);
    assert_eq!(
        error(S_DEFRANGE_FRAMEPOINTER_REL, &odd),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn decodes_full_scope_def_range() {
    let payload = Payload::default().i32(-8);
    decode(S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE, &payload, |symbol| {
        assert_eq!(
            *symbol.data(),
            SymbolData::DefRangeFramePointerRelativeFullScope(-8)
        );
    });
}

#[test]
fn decodes_register_def_range() {
    let payload = Payload::default().u16(330).u16(1).bytes(&range(&[]).0);
    decode(S_DEFRANGE_REGISTER, &payload, |symbol| {
        let SymbolData::DefRangeRegister(value) = symbol.data() else {
            panic!("expected S_DEFRANGE_REGISTER");
        };
        assert_eq!(value.register, 330);
        assert!(value.may_have_no_name());
        assert!(value.gaps.is_empty());
    });
}

#[test]
fn decodes_register_relative_def_ranges() {
    let payload = Payload::default()
        .u16(331)
        .u16(0x41)
        .i32(-16)
        .bytes(&range(&[]).0);
    decode(S_DEFRANGE_REGISTER_REL, &payload, |symbol| {
        let SymbolData::DefRangeRegisterRelative(value) = symbol.data() else {
            panic!("expected S_DEFRANGE_REGISTER_REL");
        };
        assert_eq!((value.register, value.base_offset), (331, -16));
        assert!(value.spilled_member());
        assert_eq!(value.offset_in_parent(), 4);
    });
    let payload = Payload::default()
        .u16(335)
        .u16(0x20)
        .i32(40)
        .i32(-8)
        .bytes(&range(&[4, 0, 2, 0]).0);
    decode(S_DEFRANGE_REGISTER_REL_INDIR, &payload, |symbol| {
        let SymbolData::DefRangeRegisterRelativeIndirect(value) = symbol.data() else {
            panic!("expected S_DEFRANGE_REGISTER_REL_INDIR");
        };
        assert_eq!((value.register, value.base_offset), (335, 40));
        assert_eq!(value.offset_in_udt, -8);
        assert!(!value.spilled_member());
        assert_eq!(value.offset_in_parent(), 2);
        assert_eq!(value.range.length, 31);
        let gap = value.gaps.iter().next().unwrap();
        assert_eq!((gap.start, gap.length), (4, 2));
    });
}

#[test]
fn decodes_register_relative_variable() {
    let payload = Payload::default()
        .u32(32)
        .u32(0x30)
        .u16(335)
        .bytes(b"has_cctor\0");
    decode(S_REGREL32, &payload, |symbol| {
        let SymbolData::RegisterRelative(value) = symbol.data() else {
            panic!("expected S_REGREL32");
        };
        assert_eq!((value.offset, value.register), (32, 335));
        assert_eq!(symbol.type_index(), Some(TypeIndex(0x30)));
    });
}

#[test]
fn decodes_section_and_checks_reserved_byte() {
    let payload = Payload::default()
        .u16(1)
        .u8(12)
        .u8(0)
        .u32(0x1000)
        .u32(131_496)
        .u32(0x6000_0020)
        .bytes(b".text\0");
    decode(S_SECTION, &payload, |symbol| {
        let SymbolData::Section(value) = symbol.data() else {
            panic!("expected S_SECTION");
        };
        assert_eq!((value.number, value.alignment), (1, 12));
        assert_eq!((value.rva, value.length), (0x1000, 131_496));
        assert_eq!(value.characteristics, 0x6000_0020);
        assert_eq!(symbol.name(), Some(&b".text"[..]));
        assert_eq!(symbol.address(), None);
    });
    let mut reserved = payload.0.clone();
    reserved[3] = 1;
    assert_eq!(
        error(S_SECTION, &Payload(reserved)),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn decodes_coff_group() {
    let payload = Payload::default()
        .u32(3312)
        .u32(0x6000_0020)
        .u32(123_504)
        .u16(1)
        .bytes(b".text$mn\0");
    decode(S_COFFGROUP, &payload, |symbol| {
        let SymbolData::CoffGroup(value) = symbol.data() else {
            panic!("expected S_COFFGROUP");
        };
        assert_eq!((value.length, value.characteristics), (3312, 0x6000_0020));
        let address = SectionOffset {
            section: 1,
            offset: 123_504,
        };
        assert_eq!(symbol.address(), Some(address));
    });
}

#[test]
fn decodes_user_defined_type() {
    let payload = Payload::default().u32(0x1010).bytes(b"T\0");
    decode(S_UDT, &payload, |symbol| {
        assert_eq!(symbol.type_index(), Some(TypeIndex(0x1010)));
        assert_eq!(symbol.name(), Some(&b"T"[..]));
    });
}

#[test]
fn decodes_constant_values() {
    let large = Payload::default()
        .u32(0x100c)
        .u16(0x800a)
        .bytes(&u64::MAX.to_le_bytes())
        .bytes(b"DISCR_EXACT\0");
    decode(S_CONSTANT, &large, |symbol| {
        let SymbolData::Constant(value) = symbol.data() else {
            panic!("expected S_CONSTANT");
        };
        assert_eq!(value.value, Integer::Unsigned(u128::from(u64::MAX)));
        assert_eq!(value.name, b"DISCR_EXACT");
        assert_eq!(symbol.type_index(), Some(TypeIndex(0x100c)));
    });
    let small = Payload::default().u32(0x101b).u16(1).bytes(b"NAME\0");
    decode(S_CONSTANT, &small, |symbol| {
        let SymbolData::Constant(value) = symbol.data() else {
            panic!("expected S_CONSTANT");
        };
        assert_eq!(value.value, Integer::Unsigned(1));
    });
    let real = Payload::default()
        .u32(0x40)
        .u16(0x8005)
        .u32(0)
        .bytes(b"F\0");
    assert_eq!(error(S_CONSTANT, &real), ErrorKind::UnsupportedVersion);
    let truncated = Payload::default().u32(0x40).u16(0x8004).u16(1);
    assert_eq!(error(S_CONSTANT, &truncated), ErrorKind::InvalidFormat);
}

#[test]
fn decodes_procedure_references() {
    for kind in [S_PROCREF, S_LPROCREF] {
        let payload = Payload::default().u32(0).u32(396).u16(3).bytes(b"main\0");
        decode(kind, &payload, |symbol| {
            let reference = symbol.procedure_reference().unwrap();
            assert_eq!((reference.offset, reference.module), (396, 3));
            assert_eq!(reference.module_index(), Some(2));
            assert_eq!(reference.name_checksum, 0);
            assert_eq!(symbol.name(), Some(&b"main"[..]));
        });
    }
    let payload = Payload::default().u32(0).u32(4).u16(0).bytes(b"x\0");
    decode(S_PROCREF, &payload, |symbol| {
        assert_eq!(symbol.procedure_reference().unwrap().module_index(), None);
    });
}

#[test]
fn keeps_skip_filler_without_padding_rules() {
    // LLD fills a replaced record with zero bytes; other writers can leave any bytes.
    for filler in [&[0_u8; 22][..], b"\x01\x02\x03\x04\x05\x06"] {
        let bytes = unpadded(S_SKIP, filler);
        let record = Records::new(&bytes).next().unwrap().unwrap();
        let symbol = record.symbol().unwrap();
        assert_eq!(*symbol.data(), SymbolData::Skip(filler));
        assert_eq!(symbol.name(), None);
        assert_eq!(symbol.address(), None);
    }
}
