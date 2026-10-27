//! Type indices, ID indices and primitive type encodings.
//!
//! A type index below [`FIRST_RECORD_INDEX`] encodes a primitive type. It
//! does not refer to a TPI record. Bits 0 to 7 hold the primitive kind, and
//! bits 8 to 11 hold the pointer mode. A higher index refers to a record in
//! the TPI stream. ID indices refer to records in the IPI stream. The two
//! index spaces are separate even though they use the same first index.

use crate::{Error, ErrorKind, Result};

/// The first index that refers to a TPI or IPI record.
pub const FIRST_RECORD_INDEX: u32 = 0x1000;

/// An index in the TPI stream, or a primitive type.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TypeIndex(pub u32);

impl TypeIndex {
    /// The index that means "no type" (`T_NOTYPE`).
    pub const NONE: Self = Self(0);

    /// Return `true` if the index encodes a primitive type.
    #[must_use]
    pub const fn is_primitive(self) -> bool {
        self.0 < FIRST_RECORD_INDEX
    }

    /// Decode a primitive type index.
    ///
    /// Returns `Ok(None)` for an index that refers to a TPI record.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the index is below
    /// [`FIRST_RECORD_INDEX`] but its kind or mode is not a known value.
    pub fn primitive(self) -> Result<Option<Primitive>> {
        if !self.is_primitive() {
            return Ok(None);
        }
        // The index is below 0x1000, so both fields fit in `u8`.
        let [kind, mode, ..] = self.0.to_le_bytes();
        let kind = PrimitiveKind::from_raw(kind);
        let mode = PrimitiveMode::from_raw(mode);
        match (kind, mode) {
            (
                Some(PrimitiveKind::CvPointer),
                Some(PrimitiveMode::Direct | PrimitiveMode::NearPointer128),
            )
            | (None, _)
            | (_, None) => Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("primitive type index has an unknown kind or mode")),
            (Some(kind), Some(mode)) => Ok(Some(Primitive { kind, mode })),
        }
    }
}

/// An index in the IPI stream.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IdIndex(pub u32);

impl IdIndex {
    /// The index that means "no ID".
    pub const NONE: Self = Self(0);
}

/// The 0-based index of a module in the DBI module list.
///
/// Some records store a 1-based module number. A decoder converts the number 0 of such a
/// record to `None` and the number `n` to `ModuleIndex(n - 1)`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ModuleIndex(pub u32);

/// The byte offset of a symbol record in the symbol substream of a module stream.
///
/// The offset starts at the start of the module stream, so the first record after the
/// signature has offset 4. It is not an offset in the global symbol stream.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ModuleSymbolOffset(pub u32);

/// A symbol record of a module stream before it has a module stream offset.
///
/// A builder gives the handle for each module symbol record. A
/// [`ModuleSymbolTarget::Handle`](crate::native::codeview::symbol::ModuleSymbolTarget::Handle)
/// holds it. Only a write plan converts the handle to a module index and a
/// [`ModuleSymbolOffset`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ModuleSymbolHandle {
    /// The module index.
    pub(crate) module: u32,
    /// The position of the record in the module.
    pub(crate) position: u32,
}

impl ModuleSymbolHandle {
    /// Return the index of the module of the record in the DBI module list.
    #[must_use]
    pub const fn module(self) -> ModuleIndex {
        ModuleIndex(self.module)
    }
}

/// A record of the symbol record stream before it has a symbol offset.
///
/// A builder gives the handle for each record of the symbol record stream.
/// Only a write plan converts it to a
/// [`SymbolOffset`](crate::native::symbols::SymbolOffset).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SymbolHandle(pub(crate) u32);

/// The byte offset of a string in the string buffer of the `/names` stream.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NameOffset(pub u32);

/// The byte offset of an entry in the `DEBUG_S_FILECHKSMS` subsection of a module.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ChecksumOffset(pub u32);

/// A decoded primitive type index.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Primitive {
    /// The value type.
    pub kind: PrimitiveKind,
    /// Direct use or the pointer form.
    pub mode: PrimitiveMode,
}

impl Primitive {
    /// Make a record with a kind and a mode.
    #[must_use]
    pub const fn new(kind: PrimitiveKind, mode: PrimitiveMode) -> Self {
        Self { kind, mode }
    }

    /// Return the type index of the primitive type.
    #[must_use]
    pub fn index(self) -> TypeIndex {
        TypeIndex(u32::from(self.mode.raw()) << 8 | u32::from(self.kind.raw()))
    }

    /// Return the size of a value of the type in bytes.
    ///
    /// A pointer mode gives the size of the pointer. The direct mode gives
    /// [`PrimitiveKind::size`].
    #[must_use]
    pub const fn size(self) -> Option<u32> {
        match self.mode.pointer_size() {
            Some(size) => Some(size),
            None => self.kind.size(),
        }
    }
}

/// The pointer mode of a primitive type index (`CV_prmode_e`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum PrimitiveMode {
    /// The value itself, not a pointer.
    Direct,
    /// A 16-bit near pointer.
    NearPointer16,
    /// A 16:16 far pointer.
    FarPointer16,
    /// A 16:16 huge pointer.
    HugePointer16,
    /// A 32-bit near pointer.
    NearPointer32,
    /// A 16:32 far pointer.
    FarPointer32,
    /// A 64-bit pointer.
    NearPointer64,
    /// A 128-bit pointer.
    NearPointer128,
}

impl PrimitiveMode {
    /// Return the value in bits 8 to 11 of a primitive type index.
    #[must_use]
    pub const fn raw(self) -> u8 {
        match self {
            Self::Direct => 0,
            Self::NearPointer16 => 1,
            Self::FarPointer16 => 2,
            Self::HugePointer16 => 3,
            Self::NearPointer32 => 4,
            Self::FarPointer32 => 5,
            Self::NearPointer64 => 6,
            Self::NearPointer128 => 7,
        }
    }

    /// Return the size of a pointer of this mode in bytes, or `None` for
    /// [`PrimitiveMode::Direct`].
    #[must_use]
    pub const fn pointer_size(self) -> Option<u32> {
        Some(match self {
            Self::Direct => return None,
            Self::NearPointer16 => 2,
            Self::FarPointer16 | Self::HugePointer16 | Self::NearPointer32 => 4,
            Self::FarPointer32 => 6,
            Self::NearPointer64 => 8,
            Self::NearPointer128 => 16,
        })
    }

    /// Decode bits 8 to 11 of a primitive type index.
    const fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            0 => Self::Direct,
            1 => Self::NearPointer16,
            2 => Self::FarPointer16,
            3 => Self::HugePointer16,
            4 => Self::NearPointer32,
            5 => Self::FarPointer32,
            6 => Self::NearPointer64,
            7 => Self::NearPointer128,
            _ => return None,
        })
    }
}

/// Define [`PrimitiveKind`] and its raw encoding from one table.
macro_rules! primitive_kinds {
    ($($(#[$meta:meta])* $name:ident = $value:literal,)*) => {
        /// The value type of a primitive type index (`TYPE_ENUM_e`).
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        #[non_exhaustive]
        pub enum PrimitiveKind {
            $($(#[$meta])* $name,)*
        }

        impl PrimitiveKind {
            /// Return the raw value in bits 0 to 7 of a primitive type index.
            #[must_use]
            pub const fn raw(self) -> u8 {
                match self {
                    $(Self::$name => $value,)*
                }
            }

            /// Decode bits 0 to 7 of a primitive type index.
            const fn from_raw(raw: u8) -> Option<Self> {
                match raw {
                    $($value => Some(Self::$name),)*
                    _ => None,
                }
            }
        }
    };
}

primitive_kinds! {
    /// No type (`T_NOTYPE`).
    NoType = 0x00,
    /// An absolute symbol (`T_ABS`).
    Absolute = 0x01,
    /// A segment (`T_SEGMENT`).
    Segment = 0x02,
    /// `void` (`T_VOID`).
    Void = 0x03,
    /// A BASIC 8-byte currency value (`T_CURRENCY`).
    Currency = 0x04,
    /// A near BASIC string (`T_NBASICSTR`).
    NearBasicString = 0x05,
    /// A far BASIC string (`T_FBASICSTR`).
    FarBasicString = 0x06,
    /// A type that was not translated (`T_NOTTRANS`).
    NotTranslated = 0x07,
    /// An OLE `HRESULT` (`T_HRESULT`).
    HResult = 0x08,
    /// A signed 8-bit character (`T_CHAR`).
    SignedCharacter = 0x10,
    /// A signed 16-bit integer (`T_SHORT`).
    Int16Short = 0x11,
    /// A signed 32-bit integer (`T_LONG`).
    Int32Long = 0x12,
    /// A signed 64-bit integer (`T_QUAD`).
    Int64Quad = 0x13,
    /// A signed 128-bit integer (`T_OCT`).
    Int128Oct = 0x14,
    /// An unsigned 8-bit character (`T_UCHAR`).
    UnsignedCharacter = 0x20,
    /// An unsigned 16-bit integer (`T_USHORT`).
    UInt16Short = 0x21,
    /// An unsigned 32-bit integer (`T_ULONG`).
    UInt32Long = 0x22,
    /// An unsigned 64-bit integer (`T_UQUAD`).
    UInt64Quad = 0x23,
    /// An unsigned 128-bit integer (`T_UOCT`).
    UInt128Oct = 0x24,
    /// An 8-bit Boolean (`T_BOOL08`).
    Boolean8 = 0x30,
    /// A 16-bit Boolean (`T_BOOL16`).
    Boolean16 = 0x31,
    /// A 32-bit Boolean (`T_BOOL32`).
    Boolean32 = 0x32,
    /// A 64-bit Boolean (`T_BOOL64`).
    Boolean64 = 0x33,
    /// A 128-bit Boolean.
    Boolean128 = 0x34,
    /// A 32-bit floating-point value (`T_REAL32`).
    Float32 = 0x40,
    /// A 64-bit floating-point value (`T_REAL64`).
    Float64 = 0x41,
    /// An 80-bit floating-point value (`T_REAL80`).
    Float80 = 0x42,
    /// A 128-bit floating-point value (`T_REAL128`).
    Float128 = 0x43,
    /// A 48-bit floating-point value (`T_REAL48`).
    Float48 = 0x44,
    /// A 32-bit partial-precision floating-point value (`T_REAL32PP`).
    Float32PartialPrecision = 0x45,
    /// A 16-bit floating-point value (`T_REAL16`).
    Float16 = 0x46,
    /// A 32-bit complex value (`T_CPLX32`).
    Complex32 = 0x50,
    /// A 64-bit complex value (`T_CPLX64`).
    Complex64 = 0x51,
    /// An 80-bit complex value (`T_CPLX80`).
    Complex80 = 0x52,
    /// A 128-bit complex value (`T_CPLX128`).
    Complex128 = 0x53,
    /// A 48-bit complex value.
    Complex48 = 0x54,
    /// A 32-bit partial-precision complex value.
    Complex32PartialPrecision = 0x55,
    /// A 16-bit complex value.
    Complex16 = 0x56,
    /// A bit (`T_BIT`).
    Bit = 0x60,
    /// A Pascal `CHAR` (`T_PASCHAR`).
    PascalCharacter = 0x61,
    /// A 32-bit Boolean where true is `0xffffffff` (`T_BOOL32FF`).
    Boolean32FF = 0x62,
    /// A signed 8-bit integer (`T_INT1`).
    SByte = 0x68,
    /// An unsigned 8-bit integer (`T_UINT1`).
    Byte = 0x69,
    /// A narrow character (`T_RCHAR`).
    NarrowCharacter = 0x70,
    /// A wide character (`T_WCHAR`).
    WideCharacter = 0x71,
    /// A signed 16-bit integer (`T_INT2`).
    Int16 = 0x72,
    /// An unsigned 16-bit integer (`T_UINT2`).
    UInt16 = 0x73,
    /// A signed 32-bit integer (`T_INT4`).
    Int32 = 0x74,
    /// An unsigned 32-bit integer (`T_UINT4`).
    UInt32 = 0x75,
    /// A signed 64-bit integer (`T_INT8`).
    Int64 = 0x76,
    /// An unsigned 64-bit integer (`T_UINT8`).
    UInt64 = 0x77,
    /// A signed 128-bit integer (`T_INT16`).
    Int128 = 0x78,
    /// An unsigned 128-bit integer (`T_UINT16`).
    UInt128 = 0x79,
    /// A 16-bit Unicode character (`T_CHAR16`).
    Character16 = 0x7a,
    /// A 32-bit Unicode character (`T_CHAR32`).
    Character32 = 0x7b,
    /// An 8-bit Unicode character (`T_CHAR8`).
    Character8 = 0x7c,
    /// A pointer that `CodeView` created (`T_NCVPTR`, `T_32NCVPTR` and the
    /// other `CVPTR` indices). Only the pointer modes 1 to 6 are valid.
    CvPointer = 0xf0,
}

impl PrimitiveKind {
    /// Return the size of a value of the kind in bytes.
    ///
    /// Returns `None` for kinds without a value size: no type, absolute
    /// symbols, segments, `void`, BASIC strings, types that were not
    /// translated, bits, Pascal characters and `CodeView` pointers.
    #[must_use]
    pub const fn size(self) -> Option<u32> {
        Some(match self {
            Self::SignedCharacter
            | Self::UnsignedCharacter
            | Self::Boolean8
            | Self::SByte
            | Self::Byte
            | Self::NarrowCharacter
            | Self::Character8 => 1,
            Self::Int16Short
            | Self::UInt16Short
            | Self::Boolean16
            | Self::Float16
            | Self::WideCharacter
            | Self::Int16
            | Self::UInt16
            | Self::Character16 => 2,
            Self::HResult
            | Self::Int32Long
            | Self::UInt32Long
            | Self::Boolean32
            | Self::Float32
            | Self::Float32PartialPrecision
            | Self::Complex16
            | Self::Boolean32FF
            | Self::Int32
            | Self::UInt32
            | Self::Character32 => 4,
            Self::Float48 => 6,
            Self::Currency
            | Self::Int64Quad
            | Self::UInt64Quad
            | Self::Boolean64
            | Self::Float64
            | Self::Complex32
            | Self::Complex32PartialPrecision
            | Self::Int64
            | Self::UInt64 => 8,
            Self::Float80 => 10,
            Self::Complex48 => 12,
            Self::Int128Oct
            | Self::UInt128Oct
            | Self::Boolean128
            | Self::Float128
            | Self::Complex64
            | Self::Int128
            | Self::UInt128 => 16,
            Self::Complex80 => 20,
            Self::Complex128 => 32,
            Self::NoType
            | Self::Absolute
            | Self::Segment
            | Self::Void
            | Self::NearBasicString
            | Self::FarBasicString
            | Self::NotTranslated
            | Self::Bit
            | Self::PascalCharacter
            | Self::CvPointer => return None,
        })
    }

    /// Return `Some(true)` for a signed integer or character kind,
    /// `Some(false)` for an unsigned one, and `None` for other kinds.
    #[must_use]
    pub const fn is_signed_integer(self) -> Option<bool> {
        match self {
            Self::SignedCharacter
            | Self::Int16Short
            | Self::Int32Long
            | Self::Int64Quad
            | Self::Int128Oct
            | Self::SByte
            | Self::NarrowCharacter
            | Self::Int16
            | Self::Int32
            | Self::Int64
            | Self::Int128 => Some(true),
            Self::UnsignedCharacter
            | Self::UInt16Short
            | Self::UInt32Long
            | Self::UInt64Quad
            | Self::UInt128Oct
            | Self::Byte
            | Self::WideCharacter
            | Self::UInt16
            | Self::UInt32
            | Self::UInt64
            | Self::UInt128
            | Self::Character8
            | Self::Character16
            | Self::Character32 => Some(false),
            _ => None,
        }
    }
}
