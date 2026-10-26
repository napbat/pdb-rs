//! The exception data stream (slot 1): `IMAGE_FUNCTION_ENTRY` records.
//!
//! `DbgFunc1` in `dbi.h` stores 12-byte `IMAGE_FUNCTION_ENTRY` records of
//! `winnt.h` and sorts them by `StartingAddress`. `pdb.h` marks the slot as
//! deprecated. The decoder does not check the order.

use alloc::vec::Vec;

use super::{Codec, DebugRecord, RecordStream};
use crate::Result;
use crate::native::output::Output;
use crate::native::words::word;

/// One `IMAGE_FUNCTION_ENTRY` record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct FunctionEntry {
    /// The address of the first byte of the function (`StartingAddress`).
    pub start: u32,
    /// The address after the last byte of the function (`EndingAddress`).
    pub end: u32,
    /// The address of the end of the prologue (`EndOfPrologue`).
    pub end_of_prologue: u32,
}

impl FunctionEntry {
    /// Make a record from its fields.
    #[must_use]
    pub const fn new(start: u32, end: u32, end_of_prologue: u32) -> Self {
        Self {
            start,
            end,
            end_of_prologue,
        }
    }
}

impl Codec for FunctionEntry {
    const TRUNCATED: &'static str = "function entry record is truncated";

    fn decode(record: &[[u8; 4]]) -> Self {
        Self {
            start: word(record, 0),
            end: word(record, 1),
            end_of_prologue: word(record, 2),
        }
    }
}

impl DebugRecord for FunctionEntry {
    const SIZE: usize = 12;

    fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut out = Output::new(out);
        out.reserve(Self::SIZE)?;
        out.u32(self.start)?;
        out.u32(self.end)?;
        out.u32(self.end_of_prologue)
    }
}

/// A decoded exception data stream (slot 1).
pub type FunctionEntryStream = RecordStream<FunctionEntry>;
