//! The module info substream of the DBI stream.
//!
//! Each record has a 64-byte prefix, two NUL-terminated names and padding to
//! the next 4-byte boundary. The prefix is `MODI_60_Persist` in Microsoft
//! `PDB/dbi/dbi.h` and `ModuleInfoHeader` in LLVM `RawTypes.h`:
//!
//! | Offset | Field |
//! |---|---|
//! | 0 | `unused1` (`pmod`, a stale pointer) |
//! | 4 | the first section contribution (`SectionContrib`, 28 bytes) |
//! | 32 | the flags |
//! | 34 | the module stream |
//! | 36, 40, 44 | the symbol, C11 line and C13 line region sizes |
//! | 48 | the source file count |
//! | 50 | padding |
//! | 52 | `mpifileichFile`, a stale pointer |
//! | 56, 60 | the EC source file name index and the PDB file path index |

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::{
    FIXED_SIZE, SectionContribution, SectionContributionVersion, align4, c_string, copy_bytes,
    dbi_error, stream_id, stream_index, u16_at, u32_at,
};
use crate::native::codeview::index::ModuleIndex;
use crate::native::msf::StreamId;
use crate::native::output::Output;
use crate::{ErrorKind, Result};

/// The flag bit that marks a module that the writer wrote since it opened the DBI stream.
const FLAG_WRITTEN: u16 = 1 << 0;
/// The flag bit that marks a module with edit and continue information.
const FLAG_EC_ENABLED: u16 = 1 << 1;

/// One module info record from the DBI stream.
///
/// Names stay raw bytes. They are often paths in an unspecified code page.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Module<'a> {
    /// The zero-based position of the record in the module info substream.
    pub index: ModuleIndex,
    /// The logical offset of the record in the DBI stream.
    pub offset: u32,
    /// The `u32` at offset 0. Microsoft writers store a stale pointer here.
    pub unused1: u32,
    /// The first section contribution of the module.
    ///
    /// It has the [`SectionContributionVersion::V60`] layout.
    pub section_contribution: SectionContribution,
    /// The module flags. [`Module::written`], [`Module::ec_enabled`] and
    /// [`Module::type_server_index`] decode the bit fields.
    pub flags: u16,
    /// The module stream, or `None` if the module has no debug data.
    pub stream: Option<StreamId>,
    /// The size of the symbol region in the module stream, with its signature.
    pub symbol_bytes: u32,
    /// The size of the C11 line region in the module stream.
    pub c11_bytes: u32,
    /// The size of the C13 line region in the module stream.
    pub c13_bytes: u32,
    /// The number of source files that contributed to the module.
    pub source_file_count: u16,
    /// The raw padding `u16` at offset 50.
    pub padding: u16,
    /// The `mpifileichFile` value at offset 52.
    ///
    /// Microsoft writers store a stale pointer here. Readers ignore it.
    pub file_name_offset: u32,
    /// The index of the source file name in the EC string table (`niSrcFile`).
    pub source_file_name_index: u32,
    /// The index of the compiler PDB path in the EC string table (`niPdbFile`).
    pub pdb_file_path_index: u32,
    /// The module name without its terminating NUL.
    pub name: &'a [u8],
    /// The object file name without its terminating NUL.
    pub object_file: &'a [u8],
    /// The bytes after the object file name up to the next 4-byte boundary.
    ///
    /// The record uses only the first `n` bytes, where `n` makes the record
    /// size a multiple of 4. The other bytes are zero.
    pub name_padding: [u8; 3],
}

impl Module<'_> {
    /// Return `true` if bit 0 of the flags (`fWritten`) is set.
    #[must_use]
    pub const fn written(&self) -> bool {
        self.flags & FLAG_WRITTEN != 0
    }

    /// Return `true` if bit 1 of the flags (`fECEnabled`) is set.
    #[must_use]
    pub const fn ec_enabled(&self) -> bool {
        self.flags & FLAG_EC_ENABLED != 0
    }

    /// Return the type server map index in bits 8 to 15 of the flags (`iTSM`).
    #[must_use]
    pub const fn type_server_index(&self) -> u8 {
        self.flags.to_le_bytes()[1]
    }

    /// Append the record with its names and padding to `out`.
    ///
    /// The function ignores [`Module::index`] and [`Module::offset`].
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if a name contains a NUL byte or if the
    ///   section contribution does not have the
    ///   [`SectionContributionVersion::V60`] fields.
    /// - [`ErrorKind::LimitExceeded`] if the stream index is larger than
    ///   `0xfffe`, if the module index of the section contribution does not
    ///   fit in 16 bits or if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let end = FIXED_SIZE + self.name.len() + 1 + self.object_file.len() + 1;
        let padding = end.next_multiple_of(4) - end;
        Output::new(out).reserve(end + padding)?;
        Output::new(out).u32(self.unused1)?;
        self.section_contribution
            .encode(SectionContributionVersion::V60, out)?;
        let mut output = Output::new(out);
        output.u16(self.flags)?;
        output.u16(stream_index(self.stream)?)?;
        output.u32(self.symbol_bytes)?;
        output.u32(self.c11_bytes)?;
        output.u32(self.c13_bytes)?;
        output.u16(self.source_file_count)?;
        output.u16(self.padding)?;
        output.u32(self.file_name_offset)?;
        output.u32(self.source_file_name_index)?;
        output.u32(self.pdb_file_path_index)?;
        output.c_string(self.name)?;
        output.c_string(self.object_file)?;
        output.bytes(&self.name_padding[..padding])
    }
}

/// An owned module info record that a writer can fill.
///
/// The fields have the meaning of the [`Module`] fields with the same names.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct OwnedModule {
    /// The `u32` at offset 0.
    pub unused1: u32,
    /// The first section contribution, with the [`SectionContributionVersion::V60`] fields.
    pub section_contribution: SectionContribution,
    /// The module flags.
    pub flags: u16,
    /// The module stream, or `None` if the module has no debug data.
    pub stream: Option<StreamId>,
    /// The size of the symbol region in the module stream, with its signature.
    pub symbol_bytes: u32,
    /// The size of the C11 line region in the module stream.
    pub c11_bytes: u32,
    /// The size of the C13 line region in the module stream.
    pub c13_bytes: u32,
    /// The number of source files that contributed to the module.
    pub source_file_count: u16,
    /// The raw padding `u16` at offset 50.
    pub padding: u16,
    /// The `mpifileichFile` value at offset 52.
    pub file_name_offset: u32,
    /// The index of the source file name in the EC string table.
    pub source_file_name_index: u32,
    /// The index of the compiler PDB path in the EC string table.
    pub pdb_file_path_index: u32,
    /// The module name without its terminating NUL.
    pub name: Vec<u8>,
    /// The object file name without its terminating NUL.
    pub object_file: Vec<u8>,
    /// The bytes after the object file name up to the next 4-byte boundary.
    pub name_padding: [u8; 3],
}

impl OwnedModule {
    /// Copy a decoded module info record.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn from_module(module: &Module<'_>) -> Result<Self> {
        Ok(Self {
            unused1: module.unused1,
            section_contribution: module.section_contribution,
            flags: module.flags,
            stream: module.stream,
            symbol_bytes: module.symbol_bytes,
            c11_bytes: module.c11_bytes,
            c13_bytes: module.c13_bytes,
            source_file_count: module.source_file_count,
            padding: module.padding,
            file_name_offset: module.file_name_offset,
            source_file_name_index: module.source_file_name_index,
            pdb_file_path_index: module.pdb_file_path_index,
            name: copy_bytes(module.name)?,
            object_file: copy_bytes(module.object_file)?,
            name_padding: module.name_padding,
        })
    }

    /// Return a [`Module`] view of the record with index 0 and offset 0.
    #[must_use]
    pub fn as_module(&self) -> Module<'_> {
        Module {
            index: ModuleIndex(0),
            offset: 0,
            unused1: self.unused1,
            section_contribution: self.section_contribution,
            flags: self.flags,
            stream: self.stream,
            symbol_bytes: self.symbol_bytes,
            c11_bytes: self.c11_bytes,
            c13_bytes: self.c13_bytes,
            source_file_count: self.source_file_count,
            padding: self.padding,
            file_name_offset: self.file_name_offset,
            source_file_name_index: self.source_file_name_index,
            pdb_file_path_index: self.pdb_file_path_index,
            name: &self.name,
            object_file: &self.object_file,
            name_padding: self.name_padding,
        }
    }

    /// Append the record with its names and padding to `out`.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Module::encode`].
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        self.as_module().encode(out)
    }
}

/// An iterator over the module info records of a DBI stream.
///
/// The iterator returns one error for a malformed record and then stops.
#[derive(Clone, Debug)]
pub struct Modules<'a> {
    /// The module info substream.
    pub(super) bytes: &'a [u8],
    /// The logical offset of the substream in the DBI stream.
    pub(super) base: usize,
    /// The offset of the next record in `bytes`.
    pub(super) position: usize,
    /// The index of the next record.
    pub(super) index: u32,
}

impl<'a> Modules<'a> {
    /// Decode the record at the current position.
    fn read(&self) -> Result<(Module<'a>, usize)> {
        let start = self.position;
        let location = |offset: usize| self.base.saturating_add(offset);
        let truncated = |context| {
            dbi_error(
                ErrorKind::UnexpectedEof,
                location(self.bytes.len()),
                context,
            )
        };
        let rest = &self.bytes[start..];
        let fixed = rest
            .first_chunk::<FIXED_SIZE>()
            .ok_or_else(|| truncated("DBI module info prefix is truncated"))?;
        let (name, after_name) = c_string(rest, FIXED_SIZE)
            .ok_or_else(|| truncated("DBI module name has no terminating NUL"))?;
        let (object_file, after_object) = c_string(rest, after_name)
            .ok_or_else(|| truncated("DBI object file name has no terminating NUL"))?;
        // The record starts at a 4-byte boundary, so the record length aligns the end.
        let padded = align4(after_object)
            .filter(|&end| end <= rest.len())
            .ok_or_else(|| truncated("DBI module info padding is truncated"))?;
        let offset = u32::try_from(location(start)).map_err(|_| {
            dbi_error(
                ErrorKind::OffsetOverflow,
                location(start),
                "DBI module offset does not fit in 32 bits",
            )
        })?;
        let mut name_padding = [0; 3];
        name_padding[..padded - after_object].copy_from_slice(&rest[after_object..padded]);
        let contribution = fixed[4..32].as_chunks::<4>().0;
        let module = Module {
            index: ModuleIndex(self.index),
            offset,
            unused1: u32_at(fixed, 0),
            section_contribution: SectionContribution::decode(
                SectionContributionVersion::V60,
                contribution,
            ),
            flags: u16_at(fixed, 32),
            stream: stream_id(u16_at(fixed, 34)),
            symbol_bytes: u32_at(fixed, 36),
            c11_bytes: u32_at(fixed, 40),
            c13_bytes: u32_at(fixed, 44),
            source_file_count: u16_at(fixed, 48),
            padding: u16_at(fixed, 50),
            file_name_offset: u32_at(fixed, 52),
            source_file_name_index: u32_at(fixed, 56),
            pdb_file_path_index: u32_at(fixed, 60),
            name,
            object_file,
            name_padding,
        };
        Ok((module, start + padded))
    }
}

impl<'a> Iterator for Modules<'a> {
    type Item = Result<Module<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.position >= self.bytes.len() {
            return None;
        }
        match self.read() {
            Ok((module, next)) => {
                self.position = next;
                // Each record has at least 64 bytes, so the count fits when offsets fit.
                self.index = self.index.saturating_add(1);
                Some(Ok(module))
            }
            Err(error) => {
                self.position = self.bytes.len();
                Some(Err(error))
            }
        }
    }
}

impl FusedIterator for Modules<'_> {}
