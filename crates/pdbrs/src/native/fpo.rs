//! The frame pointer omission (FPO) streams of x86 images.
//!
//! The DBI optional debug header names two streams with frame data:
//!
//! - The [`DebugStream::Fpo`] slot names an array of 16-byte `FPO_DATA`
//!   records ([`FpoStream`]). The layout is `FPO_DATA` in `winnt.h` and the
//!   PE/COFF specification (`IMAGE_DEBUG_TYPE_FPO`).
//! - The [`DebugStream::NewFpo`] slot names an array of 32-byte `FRAMEDATA`
//!   records ([`FrameDataStream`]). The layout is `tagFRAMEDATA` in
//!   `include/cvinfo.h` of `microsoft/microsoft-pdb` and `FrameData` in LLVM
//!   `CodeView.h`. The frame program of a record is a byte offset in the
//!   `/names` stream.
//!
//! The streams have no header. LLVM `DebugFrameDataSubsectionRef` accepts a
//! 4-byte relocation prefix only in the `DEBUG_S_FRAMEDATA` subsection of an
//! object file, not in this PDB stream.
//!
//! [`DebugStream::Fpo`]: super::dbi::DebugStream::Fpo
//! [`DebugStream::NewFpo`]: super::dbi::DebugStream::NewFpo

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::msf::StreamId;
use super::words::{halves, word};
use crate::{Error, ErrorKind, Result};

/// The size of one `FPO_DATA` record.
pub const FPO_RECORD_SIZE: usize = 16;
/// The size of one `FRAMEDATA` record.
pub const FRAME_DATA_SIZE: usize = 32;

/// The frame kind of an `FPO_DATA` record (`cbFrame`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FrameKind {
    /// The function omits the frame pointer (`FRAME_FPO`, 0).
    Fpo,
    /// A kernel trap frame (`FRAME_TRAP`, 1).
    Trap,
    /// A kernel task state segment frame (`FRAME_TSS`, 2).
    Tss,
    /// A standard frame with a frame pointer (`FRAME_NONFPO`, 3).
    NonFpo,
}

/// One `FPO_DATA` record.
///
/// The attribute word holds bit fields from bit 0 up: `cbProlog` (8 bits),
/// `cbRegs` (3 bits), `fHasSEH`, `fUseBP`, `reserved` and `cbFrame` (2 bits).
/// LLVM `object::FpoData` reads `cbProlog` from 4 bits and `fHasSEH` and
/// `fUseBP` from bits 9 and 10. This decoder follows `winnt.h`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct FpoData {
    /// The RVA of the first byte of the function (`ulOffStart`).
    pub start: u32,
    /// The size of the function in bytes (`cbProcSize`).
    pub size: u32,
    /// The size of the locals in 4-byte units (`cdwLocals`).
    pub locals: u32,
    /// The size of the parameters in 4-byte units (`cdwParams`).
    pub params: u16,
    /// The size of the prolog in bytes (`cbProlog`).
    pub prolog_size: u8,
    /// The number of saved registers (`cbRegs`, 0 to 7).
    pub saved_registers: u8,
    /// The function has structured exception handling (`fHasSEH`).
    pub has_seh: bool,
    /// The function uses EBP as the frame pointer (`fUseBP`).
    pub uses_bp: bool,
    /// The reserved bit.
    pub reserved: bool,
    /// The frame kind (`cbFrame`).
    pub frame: FrameKind,
}

impl FpoData {
    /// Decode one record of 4 words.
    fn decode(record: &[[u8; 4]; 4]) -> Self {
        let (params, attributes) = halves(record, 3);
        let [prolog_size, high] = attributes.to_le_bytes();
        let frame = match high >> 6 {
            0 => FrameKind::Fpo,
            1 => FrameKind::Trap,
            2 => FrameKind::Tss,
            _ => FrameKind::NonFpo,
        };
        Self {
            start: word(record, 0),
            size: word(record, 1),
            locals: word(record, 2),
            params,
            prolog_size,
            saved_registers: high & 0x7,
            has_seh: high & 0x08 != 0,
            uses_bp: high & 0x10 != 0,
            reserved: high & 0x20 != 0,
            frame,
        }
    }
}

/// One `FRAMEDATA` record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct FrameData {
    /// The RVA of the first byte of the code block (`ulRvaStart`).
    pub start: u32,
    /// The size of the code block in bytes (`cbBlock`).
    pub size: u32,
    /// The size of the locals in bytes (`cbLocals`).
    pub locals_size: u32,
    /// The size of the parameters in bytes (`cbParams`).
    pub params_size: u32,
    /// The maximum stack size in bytes (`cbStkMax`).
    pub max_stack_size: u32,
    /// The byte offset of the frame program in the `/names` stream (`frameFunc`).
    pub program: u32,
    /// The size of the prolog in bytes (`cbProlog`).
    pub prolog_size: u16,
    /// The size of the saved registers in bytes (`cbSavedRegs`).
    pub saved_registers_size: u16,
    /// The flag word: bit 0 `fHasSEH`, bit 1 `fHasEH`, bit 2 `fIsFunctionStart`.
    pub flags: u32,
}

impl FrameData {
    /// Return `true` if the block has structured exception handling (`fHasSEH`).
    #[must_use]
    pub const fn has_seh(&self) -> bool {
        self.flags & 1 != 0
    }

    /// Return `true` if the block has C++ exception handling (`fHasEH`).
    #[must_use]
    pub const fn has_cpp_eh(&self) -> bool {
        self.flags & 2 != 0
    }

    /// Return `true` if the block starts a function (`fIsFunctionStart`).
    #[must_use]
    pub const fn is_function_start(&self) -> bool {
        self.flags & 4 != 0
    }

    /// Decode one record of 8 words.
    fn decode(record: &[[u8; 4]; 8]) -> Self {
        let (prolog_size, saved_registers_size) = halves(record, 6);
        Self {
            start: word(record, 0),
            size: word(record, 1),
            locals_size: word(record, 2),
            params_size: word(record, 3),
            max_stack_size: word(record, 4),
            program: word(record, 5),
            prolog_size,
            saved_registers_size,
            flags: word(record, 7),
        }
    }
}

/// Check that `bytes` is an array of `size`-byte records.
fn check_length(bytes: &[u8], size: usize, stream: StreamId, context: &'static str) -> Result<()> {
    let tail = bytes.len() % size;
    if tail == 0 {
        return Ok(());
    }
    let offset = u64::try_from(bytes.len() - tail).unwrap_or(u64::MAX);
    Err(Error::new(ErrorKind::UnexpectedEof, None)
        .with_context(context)
        .with_stream(stream.0, offset))
}

/// A decoded `FPO_DATA` stream with owned serialized bytes.
#[derive(Clone, Debug)]
pub struct FpoStream {
    /// The stream bytes; the length is a multiple of [`FPO_RECORD_SIZE`].
    bytes: Vec<u8>,
}

impl FpoStream {
    /// Decode the FPO stream `stream`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::UnexpectedEof`] if the length is not a multiple of
    /// [`FPO_RECORD_SIZE`]. The error identifies `stream` and the offset of the
    /// truncated record.
    pub fn parse(bytes: Vec<u8>, stream: StreamId) -> Result<Self> {
        check_length(&bytes, FPO_RECORD_SIZE, stream, "FPO record is truncated")?;
        Ok(Self { bytes })
    }

    /// Return the number of records.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len() / FPO_RECORD_SIZE
    }

    /// Return `true` if the stream has no records.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Return an iterator over the records in stream order.
    #[must_use]
    pub fn iter(&self) -> FpoIter<'_> {
        FpoIter {
            records: self.bytes.as_chunks::<4>().0.as_chunks::<4>().0.iter(),
        }
    }
}

impl<'a> IntoIterator for &'a FpoStream {
    type Item = FpoData;
    type IntoIter = FpoIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over `FPO_DATA` records.
#[derive(Clone, Debug)]
pub struct FpoIter<'a> {
    /// The records of 4 words each.
    records: core::slice::Iter<'a, [[u8; 4]; 4]>,
}

impl Iterator for FpoIter<'_> {
    type Item = FpoData;

    fn next(&mut self) -> Option<Self::Item> {
        self.records.next().map(FpoData::decode)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.records.size_hint()
    }
}

impl DoubleEndedIterator for FpoIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.records.next_back().map(FpoData::decode)
    }
}

impl ExactSizeIterator for FpoIter<'_> {}

impl FusedIterator for FpoIter<'_> {}

/// A decoded `FRAMEDATA` stream with owned serialized bytes.
#[derive(Clone, Debug)]
pub struct FrameDataStream {
    /// The stream bytes; the length is a multiple of [`FRAME_DATA_SIZE`].
    bytes: Vec<u8>,
}

impl FrameDataStream {
    /// Decode the new FPO stream `stream`.
    ///
    /// The decoder does not read the `/names` stream. Use
    /// [`crate::native::names::StringTable::get`] with [`FrameData::program`]
    /// to get the frame program.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::UnexpectedEof`] if the length is not a multiple of
    /// [`FRAME_DATA_SIZE`]. The error identifies `stream` and the offset of the
    /// truncated record.
    pub fn parse(bytes: Vec<u8>, stream: StreamId) -> Result<Self> {
        check_length(
            &bytes,
            FRAME_DATA_SIZE,
            stream,
            "frame data record is truncated",
        )?;
        Ok(Self { bytes })
    }

    /// Return the number of records.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len() / FRAME_DATA_SIZE
    }

    /// Return `true` if the stream has no records.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Return an iterator over the records in stream order.
    #[must_use]
    pub fn iter(&self) -> FrameDataIter<'_> {
        FrameDataIter {
            records: self.bytes.as_chunks::<4>().0.as_chunks::<8>().0.iter(),
        }
    }
}

impl<'a> IntoIterator for &'a FrameDataStream {
    type Item = FrameData;
    type IntoIter = FrameDataIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over `FRAMEDATA` records.
#[derive(Clone, Debug)]
pub struct FrameDataIter<'a> {
    /// The records of 8 words each.
    records: core::slice::Iter<'a, [[u8; 4]; 8]>,
}

impl Iterator for FrameDataIter<'_> {
    type Item = FrameData;

    fn next(&mut self) -> Option<Self::Item> {
        self.records.next().map(FrameData::decode)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.records.size_hint()
    }
}

impl DoubleEndedIterator for FrameDataIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.records.next_back().map(FrameData::decode)
    }
}

impl ExactSizeIterator for FrameDataIter<'_> {}

impl FusedIterator for FrameDataIter<'_> {}
