//! The `/TMCache` stream.
//!
//! Microsoft `link.exe` writes this stream in incremental links. It caches the type maps
//! (`TM` in `PDB/dbi/tm.h` of `microsoft/microsoft-pdb`) that map the type indices of an input
//! to the type indices of the PDB. The public Microsoft PDB sources, LLVM and DIA do not define
//! or read the stream.
//!
//! The layout comes from `DBI1::DumpTMCache` and `DBI1::fUpdateTMCache` in `mspdbcore.dll` of
//! MSVC 14.51, and agrees with `PascalBeyer/PDB-Documentation`. The decision and its evidence
//! are recorded in issue #13. All values are little-endian:
//!
//! | Offset | Field |
//! | --- | --- |
//! | 0 | Version, [`TM_CACHE_VERSION`] |
//! | 4 | Offset of the stream table |
//! | 8 | Size of the stream table in bytes |
//! | 12 | Offset of the entry table |
//! | 16 | Size of the entry table in bytes |
//!
//! The stream table is an array of `u16` stream numbers; `0xffff` marks a slot without a
//! stream. Each named stream is a type map stream; see [`TypeMapStream`](super::TypeMapStream).
//! An entry (16 bytes) has a `u16` slot in the stream table, 6 zero bytes and the 64-bit type
//! section checksum of an input object file. Two inputs with the same checksum share one entry.
//! The PDB does not contain the input of the checksum, so this reader does not verify it.

use core::iter::FusedIterator;

use crate::native::msf::StreamId;
use crate::{Error, ErrorKind, Result};

/// The `/TMCache` version that `link.exe` writes (`0x20200229`).
pub const TM_CACHE_VERSION: u32 = 0x2020_0229;

/// The size of the `/TMCache` header.
const HEADER_SIZE: usize = 20;
/// The size of one entry.
const ENTRY_SIZE: usize = 16;

/// One entry of a `/TMCache` stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct TmCacheEntry {
    /// The index of the entry in the entry table.
    pub index: usize,
    /// The slot in the stream table.
    pub slot: u16,
    /// The type map stream in the slot, if the slot names one.
    pub stream: Option<StreamId>,
    /// The type section checksum of the input object files.
    pub checksum: u64,
}

/// A decoded `/TMCache` stream.
///
/// An empty stream has no slots and no entries.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TmCache<'a> {
    /// The stream table.
    streams: &'a [[u8; 2]],
    /// The entry table.
    entries: &'a [[u8; ENTRY_SIZE]],
}

impl<'a> TmCache<'a> {
    /// Decode the `/TMCache` stream `stream`.
    ///
    /// The function accepts an empty stream.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] if the header, the stream table or the entry table is
    ///   truncated.
    /// - [`ErrorKind::UnsupportedVersion`] for a version other than [`TM_CACHE_VERSION`], or an
    ///   entry with non-zero reserved bytes.
    /// - [`ErrorKind::InvalidFormat`] if a table overlaps the header or the other table, if a
    ///   table size is not a multiple of its item size, or if bytes outside the tables are not
    ///   zero.
    /// - [`ErrorKind::InvalidReference`] if an entry selects a slot past the stream table.
    ///
    /// Errors identify `stream` and a logical offset in it.
    pub fn parse(bytes: &'a [u8], stream: StreamId) -> Result<Self> {
        let error = |kind, offset: usize, context| {
            Error::new(kind, None)
                .with_context(context)
                .with_stream(stream.0, u64::try_from(offset).unwrap_or(u64::MAX))
        };
        if bytes.is_empty() {
            return Ok(Self {
                streams: &[],
                entries: &[],
            });
        }
        let Some(header) = bytes.first_chunk::<HEADER_SIZE>() else {
            return Err(error(
                ErrorKind::UnexpectedEof,
                bytes.len(),
                "/TMCache header is truncated",
            ));
        };
        let field = |index: usize| {
            let start = index * 4;
            u32::from_le_bytes([
                header[start],
                header[start + 1],
                header[start + 2],
                header[start + 3],
            ])
        };
        if field(0) != TM_CACHE_VERSION {
            return Err(error(
                ErrorKind::UnsupportedVersion,
                0,
                "/TMCache version is not 0x20200229",
            ));
        }
        let table = region(bytes.len(), field(1), field(2)).ok_or_else(|| {
            error(
                ErrorKind::UnexpectedEof,
                4,
                "/TMCache stream table is truncated",
            )
        })?;
        let list = region(bytes.len(), field(3), field(4)).ok_or_else(|| {
            error(
                ErrorKind::UnexpectedEof,
                12,
                "/TMCache entry table is truncated",
            )
        })?;
        if !table.len().is_multiple_of(2) || !list.len().is_multiple_of(ENTRY_SIZE) {
            return Err(error(
                ErrorKind::InvalidFormat,
                8,
                "/TMCache table size is not a multiple of its item size",
            ));
        }
        if table.start < HEADER_SIZE || list.start < table.end {
            return Err(error(
                ErrorKind::InvalidFormat,
                4,
                "/TMCache tables overlap or are out of order",
            ));
        }
        for gap in [
            HEADER_SIZE..table.start,
            table.end..list.start,
            list.end..bytes.len(),
        ] {
            if let Some(position) = bytes[gap.clone()].iter().position(|&byte| byte != 0) {
                return Err(error(
                    ErrorKind::InvalidFormat,
                    gap.start + position,
                    "/TMCache has non-zero bytes outside its tables",
                ));
            }
        }
        let cache = Self {
            streams: bytes[table].as_chunks::<2>().0,
            entries: bytes[list.clone()].as_chunks::<ENTRY_SIZE>().0,
        };
        for (index, entry) in cache.entries.iter().enumerate() {
            let offset = list.start + index * ENTRY_SIZE;
            if entry[2..8].iter().any(|&byte| byte != 0) {
                return Err(error(
                    ErrorKind::UnsupportedVersion,
                    offset + 2,
                    "/TMCache entry has non-zero reserved bytes",
                ));
            }
            if usize::from(u16::from_le_bytes([entry[0], entry[1]])) >= cache.streams.len() {
                return Err(error(
                    ErrorKind::InvalidReference,
                    offset,
                    "/TMCache entry selects a slot past the stream table",
                ));
            }
        }
        Ok(cache)
    }

    /// Return `true` if the stream has no slots and no entries.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.streams.is_empty() && self.entries.is_empty()
    }

    /// Return the stream of each slot of the stream table, in slot order.
    ///
    /// A slot can be `None` (`0xffff`). Two slots can name the same stream.
    pub fn streams(&self) -> impl ExactSizeIterator<Item = Option<StreamId>> + FusedIterator + 'a {
        self.streams.iter().map(|&slot| slot_stream(slot))
    }

    /// Return the entries in entry table order.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = TmCacheEntry> + FusedIterator + 'a {
        let streams = self.streams;
        self.entries.iter().enumerate().map(move |(index, entry)| {
            let [s0, s1, _, _, _, _, _, _, c @ ..] = *entry;
            let slot = u16::from_le_bytes([s0, s1]);
            TmCacheEntry {
                index,
                slot,
                // `parse` checked that the slot exists.
                stream: streams
                    .get(usize::from(slot))
                    .copied()
                    .and_then(slot_stream),
                checksum: u64::from_le_bytes(c),
            }
        })
    }
}

/// Convert a stream table slot to a stream ID.
fn slot_stream(slot: [u8; 2]) -> Option<StreamId> {
    match u16::from_le_bytes(slot) {
        u16::MAX => None,
        value => Some(StreamId(u32::from(value))),
    }
}

/// Return the range of `size` bytes at `offset` if it ends at or before `end`.
fn region(end: usize, offset: u32, size: u32) -> Option<core::ops::Range<usize>> {
    let start = usize::try_from(offset).ok()?;
    let stop = start.checked_add(usize::try_from(size).ok()?)?;
    (stop <= end).then_some(start..stop)
}
