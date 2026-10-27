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
    FIXED_SIZE, SectionContribution, SectionContributionVersion, align4, c_string, dbi_error,
    stream_id, stream_index, u16_at, u32_at,
};
use crate::Bytes;
use crate::bytes::into_owned;
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
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Module<'a> {
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
    pub name: Bytes<'a>,
    /// The object file name without its terminating NUL.
    pub object_file: Bytes<'a>,
    /// The bytes after the object file name up to the next 4-byte boundary.
    ///
    /// The record uses only the first `n` bytes, where `n` makes the record
    /// size a multiple of 4. The other bytes are zero.
    pub name_padding: [u8; 3],
}

into_owned!(struct Module {
    name, object_file;
    unused1, section_contribution, flags, stream, symbol_bytes, c11_bytes, c13_bytes,
    source_file_count, padding, file_name_offset, source_file_name_index, pdb_file_path_index,
    name_padding
});

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
        output.c_string(&self.name)?;
        output.c_string(&self.object_file)?;
        output.bytes(&self.name_padding[..padding])
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
    /// `true` after an error.
    pub(super) failed: bool,
}

impl<'a> Modules<'a> {
    /// Return the offset of the next record in the module info substream.
    ///
    /// After an error, the offset identifies the start of the record that
    /// caused the error. Use this offset to add a location to the error.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

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
        let mut name_padding = [0; 3];
        name_padding[..padded - after_object].copy_from_slice(&rest[after_object..padded]);
        let contribution = fixed[4..32].as_chunks::<4>().0;
        let module = Module {
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
            name: Bytes::new(name),
            object_file: Bytes::new(object_file),
            name_padding,
        };
        Ok((module, start + padded))
    }
}

impl<'a> Iterator for Modules<'a> {
    type Item = Result<(ModuleIndex, Module<'a>)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.position >= self.bytes.len() {
            return None;
        }
        match self.read() {
            Ok((module, next)) => {
                let index = ModuleIndex(self.index);
                self.position = next;
                // Each record has at least 64 bytes, so the count fits when offsets fit.
                self.index = self.index.saturating_add(1);
                Some(Ok((index, module)))
            }
            Err(error) => {
                self.failed = true;
                Some(Err(error))
            }
        }
    }
}

impl FusedIterator for Modules<'_> {}
