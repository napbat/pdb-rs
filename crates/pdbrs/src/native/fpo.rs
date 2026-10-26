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
//! object file, not in this PDB stream. Both streams are
//! [`RecordStream`] values: their encoders encode each record again.
//!
//! [`DebugStream::Fpo`]: super::dbi::DebugStream::Fpo
//! [`DebugStream::NewFpo`]: super::dbi::DebugStream::NewFpo

use alloc::vec::Vec;

use super::debug_stream::{Codec, DebugRecord, RecordStream};
use super::output::Output;
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
    /// Make a record for a function at `start` with `size` bytes.
    ///
    /// The other fields are 0, and the frame kind is [`FrameKind::Fpo`].
    #[must_use]
    pub const fn new(start: u32, size: u32) -> Self {
        Self {
            start,
            size,
            locals: 0,
            params: 0,
            prolog_size: 0,
            saved_registers: 0,
            has_seh: false,
            uses_bp: false,
            reserved: false,
            frame: FrameKind::Fpo,
        }
    }

    /// Return the attribute word.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if `saved_registers` is greater than 7.
    fn attributes(&self) -> Result<u16> {
        if self.saved_registers > 7 {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("FPO saved register count does not fit in 3 bits"));
        }
        let frame = match self.frame {
            FrameKind::Fpo => 0,
            FrameKind::Trap => 1,
            FrameKind::Tss => 2,
            FrameKind::NonFpo => 3,
        };
        let high = self.saved_registers
            | u8::from(self.has_seh) << 3
            | u8::from(self.uses_bp) << 4
            | u8::from(self.reserved) << 5
            | frame << 6;
        Ok(u16::from_le_bytes([self.prolog_size, high]))
    }
}

impl Codec for FpoData {
    const TRUNCATED: &'static str = "FPO record is truncated";

    fn decode(record: &[[u8; 4]]) -> Self {
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

impl DebugRecord for FpoData {
    const SIZE: usize = FPO_RECORD_SIZE;

    /// Append the 16-byte `FPO_DATA` record to `out`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `saved_registers` is greater than 7.
    /// - [`ErrorKind::LimitExceeded`] if the output cannot grow.
    fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let attributes = self.attributes()?;
        let mut out = Output::new(out);
        out.reserve(Self::SIZE)?;
        out.u32(self.start)?;
        out.u32(self.size)?;
        out.u32(self.locals)?;
        out.u16(self.params)?;
        out.u16(attributes)
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
    /// Make a record for a code block at `start` with `size` bytes.
    ///
    /// The other fields are 0.
    #[must_use]
    pub const fn new(start: u32, size: u32) -> Self {
        Self {
            start,
            size,
            locals_size: 0,
            params_size: 0,
            max_stack_size: 0,
            program: 0,
            prolog_size: 0,
            saved_registers_size: 0,
            flags: 0,
        }
    }

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
}

impl FrameData {
    /// Decode one 32-byte `FRAMEDATA` record.
    pub(crate) fn from_bytes(bytes: [u8; FRAME_DATA_SIZE]) -> Self {
        let half = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let word = |index: usize| {
            let at = index * 4;
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        Self {
            start: word(0),
            size: word(1),
            locals_size: word(2),
            params_size: word(3),
            max_stack_size: word(4),
            program: word(5),
            prolog_size: half(24),
            saved_registers_size: half(26),
            flags: word(7),
        }
    }

    /// Append the 32-byte `FRAMEDATA` record to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        out.try_reserve(FRAME_DATA_SIZE).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("frame data output cannot be allocated")
        })?;
        for value in [
            self.start,
            self.size,
            self.locals_size,
            self.params_size,
            self.max_stack_size,
            self.program,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&self.prolog_size.to_le_bytes());
        out.extend_from_slice(&self.saved_registers_size.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        Ok(())
    }
}

impl Codec for FrameData {
    const TRUNCATED: &'static str = "frame data record is truncated";

    fn decode(record: &[[u8; 4]]) -> Self {
        let mut bytes = [0; FRAME_DATA_SIZE];
        for (to, from) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(record) {
            *to = *from;
        }
        Self::from_bytes(bytes)
    }
}

impl DebugRecord for FrameData {
    const SIZE: usize = FRAME_DATA_SIZE;

    fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        Self::encode(self, out)
    }
}

/// A decoded `FPO_DATA` stream (slot 0).
pub type FpoStream = RecordStream<FpoData>;

/// A decoded `FRAMEDATA` stream (slot 9).
///
/// The decoder does not read the `/names` stream. Use
/// [`crate::native::names::StringTable::get`] with [`FrameData::program`]
/// to get the frame program.
pub type FrameDataStream = RecordStream<FrameData>;
