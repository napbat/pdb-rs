//! Chunk and fragment assignment for a new MSFZ container.
//!
//! The bytes of the streams with data, in stream ID order, form one sequence: the virtual byte
//! array of the decompressed chunks. The layout cuts the sequence into chunks of the maximum
//! chunk size. With one stream for each chunk, it also cuts the sequence at the start of each
//! stream with data. A fragment is the part of one stream in one chunk.

use alloc::vec::Vec;

use super::super::CHUNK_ENTRY_SIZE;
use super::super::directory::{COMPRESSED_BIT, NIL_STREAM_SIZE};
use super::{MsfzWriteOptions, directory_too_large, plan_allocation_failed};
use crate::{Error, ErrorKind, Result};

/// The number of chunks that the 31-bit chunk index of a fragment location can name.
const MAX_CHUNKS: u64 = 1 << 31;

/// The size of a nil stream record and of the end marker of a fragment list.
const MARKER_SIZE: u64 = 4;

/// The size of one fragment record.
const FRAGMENT_SIZE: u64 = 12;

/// The part of the virtual byte array that holds one stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Span {
    /// The offset of the first byte in the virtual byte array.
    pub(super) start: u64,
    /// The stream size in bytes.
    pub(super) size: u64,
}

impl Span {
    /// Return the end of the span in the virtual byte array.
    fn end(self) -> u64 {
        // `Layout::new` checks that the sum of all sizes fits in `u64`.
        self.start + self.size
    }
}

/// The part of the virtual byte array that one chunk holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ChunkRange {
    /// The offset of the first byte in the virtual byte array.
    pub(super) start: u64,
    /// The decompressed size of the chunk, not 0.
    pub(super) size: u32,
}

impl ChunkRange {
    /// Return the end of the chunk in the virtual byte array.
    fn end(self) -> u64 {
        self.start + u64::from(self.size)
    }
}

/// One fragment of a stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Fragment {
    /// The index of the chunk that holds the fragment.
    pub(super) chunk: u32,
    /// The offset of the fragment in the decompressed chunk.
    pub(super) offset: u32,
    /// The size of the fragment, not 0.
    pub(super) size: u32,
}

impl Fragment {
    /// Return the location field of the fragment record.
    pub(super) fn location(self) -> u64 {
        COMPRESSED_BIT | (u64::from(self.chunk) << 32) | u64::from(self.offset)
    }
}

/// The chunks of a new container and the place of each stream in them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Layout {
    /// The span of each stream in stream ID order, or `None` for a nil stream.
    pub(super) streams: Vec<Option<Span>>,
    /// The chunks in Chunk Table order.
    pub(super) chunks: Vec<ChunkRange>,
}

impl Layout {
    /// Assign the streams with the sizes `sizes` to chunks.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `sizes` is empty.
    /// - [`ErrorKind::LimitExceeded`] if the stream count does not fit in `u32`, the total
    ///   size does not fit in `u64`, the chunk count does not fit in the chunk index, the Chunk
    ///   Table is larger than its limit, or an allocation fails.
    pub(super) fn new(sizes: &[Option<u64>], options: &MsfzWriteOptions) -> Result<Self> {
        if sizes.is_empty() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("MSFZ stream directory has no streams"));
        }
        if u32::try_from(sizes.len()).is_err() {
            return Err(limit("stream count does not fit in the MSFZ header"));
        }
        let max_size = u64::from(options.max_chunk_size());
        let count = chunk_count(sizes, max_size, options.one_stream_per_chunk())?;
        let max_count = (u64::from(options.limits().max_chunk_table_bytes()) / CHUNK_ENTRY_SIZE)
            .min(MAX_CHUNKS);
        if count > max_count {
            return Err(limit(
                "MSFZ chunk count exceeds the chunk index or the Chunk Table limit",
            ));
        }
        let mut streams = Vec::new();
        streams
            .try_reserve_exact(sizes.len())
            .map_err(|_| plan_allocation_failed())?;
        let mut list = Vec::new();
        list.try_reserve_exact(usize::try_from(count).unwrap_or(usize::MAX))
            .map_err(|_| plan_allocation_failed())?;
        let mut chunks = Chunks {
            list,
            max_size,
            max_count,
            start: 0,
            fill: 0,
        };
        let mut position = 0_u64;
        for size in sizes {
            let Some(size) = *size else {
                streams.push(None);
                continue;
            };
            if size != 0 && options.one_stream_per_chunk() {
                chunks.close()?;
            }
            streams.push(Some(Span {
                start: position,
                size,
            }));
            // `chunk_count` checks that the total size fits in `u64`.
            position += size;
            chunks.add(size)?;
        }
        chunks.close()?;
        Ok(Self {
            streams,
            chunks: chunks.list,
        })
    }

    /// Return the number of streams, including nil streams.
    pub(super) fn stream_count(&self) -> u32 {
        // `new` checks that the count fits in `u32`.
        u32::try_from(self.streams.len()).unwrap_or(u32::MAX)
    }

    /// Return the number of chunks.
    pub(super) fn chunk_count(&self) -> u32 {
        // `new` checks the count against the 31-bit chunk index.
        u32::try_from(self.chunks.len()).unwrap_or(u32::MAX)
    }

    /// Return the size of the Chunk Table in bytes.
    pub(super) fn chunk_table_size(&self) -> u32 {
        // `new` checks the size against the Chunk Table limit, which is a `u32` value.
        u32::try_from(u64::from(self.chunk_count()) * CHUNK_ENTRY_SIZE).unwrap_or(u32::MAX)
    }

    /// Return the index range of the chunks that hold bytes of `span`.
    fn chunk_range(&self, span: Span) -> (usize, usize) {
        let first = self
            .chunks
            .partition_point(|chunk| chunk.end() <= span.start);
        if span.size == 0 {
            return (first, first);
        }
        let end = self
            .chunks
            .partition_point(|chunk| chunk.start < span.end());
        (first, end.max(first))
    }

    /// Return the fragments of the stream with the span `span`, in stream order.
    pub(super) fn fragments(&self, span: Span) -> impl Iterator<Item = Fragment> + '_ {
        let (first, end) = self.chunk_range(span);
        (first..end).filter_map(move |index| {
            let chunk = self.chunks.get(index)?;
            let start = chunk.start.max(span.start);
            let stop = chunk.end().min(span.end());
            Some(Fragment {
                chunk: u32::try_from(index).ok()?,
                // Both values are at most the chunk size, which is a `u32` value.
                offset: u32::try_from(start - chunk.start).ok()?,
                size: u32::try_from(stop - start).ok()?,
            })
        })
    }

    /// Return the number of fragments of all streams.
    pub(super) fn fragment_count(&self) -> u64 {
        self.streams
            .iter()
            .flatten()
            .map(|&span| {
                let (first, end) = self.chunk_range(span);
                // The count is at most the chunk count.
                u64::try_from(end - first).unwrap_or(u64::MAX)
            })
            .sum()
    }

    /// Return the size of the decompressed stream directory, or `None` if it is larger than
    /// `limit`.
    fn directory_size(&self, limit: u32) -> Option<usize> {
        let mut total = 0_u64;
        for stream in &self.streams {
            let record = match stream {
                None => MARKER_SIZE,
                Some(span) => {
                    let (first, end) = self.chunk_range(*span);
                    u64::try_from(end - first)
                        .ok()?
                        .checked_mul(FRAGMENT_SIZE)?
                        .checked_add(MARKER_SIZE)?
                }
            };
            total = total
                .checked_add(record)
                .filter(|&total| total <= u64::from(limit))?;
        }
        usize::try_from(total).ok()
    }

    /// Encode the decompressed stream directory.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the directory is larger than `limit` or the
    /// allocation fails. The function checks the size before it allocates.
    pub(super) fn encode_directory(&self, limit: u32) -> Result<Vec<u8>> {
        let size = self.directory_size(limit).ok_or_else(directory_too_large)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| plan_allocation_failed())?;
        for stream in &self.streams {
            let Some(span) = stream else {
                bytes.extend_from_slice(&NIL_STREAM_SIZE.to_le_bytes());
                continue;
            };
            for fragment in self.fragments(*span) {
                bytes.extend_from_slice(&fragment.size.to_le_bytes());
                bytes.extend_from_slice(&fragment.location().to_le_bytes());
            }
            bytes.extend_from_slice(&0_u32.to_le_bytes());
        }
        debug_assert_eq!(bytes.len(), size);
        Ok(bytes)
    }
}

/// Return the number of chunks for streams of the sizes `sizes`.
///
/// Returns [`ErrorKind::LimitExceeded`] if the total size does not fit in `u64`.
fn chunk_count(sizes: &[Option<u64>], max_size: u64, one_stream_per_chunk: bool) -> Result<u64> {
    let mut total = 0_u64;
    let mut count = 0_u64;
    for &size in sizes.iter().flatten() {
        total = total
            .checked_add(size)
            .ok_or_else(|| limit("total MSFZ stream size does not fit in u64"))?;
        count = count.saturating_add(size.div_ceil(max_size));
    }
    Ok(if one_stream_per_chunk {
        count
    } else {
        total.div_ceil(max_size)
    })
}

/// The chunk list while the layout grows.
struct Chunks {
    list: Vec<ChunkRange>,
    /// The maximum chunk size.
    max_size: u64,
    /// The maximum chunk count: the Chunk Table limit and the chunk index limit.
    max_count: u64,
    /// The start of the open chunk in the virtual byte array.
    start: u64,
    /// The number of bytes in the open chunk.
    fill: u64,
}

impl Chunks {
    /// Add `size` bytes of a stream to the open chunk and to new chunks.
    fn add(&mut self, mut size: u64) -> Result<()> {
        while size != 0 {
            let count = size.min(self.max_size - self.fill);
            self.fill += count;
            size -= count;
            if self.fill == self.max_size {
                self.close()?;
            }
        }
        Ok(())
    }

    /// Close the open chunk if it has bytes.
    fn close(&mut self) -> Result<()> {
        if self.fill == 0 {
            return Ok(());
        }
        if u64::try_from(self.list.len()).unwrap_or(u64::MAX) >= self.max_count {
            return Err(limit(
                "MSFZ chunk count exceeds the chunk index or the Chunk Table limit",
            ));
        }
        self.list
            .try_reserve(1)
            .map_err(|_| plan_allocation_failed())?;
        self.list.push(ChunkRange {
            start: self.start,
            // The fill is at most the maximum chunk size, which is a `u32` value.
            size: u32::try_from(self.fill).unwrap_or(u32::MAX),
        });
        self.start += self.fill;
        self.fill = 0;
        Ok(())
    }
}

/// Return an [`ErrorKind::LimitExceeded`] error with a static context.
fn limit(context: &'static str) -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context(context)
}
