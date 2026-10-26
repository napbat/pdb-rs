//! Shader type records: `LF_HLSL`, `LF_MODIFIER_EX`, `LF_VECTOR` and
//! `LF_MATRIX`.
//!
//! The layouts follow `lfHLSL`, `lfModifierEx`, `lfVector` and `lfMatrix`
//! in `cvinfo.h`. `cvinfo.h` does not define the values of `CV_builtin_e`
//! and `CV_modifier_e`, so the records keep those values as raw numbers.

use core::iter::FusedIterator;

use super::fields;
use crate::native::codeview::index::TypeIndex;
use crate::native::codeview::numeric::Numeric;
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// The size of one modifier value of `LF_MODIFIER_EX`.
const MODIFIER_SIZE: usize = 2;

/// The bits of the property word that hold the number of properties.
const PROPERTY_COUNT_MASK: u16 = 0x000f;

/// An `LF_HLSL` record: a built-in HLSL type.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Hlsl<'a> {
    /// The sub-type, if any.
    pub subtype: TypeIndex,
    /// The built-in kind (`CV_builtin_e`).
    pub kind: u16,
    /// The raw property word: the number of properties in bits 0 to 3.
    ///
    /// `cvinfo.h` says that bits 4 to 15 must be zero, but the Windows SDK
    /// `fxc` compiler writes other values there (for example `0xfb6`). The
    /// decoder accepts and keeps these bits, so the encoder writes them
    /// again. [`Hlsl::unused_bits`] returns them.
    pub property_word: u16,
    /// The serialized numeric properties.
    properties: &'a [u8],
    /// The size of the type in bytes.
    pub size: Numeric<'a>,
}

impl<'a> Hlsl<'a> {
    /// Make a record from its fields and the serialized properties.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if `properties` does not hold
    /// exactly the number of numeric leaves in bits 0 to 3 of
    /// `property_word`.
    pub fn new(
        subtype: TypeIndex,
        kind: u16,
        property_word: u16,
        properties: &'a [u8],
        size: Numeric<'a>,
    ) -> Result<Self> {
        let mut rest = properties;
        for _ in 0..property_word & PROPERTY_COUNT_MASK {
            rest = Numeric::parse_prefix(rest)?.1;
        }
        if !rest.is_empty() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("HLSL properties do not match the property count"));
        }
        Ok(Self {
            subtype,
            kind,
            property_word,
            properties,
            size,
        })
    }

    /// Decode the payload of an `LF_HLSL` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let subtype = fields.type_index()?;
        let kind = fields.u16()?;
        let property_word = fields.u16()?;
        let start = fields.rest();
        for _ in 0..property_word & PROPERTY_COUNT_MASK {
            fields.numeric()?;
        }
        let properties = &start[..start.len() - fields.rest().len()];
        let size = fields.numeric()?;
        fields.finish_type_padding()?;
        Self::new(subtype, kind, property_word, properties, size)
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.type_index(self.subtype)?;
        writer.u16(self.kind)?;
        writer.u16(self.property_word)?;
        writer.bytes(self.properties)?;
        self.size.encode(writer.out())
    }

    /// Return an iterator over the numeric properties.
    #[must_use]
    pub const fn properties(&self) -> NumericList<'a> {
        NumericList {
            bytes: self.properties,
        }
    }

    /// Return the number of numeric properties: bits 0 to 3 of the
    /// property word.
    #[must_use]
    pub const fn property_count(&self) -> u16 {
        self.property_word & PROPERTY_COUNT_MASK
    }

    /// Return bits 4 to 15 of the property word, shifted to bit 0.
    ///
    /// `cvinfo.h` defines no meaning for these bits.
    #[must_use]
    pub const fn unused_bits(&self) -> u16 {
        self.property_word >> 4
    }
}

/// An iterator over a validated sequence of numeric leaves.
#[derive(Clone, Debug)]
pub struct NumericList<'a> {
    /// The leaves that are not read yet.
    bytes: &'a [u8],
}

impl<'a> Iterator for NumericList<'a> {
    type Item = Numeric<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        // The constructor validated the leaves.
        let (value, rest) = Numeric::parse_prefix(self.bytes).ok()?;
        self.bytes = rest;
        Some(value)
    }
}

impl FusedIterator for NumericList<'_> {}

/// An `LF_MODIFIER_EX` record: a type with a list of modifiers.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ModifierEx<'a> {
    /// The modified type.
    pub modified_type: TypeIndex,
    /// The serialized `u16` modifier values (`CV_modifier_e`).
    modifiers: &'a [u8],
}

impl<'a> ModifierEx<'a> {
    /// Make a record from serialized little-endian `u16` modifier values.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the length is odd, and
    /// [`ErrorKind::LimitExceeded`] for more than `u16::MAX` values.
    pub fn new(modified_type: TypeIndex, modifiers: &'a [u8]) -> Result<Self> {
        if !modifiers.len().is_multiple_of(MODIFIER_SIZE) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("modifier list has a partial value"));
        }
        if modifiers.len() / MODIFIER_SIZE > usize::from(u16::MAX) {
            return Err(Error::new(ErrorKind::LimitExceeded, None)
                .with_context("modifier list is longer than its count field allows"));
        }
        Ok(Self {
            modified_type,
            modifiers,
        })
    }

    /// Decode the payload of an `LF_MODIFIER_EX` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let modified_type = fields.type_index()?;
        let count = usize::from(fields.u16()?);
        let modifiers = fields.bytes(count * MODIFIER_SIZE)?;
        fields.finish_type_padding()?;
        Ok(Self {
            modified_type,
            modifiers,
        })
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        let count = u16::try_from(self.modifiers.len() / MODIFIER_SIZE).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("modifier list is longer than its count field allows")
        })?;
        writer.type_index(self.modified_type)?;
        writer.u16(count)?;
        writer.bytes(self.modifiers)
    }

    /// Return an iterator over the raw modifier values.
    pub fn modifiers(&self) -> impl ExactSizeIterator<Item = u16> + 'a {
        self.modifiers
            .as_chunks::<MODIFIER_SIZE>()
            .0
            .iter()
            .map(|chunk| u16::from_le_bytes(*chunk))
    }
}

/// An `LF_VECTOR` record: a vector of elements.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Vector<'a> {
    /// The element type.
    pub element_type: TypeIndex,
    /// The number of elements.
    pub count: u32,
    /// The size of the type in bytes.
    pub size: Numeric<'a>,
    /// The name bytes.
    pub name: &'a [u8],
}

impl<'a> Vector<'a> {
    /// Decode the payload of an `LF_VECTOR` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            element_type: fields.type_index()?,
            count: fields.u32()?,
            size: fields.numeric()?,
            name: fields.name()?,
        };
        fields.finish_type_padding()?;
        Ok(value)
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.type_index(self.element_type)?;
        writer.u32(self.count)?;
        self.size.encode(writer.out())?;
        writer.c_string(self.name)
    }
}

/// An `LF_MATRIX` record: a matrix of elements.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Matrix<'a> {
    /// The element type.
    pub element_type: TypeIndex,
    /// The number of rows.
    pub rows: u32,
    /// The number of columns.
    pub columns: u32,
    /// The distance in bytes between the starts of two major rows or columns.
    pub major_stride: u32,
    /// The raw attribute byte (`CV_matrixattr_t`): bit 0 is the row-major flag.
    pub attributes: u8,
    /// The size of the type in bytes.
    pub size: Numeric<'a>,
    /// The name bytes.
    pub name: &'a [u8],
}

impl<'a> Matrix<'a> {
    /// The attribute bit of a row-major matrix.
    pub const ROW_MAJOR: u8 = 0x01;

    /// Return `true` if the matrix has a row-major layout.
    #[must_use]
    pub const fn is_row_major(&self) -> bool {
        self.attributes & Self::ROW_MAJOR != 0
    }

    /// Decode the payload of an `LF_MATRIX` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            element_type: fields.type_index()?,
            rows: fields.u32()?,
            columns: fields.u32()?,
            major_stride: fields.u32()?,
            attributes: fields.u8()?,
            size: fields.numeric()?,
            name: fields.name()?,
        };
        fields.finish_type_padding()?;
        Ok(value)
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.type_index(self.element_type)?;
        writer.u32(self.rows)?;
        writer.u32(self.columns)?;
        writer.u32(self.major_stride)?;
        writer.u8(self.attributes)?;
        self.size.encode(writer.out())?;
        writer.c_string(self.name)
    }
}
