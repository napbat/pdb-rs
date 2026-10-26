//! Primitive type indices: every kind and mode of `TYPE_ENUM_e` and
//! `CV_prmode_e` in `cvinfo.h`, their sizes and their encoding.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::index::{Primitive, PrimitiveKind, PrimitiveMode, TypeIndex};

/// Every kind with its value and its size in bytes.
const KINDS: &[(u8, PrimitiveKind, Option<u32>)] = &[
    (0x00, PrimitiveKind::NoType, None),
    (0x01, PrimitiveKind::Absolute, None),
    (0x02, PrimitiveKind::Segment, None),
    (0x03, PrimitiveKind::Void, None),
    (0x04, PrimitiveKind::Currency, Some(8)),
    (0x05, PrimitiveKind::NearBasicString, None),
    (0x06, PrimitiveKind::FarBasicString, None),
    (0x07, PrimitiveKind::NotTranslated, None),
    (0x08, PrimitiveKind::HResult, Some(4)),
    (0x10, PrimitiveKind::SignedCharacter, Some(1)),
    (0x11, PrimitiveKind::Int16Short, Some(2)),
    (0x12, PrimitiveKind::Int32Long, Some(4)),
    (0x13, PrimitiveKind::Int64Quad, Some(8)),
    (0x14, PrimitiveKind::Int128Oct, Some(16)),
    (0x20, PrimitiveKind::UnsignedCharacter, Some(1)),
    (0x21, PrimitiveKind::UInt16Short, Some(2)),
    (0x22, PrimitiveKind::UInt32Long, Some(4)),
    (0x23, PrimitiveKind::UInt64Quad, Some(8)),
    (0x24, PrimitiveKind::UInt128Oct, Some(16)),
    (0x30, PrimitiveKind::Boolean8, Some(1)),
    (0x31, PrimitiveKind::Boolean16, Some(2)),
    (0x32, PrimitiveKind::Boolean32, Some(4)),
    (0x33, PrimitiveKind::Boolean64, Some(8)),
    (0x34, PrimitiveKind::Boolean128, Some(16)),
    (0x40, PrimitiveKind::Float32, Some(4)),
    (0x41, PrimitiveKind::Float64, Some(8)),
    (0x42, PrimitiveKind::Float80, Some(10)),
    (0x43, PrimitiveKind::Float128, Some(16)),
    (0x44, PrimitiveKind::Float48, Some(6)),
    (0x45, PrimitiveKind::Float32PartialPrecision, Some(4)),
    (0x46, PrimitiveKind::Float16, Some(2)),
    (0x50, PrimitiveKind::Complex32, Some(8)),
    (0x51, PrimitiveKind::Complex64, Some(16)),
    (0x52, PrimitiveKind::Complex80, Some(20)),
    (0x53, PrimitiveKind::Complex128, Some(32)),
    (0x54, PrimitiveKind::Complex48, Some(12)),
    (0x55, PrimitiveKind::Complex32PartialPrecision, Some(8)),
    (0x56, PrimitiveKind::Complex16, Some(4)),
    (0x60, PrimitiveKind::Bit, None),
    (0x61, PrimitiveKind::PascalCharacter, None),
    (0x62, PrimitiveKind::Boolean32FF, Some(4)),
    (0x68, PrimitiveKind::SByte, Some(1)),
    (0x69, PrimitiveKind::Byte, Some(1)),
    (0x70, PrimitiveKind::NarrowCharacter, Some(1)),
    (0x71, PrimitiveKind::WideCharacter, Some(2)),
    (0x72, PrimitiveKind::Int16, Some(2)),
    (0x73, PrimitiveKind::UInt16, Some(2)),
    (0x74, PrimitiveKind::Int32, Some(4)),
    (0x75, PrimitiveKind::UInt32, Some(4)),
    (0x76, PrimitiveKind::Int64, Some(8)),
    (0x77, PrimitiveKind::UInt64, Some(8)),
    (0x78, PrimitiveKind::Int128, Some(16)),
    (0x79, PrimitiveKind::UInt128, Some(16)),
    (0x7a, PrimitiveKind::Character16, Some(2)),
    (0x7b, PrimitiveKind::Character32, Some(4)),
    (0x7c, PrimitiveKind::Character8, Some(1)),
    (0xf0, PrimitiveKind::CvPointer, None),
];

/// Every mode with its value and its pointer size.
const MODES: &[(u8, PrimitiveMode, Option<u32>)] = &[
    (0, PrimitiveMode::Direct, None),
    (1, PrimitiveMode::NearPointer16, Some(2)),
    (2, PrimitiveMode::FarPointer16, Some(4)),
    (3, PrimitiveMode::HugePointer16, Some(4)),
    (4, PrimitiveMode::NearPointer32, Some(4)),
    (5, PrimitiveMode::FarPointer32, Some(6)),
    (6, PrimitiveMode::NearPointer64, Some(8)),
    (7, PrimitiveMode::NearPointer128, Some(16)),
];

#[test]
fn decodes_every_kind_in_every_mode() {
    for &(raw_kind, kind, kind_size) in KINDS {
        assert_eq!(kind.raw(), raw_kind);
        assert_eq!(kind.size(), kind_size, "{kind:?}");
        for &(raw_mode, mode, pointer_size) in MODES {
            assert_eq!(mode.raw(), raw_mode);
            assert_eq!(mode.pointer_size(), pointer_size);
            let index = TypeIndex(u32::from(raw_mode) << 8 | u32::from(raw_kind));
            let cv_pointer_mode = (1..=6).contains(&raw_mode);
            if kind == PrimitiveKind::CvPointer && !cv_pointer_mode {
                let error = index.primitive().unwrap_err();
                assert_eq!(error.kind(), ErrorKind::InvalidFormat, "{index:?}");
                continue;
            }
            let primitive = index.primitive().unwrap().unwrap();
            assert_eq!(primitive, Primitive::new(kind, mode));
            assert_eq!(primitive.index(), index);
            assert_eq!(primitive.size(), pointer_size.or(kind_size));
            assert!(index.is_primitive());
        }
    }
}

#[test]
fn decodes_codeview_created_pointers() {
    // `T_NCVPTR` to `T_64NCVPTR` in `cvinfo.h`.
    for (value, mode) in [
        (0x01f0, PrimitiveMode::NearPointer16),
        (0x02f0, PrimitiveMode::FarPointer16),
        (0x03f0, PrimitiveMode::HugePointer16),
        (0x04f0, PrimitiveMode::NearPointer32),
        (0x05f0, PrimitiveMode::FarPointer32),
        (0x06f0, PrimitiveMode::NearPointer64),
    ] {
        let primitive = TypeIndex(value).primitive().unwrap().unwrap();
        assert_eq!(primitive.kind, PrimitiveKind::CvPointer);
        assert_eq!(primitive.mode, mode);
    }
}

#[test]
fn rejects_unknown_kinds_and_modes() {
    let known: Vec<u8> = KINDS.iter().map(|&(raw, ..)| raw).collect();
    for raw in 0..=u8::MAX {
        if known.contains(&raw) {
            continue;
        }
        let error = TypeIndex(u32::from(raw)).primitive().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat, "kind {raw:#x}");
    }
    for mode in 8..16_u32 {
        let error = TypeIndex(mode << 8 | 0x74).primitive().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat, "mode {mode}");
    }
}

#[test]
fn record_indices_are_not_primitive() {
    assert_eq!(TypeIndex(0x1000).primitive().unwrap(), None);
    assert!(!TypeIndex(0x1000).is_primitive());
    assert!(TypeIndex(0x0fff).is_primitive());
}

#[test]
fn reports_integer_signedness() {
    assert_eq!(PrimitiveKind::Int32.is_signed_integer(), Some(true));
    assert_eq!(
        PrimitiveKind::NarrowCharacter.is_signed_integer(),
        Some(true)
    );
    assert_eq!(PrimitiveKind::UInt64Quad.is_signed_integer(), Some(false));
    assert_eq!(
        PrimitiveKind::WideCharacter.is_signed_integer(),
        Some(false)
    );
    assert_eq!(PrimitiveKind::Float32.is_signed_integer(), None);
    assert_eq!(PrimitiveKind::Boolean8.is_signed_integer(), None);
}
