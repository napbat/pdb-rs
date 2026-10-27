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
use super::to_u16;
use crate::Bytes;
use crate::bytes::into_owned;
use crate::native::output::{Output, no_memory, to_u32};
use crate::{Error, ErrorKind, Result};

/// A decoded source file info substream.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct SourceInfo<'a> {
    /// The 16-bit source file count from the header.
    ///
    /// The value is often the low 16 bits of [`SourceInfo::file_count`].
    pub raw_file_count: u16,
    /// The file list of each module, in module order.
    pub modules: Vec<ModuleSources>,
    /// The names buffer, with any trailing padding.
    pub names: Bytes<'a>,
}

into_owned!(struct SourceInfo { names; raw_file_count, modules });

impl<'a> SourceInfo<'a> {
    /// Decode a non-empty source file info substream.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] if the header, the module arrays or
    ///   the name offsets are truncated.
    /// - [`ErrorKind::LimitExceeded`] if an allocation fails.
    ///
    /// Errors do not have a stream location. The caller adds it.
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
        let mut offsets = fields.bytes(offset_bytes)?.as_chunks::<4>().0;
        let mut modules = Vec::new();
        modules
            .try_reserve_exact(module_count)
            .map_err(|_| no_memory())?;
        for (start, count) in starts.iter().zip(counts) {
            // `offsets` holds the sum of all counts, so the split is in bounds.
            let (own, rest) =
                offsets.split_at(usize::from(u16::from_le_bytes(*count)).min(offsets.len()));
            offsets = rest;
            let mut name_offsets = Vec::new();
            name_offsets
                .try_reserve_exact(own.len())
                .map_err(|_| no_memory())?;
            name_offsets.extend(own.iter().map(|offset| u32::from_le_bytes(*offset)));
            modules.push(ModuleSources {
                raw_start: u16::from_le_bytes(*start),
                name_offsets,
            });
        }
        Ok(Self {
            raw_file_count,
            modules,
            names: Bytes::new(fields.rest()),
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
    pub fn from_file_names<'n, M, F>(modules: M) -> Result<SourceInfo<'static>>
    where
        M: IntoIterator<Item = F>,
        F: IntoIterator<Item = &'n [u8]>,
    {
        let mut info = SourceInfo::default();
        let mut names = Vec::new();
        let mut known = BTreeMap::<&'n [u8], u32>::new();
        let mut total = 0_u32;
        for files in modules {
            let mut module = ModuleSources {
                raw_start: low16(total),
                name_offsets: Vec::new(),
            };
            for name in files {
                let offset = if let Some(&offset) = known.get(name) {
                    offset
                } else {
                    let offset = to_u32(names.len(), "DBI source names buffer is too large")?;
                    Output::new(&mut names).c_string(name)?;
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
        let padding = names.len().next_multiple_of(4) - names.len();
        Output::new(&mut names).zeros(padding)?;
        to_u32(names.len(), "DBI source names buffer is too large")?;
        info.raw_file_count = low16(total);
        info.names = Bytes::from(names);
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

    /// Return the number of file entries of all modules.
    ///
    /// Modules often share files, so names can repeat.
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.modules
            .iter()
            .map(|module| module.name_offsets.len())
            .sum()
    }

    /// Return the NUL-terminated file name at an offset in the names buffer.
    ///
    /// The name excludes the terminator.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if the offset is outside the
    /// names buffer or no NUL follows it.
    pub fn file_name(&self, offset: u32) -> Result<&[u8]> {
        file_name(&self.names, offset)
    }

    /// Return the name of each file of `module`, in substream order.
    ///
    /// An item is an [`ErrorKind::InvalidReference`] error if its offset
    /// does not refer to a NUL-terminated name in the names buffer.
    #[must_use]
    pub fn file_names<'s>(&'s self, module: &'s ModuleSources) -> FileNames<'s> {
        FileNames {
            names: &self.names,
            offsets: module.name_offsets.iter(),
        }
    }
}

/// The file list of one module.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ModuleSources {
    /// The 16-bit start index from the substream.
    ///
    /// Microsoft writers store the low 16 bits of the index of the first file
    /// entry. LLVM writers store the module index. The value can overflow,
    /// so readers ignore it.
    pub raw_start: u16,
    /// The offset of each file name in the names buffer, in substream order.
    pub name_offsets: Vec<u32>,
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

/// An iterator over the file names of a [`ModuleSources`] value.
///
/// An item is an [`ErrorKind::InvalidReference`] error if its offset does not refer to a
/// NUL-terminated name in the names buffer.
#[derive(Clone, Debug)]
pub struct FileNames<'a> {
    /// The names buffer.
    names: &'a [u8],
    /// The name offsets that are not returned yet.
    offsets: core::slice::Iter<'a, u32>,
}

impl<'a> Iterator for FileNames<'a> {
    type Item = Result<&'a [u8]>;

    fn next(&mut self) -> Option<Result<&'a [u8]>> {
        let offset = *self.offsets.next()?;
        Some(file_name(self.names, offset))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.offsets.size_hint()
    }
}

impl ExactSizeIterator for FileNames<'_> {}

impl FusedIterator for FileNames<'_> {}
