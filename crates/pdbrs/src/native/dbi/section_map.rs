//! The section map substream of the DBI stream.
//!
//! A 4-byte header gives the number of segment descriptors and the number of
//! logical segment descriptors. 20-byte descriptors follow it. The layouts
//! are `SecMapHeader` and `SecMapEntry` in LLVM `RawTypes.h`. The flag bits
//! are `OMFSegDescFlags` in LLVM `RawConstants.h`.

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::super::words::{halves, word};
use super::to_u16;
use crate::native::output::{Output, no_memory};
use crate::{Error, ErrorKind, Result};

/// The number of 4-byte words in one descriptor.
const WORDS: usize = 5;

/// The flags of a section map descriptor (`OMFSegDescFlags`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct SegmentFlags(pub u16);

impl SegmentFlags {
    /// The segment is readable.
    pub const READ: Self = Self(1 << 0);
    /// The segment is writable.
    pub const WRITE: Self = Self(1 << 1);
    /// The segment is executable.
    pub const EXECUTE: Self = Self(1 << 2);
    /// The descriptor describes a 32-bit linear address.
    pub const ADDRESS_IS_32_BIT: Self = Self(1 << 3);
    /// The frame is a selector.
    pub const IS_SELECTOR: Self = Self(1 << 8);
    /// The frame is an absolute address.
    pub const IS_ABSOLUTE_ADDRESS: Self = Self(1 << 9);
    /// The descriptor describes a group.
    pub const IS_GROUP: Self = Self(1 << 10);

    /// Return `true` if all bits of `other` are set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// One section map descriptor.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct SectionMapEntry {
    /// The descriptor flags.
    pub flags: SegmentFlags,
    /// The logical overlay number.
    pub overlay: u16,
    /// The group index in the descriptor array.
    pub group: u16,
    /// The frame: for an image, the 1-based section number, or 0 for the
    /// absolute segment.
    pub frame: u16,
    /// The byte index of the segment name in the segment name table, or `0xffff`.
    pub section_name: u16,
    /// The byte index of the class name in the segment name table, or `0xffff`.
    pub class_name: u16,
    /// The byte offset of the logical segment in the physical segment.
    pub offset: u32,
    /// The size of the segment in bytes.
    pub length: u32,
}

impl SectionMapEntry {
    /// Decode one descriptor of [`WORDS`] words.
    fn decode(record: &[[u8; 4]; WORDS]) -> Self {
        let (flags, overlay) = halves(record, 0);
        let (group, frame) = halves(record, 1);
        let (section_name, class_name) = halves(record, 2);
        Self {
            flags: SegmentFlags(flags),
            overlay,
            group,
            frame,
            section_name,
            class_name,
            offset: word(record, 3),
            length: word(record, 4),
        }
    }

    /// Append the 20-byte descriptor to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut output = Output::new(out);
        output.reserve(WORDS * 4)?;
        output.u16(self.flags.0)?;
        output.u16(self.overlay)?;
        output.u16(self.group)?;
        output.u16(self.frame)?;
        output.u16(self.section_name)?;
        output.u16(self.class_name)?;
        output.u32(self.offset)?;
        output.u32(self.length)
    }
}

/// A decoded section map substream.
#[derive(Clone, Copy, Debug)]
pub struct SectionMap<'a> {
    /// The number of logical segment descriptors from the header.
    logical_count: u16,
    /// The descriptors of [`WORDS`] words each.
    records: &'a [[[u8; 4]; WORDS]],
}

impl<'a> SectionMap<'a> {
    /// Decode a non-empty section map substream.
    ///
    /// The descriptor count in the header must account for all bytes.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] if the header or a descriptor is truncated.
    /// - [`ErrorKind::InvalidFormat`] if bytes follow the last descriptor.
    ///
    /// Errors do not have a stream location. The caller adds it.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (header, rest) = bytes.split_first_chunk::<4>().ok_or_else(|| {
            Error::new(ErrorKind::UnexpectedEof, None)
                .with_context("DBI section map header is truncated")
        })?;
        let (count, logical_count) = halves(&[*header], 0);
        let (words, remainder) = rest.as_chunks::<4>();
        let expected = usize::from(count) * WORDS;
        if words.len() < expected {
            return Err(Error::new(ErrorKind::UnexpectedEof, None)
                .with_context("DBI section map descriptors are truncated"));
        }
        if words.len() > expected || !remainder.is_empty() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("DBI section map has bytes after the last descriptor"));
        }
        Ok(Self {
            logical_count,
            records: words.as_chunks::<WORDS>().0,
        })
    }

    /// Return the number of descriptors from the header.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.records.len()
    }

    /// Return `true` if the map has no descriptors.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Return the number of logical segment descriptors from the header.
    #[must_use]
    pub const fn logical_count(&self) -> u16 {
        self.logical_count
    }

    /// Return an iterator over the descriptors in substream order.
    #[must_use]
    pub fn iter(&self) -> SectionMapIter<'a> {
        SectionMapIter {
            records: self.records.iter(),
        }
    }

    /// Append the substream to `out`: the header and the descriptors.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        encode_section_map(self.logical_count, self.iter(), out)
    }
}

/// An owned section map that a writer can fill.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct OwnedSectionMap {
    /// The number of logical segment descriptors for the header.
    pub logical_count: u16,
    /// The descriptors in substream order.
    pub entries: Vec<SectionMapEntry>,
}

impl OwnedSectionMap {
    /// Copy a decoded section map.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the allocation fails.
    pub fn from_map(map: &SectionMap<'_>) -> Result<Self> {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(map.len())
            .map_err(|_| no_memory())?;
        entries.extend(map.iter());
        Ok(Self {
            logical_count: map.logical_count(),
            entries,
        })
    }

    /// Append the substream to `out`: the header and the descriptors.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the map has more than 65535
    /// descriptors or if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        encode_section_map(self.logical_count, self.entries.iter().copied(), out)
    }
}

/// Append a section map header and its descriptors to `out`.
fn encode_section_map(
    logical_count: u16,
    entries: impl ExactSizeIterator<Item = SectionMapEntry>,
    out: &mut Vec<u8>,
) -> Result<()> {
    let count = to_u16(
        entries.len(),
        "DBI section map has more than 65535 descriptors",
    )?;
    let mut output = Output::new(out);
    output.reserve(entries.len().saturating_mul(WORDS * 4).saturating_add(4))?;
    output.u16(count)?;
    output.u16(logical_count)?;
    for entry in entries {
        entry.encode(out)?;
    }
    Ok(())
}

impl<'a> IntoIterator for &SectionMap<'a> {
    type Item = SectionMapEntry;
    type IntoIter = SectionMapIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over section map descriptors.
#[derive(Clone, Debug)]
pub struct SectionMapIter<'a> {
    /// The descriptors.
    records: core::slice::Iter<'a, [[u8; 4]; WORDS]>,
}

impl Iterator for SectionMapIter<'_> {
    type Item = SectionMapEntry;

    fn next(&mut self) -> Option<Self::Item> {
        self.records.next().map(SectionMapEntry::decode)
    }

    fn nth(&mut self, count: usize) -> Option<Self::Item> {
        self.records.nth(count).map(SectionMapEntry::decode)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.records.size_hint()
    }
}

impl DoubleEndedIterator for SectionMapIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.records.next_back().map(SectionMapEntry::decode)
    }
}

impl ExactSizeIterator for SectionMapIter<'_> {}

impl FusedIterator for SectionMapIter<'_> {}
