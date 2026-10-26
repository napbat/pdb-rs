//! Numeric leaves shared by type records and symbol records.
//!
//! The first two bytes hold an unsigned value below `0x8000`, or a leaf
//! tag followed by a value of the tag. [`Numeric`] decodes all leaf tags of
//! `cvinfo.h`: integers, real numbers, complex numbers, `DECIMAL`, `DATE`
//! and two string forms. It keeps the tag and the exact value bytes, so
//! [`Numeric::encode`] gives the input bytes again. [`Integer`] is the value
//! of an integer leaf without its encoding width.
//!
//! The decoders do not allocate. They do not consume bytes after the leaf.

use alloc::vec::Vec;

use super::Fields;
use crate::{Error, ErrorKind, Result};

/// The first leaf tag. Smaller values are immediate unsigned values.
pub const LF_NUMERIC: u16 = 0x8000;
/// A signed 8-bit integer.
pub const LF_CHAR: u16 = 0x8000;
/// A signed 16-bit integer.
pub const LF_SHORT: u16 = 0x8001;
/// An unsigned 16-bit integer.
pub const LF_USHORT: u16 = 0x8002;
/// A signed 32-bit integer.
pub const LF_LONG: u16 = 0x8003;
/// An unsigned 32-bit integer.
pub const LF_ULONG: u16 = 0x8004;
/// A 32-bit floating-point value.
pub const LF_REAL32: u16 = 0x8005;
/// A 64-bit floating-point value.
pub const LF_REAL64: u16 = 0x8006;
/// An 80-bit floating-point value.
pub const LF_REAL80: u16 = 0x8007;
/// A 128-bit floating-point value.
pub const LF_REAL128: u16 = 0x8008;
/// A signed 64-bit integer.
pub const LF_QUADWORD: u16 = 0x8009;
/// An unsigned 64-bit integer.
pub const LF_UQUADWORD: u16 = 0x800a;
/// A 48-bit floating-point value.
pub const LF_REAL48: u16 = 0x800b;
/// A complex value of two 32-bit floating-point values.
pub const LF_COMPLEX32: u16 = 0x800c;
/// A complex value of two 64-bit floating-point values.
pub const LF_COMPLEX64: u16 = 0x800d;
/// A complex value of two 80-bit floating-point values.
pub const LF_COMPLEX80: u16 = 0x800e;
/// A complex value of two 128-bit floating-point values.
pub const LF_COMPLEX128: u16 = 0x800f;
/// A string with a `u16` byte count.
pub const LF_VARSTRING: u16 = 0x8010;
/// A signed 128-bit integer.
pub const LF_OCTWORD: u16 = 0x8017;
/// An unsigned 128-bit integer.
pub const LF_UOCTWORD: u16 = 0x8018;
/// An OLE `DECIMAL` value.
pub const LF_DECIMAL: u16 = 0x8019;
/// An OLE `DATE` value.
pub const LF_DATE: u16 = 0x801a;
/// A NUL-terminated UTF-8 string.
pub const LF_UTF8STRING: u16 = 0x801b;
/// A 16-bit floating-point value.
pub const LF_REAL16: u16 = 0x801c;

/// An integer decoded from a `CodeView` numeric leaf.
///
/// Values are widened without loss. The encoding width is not kept here;
/// [`IntegerLeaf`] keeps it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Integer {
    /// A signed integer, including signed 128-bit values.
    Signed(i128),
    /// An unsigned integer, including unsigned 128-bit values.
    Unsigned(u128),
}

impl Integer {
    /// Decode one integer and return the bytes after it.
    ///
    /// Supports immediate values and `LF_CHAR`, `LF_SHORT`, `LF_USHORT`,
    /// `LF_LONG`, `LF_ULONG`, `LF_QUADWORD`, `LF_UQUADWORD`, `LF_OCTWORD`
    /// and `LF_UOCTWORD`. The suffix can contain a name, another field or
    /// record padding. No alignment is applied here.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::UnexpectedEof`] for a truncated tag or integer.
    /// Returns [`ErrorKind::UnsupportedVersion`] for any other leaf tag,
    /// including known non-integer tags. Use [`Numeric::parse_prefix`] for
    /// those. Errors have no physical offset; callers can attach record
    /// context.
    pub fn parse_prefix(bytes: &[u8]) -> Result<(Self, &[u8])> {
        let integer_tag = match bytes {
            [low, high, ..] => matches!(
                u16::from_le_bytes([*low, *high]),
                0..LF_NUMERIC
                    | LF_CHAR
                    | LF_SHORT
                    | LF_USHORT
                    | LF_LONG
                    | LF_ULONG
                    | LF_QUADWORD
                    | LF_UQUADWORD
                    | LF_OCTWORD
                    | LF_UOCTWORD
            ),
            // `Numeric::parse_prefix` reports the truncated tag.
            _ => true,
        };
        match (integer_tag, Numeric::parse_prefix(bytes)) {
            (true, Ok((Numeric::Integer(leaf), rest))) => Ok((leaf.value(), rest)),
            (true, Err(error)) => Err(error),
            _ => Err(Error::new(ErrorKind::UnsupportedVersion, None)
                .with_context("CodeView numeric leaf is not a supported integer")),
        }
    }

    /// Return the value as `i128`, or `None` if it does not fit.
    #[must_use]
    pub fn to_i128(self) -> Option<i128> {
        match self {
            Self::Signed(value) => Some(value),
            Self::Unsigned(value) => i128::try_from(value).ok(),
        }
    }

    /// Return the value as `u128`, or `None` for a negative value.
    #[must_use]
    pub fn to_u128(self) -> Option<u128> {
        match self {
            Self::Signed(value) => u128::try_from(value).ok(),
            Self::Unsigned(value) => Some(value),
        }
    }

    /// Return the value as `u64`, or `None` if it does not fit.
    #[must_use]
    pub fn to_u64(self) -> Option<u64> {
        self.to_u128().and_then(|value| u64::try_from(value).ok())
    }
}

/// The encoding of an integer numeric leaf.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IntegerKind {
    /// An unsigned value below `0x8000` in the tag field.
    Immediate,
    /// `LF_CHAR`: a signed 8-bit value.
    Char,
    /// `LF_SHORT`: a signed 16-bit value.
    Short,
    /// `LF_USHORT`: an unsigned 16-bit value.
    UShort,
    /// `LF_LONG`: a signed 32-bit value.
    Long,
    /// `LF_ULONG`: an unsigned 32-bit value.
    ULong,
    /// `LF_QUADWORD`: a signed 64-bit value.
    QuadWord,
    /// `LF_UQUADWORD`: an unsigned 64-bit value.
    UQuadWord,
    /// `LF_OCTWORD`: a signed 128-bit value.
    OctWord,
    /// `LF_UOCTWORD`: an unsigned 128-bit value.
    UOctWord,
}

impl IntegerKind {
    /// Return the smallest and the largest value of the encoding.
    const fn range(self) -> (i128, u128) {
        match self {
            Self::Immediate => (0, 0x7fff),
            Self::Char => (-0x80, 0x7f),
            Self::Short => (-0x8000, 0x7fff),
            Self::UShort => (0, 0xffff),
            Self::Long => (-0x8000_0000, 0x7fff_ffff),
            Self::ULong => (0, 0xffff_ffff),
            Self::QuadWord => (-0x8000_0000_0000_0000, 0x7fff_ffff_ffff_ffff),
            Self::UQuadWord => (0, 0xffff_ffff_ffff_ffff),
            Self::OctWord => (i128::MIN, u128::MAX >> 1),
            Self::UOctWord => (0, u128::MAX),
        }
    }

    /// Return `true` for the signed encodings.
    const fn is_signed(self) -> bool {
        matches!(
            self,
            Self::Char | Self::Short | Self::Long | Self::QuadWord | Self::OctWord
        )
    }

    /// Return the leaf tag, or `None` for an immediate value.
    const fn tag(self) -> Option<u16> {
        Some(match self {
            Self::Immediate => return None,
            Self::Char => LF_CHAR,
            Self::Short => LF_SHORT,
            Self::UShort => LF_USHORT,
            Self::Long => LF_LONG,
            Self::ULong => LF_ULONG,
            Self::QuadWord => LF_QUADWORD,
            Self::UQuadWord => LF_UQUADWORD,
            Self::OctWord => LF_OCTWORD,
            Self::UOctWord => LF_UOCTWORD,
        })
    }

    /// Return the number of value bytes after the tag.
    const fn width(self) -> usize {
        match self {
            Self::Immediate => 0,
            Self::Char => 1,
            Self::Short | Self::UShort => 2,
            Self::Long | Self::ULong => 4,
            Self::QuadWord | Self::UQuadWord => 8,
            Self::OctWord | Self::UOctWord => 16,
        }
    }
}

/// An integer numeric leaf: a value and its encoding.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IntegerLeaf {
    /// The encoding.
    kind: IntegerKind,
    /// The value. Signed encodings give [`Integer::Signed`], the others
    /// give [`Integer::Unsigned`].
    value: Integer,
}

impl IntegerLeaf {
    /// Make a leaf with an explicit encoding.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if `value` does not fit `kind`.
    pub fn new(kind: IntegerKind, value: Integer) -> Result<Self> {
        let (min, max) = kind.range();
        let fits = match value {
            Integer::Signed(value) => {
                value >= min && !u128::try_from(value).is_ok_and(|value| value > max)
            }
            Integer::Unsigned(value) => value <= max,
        };
        if !fits {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("integer value does not fit its numeric leaf"));
        }
        let value = match (kind.is_signed(), value) {
            (true, Integer::Unsigned(value)) => {
                // `fits` makes sure that the value is at most the signed maximum.
                Integer::Signed(i128::try_from(value).unwrap_or(i128::MAX))
            }
            (false, Integer::Signed(value)) => {
                // `fits` makes sure that the value is not negative.
                Integer::Unsigned(u128::try_from(value).unwrap_or(0))
            }
            (_, value) => value,
        };
        Ok(Self { kind, value })
    }

    /// Return the encoding.
    #[must_use]
    pub const fn kind(&self) -> IntegerKind {
        self.kind
    }

    /// Return the value.
    #[must_use]
    pub const fn value(&self) -> Integer {
        self.value
    }
}

/// The fields of an OLE `DECIMAL` value (`LF_DECIMAL`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Decimal {
    /// The reserved `u16` field.
    pub reserved: u16,
    /// The number of decimal digits after the decimal point.
    pub scale: u8,
    /// The sign byte: `0x80` for a negative value, 0 otherwise.
    pub sign: u8,
    /// The high 32 bits of the 96-bit magnitude.
    pub high: u32,
    /// The low 64 bits of the 96-bit magnitude.
    pub low: u64,
}

/// A decoded numeric leaf.
///
/// The non-integer variants keep the exact value bytes after the tag. The
/// accessors give typed values where Rust has a type for them.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Numeric<'a> {
    /// An immediate value or an integer leaf.
    Integer(IntegerLeaf),
    /// `LF_REAL16`.
    Real16([u8; 2]),
    /// `LF_REAL32`.
    Real32([u8; 4]),
    /// `LF_REAL48`.
    Real48([u8; 6]),
    /// `LF_REAL64`.
    Real64([u8; 8]),
    /// `LF_REAL80`.
    Real80([u8; 10]),
    /// `LF_REAL128`.
    Real128([u8; 16]),
    /// `LF_COMPLEX32`: the real part, then the imaginary part.
    Complex32([u8; 8]),
    /// `LF_COMPLEX64`: the real part, then the imaginary part.
    Complex64([u8; 16]),
    /// `LF_COMPLEX80`: the real part, then the imaginary part.
    Complex80([u8; 20]),
    /// `LF_COMPLEX128`: the real part, then the imaginary part.
    Complex128([u8; 32]),
    /// `LF_DECIMAL`.
    Decimal([u8; 16]),
    /// `LF_DATE`: an OLE automation date.
    Date([u8; 8]),
    /// `LF_VARSTRING`: the string bytes, without the `u16` count.
    VarString(&'a [u8]),
    /// `LF_UTF8STRING`: the string bytes, without the NUL terminator.
    Utf8String(&'a [u8]),
}

/// Read a fixed number of value bytes.
fn array<const N: usize>(fields: &mut Fields<'_>) -> Result<[u8; N]> {
    fields.array()
}

impl<'a> Numeric<'a> {
    /// Decode one numeric leaf and return the bytes after it.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] for a truncated tag or value.
    /// - [`ErrorKind::InvalidFormat`] for an `LF_UTF8STRING` value without
    ///   a NUL terminator.
    /// - [`ErrorKind::UnsupportedVersion`] for a tag that `cvinfo.h` does not
    ///   define.
    pub fn parse_prefix(bytes: &'a [u8]) -> Result<(Self, &'a [u8])> {
        let mut fields = Fields::new(
            bytes,
            ErrorKind::UnexpectedEof,
            "CodeView numeric leaf is truncated",
        );
        let tag = fields.u16()?;
        let signed = |kind, value: i128| {
            Self::Integer(IntegerLeaf {
                kind,
                value: Integer::Signed(value),
            })
        };
        let unsigned = |kind, value: u128| {
            Self::Integer(IntegerLeaf {
                kind,
                value: Integer::Unsigned(value),
            })
        };
        let value = match tag {
            0..LF_NUMERIC => unsigned(IntegerKind::Immediate, u128::from(tag)),
            LF_CHAR => signed(
                IntegerKind::Char,
                i8::from_le_bytes(array(&mut fields)?).into(),
            ),
            LF_SHORT => signed(
                IntegerKind::Short,
                i16::from_le_bytes(array(&mut fields)?).into(),
            ),
            LF_USHORT => unsigned(IntegerKind::UShort, fields.u16()?.into()),
            LF_LONG => signed(IntegerKind::Long, fields.i32()?.into()),
            LF_ULONG => unsigned(IntegerKind::ULong, fields.u32()?.into()),
            LF_QUADWORD => signed(
                IntegerKind::QuadWord,
                i64::from_le_bytes(array(&mut fields)?).into(),
            ),
            LF_UQUADWORD => unsigned(IntegerKind::UQuadWord, fields.u64()?.into()),
            LF_OCTWORD => signed(
                IntegerKind::OctWord,
                i128::from_le_bytes(array(&mut fields)?),
            ),
            LF_UOCTWORD => unsigned(
                IntegerKind::UOctWord,
                u128::from_le_bytes(array(&mut fields)?),
            ),
            LF_REAL16 => Self::Real16(array(&mut fields)?),
            LF_REAL32 => Self::Real32(array(&mut fields)?),
            LF_REAL48 => Self::Real48(array(&mut fields)?),
            LF_REAL64 => Self::Real64(array(&mut fields)?),
            LF_REAL80 => Self::Real80(array(&mut fields)?),
            LF_REAL128 => Self::Real128(array(&mut fields)?),
            LF_COMPLEX32 => Self::Complex32(array(&mut fields)?),
            LF_COMPLEX64 => Self::Complex64(array(&mut fields)?),
            LF_COMPLEX80 => Self::Complex80(array(&mut fields)?),
            LF_COMPLEX128 => Self::Complex128(array(&mut fields)?),
            LF_DECIMAL => Self::Decimal(array(&mut fields)?),
            LF_DATE => Self::Date(array(&mut fields)?),
            LF_VARSTRING => {
                let count = usize::from(fields.u16()?);
                Self::VarString(fields.bytes(count)?)
            }
            LF_UTF8STRING => Self::Utf8String(fields.name()?),
            _ => {
                return Err(Error::new(ErrorKind::UnsupportedVersion, None)
                    .with_context("CodeView numeric leaf has an unknown tag"));
            }
        };
        Ok((value, fields.rest()))
    }

    /// Return the leaf with the smallest encoding of `value`.
    ///
    /// The rules are the rules of LLVM `writeEncodedSignedInteger` and
    /// `writeEncodedUnsignedInteger`, extended to 128 bits: a value that is
    /// not negative uses an immediate value, `LF_USHORT`, `LF_ULONG`,
    /// `LF_UQUADWORD` or `LF_UOCTWORD`. A negative value uses `LF_CHAR`,
    /// `LF_SHORT`, `LF_LONG`, `LF_QUADWORD` or `LF_OCTWORD`.
    #[must_use]
    pub fn from_integer(value: Integer) -> Numeric<'static> {
        let kind = match value {
            Integer::Signed(signed) if signed < 0 => [
                IntegerKind::Char,
                IntegerKind::Short,
                IntegerKind::Long,
                IntegerKind::QuadWord,
            ]
            .into_iter()
            .find(|kind| signed >= kind.range().0)
            .unwrap_or(IntegerKind::OctWord),
            _ => {
                // A value that is not negative fits `u128`.
                let unsigned = value.to_u128().unwrap_or(0);
                [
                    IntegerKind::Immediate,
                    IntegerKind::UShort,
                    IntegerKind::ULong,
                    IntegerKind::UQuadWord,
                ]
                .into_iter()
                .find(|kind| unsigned <= kind.range().1)
                .unwrap_or(IntegerKind::UOctWord)
            }
        };
        let leaf = IntegerLeaf::new(kind, value).unwrap_or(IntegerLeaf {
            kind: IntegerKind::UOctWord,
            value,
        });
        Numeric::Integer(leaf)
    }

    /// Return the leaf tag, or the value itself for an immediate value.
    #[must_use]
    pub fn tag(&self) -> u16 {
        match self {
            Self::Integer(leaf) => leaf.kind.tag().unwrap_or_else(|| {
                // An immediate value is below 0x8000.
                leaf.value
                    .to_u128()
                    .and_then(|value| u16::try_from(value).ok())
                    .unwrap_or(0)
            }),
            Self::Real16(_) => LF_REAL16,
            Self::Real32(_) => LF_REAL32,
            Self::Real48(_) => LF_REAL48,
            Self::Real64(_) => LF_REAL64,
            Self::Real80(_) => LF_REAL80,
            Self::Real128(_) => LF_REAL128,
            Self::Complex32(_) => LF_COMPLEX32,
            Self::Complex64(_) => LF_COMPLEX64,
            Self::Complex80(_) => LF_COMPLEX80,
            Self::Complex128(_) => LF_COMPLEX128,
            Self::Decimal(_) => LF_DECIMAL,
            Self::Date(_) => LF_DATE,
            Self::VarString(_) => LF_VARSTRING,
            Self::Utf8String(_) => LF_UTF8STRING,
        }
    }

    /// Return the value of an integer leaf.
    #[must_use]
    pub const fn integer(&self) -> Option<Integer> {
        match self {
            Self::Integer(leaf) => Some(leaf.value),
            _ => None,
        }
    }

    /// Return the value of an `LF_REAL32` leaf.
    #[must_use]
    pub const fn real32(&self) -> Option<f32> {
        match self {
            Self::Real32(bytes) => Some(f32::from_le_bytes(*bytes)),
            _ => None,
        }
    }

    /// Return the value of an `LF_REAL64` leaf.
    #[must_use]
    pub const fn real64(&self) -> Option<f64> {
        match self {
            Self::Real64(bytes) => Some(f64::from_le_bytes(*bytes)),
            _ => None,
        }
    }

    /// Return the IEEE 754 binary16 bits of an `LF_REAL16` leaf.
    #[must_use]
    pub const fn real16_bits(&self) -> Option<u16> {
        match self {
            Self::Real16(bytes) => Some(u16::from_le_bytes(*bytes)),
            _ => None,
        }
    }

    /// Return the real part and the imaginary part of an `LF_COMPLEX32` leaf.
    #[must_use]
    pub fn complex32(&self) -> Option<(f32, f32)> {
        match self {
            Self::Complex32(bytes) => {
                let (real, imaginary) = bytes.split_at(4);
                Some((
                    f32::from_le_bytes(real.try_into().ok()?),
                    f32::from_le_bytes(imaginary.try_into().ok()?),
                ))
            }
            _ => None,
        }
    }

    /// Return the real part and the imaginary part of an `LF_COMPLEX64` leaf.
    #[must_use]
    pub fn complex64(&self) -> Option<(f64, f64)> {
        match self {
            Self::Complex64(bytes) => {
                let (real, imaginary) = bytes.split_at(8);
                Some((
                    f64::from_le_bytes(real.try_into().ok()?),
                    f64::from_le_bytes(imaginary.try_into().ok()?),
                ))
            }
            _ => None,
        }
    }

    /// Return the fields of an `LF_DECIMAL` leaf.
    #[must_use]
    pub fn decimal(&self) -> Option<Decimal> {
        let Self::Decimal(bytes) = self else {
            return None;
        };
        let mut fields = Fields::new(bytes, ErrorKind::UnexpectedEof, "");
        Some(Decimal {
            reserved: fields.u16().ok()?,
            scale: fields.u8().ok()?,
            sign: fields.u8().ok()?,
            high: fields.u32().ok()?,
            low: fields.u64().ok()?,
        })
    }

    /// Return the OLE automation date of an `LF_DATE` leaf: the days since
    /// 30 December 1899, with the time of day as the fraction.
    #[must_use]
    pub const fn date(&self) -> Option<f64> {
        match self {
            Self::Date(bytes) => Some(f64::from_le_bytes(*bytes)),
            _ => None,
        }
    }

    /// Return the string bytes of an `LF_VARSTRING` or `LF_UTF8STRING` leaf.
    #[must_use]
    pub const fn string(&self) -> Option<&'a [u8]> {
        match self {
            Self::VarString(bytes) | Self::Utf8String(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// Return the value bytes after the tag of a fixed-size leaf that is not
    /// an integer.
    #[must_use]
    pub const fn value_bytes(&self) -> Option<&[u8]> {
        Some(match self {
            Self::Real16(bytes) => bytes.as_slice(),
            Self::Real32(bytes) => bytes.as_slice(),
            Self::Real48(bytes) => bytes.as_slice(),
            Self::Real64(bytes) | Self::Complex32(bytes) | Self::Date(bytes) => bytes.as_slice(),
            Self::Real80(bytes) => bytes.as_slice(),
            Self::Real128(bytes) | Self::Complex64(bytes) | Self::Decimal(bytes) => {
                bytes.as_slice()
            }
            Self::Complex80(bytes) => bytes.as_slice(),
            Self::Complex128(bytes) => bytes.as_slice(),
            Self::Integer(_) | Self::VarString(_) | Self::Utf8String(_) => return None,
        })
    }

    /// Append the encoded leaf to `out`.
    ///
    /// The output is the input of [`Numeric::parse_prefix`] for a decoded
    /// leaf.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`] if `out` cannot grow, or if an
    ///   `LF_VARSTRING` value is longer than `u16::MAX` bytes.
    /// - [`ErrorKind::InvalidFormat`] if an `LF_UTF8STRING` value contains a
    ///   NUL byte.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut writer = super::writer::FieldWriter::new(out);
        writer.u16(self.tag())?;
        match self {
            Self::Integer(leaf) => {
                let width = leaf.kind.width();
                let bytes = match leaf.value {
                    Integer::Signed(value) => value.to_le_bytes(),
                    Integer::Unsigned(value) => value.to_le_bytes(),
                };
                writer.bytes(&bytes[..width])
            }
            Self::VarString(bytes) => {
                let count = u16::try_from(bytes.len()).map_err(|_| {
                    Error::new(ErrorKind::LimitExceeded, None)
                        .with_context("LF_VARSTRING value is longer than 65535 bytes")
                })?;
                writer.u16(count)?;
                writer.bytes(bytes)
            }
            Self::Utf8String(bytes) => writer.name(bytes),
            _ => writer.bytes(self.value_bytes().unwrap_or_default()),
        }
    }

    /// Return the number of bytes that [`Numeric::encode`] appends.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        2 + match self {
            Self::Integer(leaf) => leaf.kind.width(),
            Self::VarString(bytes) => 2 + bytes.len(),
            Self::Utf8String(bytes) => bytes.len() + 1,
            _ => self.value_bytes().map_or(0, <[u8]>::len),
        }
    }
}

impl From<IntegerLeaf> for Numeric<'_> {
    fn from(leaf: IntegerLeaf) -> Self {
        Self::Integer(leaf)
    }
}
