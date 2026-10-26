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

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::super::codeview::Fields;
use super::super::codeview::index::ModuleIndex;
use super::to_u16;
use crate::native::output::{Output, no_memory, to_u32};
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

    /// Append the substream to `out` with all raw values as stored.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let module_count = to_u16(self.starts.len(), "DBI source info module count")?;
        let mut output = Output::new(out);
        output.reserve(4 + self.starts.len() * 4 + self.offsets.len() * 4 + self.names.len())?;
        output.u16(module_count)?;
        output.u16(self.raw_file_count)?;
        output.bytes(self.starts.as_flattened())?;
        output.bytes(self.counts.as_flattened())?;
        output.bytes(self.offsets.as_flattened())?;
        output.bytes(self.names)
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
#[non_exhaustive]
pub struct ModuleSources<'a> {
    /// The zero-based module index.
    pub module: ModuleIndex,
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
    #[must_use]
    pub fn name_offsets(&self) -> NameOffsets<'a> {
        NameOffsets(self.offsets.iter())
    }

    /// Return the name of each file, in substream order.
    ///
    /// An item is an [`ErrorKind::InvalidReference`] error if its offset
    /// does not refer to a NUL-terminated name in the names buffer.
    #[must_use]
    pub fn file_names(&self) -> FileNames<'a> {
        FileNames {
            names: self.names,
            offsets: self.name_offsets(),
        }
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
    index: u32,
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
            module: ModuleIndex(self.index),
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

/// The owned file list of one module in an [`OwnedSourceInfo`].
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct OwnedModuleSources {
    /// The 16-bit start index for the substream.
    ///
    /// Microsoft writers store the low 16 bits of the index of the first file
    /// entry. LLVM writers store the module index. Readers ignore the value.
    pub raw_start: u16,
    /// The offset of each file name in the names buffer.
    pub name_offsets: Vec<u32>,
}

/// An owned source file info substream that a writer can fill.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct OwnedSourceInfo {
    /// The 16-bit source file count for the header.
    pub raw_file_count: u16,
    /// The file list of each module, in module order.
    pub modules: Vec<OwnedModuleSources>,
    /// The names buffer, with the padding to the next 4-byte boundary.
    pub names: Vec<u8>,
}

impl OwnedSourceInfo {
    /// Copy a decoded substream with all raw values.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn from_info(info: &SourceInfo<'_>) -> Result<Self> {
        let mut modules = Vec::new();
        modules
            .try_reserve_exact(info.module_count())
            .map_err(|_| no_memory())?;
        for module in info.modules() {
            let mut name_offsets = Vec::new();
            name_offsets
                .try_reserve_exact(module.file_count())
                .map_err(|_| no_memory())?;
            name_offsets.extend(module.name_offsets());
            modules.push(OwnedModuleSources {
                raw_start: module.raw_start,
                name_offsets,
            });
        }
        let mut names = Vec::new();
        names
            .try_reserve_exact(info.names().len())
            .map_err(|_| no_memory())?;
        names.extend_from_slice(info.names());
        Ok(Self {
            raw_file_count: info.raw_file_count(),
            modules,
            names,
        })
    }

    /// Build a substream from the file names of each module.
    ///
    /// The function follows Microsoft `DBI1::QueryFileInfo`:
    ///
    /// 1. Each distinct name goes once into the names buffer, in the order of
    ///    its first use.
    /// 2. The start index of a module is the low 16 bits of the number of file
    ///    entries of the modules before it.
    /// 3. The header count is the low 16 bits of the number of all file entries.
    /// 4. Zero bytes pad the names buffer to a 4-byte boundary.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if a name contains a NUL byte.
    /// - [`ErrorKind::LimitExceeded`] if the names buffer or the number of
    ///   file entries does not fit in `u32`, or if an allocation fails.
    pub fn from_file_names<'n, M, F>(modules: M) -> Result<Self>
    where
        M: IntoIterator<Item = F>,
        F: IntoIterator<Item = &'n [u8]>,
    {
        let mut info = Self::default();
        let mut known = BTreeMap::<&'n [u8], u32>::new();
        let mut total = 0_u32;
        for files in modules {
            let mut module = OwnedModuleSources {
                raw_start: low16(total),
                name_offsets: Vec::new(),
            };
            for name in files {
                let offset = if let Some(&offset) = known.get(name) {
                    offset
                } else {
                    let offset = to_u32(info.names.len(), "DBI source names buffer is too large")?;
                    Output::new(&mut info.names).c_string(name)?;
                    known.insert(name, offset);
                    offset
                };
                module
                    .name_offsets
                    .try_reserve(1)
                    .map_err(|_| no_memory())?;
                module.name_offsets.push(offset);
                total = total.checked_add(1).ok_or_else(|| {
                    Error::new(ErrorKind::LimitExceeded, None)
                        .with_context("DBI source info has too many file entries")
                })?;
            }
            info.modules.try_reserve(1).map_err(|_| no_memory())?;
            info.modules.push(module);
        }
        let padding = info.names.len().next_multiple_of(4) - info.names.len();
        Output::new(&mut info.names).zeros(padding)?;
        to_u32(info.names.len(), "DBI source names buffer is too large")?;
        info.raw_file_count = low16(total);
        Ok(info)
    }

    /// Append the substream to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the substream has more than
    /// 65535 modules, if a module has more than 65535 files or if the output
    /// cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let module_count = to_u16(
            self.modules.len(),
            "DBI source info has more than 65535 modules",
        )?;
        let files: usize = self
            .modules
            .iter()
            .map(|module| module.name_offsets.len())
            .sum();
        let mut output = Output::new(out);
        output.reserve(4 + self.modules.len() * 4 + files * 4 + self.names.len())?;
        output.u16(module_count)?;
        output.u16(self.raw_file_count)?;
        for module in &self.modules {
            output.u16(module.raw_start)?;
        }
        for module in &self.modules {
            output.u16(to_u16(
                module.name_offsets.len(),
                "DBI source info module has more than 65535 files",
            )?)?;
        }
        for offset in self.modules.iter().flat_map(|module| &module.name_offsets) {
            output.u32(*offset)?;
        }
        output.bytes(&self.names)
    }
}

/// Return the low 16 bits of `value`, as Microsoft writers store them.
fn low16(value: u32) -> u16 {
    let [low, high, _, _] = value.to_le_bytes();
    u16::from_le_bytes([low, high])
}

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

/// An iterator over the name offsets of the files of a [`ModuleSources`] value.
#[derive(Clone, Debug)]
pub struct NameOffsets<'a>(core::slice::Iter<'a, [u8; 4]>);

impl Iterator for NameOffsets<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        self.0.next().map(|x| u32::from_le_bytes(*x))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl ExactSizeIterator for NameOffsets<'_> {}

impl core::iter::FusedIterator for NameOffsets<'_> {}

/// An iterator over the file names of a [`ModuleSources`] value.
///
/// An item is an [`ErrorKind::InvalidReference`] error if its offset does not refer to a
/// NUL-terminated name in the names buffer.
#[derive(Clone, Debug)]
pub struct FileNames<'a> {
    /// The names buffer.
    names: &'a [u8],
    /// The name offsets that are not returned yet.
    offsets: NameOffsets<'a>,
}

impl<'a> Iterator for FileNames<'a> {
    type Item = Result<&'a [u8]>;

    fn next(&mut self) -> Option<Result<&'a [u8]>> {
        let offset = self.offsets.next()?;
        Some(file_name(self.names, offset))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.offsets.size_hint()
    }
}

impl ExactSizeIterator for FileNames<'_> {}

impl FusedIterator for FileNames<'_> {}
