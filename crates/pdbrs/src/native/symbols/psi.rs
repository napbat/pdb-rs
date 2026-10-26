//! The public symbol index stream.
//!
//! The stream has five parts:
//!
//! 1. A 28-byte header (`PSGSIHDR`).
//! 2. A name hash table of the `S_PUB32` records, `hash_size` bytes long.
//! 3. The address map, `address_map_size` bytes long: one symbol offset for
//!    each public symbol, sorted by section and then by offset.
//! 4. The thunk map: `thunk_count` 32-bit thunk target addresses.
//! 5. The section offsets (`SO`): `section_count` 8-byte entries.
//!
//! `PSGSI1::writeStream` in `PDB/dbi/gsi.cpp` writes the thunk map and the
//! section offsets that `PSGSI1::addThunkMap` set. The `PSGSIHDR` constructor does
//! not set `nSects`, so `section_count` is not reliable when the section
//! offsets are absent. Like LLVM `PublicsStream.cpp`, this module reads the
//! section offsets only if bytes follow the thunk map.

use alloc::vec::Vec;
use core::iter::FusedIterator;
use core::ops::Range;
use core::slice::Iter;

use super::table::{HashTable, Layout};
use super::{BucketCount, RecordOffsets, SymbolOffset, index_error, region, u32_at};
use crate::native::msf::StreamId;
use crate::{ErrorKind, Result};

/// The size of the public symbol index header.
const HEADER_SIZE: usize = 28;
/// The size of one section offset entry.
const SECTION_OFFSET_SIZE: usize = 8;

/// The decoded fields of a public symbol index header (`PSGSIHDR`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PublicHeader {
    /// The size of the name hash table in bytes.
    pub hash_size: u32,
    /// The size of the address map in bytes.
    pub address_map_size: u32,
    /// The number of thunks and of thunk map entries.
    pub thunk_count: u32,
    /// The size of one thunk in the image, in bytes.
    pub thunk_size: u32,
    /// The section number of the thunk table in the image.
    pub thunk_table_section: u16,
    /// The two bytes after `thunk_table_section`.
    ///
    /// They are alignment padding of the C structure. Writers do not always
    /// set them, so this crate does not check their value.
    pub padding: [u8; 2],
    /// The offset of the thunk table in its section.
    pub thunk_table_offset: u32,
    /// The number of section offset entries.
    ///
    /// This value is not reliable when the stream has no section offsets.
    /// See [`PublicSymbolIndex::section_offsets`].
    pub section_count: u32,
}

/// A decoded public symbol index stream with owned serialized bytes.
#[derive(Clone, Debug)]
pub struct PublicSymbolIndex {
    /// The stream bytes.
    bytes: Vec<u8>,
    /// The stream ID for error locations.
    stream: StreamId,
    /// The decoded header.
    header: PublicHeader,
    /// The validated hash table.
    table: Layout,
    /// The address map in the stream.
    address_map: Range<usize>,
    /// The thunk map in the stream.
    thunk_map: Range<usize>,
    /// The section offsets in the stream. The range is empty if they are absent.
    section_offsets: Range<usize>,
}

impl PublicSymbolIndex {
    /// Decode and check a public symbol index stream.
    ///
    /// Get `stream` from [`crate::native::dbi::DbiHeader::public_stream`]. Get
    /// `buckets` from the PDB Info stream with [`BucketCount::from_features`].
    ///
    /// The function checks the header, the hash table and the sizes of all
    /// parts. The parts must fill the stream exactly. The function does not
    /// read the symbol record stream; use [`PublicSymbolIndex::check_records`]
    /// and [`PublicSymbolIndex::check_address_order`] for that.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] if the header or a part is truncated.
    /// - [`ErrorKind::UnsupportedVersion`] if the hash header does not have
    ///   the compressed-bucket signature and version.
    /// - [`ErrorKind::InvalidReference`] for a hash record offset that is not
    ///   a positive 32-bit value.
    /// - [`ErrorKind::InvalidFormat`] for an invalid hash table, an address
    ///   map size that is not a multiple of 4, or section offsets whose size
    ///   does not match `section_count`.
    /// - [`ErrorKind::LimitExceeded`] or [`ErrorKind::OffsetOverflow`] if a
    ///   size cannot be represented.
    ///
    /// Errors identify `stream` and a logical offset in it.
    pub fn parse(bytes: Vec<u8>, stream: StreamId, buckets: BucketCount) -> Result<Self> {
        let error = |kind, offset, context| index_error(kind, stream, offset, context);
        if bytes.len() < HEADER_SIZE {
            return Err(error(
                ErrorKind::UnexpectedEof,
                bytes.len(),
                "public symbol index header is truncated",
            ));
        }
        let section = u32_at(&bytes, 16).to_le_bytes();
        let header = PublicHeader {
            hash_size: u32_at(&bytes, 0),
            address_map_size: u32_at(&bytes, 4),
            thunk_count: u32_at(&bytes, 8),
            thunk_size: u32_at(&bytes, 12),
            thunk_table_section: u16::from_le_bytes([section[0], section[1]]),
            padding: [section[2], section[3]],
            thunk_table_offset: u32_at(&bytes, 20),
            section_count: u32_at(&bytes, 24),
        };
        let end = bytes.len();
        let hash = region(HEADER_SIZE, header.hash_size, end)
            .map_err(|kind| error(kind, 0, "public symbol hash table extends past the stream"))?;
        let table = Layout::parse(&bytes, hash.clone(), stream, buckets)?;
        if !header.address_map_size.is_multiple_of(4) {
            return Err(error(
                ErrorKind::InvalidFormat,
                4,
                "public address map size is not a multiple of 4",
            ));
        }
        let address_map = region(hash.end, header.address_map_size, end)
            .map_err(|kind| error(kind, 4, "public address map extends past the stream"))?;
        let thunk_bytes = header.thunk_count.checked_mul(4).ok_or_else(|| {
            error(
                ErrorKind::OffsetOverflow,
                8,
                "public thunk map size overflows",
            )
        })?;
        let thunk_map = region(address_map.end, thunk_bytes, end)
            .map_err(|kind| error(kind, 8, "public thunk map extends past the stream"))?;
        let section_offsets = if thunk_map.end == end {
            end..end
        } else {
            section_range(thunk_map.end, end, header.section_count, stream)?
        };
        Ok(Self {
            bytes,
            stream,
            header,
            table,
            address_map,
            thunk_map,
            section_offsets,
        })
    }

    /// Return the decoded header.
    #[must_use]
    pub const fn header(&self) -> &PublicHeader {
        &self.header
    }

    /// Return the name hash table of the `S_PUB32` records.
    #[must_use]
    pub const fn hash_table(&self) -> HashTable<'_> {
        self.table.view(self.bytes.as_slice())
    }

    /// Return the address map in file order.
    ///
    /// A valid map is sorted by the section and then by the offset of each
    /// `S_PUB32` record. Use [`PublicSymbolIndex::check_address_order`] to
    /// check the order.
    #[must_use]
    pub fn address_map(&self) -> AddressMap<'_> {
        AddressMap {
            chunks: self.bytes[self.address_map.clone()].as_chunks().0.iter(),
        }
    }

    /// Return the thunk map in file order.
    ///
    /// Entry `i` is the target of thunk `i`, as a flat address. The section
    /// offsets map a flat address to a section and an offset.
    #[must_use]
    pub fn thunk_map(&self) -> ThunkMap<'_> {
        ThunkMap {
            chunks: self.bytes[self.thunk_map.clone()].as_chunks().0.iter(),
        }
    }

    /// Return the section offsets in file order.
    ///
    /// The result is empty if no bytes follow the thunk map, also when
    /// [`PublicHeader::section_count`] is not zero.
    #[must_use]
    pub fn section_offsets(&self) -> SectionOffsets<'_> {
        SectionOffsets {
            chunks: self.bytes[self.section_offsets.clone()]
                .as_chunks()
                .0
                .iter(),
        }
    }

    /// Make sure that the hash records and the address map identify records.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] for the first offset that is
    /// not inside the symbol record stream or not at the start of a record.
    /// The error identifies the index stream and the logical offset of the
    /// stored reference.
    pub fn check_records(&self, offsets: &RecordOffsets) -> Result<()> {
        self.hash_table().check_records(offsets)?;
        let start = self.address_map.start;
        for (index, symbol) in self.address_map().enumerate() {
            offsets.check(symbol, self.stream, start + index * 4)?;
        }
        Ok(())
    }

    /// Make sure that the address map is sorted by section and offset.
    ///
    /// `address` returns the section and the offset of the `S_PUB32` record
    /// at a symbol offset. The caller decodes the record, so this module does
    /// not depend on a symbol decoder. Equal addresses are valid.
    ///
    /// # Errors
    ///
    /// - Errors from `address` without a change.
    /// - [`ErrorKind::InvalidFormat`] for the first entry whose address is
    ///   less than the address of the entry before it. The error identifies
    ///   the index stream and the logical offset of the entry.
    pub fn check_address_order<F>(&self, mut address: F) -> Result<()>
    where
        F: FnMut(SymbolOffset) -> Result<(u16, u32)>,
    {
        let start = self.address_map.start;
        let mut previous = None;
        for (index, symbol) in self.address_map().enumerate() {
            let current = address(symbol)?;
            if previous.is_some_and(|last| current < last) {
                return Err(index_error(
                    ErrorKind::InvalidFormat,
                    self.stream,
                    start + index * 4,
                    "public address map is not sorted by section and offset",
                ));
            }
            previous = Some(current);
        }
        Ok(())
    }
}

/// Compute the range of the section offsets from `start` to the stream end.
fn section_range(start: usize, end: usize, count: u32, stream: StreamId) -> Result<Range<usize>> {
    let size = usize::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(SECTION_OFFSET_SIZE));
    if size != Some(end - start) {
        return Err(index_error(
            ErrorKind::InvalidFormat,
            stream,
            start,
            "public section offset size does not match the section count",
        ));
    }
    Ok(start..end)
}

/// An iterator over the public address map.
#[derive(Clone, Debug)]
pub struct AddressMap<'a> {
    /// The remaining 4-byte entries.
    chunks: Iter<'a, [u8; 4]>,
}

impl Iterator for AddressMap<'_> {
    type Item = SymbolOffset;

    fn next(&mut self) -> Option<Self::Item> {
        self.chunks
            .next()
            .map(|&chunk| SymbolOffset(u32::from_le_bytes(chunk)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.chunks.size_hint()
    }
}

impl DoubleEndedIterator for AddressMap<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.chunks
            .next_back()
            .map(|&chunk| SymbolOffset(u32::from_le_bytes(chunk)))
    }
}

impl ExactSizeIterator for AddressMap<'_> {}

impl FusedIterator for AddressMap<'_> {}

/// An iterator over the thunk map. Each item is a flat thunk target address.
#[derive(Clone, Debug)]
pub struct ThunkMap<'a> {
    /// The remaining 4-byte entries.
    chunks: Iter<'a, [u8; 4]>,
}

impl Iterator for ThunkMap<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        self.chunks.next().map(|&chunk| u32::from_le_bytes(chunk))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.chunks.size_hint()
    }
}

impl DoubleEndedIterator for ThunkMap<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.chunks
            .next_back()
            .map(|&chunk| u32::from_le_bytes(chunk))
    }
}

impl ExactSizeIterator for ThunkMap<'_> {}

impl FusedIterator for ThunkMap<'_> {}

/// One section offset entry (`SO`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SectionOffset {
    /// The flat address where the section starts.
    pub offset: u32,
    /// The section number.
    pub section: u16,
    /// The `pad` field of `SO`.
    ///
    /// Writers do not use it. This crate does not check its value.
    pub padding: [u8; 2],
}

/// An iterator over the section offset entries.
#[derive(Clone, Debug)]
pub struct SectionOffsets<'a> {
    /// The remaining 8-byte entries.
    chunks: Iter<'a, [u8; SECTION_OFFSET_SIZE]>,
}

impl SectionOffsets<'_> {
    /// Decode one entry.
    fn decode(chunk: [u8; SECTION_OFFSET_SIZE]) -> SectionOffset {
        let [o0, o1, o2, o3, s0, s1, p0, p1] = chunk;
        SectionOffset {
            offset: u32::from_le_bytes([o0, o1, o2, o3]),
            section: u16::from_le_bytes([s0, s1]),
            padding: [p0, p1],
        }
    }
}

impl Iterator for SectionOffsets<'_> {
    type Item = SectionOffset;

    fn next(&mut self) -> Option<Self::Item> {
        self.chunks.next().copied().map(Self::decode)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.chunks.size_hint()
    }
}

impl DoubleEndedIterator for SectionOffsets<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.chunks.next_back().copied().map(Self::decode)
    }
}

impl ExactSizeIterator for SectionOffsets<'_> {}

impl FusedIterator for SectionOffsets<'_> {}
