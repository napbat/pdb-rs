//! Custom debug information of state machine methods: async method stepping
//! information and hoisted local scopes.
//!
//! The layouts follow `SerializeAsyncMethodSteppingInfo` and
//! `SerializeStateMachineLocalScopes` in Roslyn `MetadataWriter.PortablePdb.cs`
//! (commit `e3c31f15b79e226656e3b082a921d4d60127b1fa`) and the "State Machine
//! Hoisted Local Scopes" section of the Portable PDB specification.

use alloc::vec::Vec;

use super::blob::{Reader, Writer, invalid, too_large, truncated};
use super::{Entries, count_to_end};
use crate::Result;
use crate::portable::metadata::{IlOffset, MethodDefId};

/// The first value that a hoisted scope offset or length cannot have.
const SCOPE_LIMIT: u32 = 0x8000_0000;

/// The size of one hoisted local scope.
const SCOPE_SIZE: usize = 8;

/// The async method stepping information of a `MoveNext` method.
///
/// The blob is a `u32` catch handler offset plus 1 (0 for no handler), and
/// then one step for each `await`, to the end of the blob.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AsyncMethodSteppingInformation<'a> {
    /// The catch handler IL offset.
    catch_handler_offset: Option<IlOffset>,
    /// The steps. [`AsyncMethodSteppingInformation::parse`] checked them.
    steps: &'a [u8],
    /// The number of steps.
    len: usize,
}

/// One `await` of an async method.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AsyncStep {
    /// The IL offset of the yield point in the `MoveNext` method.
    pub yield_offset: IlOffset,
    /// The IL offset of the resume point in [`AsyncStep::move_next_method`].
    pub resume_offset: IlOffset,
    /// The method that has the resume point. It is not nil.
    pub move_next_method: MethodDefId,
}

impl<'a> AsyncMethodSteppingInformation<'a> {
    /// Decode an async method stepping information blob.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`](crate::ErrorKind::UnexpectedEof) if the
    ///   blob ends inside a field.
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a
    ///   compressed integer is not valid or a method row ID is 0.
    pub fn parse(blob: &'a [u8]) -> Result<Self> {
        let mut reader = Reader::new(blob);
        let catch = reader.u32("async stepping catch handler offset is truncated")?;
        let steps = reader.rest();
        let len = count_to_end(steps, 0, read_step)?;
        Ok(Self {
            catch_handler_offset: catch.checked_sub(1).map(IlOffset::new),
            steps,
            len,
        })
    }

    /// Return the IL offset of the catch handler, if there is one.
    #[must_use]
    pub const fn catch_handler_offset(&self) -> Option<IlOffset> {
        self.catch_handler_offset
    }

    /// Return the number of steps.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Return `true` if there are no steps.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return an iterator over the steps.
    #[must_use]
    pub const fn iter(&self) -> Entries<'a, AsyncStep> {
        Entries::new(self.steps, self.len, 0, read_step)
    }

    /// Encode an async method stepping information blob.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if the
    ///   catch handler offset is `u32::MAX`, if a method row ID does not fit
    ///   in a compressed integer, or if an allocation fails.
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a
    ///   method is nil.
    pub fn encode(
        catch_handler_offset: Option<IlOffset>,
        steps: impl IntoIterator<Item = AsyncStep>,
    ) -> Result<Vec<u8>> {
        let catch = match catch_handler_offset {
            None => 0,
            Some(offset) => offset
                .get()
                .checked_add(1)
                .ok_or_else(|| too_large("async stepping catch handler offset is too large"))?,
        };
        let mut writer = Writer::new();
        writer.u32(catch)?;
        for step in steps {
            if step.move_next_method.is_nil() {
                return Err(invalid("async stepping method row ID is 0"));
            }
            writer.u32(step.yield_offset.get())?;
            writer.u32(step.resume_offset.get())?;
            writer.unsigned(step.move_next_method.get())?;
        }
        Ok(writer.finish())
    }
}

iterable!(AsyncMethodSteppingInformation => Entries<'a, AsyncStep>);

/// Read one async step.
fn read_step(reader: &mut Reader<'_>, _: i32) -> Result<AsyncStep> {
    const SHORT: &str = "async stepping step is truncated";
    let yield_offset = IlOffset::new(reader.u32(SHORT)?);
    let resume_offset = IlOffset::new(reader.u32(SHORT)?);
    let move_next_method = MethodDefId::new(reader.unsigned()?);
    if move_next_method.is_nil() {
        return Err(invalid("async stepping method row ID is 0"));
    }
    Ok(AsyncStep {
        yield_offset,
        resume_offset,
        move_next_method,
    })
}

/// The scopes of the local variables that a state machine hoists to fields,
/// one for each hoisted variable slot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HoistedLocalScopes<'a> {
    /// The 8-byte scopes. [`HoistedLocalScopes::parse`] checked them.
    bytes: &'a [u8],
}

/// The IL range of a hoisted local variable: `start..start + length`.
///
/// Roslyn writes `(0, 0)` for a slot without a scope, for example a
/// synthesized variable.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct HoistedLocalScope {
    /// The first IL offset of the scope, smaller than `0x80000000`.
    pub start: IlOffset,
    /// The length of the scope in bytes, smaller than `0x80000000`.
    pub length: u32,
}

impl<'a> HoistedLocalScopes<'a> {
    /// Decode a hoisted local scopes blob.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`](crate::ErrorKind::UnexpectedEof) if the
    ///   blob size is not a multiple of 8.
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a
    ///   start offset or a length is `0x80000000` or more.
    pub fn parse(blob: &'a [u8]) -> Result<Self> {
        if !blob.len().is_multiple_of(SCOPE_SIZE) {
            return Err(truncated("hoisted local scope is truncated"));
        }
        count_to_end(blob, 0, read_scope)?;
        Ok(Self { bytes: blob })
    }

    /// Return the blob bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Return the number of scopes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len() / SCOPE_SIZE
    }

    /// Return `true` if there are no scopes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Return an iterator over the scopes.
    #[must_use]
    pub const fn iter(&self) -> Entries<'a, HoistedLocalScope> {
        Entries::new(self.bytes, self.len(), 0, read_scope)
    }

    /// Encode a hoisted local scopes blob.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a
    ///   start offset or a length is `0x80000000` or more.
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if an
    ///   allocation fails.
    pub fn encode(scopes: impl IntoIterator<Item = HoistedLocalScope>) -> Result<Vec<u8>> {
        let mut writer = Writer::new();
        for scope in scopes {
            check_scope(scope)?;
            writer.u32(scope.start.get())?;
            writer.u32(scope.length)?;
        }
        Ok(writer.finish())
    }
}

iterable!(HoistedLocalScopes => Entries<'a, HoistedLocalScope>);

/// Read one hoisted local scope.
fn read_scope(reader: &mut Reader<'_>, _: i32) -> Result<HoistedLocalScope> {
    const SHORT: &str = "hoisted local scope is truncated";
    let scope = HoistedLocalScope {
        start: IlOffset::new(reader.u32(SHORT)?),
        length: reader.u32(SHORT)?,
    };
    check_scope(scope)?;
    Ok(scope)
}

/// Check the ranges of a hoisted local scope.
fn check_scope(scope: HoistedLocalScope) -> Result<()> {
    if scope.start.get() >= SCOPE_LIMIT || scope.length >= SCOPE_LIMIT {
        return Err(invalid("hoisted local scope offset or length is too large"));
    }
    Ok(())
}
