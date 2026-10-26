//! MSFZ container access (PDZ files).
//!
//! An MSFZ container stores the same numbered streams as an MSF 7.00 container, but it has no
//! pages. Each stream is a list of fragments. A fragment is either a range of the file or a
//! range of the decompressed chunks. The decompressed chunks form one virtual byte array in
//! Chunk Table order, so a fragment can continue from one chunk into the next chunks.
//!
//! [`Msfz`] reads the header, the stream directory and the Chunk Table. It does not read stream
//! payloads. [`Stream`] reads stream bytes on demand through [`ReadAt`]. A bounded cache keeps
//! decompressed chunks (see [`MsfzLimits::max_cache_bytes`]).
//!
//! The specification is `docs/reference/microsoft-pdb-rs/docs/pdb/msfz.md`. Where it differs
//! from Microsoft's reader and writer (`ms-pdb-msfz` 0.1.10), the reader follows the decisions
//! recorded in #37.
//!
//! [`MsfzBuilder::plan`] and `Container::to_msfz` (see
//! [`Container`](crate::native::container::Container)) write new containers through a
//! [`WritePlan`]. The decisions of the writer are recorded in #38.
//!
//! # Validation
//!
//! [`Msfz::open`] and [`Msfz::with_limits`] check these rules before they return:
//!
//! - The signature matches and the version is 0. Other versions give
//!   [`ErrorKind::UnsupportedVersion`].
//! - The stream count is at least 1.
//! - The compression codes of the stream directory and of each chunk are 0 (stored), 1 (zstd)
//!   or 2 (raw DEFLATE). Other codes give [`ErrorKind::UnsupportedVersion`]. Stored data has the
//!   same compressed and uncompressed size.
//! - The Chunk Table has 20 bytes for each chunk. Each chunk has a non-zero stored size and a
//!   non-zero decompressed size.
//! - The stream directory, the Chunk Table, each chunk and each uncompressed fragment are inside
//!   the file. The stream directory has exactly one record for each stream and no bytes after
//!   the last record.
//! - The reserved bits 48 to 62 of an uncompressed fragment location are 0.
//! - A compressed fragment names a chunk in the Chunk Table, starts inside that chunk and ends
//!   inside the virtual byte array of the chunks.
//! - No chunk or uncompressed fragment overlaps another one, the 80-byte header, the stream
//!   directory or the Chunk Table. Gaps are allowed.
//!
//! Opening does not decompress chunks. A read decompresses each chunk that it needs. The
//! decoder checks that the chunk gives exactly its declared size.
//!
//! A nil stream is distinct from an empty stream. A stream is the sum of its fragment sizes,
//! so its size can be larger than 4 GiB.
//!
//! # Offsets in errors
//!
//! Errors in the header and the Chunk Table have a physical file offset. Errors in an
//! uncompressed stream directory have the file offset of the stream record. Errors in a
//! compressed stream directory and in chunk data have no offset. Errors from a [`Stream`] have
//! a stream ID and a logical offset in the stream.

use alloc::vec::Vec;
use core::fmt;

use crate::compression::{deflate, zstd};
use crate::format::{ContainerKind, probe};
use crate::io::ReadAt;
use crate::native::msf::StreamId;
use crate::{Error, ErrorKind, Result};

mod cache;
mod directory;
mod header;
mod layout;
#[cfg(test)]
mod tests;
mod write;

pub use write::{
    Codec, DEFAULT_MAX_CHUNK_SIZE, MAX_CHUNK_SIZE, MIN_CHUNK_SIZE, MsfzBuilder, PAD_16K_SIZE,
    WriteOptions, WritePlan,
};

use cache::ChunkCache;
use directory::{Chunk, Entry, Fragment, Location};
use header::{Compression, Header};

/// The size of one Chunk Table entry in bytes.
const CHUNK_ENTRY_SIZE: u64 = 20;

/// The default value of [`MsfzLimits::max_directory_bytes`] (64 MiB).
const DEFAULT_MAX_DIRECTORY_BYTES: u32 = 64 << 20;

/// The default value of [`MsfzLimits::max_chunk_table_bytes`] (64 MiB).
const DEFAULT_MAX_CHUNK_TABLE_BYTES: u32 = 64 << 20;

/// The default value of [`MsfzLimits::max_chunk_bytes`] (256 MiB).
const DEFAULT_MAX_CHUNK_BYTES: u32 = 256 << 20;

/// The default value of [`MsfzLimits::max_cache_bytes`] (256 MiB).
const DEFAULT_MAX_CACHE_BYTES: usize = 256 << 20;

/// Resource limits for [`Msfz::with_limits`].
///
/// The memory that the stream directory uses is proportional to the decompressed directory
/// size: about 2 bytes for each directory byte. The Chunk Table uses about 2 bytes for each
/// table byte. The reader does not allocate memory in proportion to a declared count before it
/// checks the count against the bytes that hold it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MsfzLimits {
    /// The maximum size of the stream directory in bytes.
    directory: u32,
    /// The maximum size of the Chunk Table in bytes.
    chunk_table: u32,
    /// The maximum size of one chunk in bytes.
    chunk: u32,
    /// The maximum number of cached chunk bytes.
    cache: usize,
}

impl MsfzLimits {
    /// Return the maximum size of the stream directory in bytes, in the file and decompressed.
    #[must_use]
    pub const fn max_directory_bytes(self) -> u32 {
        self.directory
    }

    /// Set the maximum size of the stream directory in bytes, in the file and decompressed.
    ///
    /// The reader checks this limit before it allocates memory for the directory.
    #[must_use]
    pub const fn with_max_directory_bytes(mut self, bytes: u32) -> Self {
        self.directory = bytes;
        self
    }

    /// Return the maximum size of the Chunk Table in bytes.
    #[must_use]
    pub const fn max_chunk_table_bytes(self) -> u32 {
        self.chunk_table
    }

    /// Set the maximum size of the Chunk Table in bytes.
    ///
    /// The reader checks this limit before it allocates memory for the Chunk Table.
    #[must_use]
    pub const fn with_max_chunk_table_bytes(mut self, bytes: u32) -> Self {
        self.chunk_table = bytes;
        self
    }

    /// Return the maximum size of one chunk in bytes, stored and decompressed.
    #[must_use]
    pub const fn max_chunk_bytes(self) -> u32 {
        self.chunk
    }

    /// Set the maximum size of one chunk in bytes, stored and decompressed.
    ///
    /// A read that needs a larger chunk gives [`ErrorKind::LimitExceeded`]. The reader checks
    /// this limit before it allocates memory for the chunk.
    #[must_use]
    pub const fn with_max_chunk_bytes(mut self, bytes: u32) -> Self {
        self.chunk = bytes;
        self
    }

    /// Return the maximum number of decompressed chunk bytes that the cache keeps.
    #[must_use]
    pub const fn max_cache_bytes(self) -> usize {
        self.cache
    }

    /// Set the maximum number of decompressed chunk bytes that the cache keeps.
    ///
    /// The cache removes the least recently used chunks first. A chunk that is larger than
    /// this limit is decompressed for each read and not kept. A value of 0 disables the cache.
    #[must_use]
    pub const fn with_max_cache_bytes(mut self, bytes: usize) -> Self {
        self.cache = bytes;
        self
    }
}

impl Default for MsfzLimits {
    /// Return limits of 64 MiB for the stream directory and the Chunk Table, 256 MiB for one
    /// chunk and 256 MiB for the chunk cache.
    fn default() -> Self {
        Self {
            directory: DEFAULT_MAX_DIRECTORY_BYTES,
            chunk_table: DEFAULT_MAX_CHUNK_TABLE_BYTES,
            chunk: DEFAULT_MAX_CHUNK_BYTES,
            cache: DEFAULT_MAX_CACHE_BYTES,
        }
    }
}

/// Return the error for a metadata allocation that failed.
fn no_memory() -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context("MSFZ metadata cannot be allocated")
}

/// An MSFZ container.
///
/// The value owns its source and keeps the stream directory and the Chunk Table in memory.
/// Stream payloads stay in the source until a caller reads them.
pub struct Msfz<S> {
    source: S,
    limits: MsfzLimits,
    directory_compressed: bool,
    chunks: Vec<Chunk>,
    entries: Vec<Entry>,
    fragments: Vec<Fragment>,
    cache: ChunkCache,
}

impl<S: ReadAt> Msfz<S> {
    /// Open a container with the default [`MsfzLimits`].
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Msfz::with_limits`].
    pub fn open(source: S) -> Result<Self> {
        Self::with_limits(source, MsfzLimits::default())
    }

    /// Open a container and read its stream directory and Chunk Table.
    ///
    /// The function checks the rules in the [module documentation](self). It does not read
    /// stream payloads and does not decompress chunks.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidSignature`] if the input is not an MSFZ container.
    /// - [`ErrorKind::UnsupportedVersion`] for a version other than 0 or an unknown compression
    ///   code.
    /// - [`ErrorKind::UnexpectedEof`] if the header is incomplete or a structure extends past
    ///   the end of the input.
    /// - [`ErrorKind::InvalidFormat`] if a header field, the stream directory, the Chunk Table
    ///   or the compressed directory is not valid, or if two structures overlap.
    /// - [`ErrorKind::InvalidReference`] if a fragment names a chunk that does not exist or
    ///   starts after the end of its chunk.
    /// - [`ErrorKind::LimitExceeded`] if the directory or the Chunk Table exceeds `limits` or
    ///   an allocation fails.
    /// - An error from `source` if a read fails.
    pub fn with_limits(source: S, limits: MsfzLimits) -> Result<Self> {
        if probe(&source)? != ContainerKind::NativeMsfz {
            return Err(Error::new(ErrorKind::InvalidSignature, Some(0))
                .with_context("not an MSFZ container"));
        }
        let header = Header::read(&source)?;
        let length = source.len()?;
        directory::check_ranges(&header, length)?;
        layout::check_fixed(&header)?;
        let (chunks, virtual_size) = directory::read_chunk_table(&source, &header, length, limits)?;
        let bytes = directory::read_directory(&source, &header, limits)?;
        let directory = directory::decode(&bytes, &header, &chunks, virtual_size, length)?;
        layout::check(&header, &chunks, &directory.fragments)?;
        Ok(Self {
            source,
            limits,
            directory_compressed: header.directory_compression != Compression::None,
            chunks,
            entries: directory.entries,
            fragments: directory.fragments,
            cache: ChunkCache::new(limits.max_cache_bytes()),
        })
    }

    /// Return a reader for the stream `id`.
    ///
    /// Returns `Ok(None)` if the directory marks the stream as nil.
    /// An empty stream is `Some` with a length of zero.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidStreamIndex`] if `id` is not less than
    /// [`Msfz::stream_count`].
    pub fn stream(&self, id: StreamId) -> Result<Option<Stream<'_, S>>> {
        let entry = usize::try_from(id.0)
            .ok()
            .and_then(|index| self.entries.get(index))
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidStreamIndex, None)
                    .with_context("stream index is outside the stream directory")
                    .with_stream(id.0, 0)
            })?;
        if entry.nil {
            return Ok(None);
        }
        Ok(Some(Stream {
            msfz: self,
            id,
            size: entry.size,
            fragments: self
                .fragments
                .get(entry.first..entry.end)
                .unwrap_or_default(),
        }))
    }

    /// Copy the bytes at `position` of the virtual byte array of the chunks into `buffer`.
    fn read_chunks(&self, mut position: u64, mut buffer: &mut [u8]) -> Result<()> {
        while !buffer.is_empty() {
            let index = self.chunks.partition_point(|chunk| chunk.end() <= position);
            let chunk = self.chunks.get(index).ok_or_else(|| {
                Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("read extends past the end of the last MSFZ chunk")
            })?;
            let inner = usize::try_from(position - chunk.start).map_err(|_| no_memory())?;
            let available = usize::try_from(chunk.end() - position).unwrap_or(usize::MAX);
            let count = available.min(buffer.len());
            let (head, tail) = buffer.split_at_mut(count);
            let index = u32::try_from(index).map_err(|_| no_memory())?;
            let copied = self.cache.with_chunk(
                index,
                || self.load_chunk(chunk),
                |data| {
                    data.get(inner..inner + count)
                        .map(|bytes| head.copy_from_slice(bytes))
                        .is_some()
                },
            )?;
            if !copied {
                return Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("decompressed MSFZ chunk is shorter than its declared size"));
            }
            buffer = tail;
            position += u64::try_from(count).map_err(|_| no_memory())?;
        }
        Ok(())
    }

    /// Read and decompress one chunk.
    fn load_chunk(&self, chunk: &Chunk) -> Result<Vec<u8>> {
        let limit = self.limits.max_chunk_bytes();
        if chunk.size > limit || chunk.stored_size > limit {
            return Err(Error::new(ErrorKind::LimitExceeded, Some(chunk.offset))
                .with_context("MSFZ chunk is larger than the chunk limit"));
        }
        let stored = directory::read_bytes(&self.source, chunk.offset, chunk.stored_size)?;
        let declared = u64::from(chunk.size);
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        match chunk.compression {
            Compression::None => Ok(stored),
            Compression::Zstd => zstd::decode(&stored, declared, limit),
            Compression::Deflate => deflate::decode(&stored, declared, limit),
        }
    }
}

impl<S> Msfz<S> {
    /// Return the number of streams in the stream directory, including nil streams.
    #[must_use]
    pub fn stream_count(&self) -> u32 {
        // The header gives the count as a `u32` value.
        u32::try_from(self.entries.len()).unwrap_or(u32::MAX)
    }

    /// Return the number of chunks in the Chunk Table.
    #[must_use]
    pub fn chunk_count(&self) -> u32 {
        // The header gives the count as a `u32` value.
        u32::try_from(self.chunks.len()).unwrap_or(u32::MAX)
    }

    /// Return the number of fragments of all streams.
    #[must_use]
    pub const fn fragment_count(&self) -> usize {
        self.fragments.len()
    }

    /// Return `true` if the stream directory is compressed in the file.
    #[must_use]
    pub const fn is_directory_compressed(&self) -> bool {
        self.directory_compressed
    }

    /// Return the limits that the container uses.
    #[must_use]
    pub const fn limits(&self) -> MsfzLimits {
        self.limits
    }
}

impl<S> fmt::Debug for Msfz<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Msfz")
            .field("stream_count", &self.entries.len())
            .field("chunk_count", &self.chunks.len())
            .field("fragment_count", &self.fragments.len())
            .finish_non_exhaustive()
    }
}

/// A reader for the bytes of one stream.
///
/// Offsets for [`ReadAt`] are logical offsets in the stream. A read copies only the requested
/// bytes. It reads uncompressed fragments from the source and compressed fragments from the
/// decompressed chunks. It decompresses each chunk that it needs and does not have in the
/// cache.
///
/// Errors have the stream ID and the logical offset of the failed fragment. Errors from the
/// source keep their physical offset.
pub struct Stream<'a, S> {
    msfz: &'a Msfz<S>,
    id: StreamId,
    size: u64,
    fragments: &'a [Fragment],
}

impl<S> Stream<'_, S> {
    /// Return the ID of the stream.
    #[must_use]
    pub const fn id(&self) -> StreamId {
        self.id
    }

    /// Return the number of fragments of the stream.
    #[must_use]
    pub const fn fragment_count(&self) -> usize {
        self.fragments.len()
    }
}

impl<S: ReadAt> Stream<'_, S> {
    /// Read the full stream into a new vector.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`] if the stream is larger than `max_bytes` or the allocation
    ///   fails. The function checks the size before it allocates memory.
    /// - The errors of [`ReadAt::read_exact_at`].
    pub fn read_all(&self, max_bytes: usize) -> Result<Vec<u8>> {
        let limit = |context| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context(context)
                .with_stream(self.id.0, 0)
        };
        let size = usize::try_from(self.size)
            .ok()
            .filter(|&size| size <= max_bytes)
            .ok_or_else(|| limit("stream is larger than the read limit"))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| limit("stream buffer allocation failed"))?;
        bytes.resize(size, 0);
        self.read_exact_at(0, &mut bytes)?;
        Ok(bytes)
    }
}

impl<S: ReadAt> ReadAt for Stream<'_, S> {
    fn len(&self) -> Result<u64> {
        Ok(self.size)
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        let stream = self.id.0;
        let end = u64::try_from(buffer.len())
            .ok()
            .and_then(|count| offset.checked_add(count))
            .ok_or_else(|| {
                Error::new(ErrorKind::OffsetOverflow, None)
                    .with_context("stream position is not addressable")
                    .with_stream(stream, offset)
            })?;
        if end > self.size {
            return Err(Error::new(ErrorKind::UnexpectedEof, None)
                .with_stream(stream, offset.max(self.size)));
        }
        let first = self
            .fragments
            .partition_point(|fragment| fragment.end() <= offset);
        let mut logical = offset;
        let mut rest = buffer;
        for fragment in self.fragments.get(first..).unwrap_or_default() {
            if rest.is_empty() {
                break;
            }
            let inner = logical - fragment.start;
            let available = usize::try_from(fragment.end() - logical).unwrap_or(usize::MAX);
            let count = available.min(rest.len());
            let (head, tail) = rest.split_at_mut(count);
            match fragment.location {
                Location::File(start) => self.msfz.source.read_exact_at(start + inner, head),
                Location::Chunks(start) => self.msfz.read_chunks(start + inner, head),
            }
            .map_err(|error| error.with_stream(stream, logical))?;
            rest = tail;
            logical += u64::try_from(count).map_err(|_| no_memory())?;
        }
        Ok(())
    }
}

impl<S> fmt::Debug for Stream<'_, S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Stream")
            .field("id", &self.id)
            .field("size", &self.size)
            .field("fragment_count", &self.fragments.len())
            .finish_non_exhaustive()
    }
}
