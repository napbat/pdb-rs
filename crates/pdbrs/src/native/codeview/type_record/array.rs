//! Array type records other than `LF_ARRAY`: dimensioned arrays, their
//! dimension records and strided arrays.
//!
//! The layouts follow `lfDimArray`, `lfDimCon`, `lfDimVar` and
//! `lfStridedArray` in `cvinfo.h`. The names are NUL-terminated, because
//! these leaf kinds are after `LF_ST_MAX`.

use core::iter::FusedIterator;

use super::{TypeIndexList, fields};
use crate::native::codeview::index::{PrimitiveMode, TypeIndex};
use crate::native::codeview::numeric::{Integer, Numeric};
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// An `LF_DIMARRAY` record: an array with a dimension record.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DimensionedArray<'a> {
    /// The element type.
    pub element_type: TypeIndex,
    /// The dimension record: `LF_DIMCONU`, `LF_DIMCONLU`, `LF_DIMVARU` or
    /// `LF_DIMVARLU`.
    pub dimensions: TypeIndex,
    /// The name bytes.
    pub name: &'a [u8],
}

impl<'a> DimensionedArray<'a> {
    /// Decode the payload of an `LF_DIMARRAY` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            element_type: fields.type_index()?,
            dimensions: fields.type_index()?,
            name: fields.name()?,
        };
        fields.finish_type_padding()?;
        Ok(value)
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.type_index(self.element_type)?;
        writer.type_index(self.dimensions)?;
        writer.c_string(self.name)
    }
}

/// The bounds of one dimension.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DimensionBound {
    /// The lower bound. `LF_DIMCONU` has no lower bounds.
    pub lower: Option<Integer>,
    /// The upper bound.
    pub upper: Integer,
}

impl DimensionBound {
    /// Make a record with `upper`. The other fields are zero or empty.
    #[must_use]
    pub const fn new(upper: Integer) -> Self {
        Self { lower: None, upper }
    }
}

/// An `LF_DIMCONU` or `LF_DIMCONLU` record: constant dimension bounds.
///
/// Each bound is a value of the index type. The index type must be a direct
/// primitive integer type, because the record has no other size for a bound.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ConstantDimensions<'a> {
    /// `true` for `LF_DIMCONLU`: each dimension has a lower and an upper bound.
    pub lower_bounds: bool,
    /// The type of the bounds.
    pub index_type: TypeIndex,
    /// The number of dimensions.
    pub rank: u16,
    /// The serialized bounds.
    bounds: &'a [u8],
    /// The size of one bound in bytes.
    width: usize,
    /// `true` if the bounds are signed.
    signed: bool,
}

/// Return the size and the signedness of a bound of `index_type`.
fn bound_layout(index_type: TypeIndex) -> Result<(usize, bool)> {
    let layout = index_type.primitive()?.and_then(|primitive| {
        let direct = primitive.mode == PrimitiveMode::Direct;
        let signed = primitive.kind.is_signed_integer()?;
        let size = primitive.kind.size()?;
        direct.then_some((usize::try_from(size).ok()?, signed))
    });
    layout.ok_or_else(|| {
        Error::new(ErrorKind::UnsupportedVersion, None)
            .with_context("dimension bound type is not a direct primitive integer type")
    })
}

impl<'a> ConstantDimensions<'a> {
    /// Make a record from its fields and the serialized bounds.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedVersion`] if `index_type` is not a direct
    ///   primitive integer type.
    /// - [`ErrorKind::InvalidFormat`] if `bounds` does not have one or two
    ///   bounds for each dimension.
    pub fn new(
        lower_bounds: bool,
        index_type: TypeIndex,
        rank: u16,
        bounds: &'a [u8],
    ) -> Result<Self> {
        let (width, signed) = bound_layout(index_type)?;
        let per_dimension = if lower_bounds { 2 } else { 1 };
        if bounds.len() != usize::from(rank) * per_dimension * width {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("dimension bounds do not match the rank"));
        }
        Ok(Self {
            lower_bounds,
            index_type,
            rank,
            bounds,
            width,
            signed,
        })
    }

    /// Decode the payload of an `LF_DIMCONU` or `LF_DIMCONLU` record.
    pub(super) fn decode(lower_bounds: bool, data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let index_type = fields.type_index()?;
        let rank = fields.u16()?;
        let (width, _) = bound_layout(index_type)?;
        let per_dimension = if lower_bounds { 2 } else { 1 };
        let bounds = fields.bytes(usize::from(rank) * per_dimension * width)?;
        fields.finish_type_padding()?;
        Self::new(lower_bounds, index_type, rank, bounds)
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.type_index(self.index_type)?;
        writer.u16(self.rank)?;
        writer.bytes(self.bounds)
    }

    /// Return the serialized bounds.
    #[must_use]
    pub const fn bound_bytes(&self) -> &'a [u8] {
        self.bounds
    }

    /// Return an iterator over the bounds of each dimension.
    #[must_use]
    pub fn bounds(&self) -> DimensionBounds<'a> {
        DimensionBounds {
            bytes: self.bounds,
            width: self.width,
            signed: self.signed,
            lower_bounds: self.lower_bounds,
        }
    }
}

/// An iterator over the bounds of a [`ConstantDimensions`] record.
#[derive(Clone, Debug)]
pub struct DimensionBounds<'a> {
    /// The bounds that are not read yet.
    bytes: &'a [u8],
    /// The size of one bound.
    width: usize,
    /// `true` if the bounds are signed.
    signed: bool,
    /// `true` if each dimension has a lower bound.
    lower_bounds: bool,
}

impl DimensionBounds<'_> {
    /// Read one bound.
    fn bound(&mut self) -> Option<Integer> {
        let (head, rest) = self.bytes.split_at_checked(self.width)?;
        self.bytes = rest;
        let fill = if self.signed && head.last().is_some_and(|byte| byte & 0x80 != 0) {
            0xff
        } else {
            0
        };
        let mut wide = [fill; 16];
        wide[..head.len()].copy_from_slice(head);
        Some(if self.signed {
            Integer::Signed(i128::from_le_bytes(wide))
        } else {
            Integer::Unsigned(u128::from_le_bytes(wide))
        })
    }
}

impl Iterator for DimensionBounds<'_> {
    type Item = DimensionBound;

    fn next(&mut self) -> Option<Self::Item> {
        let lower = if self.lower_bounds {
            Some(self.bound()?)
        } else {
            None
        };
        Some(DimensionBound {
            lower,
            upper: self.bound()?,
        })
    }
}

impl FusedIterator for DimensionBounds<'_> {}

/// An `LF_DIMVARU` or `LF_DIMVARLU` record: variable dimension bounds.
///
/// Each bound is a type index. `cvinfo.h` says that the referenced records
/// are `LF_REFSYM` records or `T_VOID`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct VariableDimensions<'a> {
    /// `true` for `LF_DIMVARLU`: each dimension has a lower and an upper bound.
    pub lower_bounds: bool,
    /// The number of dimensions.
    pub rank: u32,
    /// The type of the index.
    pub index_type: TypeIndex,
    /// The bound type indices: `rank` indices, or `2 * rank` with lower bounds.
    pub bounds: TypeIndexList<'a>,
}

impl<'a> VariableDimensions<'a> {
    /// Return the number of bound indices for `rank`.
    fn bound_count(lower_bounds: bool, rank: u32) -> Result<u32> {
        let per_dimension = if lower_bounds { 2 } else { 1 };
        rank.checked_mul(per_dimension).ok_or_else(|| {
            Error::new(ErrorKind::InvalidFormat, None)
                .with_context("variable dimension rank is too large")
        })
    }

    /// Decode the payload of an `LF_DIMVARU` or `LF_DIMVARLU` record.
    pub(super) fn decode(lower_bounds: bool, data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let rank = fields.u32()?;
        let index_type = fields.type_index()?;
        let count = Self::bound_count(lower_bounds, rank)?;
        let bounds = TypeIndexList::decode_count(&mut fields, count)?;
        fields.finish_type_padding()?;
        Ok(Self {
            lower_bounds,
            rank,
            index_type,
            bounds,
        })
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        if self.bounds.count()? != Self::bound_count(self.lower_bounds, self.rank)? {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("variable dimension bounds do not match the rank"));
        }
        writer.u32(self.rank)?;
        writer.type_index(self.index_type)?;
        writer.bytes(self.bounds.as_bytes())
    }
}

/// An `LF_STRIDED_ARRAY` record: an array with a stride between elements.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct StridedArray<'a> {
    /// The element type.
    pub element_type: TypeIndex,
    /// The type of the index.
    pub index_type: TypeIndex,
    /// The distance between the starts of two adjacent elements.
    pub stride: u32,
    /// The size of the array in bytes.
    pub size: Numeric<'a>,
    /// The name bytes.
    pub name: &'a [u8],
}

impl<'a> StridedArray<'a> {
    /// Decode the payload of an `LF_STRIDED_ARRAY` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            element_type: fields.type_index()?,
            index_type: fields.type_index()?,
            stride: fields.u32()?,
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
        writer.u32(self.stride)?;
        self.size.encode(writer.out())?;
        writer.c_string(self.name)
    }
}
