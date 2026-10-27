//! Custom debug information about local variables: `DynamicLocals` and
//! `TupleElementNames`.
//!
//! The layouts follow `DecodeDynamicLocalsRecord`, `DecodeTupleElementNamesRecord`,
//! `AddDynamicLocals` and `AddTupleElementNames` in Roslyn
//! `CustomDebugInfoReader.cs` and `CustomDebugInfoEncoder.cs` (commit
//! `e3c31f15b79e226656e3b082a921d4d60127b1fa`).

use core::iter::FusedIterator;

use super::super::super::super::Fields;
use super::super::{SHORT_VALUE, count, invalid, split_utf16, zero_padding};
use crate::bytes::into_owned;
use crate::portable::metadata::IlOffset;
use crate::{Bytes, ErrorKind, Result};

/// The size of the flag field of a dynamic local.
const FLAG_BYTES: usize = 64;

/// The size of the UTF-16 name field of a dynamic local.
const NAME_BYTES: usize = 128;

/// The size of one dynamic local entry.
const DYNAMIC_LOCAL_SIZE: usize = FLAG_BYTES + 4 + 4 + NAME_BYTES;

/// The `dynamic` flags of local variables and constants.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct DynamicLocals<'a> {
    /// The 200-byte entries.
    bytes: Bytes<'a>,
}

into_owned!(struct DynamicLocals { bytes; });

impl<'a> DynamicLocals<'a> {
    /// Read an `i32` count, the entries and the zero padding.
    pub(super) fn parse(data: &'a [u8]) -> Result<Self> {
        let mut fields = Fields::new(data, ErrorKind::InvalidFormat, SHORT_VALUE);
        let len = count(fields.u32()?)?;
        let size = len
            .checked_mul(DYNAMIC_LOCAL_SIZE)
            .ok_or_else(|| invalid(SHORT_VALUE))?;
        let bytes = Bytes::new(fields.bytes(size)?);
        zero_padding(fields.rest(), "bytes follow the dynamic locals")?;
        Ok(Self { bytes })
    }

    /// Return the number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len() / DYNAMIC_LOCAL_SIZE
    }

    /// Return `true` if there are no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Return an iterator over the entries.
    #[must_use]
    pub fn iter(&self) -> DynamicLocalsIter<'_> {
        DynamicLocalsIter {
            chunks: self.bytes.as_chunks::<DYNAMIC_LOCAL_SIZE>().0.iter(),
        }
    }
}

iterable!(DynamicLocals, DynamicLocalsIter);

/// The `dynamic` flags of one local variable or constant.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DynamicLocal<'a> {
    /// The flag bytes: one byte for each type in the signature, nonzero for
    /// `dynamic`. The slice has [`DynamicLocal::flag_count`] bytes, at most 64.
    pub flags: Bytes<'a>,
    /// The stored flag count. It can be larger than 64.
    pub flag_count: u32,
    /// The local variable slot. Roslyn writes 0 for a constant.
    pub slot: u32,
    /// The name as UTF-16LE bytes, without the NUL code unit and the zero fill.
    pub name: Bytes<'a>,
}

into_owned!(struct DynamicLocal { flags, name; flag_count, slot });

/// An iterator over [`DynamicLocals`].
#[derive(Clone, Debug)]
pub struct DynamicLocalsIter<'a> {
    /// The remaining entries.
    chunks: core::slice::Iter<'a, [u8; DYNAMIC_LOCAL_SIZE]>,
}

impl<'a> Iterator for DynamicLocalsIter<'a> {
    type Item = DynamicLocal<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let chunk = self.chunks.next()?;
        let (flags, rest) = chunk.split_at(FLAG_BYTES);
        let (counts, name) = rest.split_at(8);
        let flag_count = u32::from_le_bytes([counts[0], counts[1], counts[2], counts[3]]);
        let slot = u32::from_le_bytes([counts[4], counts[5], counts[6], counts[7]]);
        let used = usize::try_from(flag_count).map_or(FLAG_BYTES, |n| n.min(FLAG_BYTES));
        let name = split_utf16(name).map_or(name, |(name, _)| name);
        Some(DynamicLocal {
            flags: Bytes::new(&flags[..used]),
            flag_count,
            slot,
            name: Bytes::new(name),
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.chunks.size_hint()
    }
}

impl ExactSizeIterator for DynamicLocalsIter<'_> {}

impl FusedIterator for DynamicLocalsIter<'_> {}

/// The tuple element names of local variables and constants.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct TupleElementNames<'a> {
    /// The entries. [`TupleElementNames::parse`] checked them.
    bytes: Bytes<'a>,
    /// The number of entries.
    len: usize,
}

into_owned!(struct TupleElementNames { bytes; len });

impl<'a> TupleElementNames<'a> {
    /// Read an `i32` count, the entries and the zero padding.
    pub(super) fn parse(data: &'a [u8]) -> Result<Self> {
        let mut fields = Fields::new(data, ErrorKind::InvalidFormat, SHORT_VALUE);
        let len = count(fields.u32()?)?;
        let start = fields.rest();
        for _ in 0..len {
            TupleElementNamesEntry::read(&mut fields)?;
        }
        let rest = fields.rest();
        zero_padding(rest, "bytes follow the tuple element names")?;
        Ok(Self {
            bytes: Bytes::new(&start[..start.len() - rest.len()]),
            len,
        })
    }

    /// Return the number of entries.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Return `true` if there are no entries.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return an iterator over the entries.
    #[must_use]
    pub fn iter(&self) -> TupleElementNamesIter<'_> {
        TupleElementNamesIter { bytes: &self.bytes }
    }
}

iterable!(TupleElementNames, TupleElementNamesIter);

/// The tuple element names of one local variable or constant.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct TupleElementNamesEntry<'a> {
    /// The element names. An empty name means that the element has no name.
    pub element_names: Utf8Names<'a>,
    /// The local variable slot, or -1 for a constant.
    pub slot: i32,
    /// The first IL offset of the scope of a constant, or 0 for a variable.
    pub scope_start: IlOffset,
    /// The end IL offset of the scope of a constant, or 0 for a variable.
    pub scope_end: IlOffset,
    /// The UTF-8 name of the local variable or constant, not decoded as text.
    pub name: Bytes<'a>,
}

into_owned!(struct TupleElementNamesEntry { element_names, name; slot, scope_start, scope_end });

impl<'a> TupleElementNamesEntry<'a> {
    /// Read one entry.
    fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let len = count(fields.u32()?)?;
        let start = fields.rest();
        for _ in 0..len {
            fields.name()?;
        }
        let bytes = &start[..start.len() - fields.rest().len()];
        let slot = fields.i32()?;
        let scope_start = IlOffset(fields.u32()?);
        let scope_end = IlOffset(fields.u32()?);
        let name = Bytes::new(fields.name()?);
        Ok(Self {
            element_names: Utf8Names {
                bytes: Bytes::new(bytes),
                len,
            },
            slot,
            scope_start,
            scope_end,
            name,
        })
    }
}

/// An iterator over [`TupleElementNames`].
#[derive(Clone, Debug)]
pub struct TupleElementNamesIter<'a> {
    /// The remaining entries. [`TupleElementNames`] checked them.
    bytes: &'a [u8],
}

impl<'a> Iterator for TupleElementNamesIter<'a> {
    type Item = TupleElementNamesEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.bytes.is_empty() {
            return None;
        }
        let mut fields = Fields::new(self.bytes, ErrorKind::InvalidFormat, SHORT_VALUE);
        let entry = TupleElementNamesEntry::read(&mut fields).ok()?;
        self.bytes = fields.rest();
        Some(entry)
    }
}

impl FusedIterator for TupleElementNamesIter<'_> {}

/// A list of NUL-terminated UTF-8 names.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct Utf8Names<'a> {
    /// The names with their NUL terminators.
    bytes: Bytes<'a>,
    /// The number of names.
    len: usize,
}

into_owned!(struct Utf8Names { bytes; len });

impl Utf8Names<'_> {
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
    pub fn iter(&self) -> Utf8NamesIter<'_> {
        Utf8NamesIter { bytes: &self.bytes }
    }
}

iterable!(Utf8Names, Utf8NamesIter);

/// An iterator over [`Utf8Names`].
#[derive(Clone, Debug)]
pub struct Utf8NamesIter<'a> {
    /// The remaining names.
    bytes: &'a [u8],
}

impl<'a> Iterator for Utf8NamesIter<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        let end = self.bytes.iter().position(|&byte| byte == 0)?;
        let name = &self.bytes[..end];
        self.bytes = &self.bytes[end + 1..];
        Some(name)
    }
}

impl FusedIterator for Utf8NamesIter<'_> {}
