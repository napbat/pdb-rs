//! The source file info substream of the DBI stream.
//!
//! The substream has this layout (`FileInfoSubstreamHeader` in LLVM
//! `RawTypes.h`; Microsoft pdb-rs `dbi_sources.md`):
//!
//! 1. The module count and a 16-bit source file count.
//! 2. A `u16` start index for each module.
//! 3. A `u16` file count for each module.
//! 4. A `u32` name offset for each file of each module.
//! 5. The names buffer with NUL-terminated file names.
//!
//! The 16-bit source file count and the 16-bit start indexes overflow for
//! more than 65535 file entries. The decoder therefore ignores them for
//! layout: the sum of the module file counts gives the number of name
//! offsets, and each module starts after the files of the modules before
//! it. LLVM `DbiModuleList.cpp` (`initializeFileInfo`) does the same.
//!
//! Nothing marks the end of the names buffer. Alignment padding can follow
//! the last name, so [`SourceInfo::names`] includes that padding.

use core::iter::FusedIterator;

use super::super::codeview::Fields;
use crate::{Error, ErrorKind, Result};

/// A decoded source file info substream.
#[derive(Clone, Copy, Debug)]
pub struct SourceInfo<'a> {
    /// The 16-bit source file count from the header.
    raw_file_count: u16,
    /// The start index of each module.
    starts: &'a [[u8; 2]],
    /// The file count of each module.
    counts: &'a [[u8; 2]],
    /// The name offset of each file entry.
    offsets: &'a [[u8; 4]],
    /// The names buffer, with any trailing padding.
    names: &'a [u8],
}

impl<'a> SourceInfo<'a> {
    /// Decode a non-empty source file info substream.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::UnexpectedEof`] if the header, the module arrays
    /// or the name offsets are truncated. Errors do not have a stream
    /// location. The caller adds it.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut fields = Fields::new(
            bytes,
            ErrorKind::UnexpectedEof,
            "DBI source info is truncated",
        );
        let module_count = usize::from(fields.u16()?);
        let raw_file_count = fields.u16()?;
        let starts = fields.bytes(module_count * 2)?.as_chunks::<2>().0;
        let counts = fields.bytes(module_count * 2)?.as_chunks::<2>().0;
        let file_count: usize = counts
            .iter()
            .map(|count| usize::from(u16::from_le_bytes(*count)))
            .sum();
        let offset_bytes = file_count.checked_mul(4).ok_or_else(|| {
            Error::new(ErrorKind::UnexpectedEof, None).with_context("DBI source info is truncated")
        })?;
        let offsets = fields.bytes(offset_bytes)?.as_chunks::<4>().0;
        Ok(Self {
            raw_file_count,
            starts,
            counts,
            offsets,
            names: fields.rest(),
        })
    }

    /// Return the number of modules.
    #[must_use]
    pub const fn module_count(&self) -> usize {
        self.starts.len()
    }

    /// Return the 16-bit source file count from the header.
    ///
    /// The value is often the low 16 bits of [`SourceInfo::file_count`].
    #[must_use]
    pub const fn raw_file_count(&self) -> u16 {
        self.raw_file_count
    }

    /// Return the number of file entries of all modules.
    ///
    /// Modules often share files, so names can repeat.
    #[must_use]
    pub const fn file_count(&self) -> usize {
        self.offsets.len()
    }

    /// Return the raw names buffer, with any trailing padding.
    #[must_use]
    pub const fn names(&self) -> &'a [u8] {
        self.names
    }

    /// Return the file lists of all modules, in module order.
    #[must_use]
    pub fn modules(&self) -> ModuleSourceIter<'a> {
        ModuleSourceIter {
            starts: self.starts.iter(),
            counts: self.counts.iter(),
            offsets: self.offsets,
            names: self.names,
            index: 0,
            next_file: 0,
        }
    }

    /// Return the NUL-terminated file name at an offset in the names buffer.
    ///
    /// The name excludes the terminator.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if the offset is outside the
    /// names buffer or no NUL follows it.
    pub fn file_name(&self, offset: u32) -> Result<&'a [u8]> {
        file_name(self.names, offset)
    }
}

/// The file list of one module.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ModuleSources<'a> {
    /// The zero-based module index.
    pub module: u16,
    /// The 16-bit start index from the substream. It can overflow, so the
    /// decoder does not use it.
    pub raw_start: u16,
    /// The index of the first file entry of the module in all file entries.
    pub first_file: u32,
    /// The name offset of each file of the module.
    offsets: &'a [[u8; 4]],
    /// The names buffer of the substream.
    names: &'a [u8],
}

impl<'a> ModuleSources<'a> {
    /// Return the number of files of the module.
    #[must_use]
    pub const fn file_count(&self) -> usize {
        self.offsets.len()
    }

    /// Return the name offset of each file, in substream order.
    pub fn name_offsets(&self) -> impl ExactSizeIterator<Item = u32> + FusedIterator + 'a {
        self.offsets
            .iter()
            .map(|offset| u32::from_le_bytes(*offset))
    }

    /// Return the name of each file, in substream order.
    ///
    /// An item is an [`ErrorKind::InvalidReference`] error if its offset
    /// does not refer to a NUL-terminated name in the names buffer.
    pub fn file_names(
        &self,
    ) -> impl ExactSizeIterator<Item = Result<&'a [u8]>> + FusedIterator + 'a {
        let names = self.names;
        self.name_offsets()
            .map(move |offset| file_name(names, offset))
    }
}

/// An iterator over the file lists of the modules.
#[derive(Clone, Debug)]
pub struct ModuleSourceIter<'a> {
    /// The remaining start indexes.
    starts: core::slice::Iter<'a, [u8; 2]>,
    /// The remaining file counts.
    counts: core::slice::Iter<'a, [u8; 2]>,
    /// The name offsets of the remaining modules.
    offsets: &'a [[u8; 4]],
    /// The names buffer.
    names: &'a [u8],
    /// The index of the next module.
    index: u16,
    /// The index of the first file entry of the next module.
    next_file: u32,
}

impl<'a> Iterator for ModuleSourceIter<'a> {
    type Item = ModuleSources<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.starts.next()?;
        let count = u16::from_le_bytes(*self.counts.next()?);
        // `parse` sized the offsets as the sum of all counts, so the split is in bounds.
        let (offsets, rest) = self
            .offsets
            .split_at(usize::from(count).min(self.offsets.len()));
        self.offsets = rest;
        let module = ModuleSources {
            module: self.index,
            raw_start: u16::from_le_bytes(*start),
            first_file: self.next_file,
            offsets,
            names: self.names,
        };
        // At most 65535 modules have at most 65535 files each, so the values fit.
        self.index = self.index.saturating_add(1);
        self.next_file = self.next_file.saturating_add(u32::from(count));
        Some(module)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.starts.size_hint()
    }
}

impl ExactSizeIterator for ModuleSourceIter<'_> {}

impl FusedIterator for ModuleSourceIter<'_> {}

/// Return the NUL-terminated name at `offset` of `names`.
fn file_name(names: &[u8], offset: u32) -> Result<&[u8]> {
    usize::try_from(offset)
        .ok()
        .and_then(|start| names.get(start..))
        .and_then(|rest| {
            rest.iter()
                .position(|&byte| byte == 0)
                .map(|end| &rest[..end])
        })
        .ok_or_else(|| {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("DBI source file name offset does not refer to a NUL-terminated name")
        })
}
