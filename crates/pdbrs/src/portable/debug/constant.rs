//! The `LocalConstant` table: local constant signatures and the typed row.
//!
//! A signature is `CustomMod* (PrimitiveConstant | EnumConstant | GeneralConstant)`. The type
//! codes come from ECMA-335 II.23.1.16. Type handles are `TypeDefOrRefOrSpecEncoded` values of
//! external tables.
//!
//! The `GeneralValue` of a `CLASS` or `VALUETYPE` constant depends on the type: `System.Decimal`
//! has 13 bytes and `System.DateTime` has 8 bytes. The type is an external row, so the decoder
//! uses the length: no bytes is the default value, 8 bytes are `DateTime` ticks and 13 bytes
//! are a `Decimal`. Other lengths are not valid.

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::rows::{TypedRows, each_row};
use super::type_handle;
use crate::Result;
use crate::bytes::{Bytes, into_owned};
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::custom::blob::{Reader, Writer, invalid};
use crate::portable::metadata::{LocalConstantId, LocalConstantRow, MetadataToken, TableId};

/// `ELEMENT_TYPE_BOOLEAN`.
const BOOLEAN: u8 = 0x02;
/// `ELEMENT_TYPE_CHAR`.
const CHAR: u8 = 0x03;
/// `ELEMENT_TYPE_I1`.
const I1: u8 = 0x04;
/// `ELEMENT_TYPE_U1`.
const U1: u8 = 0x05;
/// `ELEMENT_TYPE_I2`.
const I2: u8 = 0x06;
/// `ELEMENT_TYPE_U2`.
const U2: u8 = 0x07;
/// `ELEMENT_TYPE_I4`.
const I4: u8 = 0x08;
/// `ELEMENT_TYPE_U4`.
const U4: u8 = 0x09;
/// `ELEMENT_TYPE_I8`.
const I8: u8 = 0x0a;
/// `ELEMENT_TYPE_U8`.
const U8: u8 = 0x0b;
/// `ELEMENT_TYPE_R4`.
const R4: u8 = 0x0c;
/// `ELEMENT_TYPE_R8`.
const R8: u8 = 0x0d;
/// `ELEMENT_TYPE_STRING`.
const STRING: u8 = 0x0e;
/// `ELEMENT_TYPE_VALUETYPE`.
const VALUETYPE: u8 = 0x11;
/// `ELEMENT_TYPE_CLASS`.
const CLASS: u8 = 0x12;
/// `ELEMENT_TYPE_OBJECT`.
const OBJECT: u8 = 0x1c;
/// `ELEMENT_TYPE_CMOD_REQD`.
const CMOD_REQD: u8 = 0x1f;
/// `ELEMENT_TYPE_CMOD_OPT`.
const CMOD_OPT: u8 = 0x20;

/// The byte of a null string constant.
const NULL_STRING: u8 = 0xff;
/// The size of a `Decimal` general value.
const DECIMAL_LEN: usize = 13;
/// The size of a `DateTime` general value.
const DATE_TIME_LEN: usize = 8;
/// The largest scale of a `Decimal`.
const MAX_DECIMAL_SCALE: u8 = 28;

/// A custom modifier (ECMA-335 II.23.2.7).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct CustomModifier {
    /// `true` for `CMOD_REQD`, `false` for `CMOD_OPT`.
    pub required: bool,
    /// The modifier type: a `TypeDef`, `TypeRef` or `TypeSpec` token.
    pub modifier: MetadataToken,
}

/// An iterator over the checked custom modifiers of a [`LocalConstantSignature`].
#[derive(Clone, Debug)]
pub struct CustomModifiers<'a> {
    reader: Reader<'a>,
}

impl Iterator for CustomModifiers<'_> {
    type Item = CustomModifier;

    fn next(&mut self) -> Option<CustomModifier> {
        // `LocalConstantSignature::parse` checked all modifiers.
        if self.reader.is_empty() {
            return None;
        }
        read_modifier(&mut self.reader).ok().flatten()
    }
}

impl FusedIterator for CustomModifiers<'_> {}

/// A primitive constant value.
#[derive(Clone, Debug, PartialEq)]
pub enum PrimitiveConstant<'a> {
    /// `BOOLEAN`: 0 is false and 1 is true.
    Boolean(bool),
    /// `CHAR`: a UTF-16 code unit.
    Char(u16),
    /// `I1`.
    I1(i8),
    /// `U1`.
    U1(u8),
    /// `I2`.
    I2(i16),
    /// `U2`.
    U2(u16),
    /// `I4`.
    I4(i32),
    /// `U4`.
    U4(u32),
    /// `I8`.
    I8(i64),
    /// `U8`.
    U8(u64),
    /// `R4`.
    R4(f32),
    /// `R8`.
    R8(f64),
    /// `STRING`: UTF-16LE bytes (an even count), or `None` for a null string.
    String(Option<Bytes<'a>>),
}

into_owned!(enum PrimitiveConstant {
    String;
    Boolean, Char, I1, U1, I2, U2, I4, U4, I8, U8, R4, R8;
});

impl PrimitiveConstant<'_> {
    /// Return the type code.
    #[must_use]
    pub const fn type_code(&self) -> u8 {
        match self {
            Self::Boolean(_) => BOOLEAN,
            Self::Char(_) => CHAR,
            Self::I1(_) => I1,
            Self::U1(_) => U1,
            Self::I2(_) => I2,
            Self::U2(_) => U2,
            Self::I4(_) => I4,
            Self::U4(_) => U4,
            Self::I8(_) => I8,
            Self::U8(_) => U8,
            Self::R4(_) => R4,
            Self::R8(_) => R8,
            Self::String(_) => STRING,
        }
    }

    /// Return `true` if the type code can be the underlying type of an enum: `BOOLEAN`, `CHAR`
    /// or an integer type.
    #[must_use]
    pub const fn is_enum_type(&self) -> bool {
        !matches!(self, Self::R4(_) | Self::R8(_) | Self::String(_))
    }
}

/// A `System.Decimal` value: sign, scale and a 96-bit integer.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Decimal {
    /// `true` for a negative value.
    pub negative: bool,
    /// The power of 10 that divides the integer (0 to 28).
    pub scale: u8,
    /// Bits 0 to 31 of the integer.
    pub low: u32,
    /// Bits 32 to 63 of the integer.
    pub mid: u32,
    /// Bits 64 to 95 of the integer.
    pub high: u32,
}

/// The `GeneralValue` of a `CLASS` or `VALUETYPE` constant.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GeneralValue {
    /// No value: the default value of the type, for example a null reference.
    Default,
    /// A `System.DateTime` value as ticks.
    DateTime(i64),
    /// A `System.Decimal` value.
    Decimal(Decimal),
}

/// The value of a local constant.
#[derive(Clone, Debug, PartialEq)]
pub enum ConstantValue<'a> {
    /// A primitive constant.
    Primitive(PrimitiveConstant<'a>),
    /// An enum constant: the underlying value and the enum type.
    Enum {
        /// The underlying value. Its type code is `BOOLEAN`, `CHAR` or an integer type.
        value: PrimitiveConstant<'a>,
        /// The enum type: a `TypeDef`, `TypeRef` or `TypeSpec` token.
        enum_type: MetadataToken,
    },
    /// A `CLASS` or `VALUETYPE` constant.
    General {
        /// `true` for `VALUETYPE`, `false` for `CLASS`.
        value_type: bool,
        /// The type: a `TypeDef`, `TypeRef` or `TypeSpec` token.
        constant_type: MetadataToken,
        /// The value.
        value: GeneralValue,
    },
    /// An `OBJECT` constant: a null reference.
    Object,
}

impl ConstantValue<'_> {
    /// Return a record that owns its bytes. Copy borrowed bytes.
    #[must_use]
    pub fn into_owned(self) -> ConstantValue<'static> {
        match self {
            Self::Primitive(value) => ConstantValue::Primitive(value.into_owned()),
            Self::Enum { value, enum_type } => ConstantValue::Enum {
                value: value.into_owned(),
                enum_type,
            },
            Self::General {
                value_type,
                constant_type,
                value,
            } => ConstantValue::General {
                value_type,
                constant_type,
                value,
            },
            Self::Object => ConstantValue::Object,
        }
    }
}

impl crate::bytes::IntoOwned for ConstantValue<'_> {
    type Owned = ConstantValue<'static>;

    fn to_static(self) -> Self::Owned {
        self.into_owned()
    }
}

/// A checked local constant signature. It borrows the blob bytes until
/// [`LocalConstantSignature::into_owned`] copies them.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalConstantSignature<'a> {
    modifiers: Bytes<'a>,
    value: ConstantValue<'a>,
}

into_owned!(struct LocalConstantSignature { modifiers, value; });

impl<'a> LocalConstantSignature<'a> {
    /// Decode and check a local constant signature.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`](crate::ErrorKind::UnexpectedEof) if the blob ends
    ///   inside a value.
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) for an unknown type
    ///   code, a type handle with tag 3 or row 0, a `BOOLEAN` value other than 0 and 1, an odd
    ///   string size, a general value of another size than 0, 8 or 13 bytes, a `Decimal`
    ///   scale above 28, or bytes after the value.
    pub fn parse(blob: &'a [u8]) -> Result<Self> {
        let mut reader = Reader::new(blob);
        let start = reader.rest();
        while read_modifier(&mut reader)?.is_some() {}
        let modifiers = &start[..start.len() - reader.rest().len()];
        let code = reader.u8("local constant signature has no type code")?;
        let value = match code {
            STRING => ConstantValue::Primitive(PrimitiveConstant::String(
                read_string(reader.rest())?.map(Bytes::new),
            )),
            BOOLEAN..=R8 => {
                let value = read_primitive(&mut reader, code)?;
                if reader.is_empty() {
                    ConstantValue::Primitive(value)
                } else if value.is_enum_type() {
                    let enum_type = type_handle::read(&mut reader)?;
                    end(&reader)?;
                    ConstantValue::Enum { value, enum_type }
                } else {
                    return Err(invalid(
                        "local constant signature has bytes after the value",
                    ));
                }
            }
            CLASS | VALUETYPE => {
                let constant_type = type_handle::read(&mut reader)?;
                ConstantValue::General {
                    value_type: code == VALUETYPE,
                    constant_type,
                    value: read_general(reader.rest())?,
                }
            }
            OBJECT => {
                end(&reader)?;
                ConstantValue::Object
            }
            _ => return Err(invalid("local constant signature has an unknown type code")),
        };
        Ok(Self {
            modifiers: Bytes::new(modifiers),
            value,
        })
    }

    /// Return the custom modifiers.
    #[must_use]
    pub fn modifiers(&self) -> CustomModifiers<'_> {
        CustomModifiers {
            reader: Reader::new(&self.modifiers),
        }
    }

    /// Return the value.
    #[must_use]
    pub const fn value(&self) -> &ConstantValue<'a> {
        &self.value
    }

    /// Encode a local constant signature in the form that Roslyn writes, and append it to
    /// `out`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) for a type token that
    ///   is not a non-nil `TypeDef`, `TypeRef` or `TypeSpec` token, an enum value of type `R4`,
    ///   `R8` or `STRING`, an odd string size, or a `Decimal` scale above 28.
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if a type row does not
    ///   fit.
    ///
    /// On error, `out` does not change.
    pub fn encode_parts(
        modifiers: &[CustomModifier],
        value: &ConstantValue<'_>,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        let mut writer = Writer::append(out);
        for modifier in modifiers {
            writer.u8(if modifier.required {
                CMOD_REQD
            } else {
                CMOD_OPT
            })?;
            type_handle::write(&mut writer, modifier.modifier)?;
        }
        match value {
            ConstantValue::Primitive(value) => write_primitive(&mut writer, value)?,
            ConstantValue::Enum { value, enum_type } => {
                if !value.is_enum_type() {
                    return Err(invalid("enum constant has a type code of R4, R8 or STRING"));
                }
                write_primitive(&mut writer, value)?;
                type_handle::write(&mut writer, *enum_type)?;
            }
            ConstantValue::General {
                value_type,
                constant_type,
                value,
            } => {
                writer.u8(if *value_type { VALUETYPE } else { CLASS })?;
                type_handle::write(&mut writer, *constant_type)?;
                write_general(&mut writer, value)?;
            }
            ConstantValue::Object => writer.u8(OBJECT)?,
        }
        writer.finish();
        Ok(())
    }
}

/// Read a custom modifier, or return `None` if the next byte is not a modifier.
fn read_modifier(reader: &mut Reader<'_>) -> Result<Option<CustomModifier>> {
    let required = match reader.rest().first() {
        Some(&CMOD_REQD) => true,
        Some(&CMOD_OPT) => false,
        _ => return Ok(None),
    };
    reader.u8("custom modifier")?;
    Ok(Some(CustomModifier {
        required,
        modifier: type_handle::read(reader)?,
    }))
}

/// Return an error if bytes remain after the value.
fn end(reader: &Reader<'_>) -> Result<()> {
    if reader.is_empty() {
        Ok(())
    } else {
        Err(invalid(
            "local constant signature has bytes after the value",
        ))
    }
}

/// Read the value of a primitive type code other than `STRING`.
fn read_primitive<'a>(reader: &mut Reader<'a>, code: u8) -> Result<PrimitiveConstant<'a>> {
    const CONTEXT: &str = "local constant value is truncated";
    Ok(match code {
        BOOLEAN => match reader.u8(CONTEXT)? {
            0 => PrimitiveConstant::Boolean(false),
            1 => PrimitiveConstant::Boolean(true),
            _ => return Err(invalid("BOOLEAN constant is not 0 or 1")),
        },
        CHAR => PrimitiveConstant::Char(reader.u16(CONTEXT)?),
        I1 => PrimitiveConstant::I1(reader.u8(CONTEXT)?.cast_signed()),
        U1 => PrimitiveConstant::U1(reader.u8(CONTEXT)?),
        I2 => PrimitiveConstant::I2(reader.u16(CONTEXT)?.cast_signed()),
        U2 => PrimitiveConstant::U2(reader.u16(CONTEXT)?),
        I4 => PrimitiveConstant::I4(reader.u32(CONTEXT)?.cast_signed()),
        U4 => PrimitiveConstant::U4(reader.u32(CONTEXT)?),
        I8 => PrimitiveConstant::I8(reader.u64(CONTEXT)?.cast_signed()),
        U8 => PrimitiveConstant::U8(reader.u64(CONTEXT)?),
        R4 => PrimitiveConstant::R4(f32::from_bits(reader.u32(CONTEXT)?)),
        _ => PrimitiveConstant::R8(f64::from_bits(reader.u64(CONTEXT)?)),
    })
}

/// Read a string value: the rest of the blob.
fn read_string(bytes: &[u8]) -> Result<Option<&[u8]>> {
    if bytes == [NULL_STRING] {
        return Ok(None);
    }
    if !bytes.len().is_multiple_of(2) {
        return Err(invalid("STRING constant has an odd size"));
    }
    Ok(Some(bytes))
}

/// Read a general value from its size.
fn read_general(bytes: &[u8]) -> Result<GeneralValue> {
    const CONTEXT: &str = "general value is truncated";
    let mut reader = Reader::new(bytes);
    match bytes.len() {
        0 => Ok(GeneralValue::Default),
        DATE_TIME_LEN => Ok(GeneralValue::DateTime(reader.u64(CONTEXT)?.cast_signed())),
        DECIMAL_LEN => {
            let flags = reader.u8(CONTEXT)?;
            let scale = flags & 0x7f;
            if scale > MAX_DECIMAL_SCALE {
                return Err(invalid("Decimal constant scale is larger than 28"));
            }
            Ok(GeneralValue::Decimal(Decimal {
                negative: flags & 0x80 != 0,
                scale,
                low: reader.u32(CONTEXT)?,
                mid: reader.u32(CONTEXT)?,
                high: reader.u32(CONTEXT)?,
            }))
        }
        _ => Err(invalid(
            "general value is not 0 bytes, 8 bytes (DateTime) or 13 bytes (Decimal)",
        )),
    }
}

/// Write a type code and a primitive value.
fn write_primitive(writer: &mut Writer<'_>, value: &PrimitiveConstant<'_>) -> Result<()> {
    writer.u8(value.type_code())?;
    match value {
        PrimitiveConstant::Boolean(value) => writer.u8(u8::from(*value)),
        PrimitiveConstant::Char(value) | PrimitiveConstant::U2(value) => writer.u16(*value),
        PrimitiveConstant::I1(value) => writer.u8(value.cast_unsigned()),
        PrimitiveConstant::U1(value) => writer.u8(*value),
        PrimitiveConstant::I2(value) => writer.u16(value.cast_unsigned()),
        PrimitiveConstant::I4(value) => writer.u32(value.cast_unsigned()),
        PrimitiveConstant::U4(value) => writer.u32(*value),
        PrimitiveConstant::I8(value) => writer.u64(value.cast_unsigned()),
        PrimitiveConstant::U8(value) => writer.u64(*value),
        PrimitiveConstant::R4(value) => writer.u32(value.to_bits()),
        PrimitiveConstant::R8(value) => writer.u64(value.to_bits()),
        PrimitiveConstant::String(None) => writer.u8(NULL_STRING),
        PrimitiveConstant::String(Some(bytes)) => {
            if !bytes.len().is_multiple_of(2) {
                return Err(invalid("STRING constant has an odd size"));
            }
            writer.bytes(bytes)
        }
    }
}

/// Write a general value.
fn write_general(writer: &mut Writer<'_>, value: &GeneralValue) -> Result<()> {
    match value {
        GeneralValue::Default => Ok(()),
        GeneralValue::DateTime(ticks) => writer.u64(ticks.cast_unsigned()),
        GeneralValue::Decimal(decimal) => {
            if decimal.scale > MAX_DECIMAL_SCALE {
                return Err(invalid("Decimal constant scale is larger than 28"));
            }
            writer.u8(decimal.scale | if decimal.negative { 0x80 } else { 0 })?;
            writer.u32(decimal.low)?;
            writer.u32(decimal.mid)?;
            writer.u32(decimal.high)
        }
    }
}

/// A local constant of a local scope: its name and its signature blob.
///
/// The signature stays encoded. [`LocalConstant::decode`] decodes it. Encode a new signature
/// with [`LocalConstantSignature::encode_parts`].
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct LocalConstant<'a> {
    /// The name. The bytes are UTF-8 in valid metadata.
    pub name: Bytes<'a>,
    /// The local constant signature blob.
    pub signature: Bytes<'a>,
}

into_owned!(struct LocalConstant { name, signature; });

impl LocalConstant<'_> {
    /// Decode the signature.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`LocalConstantSignature::parse`]. The errors have no location.
    pub fn decode(&self) -> Result<LocalConstantSignature<'_>> {
        LocalConstantSignature::parse(&self.signature)
    }
}

impl<S: ReadAt> Pdb<S> {
    /// Read a `LocalConstant` row with its name and signature blob.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::row`], [`Pdb::string`] and [`Pdb::blob`].
    pub fn local_constant(&self, id: LocalConstantId) -> Result<LocalConstant<'static>> {
        let row: LocalConstantRow = self.row(id)?;
        Ok(LocalConstant {
            name: self.string(row.name)?.into(),
            signature: self.blob(row.signature)?.into(),
        })
    }

    /// Read all `LocalConstant` rows in row order, each with its row ID.
    #[must_use]
    pub fn local_constants(&self) -> TypedRows<'_, S, (LocalConstantId, LocalConstant<'static>)> {
        each_row(self, TableId::LOCAL_CONSTANT, |pdb, row| {
            let id = LocalConstantId(row);
            pdb.local_constant(id).map(|constant| (id, constant))
        })
    }
}
