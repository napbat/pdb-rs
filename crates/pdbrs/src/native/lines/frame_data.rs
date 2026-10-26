//! `DEBUG_S_FRAMEDATA` subsections: x86 frame data of an object file.
//!
//! The data is an optional `u32` relocation value and an array of 32-byte
//! `FRAMEDATA` records ([`FrameData`]). `cvdump` (`DumpModFramedata`) and
//! `Mod1::processC13` in `PDB/dbi/mod.cpp` always read the relocation
//! value. LLVM `DebugFrameDataSubsectionRef` reads it only if the size is
//! not a multiple of 32. Both rules give the same result for MSVC data,
//! because 4 plus a multiple of 32 is never a multiple of 32. `link.exe`
//! moves the records to the new FPO stream of the DBI stream.

use alloc::vec::Vec;

use super::{entry_list, invalid, put_u32};
use crate::Result;
use crate::native::fpo::{FRAME_DATA_SIZE, FrameData};

/// The size of the relocation value.
const RELOCATION_SIZE: usize = 4;

entry_list!(
    /// The `FRAMEDATA` records of a [`FrameDataSubsection`].
    FrameDataRecords,
    /// An iterator over [`FrameDataRecords`].
    FrameDataRecordsIter,
    FrameData,
    FRAME_DATA_SIZE,
    FrameData::from_bytes
);

/// A decoded `DEBUG_S_FRAMEDATA` subsection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FrameDataSubsection<'a> {
    /// The relocation value before the records, if present.
    relocation: Option<u32>,
    /// The records.
    records: FrameDataRecords<'a>,
}

impl<'a> FrameDataSubsection<'a> {
    /// Decode the data of a `DEBUG_S_FRAMEDATA` subsection.
    ///
    /// A size of 4 plus a multiple of 32 has the relocation value. A
    /// multiple of 32 has no relocation value.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat)
    /// for another size. The error does not have a file offset or a stream
    /// location.
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        let (relocation, records) = match data.len() % FRAME_DATA_SIZE {
            0 => (None, data),
            RELOCATION_SIZE => {
                let (value, records) = data
                    .split_first_chunk::<RELOCATION_SIZE>()
                    .ok_or_else(|| invalid("frame data relocation value is truncated"))?;
                (Some(u32::from_le_bytes(*value)), records)
            }
            _ => return Err(invalid("frame data size is not a multiple of 32 bytes")),
        };
        Ok(Self {
            relocation,
            records: FrameDataRecords::new(records),
        })
    }

    /// Return the relocation value (`RVACon` in `cvdump`), if present.
    ///
    /// In an object file, a relocation sets the value to the address of the
    /// code. `Mod1::processC13` applies it to the records.
    #[must_use]
    pub const fn relocation(&self) -> Option<u32> {
        self.relocation
    }

    /// Return the records.
    #[must_use]
    pub const fn records(&self) -> FrameDataRecords<'a> {
        self.records
    }

    /// Append the subsection data to `out`.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`FrameDataSubsection::encode_parts`].
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        if let Some(relocation) = self.relocation {
            put_u32(out, relocation)?;
        }
        for record in self.records {
            record.encode(out)?;
        }
        Ok(())
    }

    /// Append the data of a `DEBUG_S_FRAMEDATA` subsection to `out`: the
    /// optional relocation value and the records.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded)
    /// if the output cannot grow.
    pub fn encode_parts(
        relocation: Option<u32>,
        records: &[FrameData],
        out: &mut Vec<u8>,
    ) -> Result<()> {
        if let Some(relocation) = relocation {
            put_u32(out, relocation)?;
        }
        for record in records {
            record.encode(out)?;
        }
        Ok(())
    }
}
