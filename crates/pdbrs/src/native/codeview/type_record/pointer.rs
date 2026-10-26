//! Pointer type records (`LF_POINTER`) and their based-pointer data.
//!
//! The layout follows `lfPointer` in `cvinfo.h`. After the attribute word,
//! a pointer to member has its containing class and representation. A
//! based pointer has data that depends on its kind (`lfPointer::pbase`):
//!
//! - `CV_PTR_BASE_SEG`: a `u16` segment.
//! - `CV_PTR_BASE_VAL`, `CV_PTR_BASE_SEGVAL`, `CV_PTR_BASE_ADDR` and
//!   `CV_PTR_BASE_SEGADDR`: a copy of the base symbol record, with its
//!   length field.
//! - `CV_PTR_BASE_TYPE`: a type index and a name.
//! - `CV_PTR_BASE_SELF`: no data.

use super::{MemberPointerRepresentation, PointerAttributes, PointerKind, fields};
use crate::native::codeview::index::TypeIndex;
use crate::native::codeview::{Fields, Record, Records};
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// The containing class of a pointer to member.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct MemberPointer {
    /// The class that contains the member.
    pub containing_type: TypeIndex,
    /// The representation of the pointer.
    pub representation: MemberPointerRepresentation,
}

impl MemberPointer {
    /// Make a record with `representation`. The other fields are zero or
    /// empty.
    #[must_use]
    pub const fn new(representation: MemberPointerRepresentation) -> Self {
        Self {
            containing_type: TypeIndex::NONE,
            representation,
        }
    }
}

/// The base of a based pointer (`lfPointer::pbase`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BasedPointer<'a> {
    /// `CV_PTR_BASE_SEG`: the pointer is based on a segment.
    Segment(u16),
    /// The pointer is based on a symbol: a copy of the complete symbol
    /// record, with its length field, its kind and its padding.
    ///
    /// The pointer kinds `CV_PTR_BASE_VAL`, `CV_PTR_BASE_SEGVAL`,
    /// `CV_PTR_BASE_ADDR` and `CV_PTR_BASE_SEGADDR` have this data.
    Symbol(&'a [u8]),
    /// `CV_PTR_BASE_TYPE`: the pointer is based on a type.
    Type {
        /// The base type.
        base_type: TypeIndex,
        /// The name of the base type.
        name: &'a [u8],
    },
    /// `CV_PTR_BASE_SELF`: the pointer is based on itself.
    SelfBased,
}

impl<'a> BasedPointer<'a> {
    /// Decode the base data of a pointer of `kind`.
    fn decode(kind: PointerKind, fields: &mut Fields<'a>) -> Result<Option<Self>> {
        Ok(Some(match kind {
            PointerKind::BasedOnSegment => Self::Segment(fields.u16()?),
            PointerKind::BasedOnValue
            | PointerKind::BasedOnSegmentValue
            | PointerKind::BasedOnAddress
            | PointerKind::BasedOnSegmentAddress => {
                let mut records = Records::new(fields.rest());
                let record = records.next().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidFormat, None)
                        .with_context("based pointer has no base symbol record")
                })??;
                Self::Symbol(fields.bytes(4 + record.data.len())?)
            }
            PointerKind::BasedOnType => Self::Type {
                base_type: fields.type_index()?,
                name: fields.name()?,
            },
            PointerKind::BasedOnSelf => Self::SelfBased,
            _ => return Ok(None),
        }))
    }

    /// Return the base symbol record of a [`BasedPointer::Symbol`] base.
    ///
    /// [`Record::offset`] is 0.
    #[must_use]
    pub fn symbol_record(&self) -> Option<Record<'a>> {
        match self {
            Self::Symbol(bytes) => Records::new(bytes).next()?.ok(),
            _ => None,
        }
    }

    /// Return `true` if this base is valid for a pointer of `kind`.
    const fn matches(&self, kind: PointerKind) -> bool {
        match self {
            Self::Segment(_) => matches!(kind, PointerKind::BasedOnSegment),
            Self::Symbol(_) => matches!(
                kind,
                PointerKind::BasedOnValue
                    | PointerKind::BasedOnSegmentValue
                    | PointerKind::BasedOnAddress
                    | PointerKind::BasedOnSegmentAddress
            ),
            Self::Type { .. } => matches!(kind, PointerKind::BasedOnType),
            Self::SelfBased => matches!(kind, PointerKind::BasedOnSelf),
        }
    }

    /// Append the base data.
    fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        match self {
            Self::Segment(segment) => writer.u16(*segment),
            Self::Symbol(bytes) => {
                let mut records = Records::new(bytes);
                let complete = matches!(records.next(), Some(Ok(_))) && records.next().is_none();
                if !complete {
                    return Err(Error::new(ErrorKind::InvalidFormat, None)
                        .with_context("based pointer symbol is not one complete record"));
                }
                writer.bytes(bytes)
            }
            Self::Type { base_type, name } => {
                writer.type_index(*base_type)?;
                writer.c_string(name)
            }
            Self::SelfBased => Ok(()),
        }
    }
}

/// An `LF_POINTER` record.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Pointer<'a> {
    /// The type that the pointer refers to.
    pub referent_type: TypeIndex,
    /// The kind, the mode, the size and the flags.
    pub attributes: PointerAttributes,
    /// The member-pointer information, present if
    /// [`PointerMode::is_member_pointer`](super::PointerMode::is_member_pointer)
    /// is `true` for the mode.
    pub member: Option<MemberPointer>,
    /// The base of a based pointer.
    ///
    /// This value is present if [`PointerKind::is_based`] is `true` for the
    /// kind and the mode is not a member-pointer mode.
    pub based: Option<BasedPointer<'a>>,
    /// The count of zero bytes after the last field, before the record
    /// padding.
    ///
    /// Most producers write none. The Microsoft macro assemblers (`ml64` and
    /// `ml` 14.51) write 6 zero bytes in each `LF_POINTER` record. LLVM, DIA
    /// and `pdbtool` ignore these bytes. The encoder writes them again.
    pub trailing_zeros: usize,
}

impl<'a> Pointer<'a> {
    /// Decode the payload of an `LF_POINTER` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let referent_type = fields.type_index()?;
        let attributes = PointerAttributes::from_raw(fields.u32()?)?;
        let mut member = None;
        let mut based = None;
        if attributes.mode().is_member_pointer() {
            member = Some(MemberPointer {
                containing_type: fields.type_index()?,
                representation: MemberPointerRepresentation::from_raw(fields.u16()?)?,
            });
        } else {
            based = BasedPointer::decode(attributes.kind(), &mut fields)?;
        }
        let trailing_zeros = fields.rest().iter().take_while(|&&byte| byte == 0).count();
        fields.bytes(trailing_zeros)?;
        fields.finish_type_padding()?;
        Ok(Self {
            referent_type,
            attributes,
            member,
            based,
            trailing_zeros,
        })
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        let member_mode = self.attributes.mode().is_member_pointer();
        let based_kind = !member_mode && self.attributes.kind().is_based();
        let valid = member_mode == self.member.is_some()
            && match self.based {
                Some(based) => based.matches(self.attributes.kind()) && !member_mode,
                None => !based_kind,
            };
        if !valid {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("pointer data does not match its kind and mode"));
        }
        writer.type_index(self.referent_type)?;
        writer.u32(self.attributes.raw())?;
        if let Some(member) = self.member {
            writer.type_index(member.containing_type)?;
            writer.u16(member.representation.raw())?;
        }
        if let Some(based) = self.based {
            based.encode(writer)?;
        }
        for _ in 0..self.trailing_zeros {
            writer.u8(0)?;
        }
        Ok(())
    }
}
