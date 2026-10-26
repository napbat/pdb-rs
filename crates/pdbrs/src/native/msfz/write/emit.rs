//! Chunk compression and sequential output of a planned MSFZ container.
//!
//! The writer keeps one chunk buffer. It fills the buffer from the streams in stream ID order,
//! compresses it and writes the compressed bytes. Then it reuses the buffer for the next chunk.

use alloc::vec::Vec;

use super::super::header::HEADER_SIZE;
use super::layout::{Layout, Span};
use super::{MsfzWriteOptions, MsfzWritePlan, plan_allocation_failed};
use crate::ContainerKind;
use crate::io::Write;
use crate::native::msf::StreamSource;
use crate::{Error, ErrorKind, Result};

/// The size of the zero buffer for gaps and padding.
const ZERO_BLOCK: usize = 512;

/// Compress each chunk of `layout` and return the compressed sizes.
///
/// # Errors
///
/// - An error from `source` if a stream read fails.
/// - [`ErrorKind::LimitExceeded`] if a compressed chunk is larger than the chunk limit of
///   `options` or an allocation fails.
pub(super) fn measure(
    layout: &Layout,
    source: &dyn StreamSource,
    options: &MsfzWriteOptions,
) -> Result<Vec<u32>> {
    let limit = options.limits().max_chunk_bytes();
    let mut sizes = Vec::new();
    sizes
        .try_reserve_exact(layout.chunks.len())
        .map_err(|_| plan_allocation_failed())?;
    let mut chunks = ChunkReader::new(layout, source)?;
    for index in 0..layout.chunks.len() {
        let stored = options.codec().encode(chunks.next_chunk(index)?)?;
        let size = u32::try_from(stored.len())
            .ok()
            .filter(|&size| size <= limit)
            .ok_or_else(|| {
                Error::new(ErrorKind::LimitExceeded, None)
                    .with_context("compressed MSFZ chunk is larger than the chunk limit")
            })?;
        sizes.push(size);
    }
    Ok(sizes)
}

/// Write all parts of `plan` to `output` in file order.
pub(super) fn write<W: Write + ?Sized>(plan: &MsfzWritePlan<'_>, output: &mut W) -> Result<()> {
    let mut sink = Sink { output, offset: 0 };
    sink.put(&header(plan))?;
    let mut chunks = ChunkReader::new(&plan.layout, plan.source)?;
    let mut table = Vec::new();
    table
        .try_reserve_exact(usize::try_from(plan.layout.chunk_table_size()).unwrap_or(usize::MAX))
        .map_err(|_| plan_allocation_failed())?;
    for (index, (chunk, &planned)) in plan
        .layout
        .chunks
        .iter()
        .zip(&plan.stored_sizes)
        .enumerate()
    {
        let stored = plan.options.codec().encode(chunks.next_chunk(index)?)?;
        if u32::try_from(stored.len()).ok() != Some(planned) {
            return Err(
                Error::new(ErrorKind::InvalidFormat, Some(sink.offset)).with_context(
                    "MSFZ chunk compresses to another size than in the plan; the source changed",
                ),
            );
        }
        table.extend_from_slice(&sink.offset.to_le_bytes());
        table.extend_from_slice(&plan.options.codec().code().to_le_bytes());
        table.extend_from_slice(&planned.to_le_bytes());
        table.extend_from_slice(&chunk.size.to_le_bytes());
        sink.put(&stored)?;
    }
    sink.zeros_to(plan.directory_offset)?;
    sink.put(&plan.directory)?;
    sink.zeros_to(plan.chunk_table_offset)?;
    sink.put(&table)?;
    sink.zeros_to(plan.file_size)
}

/// Return the 80-byte header of `plan`.
fn header(plan: &MsfzWritePlan<'_>) -> [u8; 80] {
    let mut bytes = [0_u8; 80];
    let signature = ContainerKind::NativeMsfz.signature();
    bytes[..signature.len()].copy_from_slice(signature);
    // Bytes 32 to 39 hold the version, 0.
    bytes[40..48].copy_from_slice(&plan.directory_offset.to_le_bytes());
    bytes[48..56].copy_from_slice(&plan.chunk_table_offset.to_le_bytes());
    let directory_code = plan.options.directory_codec().map_or(0, super::Codec::code);
    for (offset, value) in [
        (56, plan.stream_count()),
        (60, directory_code),
        (64, plan.stored_directory_size()),
        (68, plan.directory_size),
        (72, plan.chunk_count()),
        (76, plan.layout.chunk_table_size()),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    debug_assert_eq!(u64::try_from(bytes.len()).ok(), Some(HEADER_SIZE));
    bytes
}

/// A sequential output that knows its file offset.
struct Sink<'a, W: Write + ?Sized> {
    output: &'a mut W,
    /// The file offset of the next byte.
    offset: u64,
}

impl<W: Write + ?Sized> Sink<'_, W> {
    /// Write `bytes`.
    fn put(&mut self, bytes: &[u8]) -> Result<()> {
        self.output
            .write_all(bytes)
            .map_err(|error| Error::from_io(error, Some(self.offset)))?;
        // A slice length fits in `u64`.
        self.offset += u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        Ok(())
    }

    /// Write zero bytes up to the file offset `end`.
    fn zeros_to(&mut self, end: u64) -> Result<()> {
        let zeros = [0_u8; ZERO_BLOCK];
        while self.offset < end {
            let count =
                usize::try_from(end - self.offset).map_or(ZERO_BLOCK, |rest| rest.min(ZERO_BLOCK));
            self.put(&zeros[..count])?;
        }
        Ok(())
    }
}

/// Reads the decompressed bytes of each chunk in Chunk Table order.
struct ChunkReader<'a> {
    layout: &'a Layout,
    source: &'a dyn StreamSource,
    buffer: Vec<u8>,
    /// The stream of the next byte.
    stream: usize,
    /// The number of bytes of `stream` that the reader copied.
    done: u64,
}

impl<'a> ChunkReader<'a> {
    /// Make a reader with a buffer for the largest chunk of `layout`.
    fn new(layout: &'a Layout, source: &'a dyn StreamSource) -> Result<Self> {
        let largest = layout
            .chunks
            .iter()
            .map(|chunk| chunk.size)
            .max()
            .unwrap_or(0);
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(usize::try_from(largest).unwrap_or(usize::MAX))
            .map_err(|_| {
                Error::new(ErrorKind::LimitExceeded, None)
                    .with_context("MSFZ chunk buffer allocation failed")
            })?;
        Ok(Self {
            layout,
            source,
            buffer,
            stream: 0,
            done: 0,
        })
    }

    /// Fill the buffer with the bytes of chunk `index` and return them.
    ///
    /// The caller reads the chunks in Chunk Table order.
    fn next_chunk(&mut self, index: usize) -> Result<&[u8]> {
        let size = self.layout.chunks.get(index).map_or(0, |chunk| chunk.size);
        let mut buffer = core::mem::take(&mut self.buffer);
        buffer.clear();
        // The buffer has the capacity of the largest chunk, so this does not allocate.
        buffer.resize(usize::try_from(size).unwrap_or(usize::MAX), 0);
        let filled = self.fill(&mut buffer);
        self.buffer = buffer;
        filled?;
        Ok(&self.buffer)
    }

    /// Fill `buffer` with the next bytes of the streams.
    fn fill(&mut self, buffer: &mut [u8]) -> Result<()> {
        let mut rest = buffer;
        while !rest.is_empty() {
            let (id, span) = self.current()?;
            let available = usize::try_from(span.size - self.done).unwrap_or(usize::MAX);
            let count = available.min(rest.len());
            let (head, tail) = rest.split_at_mut(count);
            self.source.read_stream(id, self.done, head)?;
            // `count` is at most the chunk size.
            self.done += u64::try_from(count).unwrap_or(u64::MAX);
            rest = tail;
        }
        Ok(())
    }

    /// Return the ID and the span of the stream with the next byte.
    fn current(&mut self) -> Result<(u32, Span)> {
        loop {
            let entry = self.layout.streams.get(self.stream).ok_or_else(|| {
                Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("MSFZ chunks extend past the last stream")
            })?;
            if let Some(span) = entry
                && self.done < span.size
            {
                // `Layout::new` checks that the stream count fits in `u32`.
                return Ok((u32::try_from(self.stream).unwrap_or(u32::MAX), *span));
            }
            self.stream += 1;
            self.done = 0;
        }
    }
}
