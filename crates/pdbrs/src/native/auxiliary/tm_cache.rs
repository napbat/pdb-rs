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
//!
//! [`TmCache`] keeps the table offsets and the stream size, so [`TmCache::encode`] gives the
//! same bytes again. [`TmCacheBuilder`] makes a new stream with the layout of `link.exe`.

use alloc::vec::Vec;
use core::iter::FusedIterator;

use crate::native::msf::StreamId;
use crate::native::output::{Output, no_memory, to_u32};
use crate::{Error, ErrorKind, Result};

/// The `/TMCache` version that `link.exe` writes (`0x20200229`).
pub const TM_CACHE_VERSION: u32 = 0x2020_0229;

/// The size of the `/TMCache` header.
const HEADER_SIZE: usize = 20;
/// The size of one entry.
const ENTRY_SIZE: usize = 16;

/// One entry of a `/TMCache` stream.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
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
    /// The offset of the stream table.
    stream_table_offset: usize,
    /// The offset of the entry table.
    entry_table_offset: usize,
    /// The size of the stream in bytes.
    size: usize,
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
                .with_stream(stream, u64::try_from(offset).unwrap_or(u64::MAX))
        };
        if bytes.is_empty() {
            return Ok(Self {
                streams: &[],
                entries: &[],
                stream_table_offset: 0,
                entry_table_offset: 0,
                size: 0,
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
            streams: bytes[table.clone()].as_chunks::<2>().0,
            entries: bytes[list.clone()].as_chunks::<ENTRY_SIZE>().0,
            stream_table_offset: table.start,
            entry_table_offset: list.start,
            size: bytes.len(),
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

    /// Return `true` if the stream is empty.
    ///
    /// MSVC `link.exe` writes an empty stream for a link without `/INCREMENTAL`.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Return the offsets of the stream table and the entry table.
    #[must_use]
    pub const fn table_offsets(&self) -> (usize, usize) {
        (self.stream_table_offset, self.entry_table_offset)
    }

    /// Return the size of the stream in bytes.
    #[must_use]
    pub const fn size(&self) -> usize {
        self.size
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

    /// Append the stream bytes to `out`.
    ///
    /// The function keeps the table offsets and the zero bytes around the tables, so a decoded
    /// stream gives the same bytes again. An empty stream gives no bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        if self.size == 0 {
            return Ok(());
        }
        let start = out.len();
        let mut output = Output::new(out);
        output.reserve(self.size)?;
        output.u32(TM_CACHE_VERSION)?;
        output.u32(to_u32(
            self.stream_table_offset,
            "/TMCache stream table offset",
        )?)?;
        output.u32(to_u32(
            self.streams.len() * 2,
            "/TMCache stream table size",
        )?)?;
        output.u32(to_u32(
            self.entry_table_offset,
            "/TMCache entry table offset",
        )?)?;
        output.u32(to_u32(
            self.entries.len() * ENTRY_SIZE,
            "/TMCache entry table size",
        )?)?;
        // `parse` checked that the tables are in order and inside the stream.
        pad_to(out, start, self.stream_table_offset)?;
        Output::new(out).bytes(self.streams.as_flattened())?;
        pad_to(out, start, self.entry_table_offset)?;
        Output::new(out).bytes(self.entries.as_flattened())?;
        pad_to(out, start, self.size)
    }
}

/// An owned `/TMCache` stream that grows and encodes with the layout of `link.exe`.
///
/// The layout is the 20-byte header, the stream table, zero bytes to the next multiple of 4,
/// and the entry table. The `msvc-x64-incremental` fixture has this layout.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct TmCacheBuilder {
    /// The stream table slots: stream numbers or `0xffff`.
    streams: Vec<u16>,
    /// The entries: a slot and a checksum.
    entries: Vec<(u16, u64)>,
}

impl TmCacheBuilder {
    /// Create a builder with no slots and no entries.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            streams: Vec::new(),
            entries: Vec::new(),
        }
    }

    /// Copy the slots and the entries of a decoded stream.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn from_cache(cache: &TmCache<'_>) -> Result<Self> {
        let mut builder = Self::new();
        builder
            .streams
            .try_reserve_exact(cache.streams.len())
            .map_err(|_| no_memory())?;
        builder
            .entries
            .try_reserve_exact(cache.entries.len())
            .map_err(|_| no_memory())?;
        builder
            .streams
            .extend(cache.streams.iter().map(|slot| u16::from_le_bytes(*slot)));
        builder
            .entries
            .extend(cache.entries().map(|entry| (entry.slot, entry.checksum)));
        Ok(builder)
    }

    /// Add a slot to the stream table and return its number.
    ///
    /// `None` makes a slot without a stream (`0xffff`).
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the stream number does not fit in a `u16`
    /// other than `0xffff`, if the table has `0xffff` slots, or if an allocation fails.
    pub fn push_stream(&mut self, stream: Option<StreamId>) -> Result<u16> {
        let value = match stream {
            None => u16::MAX,
            Some(id) => u16::try_from(id.0)
                .ok()
                .filter(|&value| value != u16::MAX)
                .ok_or_else(|| limit("/TMCache stream number does not fit in u16"))?,
        };
        let slot = u16::try_from(self.streams.len())
            .ok()
            .filter(|&slot| slot != u16::MAX)
            .ok_or_else(|| limit("/TMCache stream table is full"))?;
        self.streams.try_reserve(1).map_err(|_| no_memory())?;
        self.streams.push(value);
        Ok(slot)
    }

    /// Add an entry for the type section checksum `checksum` that uses slot `slot`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if `slot` is not in the stream table.
    /// - [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn push_entry(&mut self, slot: u16, checksum: u64) -> Result<()> {
        if usize::from(slot) >= self.streams.len() {
            return Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("/TMCache entry selects a slot past the stream table"));
        }
        self.entries.try_reserve(1).map_err(|_| no_memory())?;
        self.entries.push((slot, checksum));
        Ok(())
    }

    /// Append the stream bytes to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if a table size does not fit in `u32` or the output
    /// cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let table_end = self
            .streams
            .len()
            .checked_mul(2)
            .and_then(|size| size.checked_add(HEADER_SIZE))
            .ok_or_else(|| limit("/TMCache stream size"))?;
        let entry_offset = table_end
            .checked_next_multiple_of(4)
            .ok_or_else(|| limit("/TMCache stream size"))?;
        let entry_size = self
            .entries
            .len()
            .checked_mul(ENTRY_SIZE)
            .ok_or_else(|| limit("/TMCache entry table size"))?;
        let end = entry_offset
            .checked_add(entry_size)
            .ok_or_else(|| limit("/TMCache stream size"))?;
        to_u32(end, "/TMCache stream size")?;
        let mut output = Output::new(out);
        output.reserve(end)?;
        output.u32(TM_CACHE_VERSION)?;
        for value in [
            HEADER_SIZE,
            table_end - HEADER_SIZE,
            entry_offset,
            entry_size,
        ] {
            output.u32(to_u32(value, "/TMCache stream size")?)?;
        }
        for slot in &self.streams {
            output.u16(*slot)?;
        }
        output.zeros(entry_offset - table_end)?;
        for (slot, checksum) in &self.entries {
            output.u16(*slot)?;
            output.zeros(6)?;
            output.u64(*checksum)?;
        }
        Ok(())
    }
}

/// Return a [`ErrorKind::LimitExceeded`] error with `context`.
fn limit(context: &'static str) -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context(context)
}

/// Append zero bytes until `out` holds `offset` bytes after `start`.
fn pad_to(out: &mut Vec<u8>, start: usize, offset: usize) -> Result<()> {
    let target = start.saturating_add(offset);
    let count = target.saturating_sub(out.len());
    Output::new(out).zeros(count)
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
