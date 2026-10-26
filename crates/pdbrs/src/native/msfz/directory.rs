//! The Chunk Table and the stream directory.

use alloc::vec::Vec;

use super::header::{Compression, Header};
use super::{CHUNK_ENTRY_SIZE, MsfzLimits, no_memory};
use crate::compression::{deflate, zstd};
use crate::io::ReadAt;
use crate::{Error, ErrorKind, Result};

/// The stream directory value that marks a nil stream.
pub(super) const NIL_STREAM_SIZE: u32 = u32::MAX;

/// Bit 63 of a fragment location: the fragment is in the chunks.
pub(super) const COMPRESSED_BIT: u64 = 1 << 63;

/// Bits 48 to 62 of an uncompressed fragment location. They are reserved and must be 0.
const RESERVED_BITS: u64 = 0x7fff << 48;

/// Bits 0 to 47 of an uncompressed fragment location: the file offset.
const FILE_OFFSET_BITS: u64 = (1 << 48) - 1;

/// One entry of the Chunk Table.
#[derive(Clone, Copy, Debug)]
pub(super) struct Chunk {
    /// The file offset of the stored bytes.
    pub(super) offset: u64,
    /// The compression of the stored bytes.
    pub(super) compression: Compression,
    /// The size of the stored bytes.
    pub(super) stored_size: u32,
    /// The size of the decompressed bytes.
    pub(super) size: u32,
    /// The offset of the decompressed bytes in the virtual byte array of all chunks.
    pub(super) start: u64,
}

impl Chunk {
    /// Return the end of the decompressed bytes in the virtual byte array.
    pub(super) fn end(&self) -> u64 {
        // `start` is a sum of `u32` sizes of at most `u32::MAX` chunks, so the sum fits in `u64`.
        self.start + u64::from(self.size)
    }
}

/// The location of the bytes of a fragment.
#[derive(Clone, Copy, Debug)]
pub(super) enum Location {
    /// Bytes in the file at this offset.
    File(u64),
    /// Bytes in the virtual byte array of the decompressed chunks at this offset.
    Chunks(u64),
}

/// One fragment of a stream.
#[derive(Clone, Copy, Debug)]
pub(super) struct Fragment {
    /// The offset of the fragment in its stream.
    pub(super) start: u64,
    /// The size of the fragment, not 0.
    pub(super) size: u32,
    /// The location of the bytes.
    pub(super) location: Location,
}

impl Fragment {
    /// Return the end of the fragment in its stream.
    pub(super) fn end(&self) -> u64 {
        // The directory decoder checks that the end of each stream fits in `u64`.
        self.start + u64::from(self.size)
    }
}

/// The directory record of one stream.
#[derive(Clone, Copy, Debug)]
pub(super) struct Entry {
    /// The index of the first fragment of the stream.
    pub(super) first: usize,
    /// The index after the last fragment of the stream.
    pub(super) end: usize,
    /// The stream size: the sum of the fragment sizes.
    pub(super) size: u64,
    /// `true` for a nil stream.
    pub(super) nil: bool,
}

/// The decoded stream directory.
pub(super) struct Directory {
    /// One entry for each stream.
    pub(super) entries: Vec<Entry>,
    /// The fragments of all streams, in stream order.
    pub(super) fragments: Vec<Fragment>,
}

/// Check that the stream directory and the Chunk Table are inside a file of `length` bytes.
///
/// An empty Chunk Table has no range. The reference writer can put its offset after the end
/// of the file.
pub(super) fn check_ranges(header: &Header, length: u64) -> Result<()> {
    check_file_range(
        header.directory_offset,
        u64::from(header.directory_size_compressed),
        length,
        "MSFZ stream directory extends past the end of the file",
    )?;
    if header.chunk_count != 0 {
        check_file_range(
            header.chunk_table_offset,
            u64::from(header.chunk_table_size),
            length,
            "MSFZ Chunk Table extends past the end of the file",
        )?;
    }
    Ok(())
}

/// Read and check the Chunk Table.
///
/// Return the chunks and the size of the virtual byte array. The caller has called
/// [`check_ranges`].
pub(super) fn read_chunk_table<S: ReadAt + ?Sized>(
    source: &S,
    header: &Header,
    length: u64,
    limits: MsfzLimits,
) -> Result<(Vec<Chunk>, u64)> {
    if header.chunk_table_size > limits.max_chunk_table_bytes() {
        return Err(Error::new(ErrorKind::LimitExceeded, Some(76))
            .with_context("MSFZ Chunk Table is larger than the Chunk Table limit"));
    }
    let mut chunks = Vec::new();
    if header.chunk_count == 0 {
        return Ok((chunks, 0));
    }
    let table_offset = header.chunk_table_offset;
    let bytes = read_bytes(source, table_offset, header.chunk_table_size)?;
    chunks
        .try_reserve_exact(usize::try_from(header.chunk_count).map_err(|_| no_memory())?)
        .map_err(|_| no_memory())?;
    let mut start = 0_u64;
    let (entries, _) = bytes.as_chunks::<20>();
    for (entry_offset, entry) in (table_offset..).step_by(20).zip(entries) {
        let field = |start: usize| {
            let mut value = [0; 4];
            value.copy_from_slice(&entry[start..start + 4]);
            u32::from_le_bytes(value)
        };
        let offset = u64::from(field(0)) | (u64::from(field(4)) << 32);
        let compression = Compression::from_code(field(8), Some(entry_offset + 8))?;
        let stored_size = field(12);
        let size = field(16);
        if stored_size == 0 || size == 0 {
            return Err(Error::new(ErrorKind::InvalidFormat, Some(entry_offset))
                .with_context("MSFZ chunk has a size of 0"));
        }
        if compression == Compression::None && stored_size != size {
            return Err(Error::new(ErrorKind::InvalidFormat, Some(entry_offset))
                .with_context("uncompressed MSFZ chunk has two different sizes"));
        }
        check_file_range(
            offset,
            u64::from(stored_size),
            length,
            "MSFZ chunk extends past the end of the file",
        )?;
        chunks.push(Chunk {
            offset,
            compression,
            stored_size,
            size,
            start,
        });
        start += u64::from(size);
    }
    debug_assert_eq!(
        u64::try_from(chunks.len()).ok(),
        Some(u64::from(header.chunk_count))
    );
    debug_assert_eq!(CHUNK_ENTRY_SIZE, 20);
    Ok((chunks, start))
}

/// Read the stream directory and decompress it if necessary.
///
/// The caller has called [`check_ranges`].
pub(super) fn read_directory<S: ReadAt + ?Sized>(
    source: &S,
    header: &Header,
    limits: MsfzLimits,
) -> Result<Vec<u8>> {
    let limit = limits.max_directory_bytes();
    if header.directory_size > limit || header.directory_size_compressed > limit {
        return Err(Error::new(ErrorKind::LimitExceeded, Some(64))
            .with_context("MSFZ stream directory is larger than the directory limit"));
    }
    let stored = read_bytes(
        source,
        header.directory_offset,
        header.directory_size_compressed,
    )?;
    let declared = u64::from(header.directory_size);
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    match header.directory_compression {
        Compression::None => Ok(stored),
        Compression::Zstd => zstd::decode(&stored, declared, limit),
        Compression::Deflate => deflate::decode(&stored, declared, limit),
    }
}

/// A reader for the `u32` values of the decompressed stream directory.
struct Cursor<'a> {
    /// The directory bytes.
    bytes: &'a [u8],
    /// The position of the next value.
    position: usize,
    /// The file offset of the directory if it is not compressed.
    file_offset: Option<u64>,
}

impl Cursor<'_> {
    /// Return the location for an error at `position`.
    fn offset(&self, position: usize) -> Option<u64> {
        self.file_offset
            .and_then(|base| base.checked_add(u64::try_from(position).ok()?))
    }

    /// Return an error at the record that starts at `position`.
    fn error(&self, kind: ErrorKind, position: usize, context: &'static str) -> Error {
        Error::new(kind, self.offset(position)).with_context(context)
    }

    /// Read the next `u32` value.
    fn u32(&mut self) -> Result<u32> {
        let value = self
            .bytes
            .get(self.position..)
            .and_then(|rest| rest.first_chunk::<4>())
            .map(|bytes| u32::from_le_bytes(*bytes))
            .ok_or_else(|| {
                self.error(
                    ErrorKind::InvalidFormat,
                    self.position,
                    "MSFZ stream directory ends inside a stream record",
                )
            })?;
        self.position += 4;
        Ok(value)
    }

    /// Read the next `u64` value.
    fn u64(&mut self) -> Result<u64> {
        let low = self.u32()?;
        let high = self.u32()?;
        Ok(u64::from(low) | (u64::from(high) << 32))
    }
}

/// Decode the stream directory and check each fragment against the file and the chunks.
///
/// `virtual_size` is the size of the virtual byte array of the chunks.
pub(super) fn decode(
    bytes: &[u8],
    header: &Header,
    chunks: &[Chunk],
    virtual_size: u64,
    length: u64,
) -> Result<Directory> {
    let mut cursor = Cursor {
        bytes,
        position: 0,
        file_offset: (header.directory_compression == Compression::None)
            .then_some(header.directory_offset),
    };
    // Each stream record has at least 4 bytes. Check the count before the allocation.
    let count = usize::try_from(header.stream_count).map_err(|_| no_memory())?;
    if count
        .checked_mul(4)
        .is_none_or(|minimum| minimum > bytes.len())
    {
        return Err(cursor.error(
            ErrorKind::InvalidFormat,
            0,
            "MSFZ stream directory is too short for the stream count",
        ));
    }
    let mut entries = Vec::new();
    entries.try_reserve_exact(count).map_err(|_| no_memory())?;
    let mut fragments = Vec::new();
    for _ in 0..count {
        let first = fragments.len();
        let mut size = cursor.u32()?;
        if size == NIL_STREAM_SIZE {
            entries.push(Entry {
                first,
                end: first,
                size: 0,
                nil: true,
            });
            continue;
        }
        let mut stream_size = 0_u64;
        while size != 0 {
            let record = cursor.position - 4;
            if size == NIL_STREAM_SIZE {
                return Err(cursor.error(
                    ErrorKind::InvalidFormat,
                    record,
                    "MSFZ fragment has the nil stream size",
                ));
            }
            let location = location(cursor.u64()?, size, chunks, virtual_size, length)
                .map_err(|(kind, context)| cursor.error(kind, record, context))?;
            fragments.try_reserve(1).map_err(|_| no_memory())?;
            fragments.push(Fragment {
                start: stream_size,
                size,
                location,
            });
            // Each fragment adds at most `u32::MAX` and the directory holds fewer than
            // `u32::MAX` fragments, so the sum fits in `u64`.
            stream_size += u64::from(size);
            size = cursor.u32()?;
        }
        entries.push(Entry {
            first,
            end: fragments.len(),
            size: stream_size,
            nil: false,
        });
    }
    if cursor.position != bytes.len() {
        return Err(cursor.error(
            ErrorKind::InvalidFormat,
            cursor.position,
            "bytes follow the last MSFZ stream record",
        ));
    }
    Ok(Directory { entries, fragments })
}

/// Decode and check the location of a fragment of `size` bytes.
fn location(
    value: u64,
    size: u32,
    chunks: &[Chunk],
    virtual_size: u64,
    length: u64,
) -> core::result::Result<Location, (ErrorKind, &'static str)> {
    let size = u64::from(size);
    if value & COMPRESSED_BIT == 0 {
        if value & RESERVED_BITS != 0 {
            return Err((
                ErrorKind::InvalidFormat,
                "reserved bits of an MSFZ fragment location are not 0",
            ));
        }
        let offset = value & FILE_OFFSET_BITS;
        // The offset has 48 bits and the size has 32 bits, so the sum fits in `u64`.
        if offset + size > length {
            return Err((
                ErrorKind::UnexpectedEof,
                "uncompressed MSFZ fragment extends past the end of the file",
            ));
        }
        return Ok(Location::File(offset));
    }
    // Bits 32 to 62 hold the chunk index and bits 0 to 31 the offset in the chunk.
    let index = (value & !COMPRESSED_BIT) >> 32;
    let inner = value & u64::from(u32::MAX);
    let chunk = usize::try_from(index)
        .ok()
        .and_then(|index| chunks.get(index))
        .ok_or((
            ErrorKind::InvalidReference,
            "MSFZ fragment names a chunk outside the Chunk Table",
        ))?;
    if inner >= u64::from(chunk.size) {
        return Err((
            ErrorKind::InvalidReference,
            "MSFZ fragment starts after the end of its first chunk",
        ));
    }
    let start = chunk.start + inner;
    if start + size > virtual_size {
        return Err((
            ErrorKind::InvalidFormat,
            "compressed MSFZ fragment extends past the end of the last chunk",
        ));
    }
    Ok(Location::Chunks(start))
}

/// Check that `size` bytes at `offset` are inside a file of `length` bytes.
fn check_file_range(offset: u64, size: u64, length: u64, context: &'static str) -> Result<()> {
    match offset.checked_add(size) {
        Some(end) if end <= length => Ok(()),
        Some(_) => Err(Error::new(ErrorKind::UnexpectedEof, Some(length)).with_context(context)),
        None => Err(Error::new(ErrorKind::OffsetOverflow, Some(offset)).with_context(context)),
    }
}

/// Read `size` bytes at `offset` into a new vector.
pub(super) fn read_bytes<S: ReadAt + ?Sized>(
    source: &S,
    offset: u64,
    size: u32,
) -> Result<Vec<u8>> {
    let size = usize::try_from(size).map_err(|_| no_memory())?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size).map_err(|_| no_memory())?;
    bytes.resize(size, 0);
    source.read_exact_at(offset, &mut bytes)?;
    Ok(bytes)
}
