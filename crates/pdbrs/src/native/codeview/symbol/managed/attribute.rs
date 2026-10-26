//! Symbol attributes in `S_OEM` records and the `asyncMethodInfo` value.
//!
//! The Microsoft native symbol writer stores each
//! `ISymUnmanagedWriter::SetSymAttribute` value in an `S_OEM` record with the
//! OEM ID [`MANAGED_ATTRIBUTE_OEM_ID`](super::MANAGED_ATTRIBUTE_OEM_ID). The
//! user data after the type index is:
//!
//! 1. The attribute name: UTF-16LE code units and a NUL code unit.
//! 2. The attribute value, then the zero padding of the record.
//!
//! Roslyn writes two attributes:
//!
//! - `MD2`: custom debug information, see [`CustomDebugInfo`]. Roslyn calls
//!   `SetSymAttribute(0, "MD2", ...)` in `SymUnmanagedWriterImpl.DefineCustomMetadata`.
//! - `asyncMethodInfo`: the async stepping information of a `MoveNext` method,
//!   see [`AsyncMethodInfo`]. Roslyn calls `SetAsyncInfo`, and the native
//!   writer writes the value.
//!
//! No Microsoft document describes the user data. The layout comes from the
//! reader in Mono.Cecil (`symbols/pdb/Microsoft.Cci.Pdb/PdbFunction.cs`,
//! `PdbSynchronizationInformation`, commit
//! `882ca5eedda1e62eb41bd5869aeb15d8f1538e51`) and agrees with the fixtures.

use super::super::super::Fields;
use super::custom_debug_info::CustomDebugInfo;
use super::{SHORT_VALUE, count, invalid, split_utf16, zero_padding};
use crate::{ErrorKind, Result};

/// The name of the custom debug information attribute (`MD2`), as UTF-16LE bytes.
pub const MD2_ATTRIBUTE: &[u8] = b"M\0D\x002\0";

/// The name of the async method attribute (`asyncMethodInfo`), as UTF-16LE bytes.
pub const ASYNC_METHOD_INFO_ATTRIBUTE: &[u8] = b"a\0s\0y\0n\0c\0M\0e\0t\0h\0o\0d\0I\0n\0f\0o\0";

/// The size of one async step in bytes.
const STEP_SIZE: usize = 12;

/// A symbol attribute in the user data of an `S_OEM` record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct SymbolAttribute<'a> {
    /// The name as UTF-16LE bytes, without the NUL code unit. The bytes are
    /// not converted to text.
    pub name: &'a [u8],
    /// The value bytes, with the zero padding of the record.
    pub value: &'a [u8],
}

impl<'a> SymbolAttribute<'a> {
    /// Split the user data of an `S_OEM` record into the name and the value.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the data has no NUL code unit.
    /// The error has no location.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let (name, value) = split_utf16(data)
            .ok_or_else(|| invalid("symbol attribute name has no NUL code unit"))?;
        Ok(Self { name, value })
    }

    /// Return the UTF-16 code units of the name.
    pub fn name_units(&self) -> impl Iterator<Item = u16> + 'a {
        self.name
            .as_chunks::<2>()
            .0
            .iter()
            .map(|unit| u16::from_le_bytes(*unit))
    }

    /// Decode the value of the attributes that Roslyn writes.
    ///
    /// # Errors
    ///
    /// The errors of [`CustomDebugInfo::parse`] and [`AsyncMethodInfo::parse`].
    pub fn decode(&self) -> Result<AttributeValue<'a>> {
        Ok(match self.name {
            MD2_ATTRIBUTE => AttributeValue::CustomDebugInfo(CustomDebugInfo::parse(self.value)?),
            ASYNC_METHOD_INFO_ATTRIBUTE => {
                AttributeValue::AsyncMethodInfo(AsyncMethodInfo::parse(self.value)?)
            }
            _ => AttributeValue::Other(self.value),
        })
    }
}

/// The decoded value of a symbol attribute.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum AttributeValue<'a> {
    /// The `MD2` attribute.
    CustomDebugInfo(CustomDebugInfo<'a>),
    /// The `asyncMethodInfo` attribute.
    AsyncMethodInfo(AsyncMethodInfo<'a>),
    /// Another attribute. The value is raw, with the record padding.
    Other(&'a [u8]),
}

/// The async stepping information of a `MoveNext` method (`asyncMethodInfo`).
///
/// The layout is `u32` kickoff method token, `u32` catch handler IL offset,
/// `u32` step count and the steps. Roslyn writes the catch handler offset
/// only if it is not negative; the native writer then stores `0xffffffff`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct AsyncMethodInfo<'a> {
    /// The metadata token of the kickoff method: the `async` method that
    /// creates the state machine.
    pub kickoff_method: u32,
    /// The IL offset of the catch handler that the compiler generated, or
    /// `None` for `0xffffffff`.
    pub catch_handler_offset: Option<u32>,
    /// The steps.
    pub steps: AsyncSteps<'a>,
}

impl<'a> AsyncMethodInfo<'a> {
    /// Decode an `asyncMethodInfo` value.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the value is too short for its
    /// steps, or if bytes other than at most three zero padding bytes follow
    /// the steps. The error has no location.
    pub fn parse(value: &'a [u8]) -> Result<Self> {
        let mut fields = Fields::new(value, ErrorKind::InvalidFormat, SHORT_VALUE);
        let kickoff_method = fields.u32()?;
        let catch_handler_offset = Some(fields.u32()?).filter(|&offset| offset != u32::MAX);
        let len = count(fields.u32()?)?;
        let size = len
            .checked_mul(STEP_SIZE)
            .ok_or_else(|| invalid(SHORT_VALUE))?;
        let bytes = fields.bytes(size)?;
        zero_padding(fields.rest(), "asyncMethodInfo has bytes after its steps")?;
        Ok(Self {
            kickoff_method,
            catch_handler_offset,
            steps: AsyncSteps { bytes },
        })
    }
}

/// One await point of an async method.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct AsyncStep {
    /// The IL offset where the method yields.
    pub yield_offset: u32,
    /// The metadata token of the method that resumes: the `MoveNext` method.
    pub resume_method: u32,
    /// The IL offset where the method resumes.
    pub resume_offset: u32,
}

/// The steps of an [`AsyncMethodInfo`], 12 bytes each.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AsyncSteps<'a> {
    /// The step bytes. The length is a multiple of 12.
    bytes: &'a [u8],
}

impl<'a> AsyncSteps<'a> {
    /// Return the number of steps.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len() / STEP_SIZE
    }

    /// Return `true` if there are no steps.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Return an iterator over the steps.
    #[must_use]
    pub fn iter(&self) -> AsyncStepIter<'a> {
        AsyncStepIter::new(self.bytes)
    }
}

iterable!(AsyncSteps, AsyncStepIter);

chunk_iter!(
    /// An iterator over [`AsyncSteps`].
    AsyncStepIter,
    STEP_SIZE,
    AsyncStep,
    |step: &[u8; STEP_SIZE]| {
        let word = |at: usize| u32::from_le_bytes([step[at], step[at + 1], step[at + 2], step[at + 3]]);
        AsyncStep {
            yield_offset: word(0),
            resume_method: word(4),
            resume_offset: word(8),
        }
    }
);
