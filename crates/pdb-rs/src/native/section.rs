//! The image section headers in a DBI debug stream.
//!
//! The [`DebugStream::SectionHeader`] slot of the DBI optional debug header
//! names a stream with the section headers of the image. The
//! [`DebugStream::OriginalSectionHeader`] slot names the headers before
//! address remapping. Both streams are arrays of 40-byte PE/COFF
//! `IMAGE_SECTION_HEADER` records. LLVM `DbiStream.cpp`
//! (`initializeSectionHeadersData`) reads them as `coff_section` arrays.
//!
//! [`SectionHeaders`] borrows the stream bytes. [`SectionHeaderStream`] owns
//! them and gives the same view with [`SectionHeaderStream::headers`].
//!
//! [`DebugStream::SectionHeader`]: super::dbi::DebugStream::SectionHeader
//! [`DebugStream::OriginalSectionHeader`]: super::dbi::DebugStream::OriginalSectionHeader

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::debug_stream::{Codec, DebugRecord, RecordStream};
use super::output::Output;
use super::words::{halves, word};
use crate::{Error, ErrorKind, Result};

/// The size of one serialized section header.
pub const SECTION_HEADER_SIZE: usize = 40;
/// The number of 4-byte words in one section header.
const WORDS: usize = SECTION_HEADER_SIZE / 4;

/// A PE/COFF machine type (`IMAGE_FILE_MACHINE_*` in `winnt.h`).
///
/// The value set is open: the type keeps values without a constant.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Machine(pub u16);

impl Machine {
    /// `IMAGE_FILE_MACHINE_UNKNOWN`.
    pub const UNKNOWN: Self = Self(0);
    /// `IMAGE_FILE_MACHINE_I386`.
    pub const I386: Self = Self(0x014c);
    /// `IMAGE_FILE_MACHINE_ARMNT`.
    pub const ARMNT: Self = Self(0x01c4);
    /// `IMAGE_FILE_MACHINE_IA64`.
    pub const IA64: Self = Self(0x0200);
    /// `IMAGE_FILE_MACHINE_AMD64`.
    pub const AMD64: Self = Self(0x8664);
    /// `IMAGE_FILE_MACHINE_ARM64`.
    pub const ARM64: Self = Self(0xaa64);
}

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
    /// Make a header with a name, a virtual size and an RVA.
    ///
    /// The other fields are 0.
    #[must_use]
    pub const fn new(name: [u8; 8], virtual_size: u32, virtual_address: u32) -> Self {
        Self {
            name,
            virtual_size,
            virtual_address,
            size_of_raw_data: 0,
            pointer_to_raw_data: 0,
            pointer_to_relocations: 0,
            pointer_to_line_numbers: 0,
            relocation_count: 0,
            line_number_count: 0,
            characteristics: 0,
        }
    }

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
}

impl Codec for SectionHeader {
    const TRUNCATED: &'static str = "section header is truncated";

    /// Decode one record of [`WORDS`] words.
    fn decode(record: &[[u8; 4]]) -> Self {
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

impl DebugRecord for SectionHeader {
    const SIZE: usize = SECTION_HEADER_SIZE;

    fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut out = Output::new(out);
        out.reserve(Self::SIZE)?;
        out.bytes(&self.name)?;
        for value in [
            self.virtual_size,
            self.virtual_address,
            self.size_of_raw_data,
            self.pointer_to_raw_data,
            self.pointer_to_relocations,
            self.pointer_to_line_numbers,
        ] {
            out.u32(value)?;
        }
        out.u16(self.relocation_count)?;
        out.u16(self.line_number_count)?;
        out.u32(self.characteristics)
    }
}

/// A decoded section header stream (slots 5 and 10) with owned serialized bytes.
pub type SectionHeaderStream = RecordStream<SectionHeader>;

impl RecordStream<SectionHeader> {
    /// Return the borrowed view of the headers.
    #[must_use]
    pub fn headers(&self) -> SectionHeaders<'_> {
        SectionHeaders {
            records: self.as_bytes().as_chunks::<4>().0.as_chunks::<WORDS>().0,
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
        self.records
            .get(index)
            .map(|record| SectionHeader::decode(record))
    }

    /// Return an iterator over the headers in stream order.
    #[must_use]
    pub fn iter(&self) -> SectionHeadersIter<'a> {
        SectionHeadersIter {
            records: self.records.iter(),
        }
    }

    /// Append the stream to `out`.
    ///
    /// The encoder encodes each decoded header again. The output is equal to
    /// the decoded bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        Output::new(out).reserve(self.records.len().saturating_mul(SECTION_HEADER_SIZE))?;
        for header in self {
            header.encode(out)?;
        }
        Ok(())
    }
}

impl<'a> IntoIterator for &SectionHeaders<'a> {
    type Item = SectionHeader;
    type IntoIter = SectionHeadersIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over section headers.
#[derive(Clone, Debug)]
pub struct SectionHeadersIter<'a> {
    /// The records of [`WORDS`] words each.
    records: core::slice::Iter<'a, [[u8; 4]; WORDS]>,
}

impl Iterator for SectionHeadersIter<'_> {
    type Item = SectionHeader;

    fn next(&mut self) -> Option<Self::Item> {
        self.records
            .next()
            .map(|record| SectionHeader::decode(record))
    }

    fn nth(&mut self, count: usize) -> Option<Self::Item> {
        self.records
            .nth(count)
            .map(|record| SectionHeader::decode(record))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.records.size_hint()
    }
}

impl DoubleEndedIterator for SectionHeadersIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.records
            .next_back()
            .map(|record| SectionHeader::decode(record))
    }
}

impl ExactSizeIterator for SectionHeadersIter<'_> {}

impl FusedIterator for SectionHeadersIter<'_> {}
