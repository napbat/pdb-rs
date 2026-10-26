//! Custom debug information of local variables and constants: dynamic flags
//! and tuple element names.
//!
//! The layouts follow the "Dynamic Local Variables" section of the Portable
//! PDB specification and `SerializeBitVector` and `SerializeTupleElementNames`
//! in Roslyn `MetadataWriter.PortablePdb.cs` (commit
//! `e3c31f15b79e226656e3b082a921d4d60127b1fa`).

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::blob::{Reader, Writer, too_large};
use super::{Entries, Utf8Bytes, count_to_end};
use crate::Result;

/// The `dynamic` flags of a local variable or constant.
///
/// Each flag is one bit, for one `System.Object` in the signature of the type.
/// A set flag means that the source code used `dynamic` there. Bit 0 of the
/// first byte is the first flag. The blob can omit trailing zero bytes, so a
/// flag after the end of the blob is clear.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DynamicLocalVariables<'a> {
    /// The flag bytes.
    bytes: &'a [u8],
}

impl<'a> DynamicLocalVariables<'a> {
    /// Decode a dynamic local variables blob. All byte sequences are valid.
    #[must_use]
    pub const fn parse(blob: &'a [u8]) -> Self {
        Self { bytes: blob }
    }

    /// Return the blob bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Return the number of stored flags: 8 for each byte.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len() * 8
    }

    /// Return `true` if the blob is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Return flag `index`. A flag after the end of the blob is clear.
    #[must_use]
    pub fn flag(&self, index: usize) -> bool {
        self.bytes
            .get(index / 8)
            .is_some_and(|byte| byte & (1 << (index % 8)) != 0)
    }

    /// Return an iterator over the stored flags.
    #[must_use]
    pub const fn iter(&self) -> DynamicLocalVariablesIter<'a> {
        DynamicLocalVariablesIter {
            flags: *self,
            front: 0,
            back: self.len(),
        }
    }

    /// Encode a dynamic local variables blob as Roslyn does, and append it to
    /// `out`. The encoder omits trailing zero bytes.
    ///
    /// # Errors
    ///
    /// Return [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if
    /// an allocation fails. On error, `out` does not change.
    pub fn encode_parts(flags: impl IntoIterator<Item = bool>, out: &mut Vec<u8>) -> Result<()> {
        let start = out.len();
        let mut used = start;
        for (index, flag) in flags.into_iter().enumerate() {
            let at = start + index / 8;
            if at == out.len() {
                if out.try_reserve(1).is_err() {
                    out.truncate(start);
                    return Err(too_large("custom debug information blob allocation failed"));
                }
                out.push(0);
            }
            if flag {
                out[at] |= 1 << (index % 8);
                used = at + 1;
            }
        }
        out.truncate(used);
        Ok(())
    }
}

iterable!(@into DynamicLocalVariables => DynamicLocalVariablesIter, bool);

/// An iterator over [`DynamicLocalVariables`].
#[derive(Clone, Debug)]
pub struct DynamicLocalVariablesIter<'a> {
    /// The flags.
    flags: DynamicLocalVariables<'a>,
    /// The index of the next flag from the front.
    front: usize,
    /// The index after the next flag from the back.
    back: usize,
}

impl Iterator for DynamicLocalVariablesIter<'_> {
    type Item = bool;

    fn next(&mut self) -> Option<bool> {
        if self.front == self.back {
            return None;
        }
        self.front += 1;
        Some(self.flags.flag(self.front - 1))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.back - self.front;
        (len, Some(len))
    }
}

impl DoubleEndedIterator for DynamicLocalVariablesIter<'_> {
    fn next_back(&mut self) -> Option<bool> {
        if self.front == self.back {
            return None;
        }
        self.back -= 1;
        Some(self.flags.flag(self.back))
    }
}

impl ExactSizeIterator for DynamicLocalVariablesIter<'_> {}

impl FusedIterator for DynamicLocalVariablesIter<'_> {}

/// The tuple element names of a local variable or constant.
///
/// The blob is a NUL-terminated UTF-8 name for each tuple element in the
/// signature, in order. An empty name means that the element has no name.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TupleElementNames<'a> {
    /// The names. [`TupleElementNames::parse`] checked them.
    bytes: &'a [u8],
    /// The number of names.
    len: usize,
}

impl<'a> TupleElementNames<'a> {
    /// Decode a tuple element names blob.
    ///
    /// # Errors
    ///
    /// Return [`ErrorKind::UnexpectedEof`](crate::ErrorKind::UnexpectedEof) if
    /// the last name has no NUL terminator.
    pub fn parse(blob: &'a [u8]) -> Result<Self> {
        let len = count_to_end(blob, 0, read_name)?;
        Ok(Self { bytes: blob, len })
    }

    /// Return the blob bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Return the number of names.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Return `true` if there are no names.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return an iterator over the names, without their NUL terminators.
    #[must_use]
    pub const fn iter(&self) -> TupleElementNamesIter<'a> {
        TupleElementNamesIter(Entries::new(self.bytes, self.len, 0, read_name))
    }

    /// Encode a tuple element names blob, and append it to `out`. Use an empty
    /// name for an element without a name.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a
    ///   name has a NUL byte.
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if an
    ///   allocation fails.
    ///
    /// On error, `out` does not change.
    pub fn encode_parts<'n>(
        names: impl IntoIterator<Item = &'n [u8]>,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        let mut writer = Writer::append(out);
        for name in names {
            writer.c_string(name, "tuple element name has a NUL byte")?;
        }
        writer.finish();
        Ok(())
    }
}

iterable!(TupleElementNames => TupleElementNamesIter, Utf8Bytes<'a>);

/// Read one tuple element name.
fn read_name<'a>(reader: &mut Reader<'a>, _: i32) -> Result<Utf8Bytes<'a>> {
    reader
        .c_string("tuple element name has no NUL terminator")
        .map(Utf8Bytes::new)
}
