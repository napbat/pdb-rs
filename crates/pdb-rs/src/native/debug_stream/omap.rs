//! The OMAP streams (slots 3 and 4): `OMAP_DATA` records.
//!
//! A tool that rewrites an image after the link (for example a basic block
//! optimizer) moves code and data. The PDB keeps the addresses of the original
//! image. Two tables map between the two address spaces:
//!
//! - The [`DebugStream::OmapToSource`] slot maps image RVAs to original RVAs.
//! - The [`DebugStream::OmapFromSource`] slot maps original RVAs to image RVAs.
//!
//! Each table is an array of 8-byte `OMAP_DATA` records (`rva`, `rvaTo`) of
//! `dbi.h`, sorted by `rva`. `DbgOmap1` sorts the records and DIA finds them
//! with a binary search. [`OmapStream::lookup`] has the semantics of DIA
//! (`msdia140.dll`, checked with `DIA2Dump -maptosrc` and `-mapfromsrc`):
//!
//! 1. Find the last record with `rva <= address`.
//! 2. If no record has `rva <= address`, the address has no mapping.
//! 3. If the record is the last record and `address` is not its `rva`, the
//!    address has no mapping. The last record ends the table.
//! 4. If the record has `rvaTo == 0`, the address has no mapping.
//! 5. Otherwise the result is `rvaTo + (address - rva)`.
//!
//! `OMAPTable::lookup` of the `pdb` crate differs in step 3: it maps all
//! addresses after the last record with that record.
//!
//! [`DebugStream::OmapToSource`]: crate::native::dbi::DebugStream::OmapToSource
//! [`DebugStream::OmapFromSource`]: crate::native::dbi::DebugStream::OmapFromSource

use alloc::vec::Vec;

use super::{Codec, DebugRecord, RecordIter, RecordStream};
use crate::native::address::{Translation, Unmapped};
use crate::native::msf::StreamId;
use crate::native::output::Output;
use crate::native::words::word;
use crate::{Error, ErrorKind, Result};

/// The size of one `OMAP_DATA` record.
pub const OMAP_RECORD_SIZE: usize = 8;

/// One `OMAP_DATA` record.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct OmapRecord {
    /// The first source address of the range (`rva`).
    pub rva: u32,
    /// The target address of `rva` (`rvaTo`). The value 0 marks a range without a mapping.
    pub rva_to: u32,
}

impl Codec for OmapRecord {
    const TRUNCATED: &'static str = "OMAP record is truncated";

    fn decode(record: &[[u8; 4]]) -> Self {
        Self {
            rva: word(record, 0),
            rva_to: word(record, 1),
        }
    }
}

impl DebugRecord for OmapRecord {
    const SIZE: usize = OMAP_RECORD_SIZE;

    fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut out = Output::new(out);
        out.reserve(Self::SIZE)?;
        out.u32(self.rva)?;
        out.u32(self.rva_to)
    }
}

/// A decoded OMAP table, sorted by source address.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct OmapStream {
    /// The records in ascending `rva` order. Equal `rva` values are permitted.
    records: RecordStream<OmapRecord>,
}

impl OmapStream {
    /// Decode the OMAP stream `stream`.
    ///
    /// An empty stream is a table without records. It maps no address.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] if the length is not a multiple of
    ///   [`OMAP_RECORD_SIZE`].
    /// - [`ErrorKind::InvalidFormat`] if a record has a smaller `rva` than the
    ///   record before it.
    ///
    /// The error identifies `stream` and the offset of the bad record.
    pub fn parse(bytes: Vec<u8>, stream: StreamId) -> Result<Self> {
        let records = RecordStream::parse(bytes, stream)?;
        if let Some(index) = first_unsorted(records.iter()) {
            let offset = u64::try_from(index * OMAP_RECORD_SIZE).unwrap_or(u64::MAX);
            return Err(unsorted().with_stream(stream, offset));
        }
        Ok(Self { records })
    }

    /// Encode `records` into a new table.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if a record has a smaller `rva` than the
    ///   record before it.
    /// - [`ErrorKind::LimitExceeded`] if the output cannot be allocated.
    pub fn from_records(records: &[OmapRecord]) -> Result<Self> {
        if first_unsorted(records.iter().copied()).is_some() {
            return Err(unsorted());
        }
        Ok(Self {
            records: RecordStream::from_records(records)?,
        })
    }

    /// Return the number of records.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.records.len()
    }

    /// Return `true` if the table has no records.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Return an iterator over the records in ascending `rva` order.
    #[must_use]
    pub fn iter(&self) -> RecordIter<'_, OmapRecord> {
        self.records.iter()
    }

    /// Return the record array.
    #[must_use]
    pub const fn records(&self) -> &RecordStream<OmapRecord> {
        &self.records
    }

    /// Return the record that maps `address`: the last record with `rva <= address`.
    #[must_use]
    pub fn record_for(&self, address: u32) -> Option<OmapRecord> {
        self.records.get(self.position(address).checked_sub(1)?)
    }

    /// Map `address` with the table.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::OffsetOverflow`] if the mapped address does not
    /// fit in 32 bits.
    pub fn lookup(&self, address: u32) -> Result<Translation<u32>> {
        let count = self.position(address);
        let Some(record) = count
            .checked_sub(1)
            .and_then(|index| self.records.get(index))
        else {
            return Ok(Translation::Unmapped(Unmapped::BeforeFirstRecord));
        };
        if count == self.records.len() && address != record.rva {
            return Ok(Translation::Unmapped(Unmapped::AfterLastRecord(record)));
        }
        if record.rva_to == 0 {
            return Ok(Translation::Unmapped(Unmapped::NoTarget(record)));
        }
        record
            .rva_to
            .checked_add(address - record.rva)
            .map(Translation::Mapped)
            .ok_or_else(|| {
                Error::new(ErrorKind::OffsetOverflow, None)
                    .with_context("OMAP target address overflows 32 bits")
            })
    }

    /// Return the number of records with `rva <= address`.
    fn position(&self, address: u32) -> usize {
        let chunks = self.records.as_bytes().as_chunks::<OMAP_RECORD_SIZE>().0;
        chunks.partition_point(|chunk| OmapRecord::decode(chunk.as_chunks::<4>().0).rva <= address)
    }

    /// Append the table to `out`.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`RecordStream::encode`].
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        self.records.encode(out)
    }

    /// Return the encoded table as a new vector.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`RecordStream::encode`].
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.records.to_bytes()
    }
}

impl<'a> IntoIterator for &'a OmapStream {
    type Item = OmapRecord;
    type IntoIter = RecordIter<'a, OmapRecord>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Return the index of the first record with a smaller `rva` than the record before it.
fn first_unsorted(records: impl Iterator<Item = OmapRecord>) -> Option<usize> {
    let mut previous = 0;
    for (index, record) in records.enumerate() {
        if record.rva < previous {
            return Some(index);
        }
        previous = record.rva;
    }
    None
}

/// Return the error for an OMAP table that is not sorted.
fn unsorted() -> Error {
    Error::new(ErrorKind::InvalidFormat, None).with_context("OMAP records are not sorted by rva")
}
