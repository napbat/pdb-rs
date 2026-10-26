//! Type records with fixed fields: modifiers, procedure types, bit fields
//! and arrays.

use super::{CallingConvention, FunctionOptions, ModifierOptions, fields};
use crate::Result;
use crate::native::codeview::index::TypeIndex;
use crate::native::codeview::numeric::Numeric;
use crate::native::output::Output;

/// An `LF_MODIFIER` record: a type with `const`, `volatile` or `__unaligned`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Modifier {
    /// The modified type.
    pub modified_type: TypeIndex,
    /// The modifiers.
    pub options: ModifierOptions,
}

impl Modifier {
    /// Decode the payload of an `LF_MODIFIER` record.
    pub(super) fn decode(data: &[u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            modified_type: fields.type_index()?,
            options: ModifierOptions(fields.u16()?),
        };
        fields.finish_type_padding()?;
        Ok(value)
    }

    /// Append the payload fields.
    pub(super) fn encode(self, writer: &mut Output<'_>) -> Result<()> {
        writer.type_index(self.modified_type)?;
        writer.u16(self.options.0)
    }
}

/// An `LF_PROCEDURE` record: the type of a function that is not a member.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Procedure {
    /// The return type.
    pub return_type: TypeIndex,
    /// The calling convention.
    pub calling_convention: CallingConvention,
    /// The function options.
    pub options: FunctionOptions,
    /// The number of parameters.
    pub parameter_count: u16,
    /// The `LF_ARGLIST` record with the parameter types.
    pub argument_list: TypeIndex,
}

impl Procedure {
    /// Make a record with `calling_convention`. The other fields are zero or
    /// empty.
    #[must_use]
    pub const fn new(calling_convention: CallingConvention) -> Self {
        Self {
            return_type: TypeIndex::NONE,
            calling_convention,
            options: FunctionOptions(0),
            parameter_count: 0,
            argument_list: TypeIndex::NONE,
        }
    }

    /// Decode the payload of an `LF_PROCEDURE` record.
    pub(super) fn decode(data: &[u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            return_type: fields.type_index()?,
            calling_convention: CallingConvention::from_raw(fields.u8()?)?,
            options: FunctionOptions(fields.u8()?),
            parameter_count: fields.u16()?,
            argument_list: fields.type_index()?,
        };
        fields.finish_type_padding()?;
        Ok(value)
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.type_index(self.return_type)?;
        writer.u8(self.calling_convention.raw())?;
        writer.u8(self.options.0)?;
        writer.u16(self.parameter_count)?;
        writer.type_index(self.argument_list)
    }
}

/// An `LF_MFUNCTION` record: the type of a member function.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct MemberFunction {
    /// The return type.
    pub return_type: TypeIndex,
    /// The class that contains the function.
    pub class_type: TypeIndex,
    /// The type of `this`, or [`TypeIndex::NONE`] for a static function.
    pub this_type: TypeIndex,
    /// The calling convention.
    pub calling_convention: CallingConvention,
    /// The function options.
    pub options: FunctionOptions,
    /// The number of parameters.
    pub parameter_count: u16,
    /// The `LF_ARGLIST` record with the parameter types.
    pub argument_list: TypeIndex,
    /// The adjustment that the function applies to `this`.
    pub this_adjustment: i32,
}

impl MemberFunction {
    /// Make a record with `calling_convention`. The other fields are zero or
    /// empty.
    #[must_use]
    pub const fn new(calling_convention: CallingConvention) -> Self {
        Self {
            return_type: TypeIndex::NONE,
            class_type: TypeIndex::NONE,
            this_type: TypeIndex::NONE,
            calling_convention,
            options: FunctionOptions(0),
            parameter_count: 0,
            argument_list: TypeIndex::NONE,
            this_adjustment: 0,
        }
    }

    /// Decode the payload of an `LF_MFUNCTION` record.
    pub(super) fn decode(data: &[u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            return_type: fields.type_index()?,
            class_type: fields.type_index()?,
            this_type: fields.type_index()?,
            calling_convention: CallingConvention::from_raw(fields.u8()?)?,
            options: FunctionOptions(fields.u8()?),
            parameter_count: fields.u16()?,
            argument_list: fields.type_index()?,
            this_adjustment: fields.i32()?,
        };
        fields.finish_type_padding()?;
        Ok(value)
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.type_index(self.return_type)?;
        writer.type_index(self.class_type)?;
        writer.type_index(self.this_type)?;
        writer.u8(self.calling_convention.raw())?;
        writer.u8(self.options.0)?;
        writer.u16(self.parameter_count)?;
        writer.type_index(self.argument_list)?;
        writer.i32(self.this_adjustment)
    }
}

/// An `LF_BITFIELD` record.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Bitfield {
    /// The type that holds the bits.
    pub underlying_type: TypeIndex,
    /// The number of bits.
    pub bit_size: u8,
    /// The position of the first bit.
    pub bit_offset: u8,
}

impl Bitfield {
    /// Decode the payload of an `LF_BITFIELD` record.
    pub(super) fn decode(data: &[u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            underlying_type: fields.type_index()?,
            bit_size: fields.u8()?,
            bit_offset: fields.u8()?,
        };
        fields.finish_type_padding()?;
        Ok(value)
    }

    /// Append the payload fields.
    pub(super) fn encode(self, writer: &mut Output<'_>) -> Result<()> {
        writer.type_index(self.underlying_type)?;
        writer.u8(self.bit_size)?;
        writer.u8(self.bit_offset)
    }
}

/// An `LF_ARRAY` record.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Array<'a> {
    /// The element type.
    pub element_type: TypeIndex,
    /// The type of the index.
    pub index_type: TypeIndex,
    /// The size of the array in bytes.
    pub size: Numeric<'a>,
    /// The name bytes. The name is usually empty.
    pub name: &'a [u8],
}

impl<'a> Array<'a> {
    /// Decode the payload of an `LF_ARRAY` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            element_type: fields.type_index()?,
            index_type: fields.type_index()?,
            size: fields.numeric()?,
            name: fields.name()?,
        };
        fields.finish_type_padding()?;
        Ok(value)
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.type_index(self.element_type)?;
        writer.type_index(self.index_type)?;
        self.size.encode(writer.out())?;
        writer.c_string(self.name)
    }
}
