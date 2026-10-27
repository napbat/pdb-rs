//! MSFZ container writing.
//!
//! [`MsfzWritePlan`] documents the layout, the limits and the error contract.

use alloc::vec::Vec;

use super::MsfzLimits;
use super::header::HEADER_SIZE;
use crate::compression::{deflate, zstd};
use crate::io::Write;
use crate::native::msf::StreamSource;
use crate::{Error, ErrorKind, Result};

mod builder;
mod emit;
#[cfg(feature = "std")]
mod file;
mod layout;
#[cfg(test)]
mod tests;

pub use builder::MsfzBuilder;

use layout::Layout;

/// The default value of [`MsfzWriteOptions::max_chunk_size`] (1 MiB).
///
/// `pdbtool pdz-encode` uses the same default (`DEFAULT_CHUNK_THRESHOLD` of `ms-pdb-msfz`).
pub const DEFAULT_MAX_CHUNK_SIZE: u32 = 1 << 20;

/// The smallest value of [`MsfzWriteOptions::max_chunk_size`] (4 KiB).
pub const MIN_CHUNK_SIZE: u32 = 4 << 10;

/// The largest value of [`MsfzWriteOptions::max_chunk_size`] (1 GiB).
pub const MAX_CHUNK_SIZE: u32 = 1 << 30;

/// The smallest file size with [`MsfzWriteOptions::with_pad_16k`] (16 KiB).
pub const PAD_16K_SIZE: u64 = 16 << 10;

/// The default value of [`MsfzWriteOptions::max_kept_chunk_bytes`] (256 MiB).
const DEFAULT_MAX_KEPT_CHUNK_BYTES: usize = 256 << 20;

/// The alignment of the stream directory and of the Chunk Table in the file.
const TABLE_ALIGNMENT: u64 = 16;

/// A compression algorithm for the chunks and the stream directory of a new container.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Codec {
    /// zstd (compression code 1). Each chunk is one zstd frame.
    #[default]
    Zstd,
    /// Raw DEFLATE (compression code 2).
    Deflate,
}

impl Codec {
    /// Return the compression code of the codec.
    #[must_use]
    pub const fn code(self) -> u32 {
        match self {
            Self::Zstd => 1,
            Self::Deflate => 2,
        }
    }

    /// Compress `input`.
    fn encode(self, input: &[u8]) -> Result<Vec<u8>> {
        match self {
            Self::Zstd => zstd::encode(input),
            Self::Deflate => deflate::encode(input),
        }
    }
}

/// Options for a new MSFZ container.
///
/// The options match those of `pdbtool pdz-encode`: the maximum chunk size, one stream for each
/// chunk, stream directory compression and padding to 16 KiB. The chunk codec selects zstd or
/// raw DEFLATE chunks. [`MsfzWriteOptions::with_max_kept_chunk_bytes`] sets the memory that the
/// plan uses to keep compressed chunks. It does not change the output.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MsfzWriteOptions {
    max_chunk_size: u32,
    one_stream_per_chunk: bool,
    codec: Codec,
    directory_codec: Option<Codec>,
    pad_16k: bool,
    limits: MsfzLimits,
    max_kept_chunk_bytes: usize,
}

impl MsfzWriteOptions {
    /// Return the maximum number of decompressed bytes in one chunk.
    #[must_use]
    pub const fn max_chunk_size(self) -> u32 {
        self.max_chunk_size
    }

    /// Set the maximum number of decompressed bytes in one chunk.
    ///
    /// Each chunk holds exactly this number of bytes, except the last chunk (and, with
    /// [`MsfzWriteOptions::with_one_stream_per_chunk`], the last chunk of each stream). The plan
    /// accepts [`MIN_CHUNK_SIZE`] to [`MAX_CHUNK_SIZE`]. `pdbtool pdz-encode` changes other
    /// values to the nearest limit; the plan rejects them.
    #[must_use]
    pub const fn with_max_chunk_size(mut self, bytes: u32) -> Self {
        self.max_chunk_size = bytes;
        self
    }

    /// Return `true` if each stream starts a new chunk.
    #[must_use]
    pub const fn one_stream_per_chunk(self) -> bool {
        self.one_stream_per_chunk
    }

    /// Start a new chunk for each stream with bytes, if `value` is `true`.
    ///
    /// By default, a chunk holds the end of one stream and the start of the next streams.
    #[must_use]
    pub const fn with_one_stream_per_chunk(mut self, value: bool) -> Self {
        self.one_stream_per_chunk = value;
        self
    }

    /// Return the chunk codec.
    #[must_use]
    pub const fn codec(self) -> Codec {
        self.codec
    }

    /// Set the chunk codec. The default is [`Codec::Zstd`].
    #[must_use]
    pub const fn with_codec(mut self, codec: Codec) -> Self {
        self.codec = codec;
        self
    }

    /// Return the codec of the stream directory, or `None` for a stored directory.
    #[must_use]
    pub const fn directory_codec(self) -> Option<Codec> {
        self.directory_codec
    }

    /// Set the codec of the stream directory. `None` (the default) stores it.
    ///
    /// Some MSFZ readers do not read a compressed stream directory, so the default is a stored
    /// directory. `pdbtool pdz-encode --compress-stream-dir` uses [`Codec::Zstd`].
    #[must_use]
    pub const fn with_directory_codec(mut self, codec: Option<Codec>) -> Self {
        self.directory_codec = codec;
        self
    }

    /// Return `true` if the plan pads the file to [`PAD_16K_SIZE`] bytes.
    #[must_use]
    pub const fn pad_16k(self) -> bool {
        self.pad_16k
    }

    /// Add zero bytes after the Chunk Table to make a file of at least [`PAD_16K_SIZE`]
    /// bytes, if `value` is `true`.
    ///
    /// This is the `--pad16k` option of `pdbtool pdz-encode`. It is a workaround for a defect
    /// of the first MSVC PDZ decoder with files smaller than 16 KiB.
    #[must_use]
    pub const fn with_pad_16k(mut self, value: bool) -> Self {
        self.pad_16k = value;
        self
    }

    /// Return the limits that the plan enforces.
    #[must_use]
    pub const fn limits(self) -> MsfzLimits {
        self.limits
    }

    /// Set the limits that the plan enforces.
    ///
    /// The plan rejects a chunk, a stream directory or a Chunk Table that is larger than the
    /// limits. A reader with the same limits can then open the output.
    #[must_use]
    pub const fn with_limits(mut self, limits: MsfzLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Return the maximum number of compressed chunk bytes that the plan keeps.
    #[must_use]
    pub const fn max_kept_chunk_bytes(self) -> usize {
        self.max_kept_chunk_bytes
    }

    /// Set the maximum number of compressed chunk bytes that the plan keeps for
    /// [`MsfzWritePlan::write_to`]. The default is 256 MiB.
    ///
    /// The plan compresses each chunk to get its size. It keeps the compressed chunks in Chunk
    /// Table order, and it stops at the first chunk that does not fit in this limit.
    /// [`MsfzWritePlan::write_to`] writes the kept chunks. It reads and compresses the other
    /// chunks again. Thus the limit changes the memory and the time of a write, but not the
    /// output bytes. A value of 0 keeps no chunk.
    ///
    /// The plan holds at most this number of bytes for the kept chunks. If an allocation for
    /// them fails, the plan keeps no more chunks.
    #[must_use]
    pub const fn with_max_kept_chunk_bytes(mut self, bytes: usize) -> Self {
        self.max_kept_chunk_bytes = bytes;
        self
    }
}

impl Default for MsfzWriteOptions {
    /// Return the defaults of `pdbtool pdz-encode`: 1 MiB chunks, streams that share chunks,
    /// zstd chunks, a stored stream directory and no padding. The limits are the default
    /// [`MsfzLimits`]. The plan keeps up to 256 MiB of compressed chunks.
    fn default() -> Self {
        Self {
            max_chunk_size: DEFAULT_MAX_CHUNK_SIZE,
            one_stream_per_chunk: false,
            codec: Codec::Zstd,
            directory_codec: None,
            pad_16k: false,
            limits: MsfzLimits::default(),
            max_kept_chunk_bytes: DEFAULT_MAX_KEPT_CHUNK_BYTES,
        }
    }
}

/// The frozen layout of a new MSFZ container and the source of its stream bytes.
///
/// The plan borrows its source. The source cannot change while the plan exists.
///
/// # Operations
///
/// [`MsfzBuilder::plan`] writes the given streams. `Container::to_msfz` (see
/// [`Container`](crate::native::container::Container)) converts an MSF 7.00 or MSFZ container
/// and keeps the stream IDs, the nil streams, the empty streams and the stream bytes. Neither
/// operation interprets the stream contents.
///
/// # Layout
///
/// The plan computes and checks the full layout before it writes a byte. The file has these
/// parts in this order:
///
/// 1. The 80-byte header at offset 0.
/// 2. The chunks in Chunk Table order, from offset 80, without gaps.
/// 3. The stream directory at the next multiple of 16.
/// 4. The Chunk Table at the next multiple of 16.
/// 5. With [`MsfzWriteOptions::with_pad_16k`], zero bytes to [`PAD_16K_SIZE`] bytes.
///
/// The bytes of the streams with data, in stream ID order, form one sequence. Each chunk holds
/// the next [`MsfzWriteOptions::max_chunk_size`] bytes of the sequence; only the last chunk can be
/// smaller. With [`MsfzWriteOptions::with_one_stream_per_chunk`], each stream with data starts a
/// new chunk. A fragment never crosses a chunk boundary, so a stream has one fragment for each
/// chunk that holds its bytes. All stream bytes are in compressed chunks: the plan writes no
/// uncompressed fragments. The gaps before the directory and the Chunk Table are zero.
///
/// The same streams and [`MsfzWriteOptions`] thus give the same bytes.
///
/// # Cost and memory
///
/// The header holds the offsets of the directory and the Chunk Table, and these depend on the
/// compressed chunk sizes. The writer writes in file order, so planning reads all stream
/// bytes and compresses each chunk to get its size. The plan keeps the compressed chunks in
/// Chunk Table order up to [`MsfzWriteOptions::max_kept_chunk_bytes`] (256 MiB by default).
/// It stops at the first chunk that does not fit. [`MsfzWritePlan::write_to`] writes the kept
/// chunks, and it reads and compresses only the other chunks again.
///
/// The plan holds the kept chunks (at most [`MsfzWriteOptions::max_kept_chunk_bytes`] bytes),
/// the stream directory and the Chunk Table metadata. While it plans or writes, the writer also
/// holds one chunk buffer and the compressed bytes of that chunk.
///
/// # Limits
///
/// Planning returns [`ErrorKind::InvalidFormat`] if there are no streams or the maximum chunk
/// size is not from [`MIN_CHUNK_SIZE`] to [`MAX_CHUNK_SIZE`]. It returns
/// [`ErrorKind::LimitExceeded`] if the stream count does not fit in `u32`, the chunk count does
/// not fit in the 31-bit chunk index, or a chunk, the stream directory or the Chunk Table is
/// larger than the [`MsfzLimits`] of the options. A stream can be larger than 4 GiB.
///
/// # Errors and partial output
///
/// A planning error writes nothing. [`MsfzWritePlan::write_to`] writes in file order. A read error,
/// a changed source or an output error can leave a partial output in the writer. Under `std`,
/// `MsfzWritePlan::write_file` writes a temporary file and renames it. On error, it deletes the
/// temporary file and does not change the destination.
///
/// Writing over the source file is not supported.
pub struct MsfzWritePlan<'a> {
    options: MsfzWriteOptions,
    layout: Layout,
    /// The stored chunk sizes and the kept chunks.
    chunks: emit::Compressed,
    /// The stream directory as the file stores it.
    directory: Vec<u8>,
    /// The size of the decompressed stream directory.
    directory_size: u32,
    directory_offset: u64,
    chunk_table_offset: u64,
    file_size: u64,
    source: &'a dyn StreamSource,
}

impl<'a> MsfzWritePlan<'a> {
    /// Make a plan for streams with the sizes `sizes`. `None` marks a nil stream.
    pub(crate) fn new(
        options: MsfzWriteOptions,
        sizes: &[Option<u64>],
        source: &'a dyn StreamSource,
    ) -> Result<Self> {
        check_options(options)?;
        let limits = options.limits;
        let layout = Layout::new(sizes, &options)?;
        let decoded = layout.encode_directory(limits.max_directory_bytes())?;
        // The encoder checks the size against the directory limit, which is a `u32` value.
        let directory_size = u32::try_from(decoded.len()).map_err(|_| directory_too_large())?;
        let directory = match options.directory_codec {
            None => decoded,
            Some(codec) => codec.encode(&decoded)?,
        };
        let stored_directory = u32::try_from(directory.len())
            .ok()
            .filter(|&size| size <= limits.max_directory_bytes())
            .ok_or_else(directory_too_large)?;
        let chunks = emit::measure(&layout, source, &options)?;
        let chunk_bytes = chunks
            .sizes
            .iter()
            .try_fold(0_u64, |sum, &size| sum.checked_add(u64::from(size)))
            .ok_or_else(file_too_large)?;
        let directory_offset = HEADER_SIZE
            .checked_add(chunk_bytes)
            .and_then(align)
            .ok_or_else(file_too_large)?;
        let chunk_table_offset = directory_offset
            .checked_add(u64::from(stored_directory))
            .and_then(align)
            .ok_or_else(file_too_large)?;
        let end = chunk_table_offset
            .checked_add(u64::from(layout.chunk_table_size()))
            .ok_or_else(file_too_large)?;
        let file_size = if options.pad_16k {
            end.max(PAD_16K_SIZE)
        } else {
            end
        };
        Ok(Self {
            options,
            layout,
            chunks,
            directory,
            directory_size,
            directory_offset,
            chunk_table_offset,
            file_size,
            source,
        })
    }

    /// Return the options of the plan.
    #[must_use]
    pub const fn options(&self) -> MsfzWriteOptions {
        self.options
    }

    /// Return the number of streams, including nil streams.
    #[must_use]
    pub fn stream_count(&self) -> u32 {
        self.layout.stream_count()
    }

    /// Return the number of chunks.
    #[must_use]
    pub fn chunk_count(&self) -> u32 {
        self.layout.chunk_count()
    }

    /// Return the number of fragments of all streams.
    #[must_use]
    pub fn fragment_count(&self) -> u64 {
        self.layout.fragment_count()
    }

    /// Return the size of the decompressed stream directory in bytes.
    #[must_use]
    pub const fn directory_size(&self) -> u32 {
        self.directory_size
    }

    /// Return the size of the stream directory in the file in bytes.
    #[must_use]
    pub fn stored_directory_size(&self) -> u32 {
        // `new` checks that the size fits in `u32`.
        u32::try_from(self.directory.len()).unwrap_or(u32::MAX)
    }

    /// Return the size of the output in bytes.
    #[must_use]
    pub const fn file_size(&self) -> u64 {
        self.file_size
    }

    /// Write the container to `output` in file order.
    ///
    /// The function writes the chunks that the plan keeps. It reads and compresses the other
    /// chunks one at a time.
    ///
    /// # Errors
    ///
    /// - An error from the source if a stream read fails.
    /// - [`ErrorKind::InvalidFormat`] if a chunk that the plan did not keep compresses to
    ///   another size than in the plan. This happens if the source changed after planning.
    /// - [`ErrorKind::Io`] if `output` fails. The error offset is the file offset of the write.
    /// - [`ErrorKind::LimitExceeded`] if a buffer allocation fails.
    ///
    /// After an error, `output` can hold a part of the container.
    pub fn write_to<W: Write + ?Sized>(&self, output: &mut W) -> Result<()> {
        emit::write(self, output)
    }

    /// Write the container to a new vector.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`] if the file size does not fit in memory or the
    ///   allocation fails. The function checks the size before it writes.
    /// - The errors of [`MsfzWritePlan::write_to`].
    pub fn to_vec(&self) -> Result<Vec<u8>> {
        let size = usize::try_from(self.file_size).map_err(|_| output_too_large())?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| output_too_large())?;
        self.write_to(&mut bytes)?;
        Ok(bytes)
    }
}

impl core::fmt::Debug for MsfzWritePlan<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MsfzWritePlan")
            .field("options", &self.options)
            .field("stream_count", &self.stream_count())
            .field("chunk_count", &self.chunk_count())
            .field("directory_size", &self.directory_size)
            .field("file_size", &self.file_size)
            .finish_non_exhaustive()
    }
}

/// Check the options that do not depend on the streams.
fn check_options(options: MsfzWriteOptions) -> Result<()> {
    if !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&options.max_chunk_size) {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("maximum MSFZ chunk size is not from 4 KiB to 1 GiB"));
    }
    if options.max_chunk_size > options.limits.max_chunk_bytes() {
        return Err(Error::new(ErrorKind::LimitExceeded, None)
            .with_context("maximum MSFZ chunk size is larger than the chunk limit"));
    }
    Ok(())
}

/// Round `offset` up to a multiple of [`TABLE_ALIGNMENT`].
fn align(offset: u64) -> Option<u64> {
    offset.checked_next_multiple_of(TABLE_ALIGNMENT)
}

/// Return the error for a stream directory that exceeds its limit.
fn directory_too_large() -> Error {
    Error::new(ErrorKind::LimitExceeded, None)
        .with_context("MSFZ stream directory is larger than the directory limit")
}

/// Return the error for a file size that does not fit in `u64`.
fn file_too_large() -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context("MSFZ file size does not fit in u64")
}

/// Return the error for an output that does not fit in memory.
fn output_too_large() -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context("output does not fit in memory")
}

/// Return the error for a failed allocation of plan metadata.
fn plan_allocation_failed() -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context("MSFZ plan allocation failed")
}
