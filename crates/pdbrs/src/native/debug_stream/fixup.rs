//! The fixup stream (slot 2): `XFIXUP_DATA` records.
//!
//! `DbgXFixup1` in `dbi.h` stores 12-byte `XFIXUP_DATA` records of
//! `cvinfo.h`: a `u16` type, a `u16` extra value, the RVA of the fixup and the
//! RVA of its target. `link.exe /DEBUGTYPE:FIXUP` writes the stream. The type
//! is a COFF relocation type of the image machine, for example
//! `IMAGE_REL_AMD64_ADDR64` (1) and `IMAGE_REL_AMD64_REL32` (4) of `winnt.h`.

use alloc::vec::Vec;

use super::{Codec, DebugRecord, RecordStream};
use crate::Result;
use crate::native::output::Output;
use crate::native::words::{halves, word};

/// One `XFIXUP_DATA` record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Fixup {
    /// The fixup type (`wType`): a COFF relocation type.
    pub kind: u16,
    /// The extra value (`wExtra`). The PDB sources do not give its meaning.
    /// An x64 `link.exe` sample has 0, `0x2000`, `0x4000` and `0x8000`.
    pub extra: u16,
    /// The RVA of the fixup (`rva`).
    pub rva: u32,
    /// The RVA of the fixup target (`rvaTarget`).
    pub target: u32,
}

impl Fixup {
    /// Make a record from its fields.
    #[must_use]
    pub const fn new(kind: u16, extra: u16, rva: u32, target: u32) -> Self {
        Self {
            kind,
            extra,
            rva,
            target,
        }
    }
}

impl Codec for Fixup {
    const TRUNCATED: &'static str = "fixup record is truncated";

    fn decode(record: &[[u8; 4]]) -> Self {
        let (kind, extra) = halves(record, 0);
        Self {
            kind,
            extra,
            rva: word(record, 1),
            target: word(record, 2),
        }
    }
}

impl DebugRecord for Fixup {
    const SIZE: usize = 12;

    fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut out = Output::new(out);
        out.reserve(Self::SIZE)?;
        out.u16(self.kind)?;
        out.u16(self.extra)?;
        out.u32(self.rva)?;
        out.u32(self.target)
    }
}

/// A decoded fixup stream (slot 2).
pub type FixupStream = RecordStream<Fixup>;
