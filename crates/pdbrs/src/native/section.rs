//! The image section headers in a DBI debug stream.
//!
//! The [`DebugStream::SectionHeader`] slot of the DBI optional debug header
//! names a stream with the section headers of the image. The
//! [`DebugStream::OriginalSectionHeader`] slot names the headers before
//! address remapping. Both streams are arrays of 40-byte PE/COFF
//! `IMAGE_SECTION_HEADER` records. LLVM `DbiStream.cpp`
//! (`initializeSectionHeadersData`) reads them as `coff_section` arrays.
//!
//! [`DebugStream::SectionHeader`]: super::dbi::DebugStream::SectionHeader
//! [`DebugStream::OriginalSectionHeader`]: super::dbi::DebugStream::OriginalSectionHeader

use core::iter::FusedIterator;

use super::words::{halves, word};
use crate::{Error, ErrorKind, Result};

/// The size of one serialized section header.
pub const SECTION_HEADER_SIZE: usize = 40;
/// The number of 4-byte words in one section header.
const WORDS: usize = SECTION_HEADER_SIZE / 4;

/// One image section header.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct SectionHeader {
    /// The raw 8-byte name. Short names have NUL padding. A long name of an
    /// object file starts with `/`; this decoder does not resolve it.
    pub name: [u8; 8],
    /// The size of the section in memory.
    pub virtual_size: u32,
    /// The RVA of the first byte of the section.
    pub virtual_address: u32,
    /// The size of the initialized data in the file.
    pub size_of_raw_data: u32,
    /// The file offset of the initialized data.
    pub pointer_to_raw_data: u32,
    /// The file offset of the relocations.
    pub pointer_to_relocations: u32,
    /// The file offset of the COFF line numbers.
    pub pointer_to_line_numbers: u32,
    /// The number of relocations.
    pub relocation_count: u16,
    /// The number of COFF line numbers.
    pub line_number_count: u16,
    /// The `IMAGE_SCN_*` flags.
    pub characteristics: u32,
}

impl SectionHeader {
    /// Return the name without its NUL padding.
    #[must_use]
    pub fn name_bytes(&self) -> &[u8] {
        let end = self
            .name
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(self.name.len());
        &self.name[..end]
    }

    /// Decode one record of [`WORDS`] words.
    fn decode(record: &[[u8; 4]; WORDS]) -> Self {
        let [n0, n1, n2, n3] = record[0];
        let [n4, n5, n6, n7] = record[1];
        let (relocation_count, line_number_count) = halves(record, 8);
        Self {
            name: [n0, n1, n2, n3, n4, n5, n6, n7],
            virtual_size: word(record, 2),
            virtual_address: word(record, 3),
            size_of_raw_data: word(record, 4),
            pointer_to_raw_data: word(record, 5),
            pointer_to_relocations: word(record, 6),
            pointer_to_line_numbers: word(record, 7),
            relocation_count,
            line_number_count,
            characteristics: word(record, 9),
        }
    }
}

/// The section headers of a section header stream.
#[derive(Clone, Copy, Debug)]
pub struct SectionHeaders<'a> {
    /// The records of [`WORDS`] words each.
    records: &'a [[[u8; 4]; WORDS]],
}

impl<'a> SectionHeaders<'a> {
    /// Decode a section header stream.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the length is not a multiple of
    /// [`SECTION_HEADER_SIZE`]. The error does not have a stream location.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        if !bytes.len().is_multiple_of(SECTION_HEADER_SIZE) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("section header stream length is not a multiple of 40"));
        }
        Ok(Self {
            records: bytes.as_chunks::<4>().0.as_chunks::<WORDS>().0,
        })
    }

    /// Return the number of section headers.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.records.len()
    }

    /// Return `true` if the stream has no section headers.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Return the header of a 1-based section number.
    ///
    /// Symbols, section contributions and line data use 1-based section
    /// numbers. Return `None` for section 0 or a number past the last header.
    #[must_use]
    pub fn section(&self, number: u16) -> Option<SectionHeader> {
        let index = usize::from(number.checked_sub(1)?);
        self.records.get(index).map(SectionHeader::decode)
    }

    /// Return an iterator over the headers in stream order.
    #[must_use]
    pub fn iter(&self) -> SectionHeaderIter<'a> {
        SectionHeaderIter {
            records: self.records.iter(),
        }
    }
}

impl<'a> IntoIterator for &SectionHeaders<'a> {
    type Item = SectionHeader;
    type IntoIter = SectionHeaderIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over section headers.
#[derive(Clone, Debug)]
pub struct SectionHeaderIter<'a> {
    /// The records of [`WORDS`] words each.
    records: core::slice::Iter<'a, [[u8; 4]; WORDS]>,
}

impl Iterator for SectionHeaderIter<'_> {
    type Item = SectionHeader;

    fn next(&mut self) -> Option<Self::Item> {
        self.records.next().map(SectionHeader::decode)
    }

    fn nth(&mut self, count: usize) -> Option<Self::Item> {
        self.records.nth(count).map(SectionHeader::decode)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.records.size_hint()
    }
}

impl DoubleEndedIterator for SectionHeaderIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.records.next_back().map(SectionHeader::decode)
    }
}

impl ExactSizeIterator for SectionHeaderIter<'_> {}

impl FusedIterator for SectionHeaderIter<'_> {}
