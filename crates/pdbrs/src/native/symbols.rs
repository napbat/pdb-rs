//! The global and public symbol indexes.
//!
//! The DBI header names three streams for global symbols:
//!
//! - The symbol record stream holds `CodeView` symbol records without a header.
//! - The global symbol index ([`GlobalSymbolIndex`]) is a name hash table.
//! - The public symbol index ([`PublicSymbolIndex`]) is a name hash table of
//!   `S_PUB32` records, an address map, a thunk map and section offsets.
//!
//! Both indexes store [`SymbolOffset`] values, which are byte offsets in the
//! symbol record stream. The indexes do not store names. To find a name, get
//! the candidate records with [`HashTable::lookup`]. Then decode each
//! candidate record and compare its name with the name that you look for.
//!
//! The layouts follow `GSIHashHdr`, `HRFile` and `PSGSIHDR` in `PDB/dbi/gsi.h`
//! and `PDB/dbi/gsi.cpp`, and `SO` in `langapi/include/pdb.h`, of
//! `microsoft/microsoft-pdb` (commit `805655a`). They also follow LLVM
//! `GlobalsStream.cpp`, `PublicsStream.cpp` and `GSIStreamBuilder.cpp`, and
//! `docs/pdb/globals.md` of `microsoft/pdb-rs`.

use alloc::vec::Vec;
use core::ops::Range;

use super::codeview::{Record, Records};
use super::hash::hash_string_v1;
use super::msf::StreamId;
use crate::{Error, ErrorKind, Result};

mod gsi;
mod psi;
mod table;

pub use gsi::GlobalSymbolIndex;
pub use psi::{AddressMap, PublicHeader, PublicSymbolIndex, SectionStart, SectionStarts, ThunkMap};
pub use table::{Bucket, Buckets, HashHeader, HashRecord, HashRecords, HashTable};

/// The PDB Info stream feature code for minimal debug information (`MINI`).
///
/// A PDB with this feature uses [`BucketCount::Minimal`] for its symbol indexes.
pub const MINIMAL_DEBUG_INFO_FEATURE: u32 = 0x494e_494d;

/// The byte offset of a record in the symbol record stream.
///
/// The offset identifies the length field of the record. It is not an offset
/// in the index stream that stores it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SymbolOffset(pub u32);

impl SymbolOffset {
    /// Frame the record at this offset in the symbol record stream.
    ///
    /// The returned [`Record::offset`] is this offset. This function only
    /// frames the record. Use the symbol decoders to read its fields.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the offset is not inside `symbol_records`.
    /// - [`ErrorKind::UnexpectedEof`] or [`ErrorKind::InvalidFormat`] if the
    ///   record at the offset is truncated or malformed.
    ///
    /// The errors do not have a location.
    pub fn record(self, symbol_records: &[u8]) -> Result<Record<'_>> {
        let rest = usize::try_from(self.0)
            .ok()
            .and_then(|start| symbol_records.get(start..))
            .filter(|rest| !rest.is_empty())
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidReference, None)
                    .with_context("symbol offset is not inside the symbol record stream")
            })?;
        let mut record = Records::new(rest).next().ok_or_else(|| {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("symbol offset is not inside the symbol record stream")
        })??;
        record.offset = self.0;
        Ok(record)
    }
}

/// The number of hash buckets in a symbol index.
///
/// The streams do not store this value. The PDB Info stream selects it: a PDB
/// with [`MINIMAL_DEBUG_INFO_FEATURE`] uses [`BucketCount::Minimal`]. All
/// other PDBs use [`BucketCount::Standard`]. See `GSI1::GSI1` in
/// `PDB/dbi/gsi.cpp`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BucketCount {
    /// 4096 buckets (`0x1000`).
    Standard,
    /// 262143 buckets (`0x3ffff`), for PDBs with minimal debug information.
    Minimal,
}

impl BucketCount {
    /// Select the bucket count from the feature codes of the PDB Info stream.
    #[must_use]
    pub fn from_features<I: IntoIterator<Item = u32>>(features: I) -> Self {
        if features
            .into_iter()
            .any(|feature| feature == MINIMAL_DEBUG_INFO_FEATURE)
        {
            Self::Minimal
        } else {
            Self::Standard
        }
    }

    /// Return the number of buckets.
    #[must_use]
    pub const fn get(self) -> u32 {
        match self {
            Self::Standard => 0x1000,
            Self::Minimal => 0x3ffff,
        }
    }

    /// Return the bucket of a name: `(hash_string_v1(name) % count) & 0xffff`.
    ///
    /// The producer truncates the bucket to 16 bits: `GSI1::hashSz` in
    /// `PDB/dbi/gsi.cpp` returns `HASH`, which `PDB/include/pdbtypdefs.h`
    /// defines as `USHORT`. The truncation has no effect for
    /// [`BucketCount::Standard`]. For [`BucketCount::Minimal`], writers never
    /// use buckets `0x10000` and higher.
    #[must_use]
    pub fn bucket_of(self, name: &[u8]) -> u32 {
        (hash_string_v1(name) % self.get()) & 0xffff
    }
}

/// The sorted start offsets of all records in a symbol record stream.
///
/// [`HashTable::check_records`] and [`PublicSymbolIndex::check_records`] use
/// this value to make sure that each stored offset starts a record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordOffsets {
    /// The record start offsets in increasing order.
    offsets: Vec<u32>,
    /// The length of the symbol record stream in bytes.
    length: usize,
}

impl RecordOffsets {
    /// Frame all records of a symbol record stream and keep their offsets.
    ///
    /// The function uses only [`Records`] framing. It does not decode the
    /// records. `stream` identifies the symbol record stream in errors.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] or [`ErrorKind::InvalidFormat`] for a
    ///   truncated or malformed record.
    /// - [`ErrorKind::OffsetOverflow`] if a record offset does not fit in 32 bits.
    /// - [`ErrorKind::LimitExceeded`] if the offset list cannot be allocated.
    ///
    /// Errors identify `stream` and the logical offset of the record.
    pub fn frame(symbol_records: &[u8], stream: StreamId) -> Result<Self> {
        let mut offsets = Vec::new();
        let mut records = Records::new(symbol_records);
        while let Some(record) = records.next() {
            let record =
                record.map_err(|error| error.with_stream(stream.0, logical(records.position())))?;
            offsets.try_reserve(1).map_err(|_| {
                Error::new(ErrorKind::LimitExceeded, None)
                    .with_context("symbol record offsets cannot be allocated")
                    .with_stream(stream.0, u64::from(record.offset))
            })?;
            offsets.push(record.offset);
        }
        Ok(Self {
            offsets,
            length: symbol_records.len(),
        })
    }

    /// Return the number of records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    /// Return `true` if the symbol record stream has no records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Return the length of the symbol record stream in bytes.
    #[must_use]
    pub const fn stream_length(&self) -> usize {
        self.length
    }

    /// Return `true` if a record starts at `offset`.
    #[must_use]
    pub fn contains(&self, offset: SymbolOffset) -> bool {
        self.offsets.binary_search(&offset.0).is_ok()
    }

    /// Return the record start offsets in increasing order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = SymbolOffset> + '_ {
        self.offsets.iter().map(|&offset| SymbolOffset(offset))
    }

    /// Make sure that a record starts at `offset`.
    ///
    /// The error identifies `stream` and `location`, the logical offset of
    /// the stored reference in the index stream.
    fn check(&self, offset: SymbolOffset, stream: StreamId, location: usize) -> Result<()> {
        let inside = usize::try_from(offset.0).is_ok_and(|start| start < self.length);
        let context = if !inside {
            "symbol offset is not inside the symbol record stream"
        } else if self.contains(offset) {
            return Ok(());
        } else {
            "symbol offset is not at the start of a record"
        };
        Err(Error::new(ErrorKind::InvalidReference, None)
            .with_context(context)
            .with_stream(stream.0, logical(location)))
    }
}

/// Compute the range of `size` bytes at `start` that must end at or before `end`.
fn region(start: usize, size: u32, end: usize) -> core::result::Result<Range<usize>, ErrorKind> {
    let size = usize::try_from(size).map_err(|_| ErrorKind::LimitExceeded)?;
    let stop = start.checked_add(size).ok_or(ErrorKind::OffsetOverflow)?;
    if stop > end {
        return Err(ErrorKind::UnexpectedEof);
    }
    Ok(start..stop)
}

/// Create an error at a logical offset in an index stream.
fn index_error(kind: ErrorKind, stream: StreamId, offset: usize, context: &'static str) -> Error {
    Error::new(kind, None)
        .with_context(context)
        .with_stream(stream.0, logical(offset))
}

/// Convert a logical offset to the error representation.
fn logical(offset: usize) -> u64 {
    u64::try_from(offset).unwrap_or(u64::MAX)
}

/// Read a little-endian `u32` at `offset`. The caller checked the range.
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    let mut value = [0; 4];
    value.copy_from_slice(&bytes[offset..offset + 4]);
    u32::from_le_bytes(value)
}
