//! Roslyn custom debug information: the value of the `MD2` symbol attribute.
//!
//! The layout follows `CustomDebugInfoReader.cs`, `CustomDebugInfoEncoder.cs`,
//! `CustomDebugInfoKind.cs` and `CustomDebugInfoConstants.cs` in
//! `src/Dependencies/CodeAnalysis.Debugging/` of Roslyn (commit
//! `e3c31f15b79e226656e3b082a921d4d60127b1fa`):
//!
//! 1. A 4-byte global header: version 4, the record count and two reserved bytes.
//! 2. The records. Each record has an 8-byte header: version, kind, a reserved
//!    byte, the alignment size and the `u32` record size with the header.
//!
//! The encoder pads each record to four bytes. It writes the number of padding
//! bytes (the alignment size) only for kinds after `DynamicLocals` (5), and 0
//! for the other kinds. This decoder removes the padding of the kinds after 5
//! from [`CustomDebugInfoRecord::data`]. For the other kinds, the typed decoders
//! accept at most three zero bytes after the fields.
//!
//! The Roslyn reader removes the padding only for kinds 6, 7 and 8. For kind 9
//! (`EditAndContinueStateMachineStateMap`) it keeps the padding bytes in the
//! data, and `Pdb2Pdb` copies them into the Portable PDB. This decoder follows
//! the encoder, so its kind 9 data has no padding bytes.
//!
//! The Edit and Continue records (kinds 6, 7 and 9) use the blob formats of
//! the Portable PDB custom debug information. They decode through the shared
//! codecs in [`crate::portable::custom`].

mod locals;

use core::iter::FusedIterator;

pub use locals::{
    DynamicLocal, DynamicLocals, DynamicLocalsIter, TupleElementNames, TupleElementNamesEntry,
    TupleElementNamesIter, Utf8Names, Utf8NamesIter,
};

use super::super::super::Fields;
use super::{SHORT_VALUE, count, invalid, split_utf16, zero_padding};
use crate::portable::custom::{EncLambdaAndClosureMap, EncLocalSlotMap, EncStateMachineStateMap};
use crate::portable::metadata::{IlOffset, MetadataToken};
use crate::{Error, ErrorKind, Result};

values! {
    /// The kind of a custom debug information record (`CustomDebugInfoKind` in Roslyn).
    CustomDebugInfoKind(u8) {
        /// The sizes of the using groups of the method (C#).
        USING_GROUPS = 0;
        /// The token of the method that has the using information (C#).
        FORWARD_METHOD = 1;
        /// The token of the method that has the module information (C#).
        FORWARD_MODULE = 2;
        /// The scopes of hoisted local variables of a state machine (C#).
        STATE_MACHINE_HOISTED_LOCAL_SCOPES = 3;
        /// The name of the state machine type of a kickoff method.
        STATE_MACHINE_TYPE_NAME = 4;
        /// The `dynamic` flags of local variables and constants (C#).
        DYNAMIC_LOCALS = 5;
        /// The edit-and-continue local slot map.
        EDIT_AND_CONTINUE_LOCAL_SLOT_MAP = 6;
        /// The edit-and-continue lambda and closure map.
        EDIT_AND_CONTINUE_LAMBDA_MAP = 7;
        /// The tuple element names of local variables and constants.
        TUPLE_ELEMENT_NAMES = 8;
        /// The edit-and-continue state machine state map.
        EDIT_AND_CONTINUE_STATE_MACHINE_STATE_MAP = 9;
    }
}

/// The only supported version of the global header and of the records.
pub const CUSTOM_DEBUG_INFO_VERSION: u8 = 4;

/// The size of a record header in bytes.
const RECORD_HEADER_SIZE: usize = 8;

/// The last kind without an alignment size.
const LAST_UNALIGNED_KIND: u8 = 5;

/// Decoded custom debug information (`MD2`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct CustomDebugInfo<'a> {
    /// The two reserved bytes of the global header, raw. Roslyn writes zeros.
    pub reserved: [u8; 2],
    /// The framed records.
    records: &'a [u8],
    /// The record count.
    count: u8,
}

impl<'a> CustomDebugInfo<'a> {
    /// Decode the global header and check the frame of each record.
    ///
    /// The value must hold exactly the record count of records. At most three
    /// zero padding bytes can follow them.
    ///
    /// # Errors
    ///
    /// The errors have no location.
    ///
    /// - [`ErrorKind::UnsupportedVersion`] if the global version is not 4.
    /// - [`ErrorKind::InvalidFormat`] if the header or a record is shorter than
    ///   its fields, if a record size is smaller than 8 or past the end, if an
    ///   alignment size is not valid or its bytes are not zero, or if other
    ///   bytes follow the records.
    pub fn parse(value: &'a [u8]) -> Result<Self> {
        let mut fields = Fields::new(value, ErrorKind::InvalidFormat, SHORT_VALUE);
        let version = fields.u8()?;
        if version != CUSTOM_DEBUG_INFO_VERSION {
            return Err(Error::new(ErrorKind::UnsupportedVersion, None)
                .with_context("custom debug information version is not 4"));
        }
        let count = fields.u8()?;
        let reserved = [fields.u8()?, fields.u8()?];
        let start = fields.rest();
        let mut rest = start;
        for _ in 0..count {
            rest = CustomDebugInfoRecord::frame(rest)?.1;
        }
        zero_padding(rest, "bytes follow the custom debug information records")?;
        Ok(Self {
            reserved,
            records: &start[..start.len() - rest.len()],
            count,
        })
    }

    /// Return the number of records.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count as usize
    }

    /// Return `true` if there are no records.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Return an iterator over the records.
    #[must_use]
    pub const fn records(&self) -> CustomDebugInfoRecords<'a> {
        CustomDebugInfoRecords {
            bytes: self.records,
        }
    }
}

/// One framed custom debug information record.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct CustomDebugInfoRecord<'a> {
    /// The record version. [`CustomDebugInfoRecord::decode`] supports version 4.
    pub version: u8,
    /// The kind.
    pub kind: CustomDebugInfoKind,
    /// The reserved byte of the header, raw. Roslyn writes 0.
    pub reserved: u8,
    /// The alignment size: the number of padding bytes at the end of the record.
    pub alignment: u8,
    /// The record data after the header. For kinds after 5, the alignment
    /// bytes are removed.
    pub data: &'a [u8],
}

impl<'a> CustomDebugInfoRecord<'a> {
    /// Frame the record at the start of `bytes` and return it with the rest.
    fn frame(bytes: &'a [u8]) -> Result<(Self, &'a [u8])> {
        let (header, rest) = bytes
            .split_first_chunk::<RECORD_HEADER_SIZE>()
            .ok_or_else(|| invalid("custom debug information record header is truncated"))?;
        let [version, kind, reserved, alignment, size @ ..] = *header;
        let body_size = usize::try_from(u32::from_le_bytes(size))
            .ok()
            .and_then(|size| size.checked_sub(RECORD_HEADER_SIZE))
            .ok_or_else(|| invalid("custom debug information record size is below 8"))?;
        if body_size > rest.len() {
            return Err(invalid(
                "custom debug information record ends past the value",
            ));
        }
        let (body, rest) = rest.split_at(body_size);
        let data = if kind > LAST_UNALIGNED_KIND {
            let alignment = usize::from(alignment);
            if alignment > 3 || alignment > body.len() {
                return Err(invalid(
                    "custom debug information alignment size is not valid",
                ));
            }
            let (data, padding) = body.split_at(body.len() - alignment);
            if padding.iter().any(|&byte| byte != 0) {
                return Err(invalid(
                    "custom debug information alignment bytes are not zero",
                ));
            }
            data
        } else if alignment == 0 {
            body
        } else {
            return Err(invalid(
                "custom debug information alignment size is set for an old kind",
            ));
        };
        let record = Self {
            version,
            kind: CustomDebugInfoKind(kind),
            reserved,
            alignment,
            data,
        };
        Ok((record, rest))
    }

    /// Decode the data of the record.
    ///
    /// # Errors
    ///
    /// The errors have no location.
    ///
    /// - [`ErrorKind::UnsupportedVersion`] if the record version is not 4.
    /// - [`ErrorKind::InvalidFormat`] if the data is shorter than its fields,
    ///   has a negative count, or has bytes other than at most three zero
    ///   padding bytes after its fields.
    /// - The errors of [`EncLocalSlotMap::parse`],
    ///   [`EncLambdaAndClosureMap::parse`] and [`EncStateMachineStateMap::parse`]
    ///   for the Edit and Continue kinds.
    pub fn decode(&self) -> Result<CustomDebugInfoData<'a>> {
        if self.version != CUSTOM_DEBUG_INFO_VERSION {
            return Err(Error::new(ErrorKind::UnsupportedVersion, None)
                .with_context("custom debug information record version is not 4"));
        }
        let data = self.data;
        Ok(match self.kind {
            CustomDebugInfoKind::USING_GROUPS => {
                CustomDebugInfoData::UsingGroups(UsingGroups::parse(data)?)
            }
            CustomDebugInfoKind::FORWARD_METHOD => CustomDebugInfoData::ForwardMethod(token(data)?),
            CustomDebugInfoKind::FORWARD_MODULE => CustomDebugInfoData::ForwardModule(token(data)?),
            CustomDebugInfoKind::STATE_MACHINE_HOISTED_LOCAL_SCOPES => {
                CustomDebugInfoData::StateMachineHoistedLocalScopes(HoistedLocalScopes::parse(
                    data,
                )?)
            }
            CustomDebugInfoKind::STATE_MACHINE_TYPE_NAME => {
                CustomDebugInfoData::StateMachineTypeName(type_name(data)?)
            }
            CustomDebugInfoKind::DYNAMIC_LOCALS => {
                CustomDebugInfoData::DynamicLocals(DynamicLocals::parse(data)?)
            }
            CustomDebugInfoKind::EDIT_AND_CONTINUE_LOCAL_SLOT_MAP => {
                CustomDebugInfoData::EditAndContinueLocalSlotMap(EncLocalSlotMap::parse(data)?)
            }
            CustomDebugInfoKind::EDIT_AND_CONTINUE_LAMBDA_MAP => {
                CustomDebugInfoData::EditAndContinueLambdaMap(EncLambdaAndClosureMap::parse(data)?)
            }
            CustomDebugInfoKind::TUPLE_ELEMENT_NAMES => {
                CustomDebugInfoData::TupleElementNames(TupleElementNames::parse(data)?)
            }
            CustomDebugInfoKind::EDIT_AND_CONTINUE_STATE_MACHINE_STATE_MAP => {
                CustomDebugInfoData::EditAndContinueStateMachineStateMap(
                    EncStateMachineStateMap::parse(data)?,
                )
            }
            _ => CustomDebugInfoData::Unknown(data),
        })
    }
}

/// An iterator over the records of a [`CustomDebugInfo`].
#[derive(Clone, Debug)]
pub struct CustomDebugInfoRecords<'a> {
    /// The remaining framed records. [`CustomDebugInfo::parse`] checked them.
    bytes: &'a [u8],
}

impl<'a> Iterator for CustomDebugInfoRecords<'a> {
    type Item = CustomDebugInfoRecord<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let (record, rest) = CustomDebugInfoRecord::frame(self.bytes).ok()?;
        self.bytes = rest;
        Some(record)
    }
}

impl FusedIterator for CustomDebugInfoRecords<'_> {}

/// The typed data of a custom debug information record.
///
/// The Edit and Continue records use the codecs of the Portable PDB custom
/// debug information.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum CustomDebugInfoData<'a> {
    /// [`CustomDebugInfoKind::USING_GROUPS`].
    UsingGroups(UsingGroups<'a>),
    /// [`CustomDebugInfoKind::FORWARD_METHOD`]: a method token.
    ForwardMethod(MetadataToken),
    /// [`CustomDebugInfoKind::FORWARD_MODULE`]: a method token.
    ForwardModule(MetadataToken),
    /// [`CustomDebugInfoKind::STATE_MACHINE_HOISTED_LOCAL_SCOPES`].
    StateMachineHoistedLocalScopes(HoistedLocalScopes<'a>),
    /// [`CustomDebugInfoKind::STATE_MACHINE_TYPE_NAME`]: UTF-16LE bytes
    /// without the NUL code unit.
    StateMachineTypeName(&'a [u8]),
    /// [`CustomDebugInfoKind::DYNAMIC_LOCALS`].
    DynamicLocals(DynamicLocals<'a>),
    /// [`CustomDebugInfoKind::EDIT_AND_CONTINUE_LOCAL_SLOT_MAP`].
    EditAndContinueLocalSlotMap(EncLocalSlotMap<'a>),
    /// [`CustomDebugInfoKind::EDIT_AND_CONTINUE_LAMBDA_MAP`].
    EditAndContinueLambdaMap(EncLambdaAndClosureMap<'a>),
    /// [`CustomDebugInfoKind::TUPLE_ELEMENT_NAMES`].
    TupleElementNames(TupleElementNames<'a>),
    /// [`CustomDebugInfoKind::EDIT_AND_CONTINUE_STATE_MACHINE_STATE_MAP`].
    EditAndContinueStateMachineStateMap(EncStateMachineStateMap<'a>),
    /// A kind that this crate does not decode, raw.
    Unknown(&'a [u8]),
}

/// Read a method token and the zero padding.
fn token(data: &[u8]) -> Result<MetadataToken> {
    let mut fields = Fields::new(data, ErrorKind::InvalidFormat, SHORT_VALUE);
    let token = fields.u32()?;
    zero_padding(fields.rest(), "bytes follow the forward method token")?;
    Ok(MetadataToken::from_raw(token))
}

/// Read a state machine type name. Old compilers omit the NUL code unit.
fn type_name(data: &[u8]) -> Result<&[u8]> {
    match split_utf16(data) {
        Some((name, rest)) => {
            zero_padding(rest, "bytes follow the state machine type name")?;
            Ok(name)
        }
        None if data.len().is_multiple_of(2) => Ok(data),
        None => Err(invalid("state machine type name has an odd size")),
    }
}

/// The using group sizes of a method, innermost namespace first.
///
/// Each value is the number of `S_UNAMESPACE` records of one namespace
/// declaration, from the innermost declaration to the global namespace.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct UsingGroups<'a> {
    /// The `u16` sizes.
    bytes: &'a [u8],
}

impl<'a> UsingGroups<'a> {
    /// Read a `u16` count, the sizes and the zero padding.
    fn parse(data: &'a [u8]) -> Result<Self> {
        let mut fields = Fields::new(data, ErrorKind::InvalidFormat, SHORT_VALUE);
        let len = usize::from(fields.u16()?);
        let bytes = fields.bytes(len * 2)?;
        zero_padding(fields.rest(), "bytes follow the using group sizes")?;
        Ok(Self { bytes })
    }

    /// Return the number of groups.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len() / 2
    }

    /// Return `true` if there are no groups.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Return an iterator over the group sizes.
    #[must_use]
    pub fn iter(&self) -> UsingGroupsIter<'a> {
        UsingGroupsIter::new(self.bytes)
    }
}

iterable!(UsingGroups, UsingGroupsIter);

chunk_iter!(
    /// An iterator over [`UsingGroups`].
    UsingGroupsIter,
    2,
    u16,
    |size: &[u8; 2]| u16::from_le_bytes(*size)
);

/// The scope of a hoisted local variable, as stored.
///
/// Both IL offsets are inclusive. The scope `[0, 0]` marks a synthesized
/// variable (Roslyn `DecodeStateMachineHoistedLocalScopesRecord`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct HoistedLocalScope {
    /// The first IL offset.
    pub start: IlOffset,
    /// The last IL offset (inclusive).
    pub end: IlOffset,
}

/// The hoisted local scopes of a state machine method, one for each hoisted
/// variable slot.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct HoistedLocalScopes<'a> {
    /// The 8-byte scopes.
    bytes: &'a [u8],
}

impl<'a> HoistedLocalScopes<'a> {
    /// Read an `i32` count, the scopes and the zero padding.
    fn parse(data: &'a [u8]) -> Result<Self> {
        let mut fields = Fields::new(data, ErrorKind::InvalidFormat, SHORT_VALUE);
        let len = count(fields.u32()?)?;
        let size = len.checked_mul(8).ok_or_else(|| invalid(SHORT_VALUE))?;
        let bytes = fields.bytes(size)?;
        zero_padding(fields.rest(), "bytes follow the hoisted local scopes")?;
        Ok(Self { bytes })
    }

    /// Return the number of scopes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len() / 8
    }

    /// Return `true` if there are no scopes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Return an iterator over the scopes.
    #[must_use]
    pub fn iter(&self) -> HoistedLocalScopesIter<'a> {
        HoistedLocalScopesIter::new(self.bytes)
    }
}

iterable!(HoistedLocalScopes, HoistedLocalScopesIter);

chunk_iter!(
    /// An iterator over [`HoistedLocalScopes`].
    HoistedLocalScopesIter,
    8,
    HoistedLocalScope,
    |scope: &[u8; 8]| HoistedLocalScope {
        start: IlOffset::new(u32::from_le_bytes([scope[0], scope[1], scope[2], scope[3]])),
        end: IlOffset::new(u32::from_le_bytes([scope[4], scope[5], scope[6], scope[7]])),
    }
);
