//! The section contribution substream of the DBI stream.
//!
//! Each record describes one block of image bytes that one module supplies.
//! A version `u32` starts the substream; fixed-size records follow it.
//! The record layouts are `SectionContrib` and `SectionContrib2` in LLVM
//! `RawTypes.h`. The version values are in LLVM `RawConstants.h`.
//!
//! Microsoft `PDB/dbi/dbi.cpp` reads a substream that starts with another
//! value as headerless VC++ 4.0 `SC40` records (`PDB/include/dbicommon.h`).
//! This decoder does the same and reports [`SectionContributionVersion::V40`].

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::super::words::{halves, word};
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// The `DBISCImpv` version value (`0xeffe0000 + 19970605`).
pub const SECTION_CONTRIBUTION_V60: u32 = 0xeffe_0000 + 19_970_605;
/// The `DBISCImpv2` version value (`0xeffe0000 + 20140516`).
pub const SECTION_CONTRIBUTION_V2: u32 = 0xeffe_0000 + 20_140_516;

/// The record layout of a section contribution substream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SectionContributionVersion {
    /// Headerless 20-byte `SC40` records without CRCs.
    V40,
    /// [`SECTION_CONTRIBUTION_V60`]: 28-byte `SectionContrib` records.
    V60,
    /// [`SECTION_CONTRIBUTION_V2`]: 32-byte `SectionContrib2` records with a COFF section index.
    V2,
}

impl SectionContributionVersion {
    /// Return the number of 4-byte words in one record.
    const fn words(self) -> usize {
        match self {
            Self::V40 => 5,
            Self::V60 => 7,
            Self::V2 => 8,
        }
    }

    /// Return the size of one record in bytes.
    #[must_use]
    pub const fn record_size(self) -> usize {
        self.words() * 4
    }

    /// Return the version value at the start of the substream.
    ///
    /// [`SectionContributionVersion::V40`] has no version value.
    #[must_use]
    pub const fn value(self) -> Option<u32> {
        match self {
            Self::V40 => None,
            Self::V60 => Some(SECTION_CONTRIBUTION_V60),
            Self::V2 => Some(SECTION_CONTRIBUTION_V2),
        }
    }
}

/// One section contribution.
///
/// The record of a module info record uses the
/// [`SectionContributionVersion::V60`] layout.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SectionContribution {
    /// The 1-based section number.
    pub section: u16,
    /// The raw padding after the section number. Writers can leave it uninitialized.
    pub padding1: u16,
    /// The byte offset of the contribution in the section.
    pub offset: i32,
    /// The size of the contribution in bytes.
    pub size: i32,
    /// The `IMAGE_SCN_*` flags of the contributing object file section.
    pub characteristics: u32,
    /// The zero-based index of the contributing module.
    pub module: u16,
    /// The raw padding after the module index. Writers can leave it uninitialized.
    pub padding2: u16,
    /// The CRC of the contributed data. [`SectionContributionVersion::V40`] has no CRC.
    pub data_crc: Option<u32>,
    /// The CRC of the relocations. [`SectionContributionVersion::V40`] has no CRC.
    pub reloc_crc: Option<u32>,
    /// The COFF section index. Only [`SectionContributionVersion::V2`] has it.
    pub coff_section: Option<u32>,
}

impl SectionContribution {
    /// Decode one record of `version`.
    pub(super) fn decode(version: SectionContributionVersion, record: &[[u8; 4]]) -> Self {
        let (section, padding1) = halves(record, 0);
        let (module, padding2) = halves(record, 4);
        let has_crc = version != SectionContributionVersion::V40;
        Self {
            section,
            padding1,
            offset: i32::from_le_bytes(record[1]),
            size: i32::from_le_bytes(record[2]),
            characteristics: word(record, 3),
            module,
            padding2,
            data_crc: has_crc.then(|| word(record, 5)),
            reloc_crc: has_crc.then(|| word(record, 6)),
            coff_section: (version == SectionContributionVersion::V2).then(|| word(record, 7)),
        }
    }

    /// Append the record in the layout of `version` to `out`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the optional fields do not match
    ///   `version`: [`SectionContributionVersion::V40`] has no CRCs, and only
    ///   [`SectionContributionVersion::V2`] has a COFF section index.
    /// - [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, version: SectionContributionVersion, out: &mut Vec<u8>) -> Result<()> {
        let has_crc = version != SectionContributionVersion::V40;
        let has_coff = version == SectionContributionVersion::V2;
        if self.data_crc.is_some() != has_crc
            || self.reloc_crc.is_some() != has_crc
            || self.coff_section.is_some() != has_coff
        {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("section contribution fields do not match the version"));
        }
        let mut output = Output::new(out);
        output.reserve(version.record_size())?;
        output.u16(self.section)?;
        output.u16(self.padding1)?;
        output.bytes(&self.offset.to_le_bytes())?;
        output.bytes(&self.size.to_le_bytes())?;
        output.u32(self.characteristics)?;
        output.u16(self.module)?;
        output.u16(self.padding2)?;
        for value in [self.data_crc, self.reloc_crc, self.coff_section]
            .into_iter()
            .flatten()
        {
            output.u32(value)?;
        }
        Ok(())
    }
}

/// Append a section contribution substream to `out`.
///
/// # Errors
///
/// - The errors of [`SectionContribution::encode`].
/// - [`ErrorKind::InvalidFormat`] if the first
///   [`SectionContributionVersion::V40`] record starts with a version value,
///   because a reader would then select another layout.
pub(super) fn encode_section_contributions(
    version: SectionContributionVersion,
    records: impl ExactSizeIterator<Item = SectionContribution>,
    out: &mut Vec<u8>,
) -> Result<()> {
    let size = records.len().saturating_mul(version.record_size());
    let mut output = Output::new(out);
    output.reserve(size.saturating_add(4))?;
    if let Some(value) = version.value() {
        output.u32(value)?;
    }
    for (index, record) in records.enumerate() {
        if index == 0 && version == SectionContributionVersion::V40 {
            let first = u32::from(record.section) | (u32::from(record.padding1) << 16);
            if first == SECTION_CONTRIBUTION_V60 || first == SECTION_CONTRIBUTION_V2 {
                return Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("first V40 section contribution starts with a version value"));
            }
        }
        record.encode(version, out)?;
    }
    Ok(())
}

/// A decoded section contribution substream.
#[derive(Clone, Copy, Debug)]
pub struct SectionContributions<'a> {
    /// The record layout.
    version: SectionContributionVersion,
    /// The records as 4-byte words.
    words: &'a [[u8; 4]],
}

impl<'a> SectionContributions<'a> {
    /// Decode a non-empty section contribution substream.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] if the substream has fewer than 4 bytes.
    /// - [`ErrorKind::InvalidFormat`] if the records do not fill the
    ///   substream exactly.
    ///
    /// Errors do not have a stream location. The caller adds it.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (head, rest) = bytes.split_first_chunk::<4>().ok_or_else(|| {
            Error::new(ErrorKind::UnexpectedEof, None)
                .with_context("DBI section contribution version is truncated")
        })?;
        let (version, records) = match u32::from_le_bytes(*head) {
            SECTION_CONTRIBUTION_V60 => (SectionContributionVersion::V60, rest),
            SECTION_CONTRIBUTION_V2 => (SectionContributionVersion::V2, rest),
            _ => (SectionContributionVersion::V40, bytes),
        };
        let (words, remainder) = records.as_chunks::<4>();
        if !remainder.is_empty() || !words.len().is_multiple_of(version.words()) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("DBI section contributions do not fill the substream"));
        }
        Ok(Self { version, words })
    }

    /// Append the substream to `out`: the version value, if any, and the records.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        encode_section_contributions(self.version, self.iter(), out)
    }

    /// Return the record layout.
    #[must_use]
    pub const fn version(&self) -> SectionContributionVersion {
        self.version
    }

    /// Return the number of records.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.words.len() / self.version.words()
    }

    /// Return `true` if the substream has no records.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Return an iterator over the records in substream order.
    #[must_use]
    pub fn iter(&self) -> SectionContributionIter<'a> {
        SectionContributionIter {
            version: self.version,
            records: self.words.chunks_exact(self.version.words()),
        }
    }
}

impl<'a> IntoIterator for &SectionContributions<'a> {
    type Item = SectionContribution;
    type IntoIter = SectionContributionIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over section contributions.
#[derive(Clone, Debug)]
pub struct SectionContributionIter<'a> {
    /// The record layout.
    version: SectionContributionVersion,
    /// The records.
    records: core::slice::ChunksExact<'a, [u8; 4]>,
}

impl Iterator for SectionContributionIter<'_> {
    type Item = SectionContribution;

    fn next(&mut self) -> Option<Self::Item> {
        let record = self.records.next()?;
        Some(SectionContribution::decode(self.version, record))
    }

    fn nth(&mut self, count: usize) -> Option<Self::Item> {
        let record = self.records.nth(count)?;
        Some(SectionContribution::decode(self.version, record))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.records.size_hint()
    }
}

impl DoubleEndedIterator for SectionContributionIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let record = self.records.next_back()?;
        Some(SectionContribution::decode(self.version, record))
    }
}

impl ExactSizeIterator for SectionContributionIter<'_> {}

impl FusedIterator for SectionContributionIter<'_> {}
