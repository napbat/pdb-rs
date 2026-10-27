//! Def-ranges with a DIA program, and def-ranges of values at entry.
//!
//! The layouts of `S_DEFRANGE` and `S_DEFRANGE_SUBFIELD` follow
//! `DEFRANGESYM` and `DEFRANGESYMSUBFIELD` in `cvinfo.h`. The pinned
//! references do not give the `_ON_ENTRY` kinds. The debug type records of
//! `cvinfo.h` in the DIA SDK of Visual Studio 18 (`lib/amd64/diaguids.lib`)
//! give `DEFRANGECONSTVAL_ON_ENTRY` (an array of `unsigned long` values) and
//! `DEFRANGEGLOBALSYM_ON_ENTRY` (a `long` offset and a section number).

use super::super::{Fields, SymbolKind};
use super::{AddressGaps, AddressRange, SectionOffset};
use crate::bytes::into_owned;
use crate::native::output::Output;
use crate::{Bytes, Error, ErrorKind, Result};

/// The kind of a def-range with a DIA program (`S_DEFRANGE`).
pub const S_DEFRANGE: SymbolKind = SymbolKind(0x113f);
/// The kind of a def-range with a DIA program for a part of a variable
/// (`S_DEFRANGE_SUBFIELD`).
pub const S_DEFRANGE_SUBFIELD: SymbolKind = SymbolKind(0x1140);
/// The kind of a constant value at procedure entry (`S_DEFRANGE_CONSTVAL_ON_ENTRY`).
pub const S_DEFRANGE_CONSTVAL_ON_ENTRY: SymbolKind = SymbolKind(0x117f);
/// The kind of a global symbol value at procedure entry
/// (`S_DEFRANGE_GLOBALSYM_ON_ENTRY`).
pub const S_DEFRANGE_GLOBALSYM_ON_ENTRY: SymbolKind = SymbolKind(0x1180);

/// A def-range with a DIA program ([`S_DEFRANGE`] and [`S_DEFRANGE_SUBFIELD`]).
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DefRangeProgram<'a> {
    /// The DIA program that evaluates the value (`program`).
    pub program: u32,
    /// The offset of the part in the parent variable for
    /// [`S_DEFRANGE_SUBFIELD`], or `None` for [`S_DEFRANGE`].
    pub parent_offset: Option<u32>,
    /// The code range where the program is valid.
    pub range: AddressRange,
    /// The parts of the range where the program is not valid.
    pub gaps: AddressGaps<'a>,
}

into_owned!(struct DefRangeProgram { gaps; program, parent_offset, range });

impl<'a> DefRangeProgram<'a> {
    /// Read the fields. Set `subfield` for [`S_DEFRANGE_SUBFIELD`].
    pub(super) fn read(fields: &mut Fields<'a>, subfield: bool) -> Result<Self> {
        let program = fields.u32()?;
        let parent_offset = if subfield { Some(fields.u32()?) } else { None };
        let range = AddressRange::read(fields)?;
        let gaps = AddressGaps::read(fields)?;
        Ok(Self {
            program,
            parent_offset,
            range,
            gaps,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.program)?;
        if let Some(offset) = self.parent_offset {
            out.u32(offset)?;
        }
        self.range.write(out)?;
        self.gaps.write(out)
    }
}

/// The values of an [`S_DEFRANGE_CONSTVAL_ON_ENTRY`] record.
///
/// No reference describes the meaning of the values. The record stores
/// little-endian 32-bit words.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct EntryValues<'a> {
    /// The little-endian 32-bit words.
    words: Bytes<'a>,
}

into_owned!(struct EntryValues { words; });

impl<'a> EntryValues<'a> {
    /// Create a value from little-endian 32-bit words.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the length of `bytes` is not
    /// a multiple of 4.
    pub fn parse(bytes: impl Into<Bytes<'a>>) -> Result<Self> {
        let words = bytes.into();
        if !words.len().is_multiple_of(4) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("S_DEFRANGE_CONSTVAL_ON_ENTRY is not a multiple of 4 bytes"));
        }
        Ok(Self { words })
    }

    /// Return the words. [`EntryValues::parse`] checked the length.
    fn words(&self) -> &[[u8; 4]] {
        self.words.as_chunks().0
    }

    /// Read all remaining bytes as words.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        Self::parse(fields.bytes(fields.rest().len())?)
    }

    /// Write the words.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.bytes(&self.words)
    }

    /// Return the number of words.
    #[must_use]
    pub fn len(&self) -> usize {
        self.words.len() / 4
    }

    /// Return `true` if the record has no words.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Return an iterator over the words.
    #[must_use]
    pub fn iter(&self) -> EntryValuesIter<'_> {
        EntryValuesIter {
            words: self.words().iter(),
        }
    }
}

/// An iterator over the words of an [`EntryValues`] record.
#[derive(Clone, Debug)]
pub struct EntryValuesIter<'a> {
    /// The 4-byte words that are not returned yet.
    words: core::slice::Iter<'a, [u8; 4]>,
}

impl Iterator for EntryValuesIter<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        self.words.next().map(|word| u32::from_le_bytes(*word))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.words.size_hint()
    }
}

impl ExactSizeIterator for EntryValuesIter<'_> {}

impl core::iter::FusedIterator for EntryValuesIter<'_> {}

impl<'a> IntoIterator for &'a EntryValues<'_> {
    type Item = u32;
    type IntoIter = EntryValuesIter<'a>;

    fn into_iter(self) -> EntryValuesIter<'a> {
        self.iter()
    }
}

/// Read the address of an [`S_DEFRANGE_GLOBALSYM_ON_ENTRY`] record.
///
/// The record stores a signed `long` offset. The value keeps its bits.
pub(super) fn read_global_on_entry(fields: &mut Fields<'_>) -> Result<SectionOffset> {
    SectionOffset::read(fields)
}
