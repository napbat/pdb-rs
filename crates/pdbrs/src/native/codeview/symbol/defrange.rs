//! Def-ranges with a DIA program, and def-ranges of values at entry.
//!
//! The layouts of `S_DEFRANGE` and `S_DEFRANGE_SUBFIELD` follow
//! `DEFRANGESYM` and `DEFRANGESYMSUBFIELD` in `cvinfo.h`. The pinned
//! references do not give the `_ON_ENTRY` kinds. The debug type records of
//! `cvinfo.h` in the DIA SDK of Visual Studio 18 (`lib/amd64/diaguids.lib`)
//! give `DEFRANGECONSTVAL_ON_ENTRY` (an array of `unsigned long` values) and
//! `DEFRANGEGLOBALSYM_ON_ENTRY` (a `long` offset and a section number).

use super::super::Fields;
use super::{AddressGaps, AddressRange, SectionOffset};
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// The kind of a def-range with a DIA program (`S_DEFRANGE`).
pub const S_DEFRANGE: u16 = 0x113f;
/// The kind of a def-range with a DIA program for a part of a variable
/// (`S_DEFRANGE_SUBFIELD`).
pub const S_DEFRANGE_SUBFIELD: u16 = 0x1140;
/// The kind of a constant value at procedure entry (`S_DEFRANGE_CONSTVAL_ON_ENTRY`).
pub const S_DEFRANGE_CONSTVAL_ON_ENTRY: u16 = 0x117f;
/// The kind of a global symbol value at procedure entry
/// (`S_DEFRANGE_GLOBALSYM_ON_ENTRY`).
pub const S_DEFRANGE_GLOBALSYM_ON_ENTRY: u16 = 0x1180;

/// A def-range with a DIA program ([`S_DEFRANGE`] and [`S_DEFRANGE_SUBFIELD`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EntryValues<'a> {
    /// The little-endian 32-bit words.
    words: &'a [[u8; 4]],
}

impl<'a> EntryValues<'a> {
    /// Create a value from little-endian 32-bit words.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the length of `bytes` is not
    /// a multiple of 4.
    pub fn from_le_bytes(bytes: &'a [u8]) -> Result<Self> {
        let (words, rest) = bytes.as_chunks();
        if !rest.is_empty() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("S_DEFRANGE_CONSTVAL_ON_ENTRY is not a multiple of 4 bytes"));
        }
        Ok(Self { words })
    }

    /// Read all remaining bytes as words.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        Self::from_le_bytes(fields.bytes(fields.rest().len())?)
    }

    /// Write the words.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        for word in self.words {
            out.bytes(word)?;
        }
        Ok(())
    }

    /// Return the number of words.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.words.len()
    }

    /// Return `true` if the record has no words.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Return an iterator over the words.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = u32> + 'a {
        self.words.iter().map(|word| u32::from_le_bytes(*word))
    }
}

/// Read the address of an [`S_DEFRANGE_GLOBALSYM_ON_ENTRY`] record.
///
/// The record stores a signed `long` offset. The value keeps its bits.
pub(super) fn read_global_on_entry(fields: &mut Fields<'_>) -> Result<SectionOffset> {
    SectionOffset::read(fields)
}
