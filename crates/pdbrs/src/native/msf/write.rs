//! Big MSF 7.00 container writing.
//!
//! [`WritePlan`] documents the write operations, the layout and the error contract.

use alloc::vec::Vec;

use super::{Msf, MsfLimits, NIL_STREAM_SIZE, StreamId};
use crate::io::{ReadAt, Write};
use crate::{Error, ErrorKind, Result};

mod builder;
mod emit;
#[cfg(feature = "std")]
mod file;
mod layout;

pub use builder::MsfBuilder;
#[cfg(feature = "std")]
pub use file::copy_exact_file;

use layout::{Layout, check_page_size};

/// The default page size of a new container.
const DEFAULT_PAGE_SIZE: u32 = 4096;

/// The size of the buffer for [`copy_exact`] (64 KiB).
const COPY_BUFFER_SIZE: usize = 64 << 10;

/// The contents of stream 0 (the previous stream directory) in a repacked container.
///
/// The previous directory names pages of the source layout. These page numbers do not
/// describe the repacked container. The free page map marks the stream 0 pages as free with
/// both policies.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum OldDirectoryPolicy {
    /// Write stream 0 as a present stream of length zero. LLD and `ms-pdb-msf` write this
    /// state.
    #[default]
    Empty,
    /// Copy the bytes of stream 0, or its nil state, from the source.
    ///
    /// The previous directory gives its page lists for the page size of the source. If
    /// stream 0 has bytes, the repack needs the page size of the source.
    Preserve,
}

/// Options for a new container layout.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WriteOptions {
    page_size: u32,
    limits: MsfLimits,
}

impl WriteOptions {
    /// Return the page size in bytes.
    #[must_use]
    pub const fn page_size(self) -> u32 {
        self.page_size
    }

    /// Set the page size in bytes.
    ///
    /// The plan accepts a power of two from 512 to 32768. It rejects 65536-byte pages with
    /// [`ErrorKind::UnsupportedVersion`], although the reader accepts them. Independent
    /// consumers reject 65536-byte pages: DIA fails with `HRESULT 806D000C`, LLVM reports
    /// "Unsupported block size" and `link.exe /PDBPAGESIZE:65536` fails with `LNK1399`.
    /// `link.exe /PDBPAGESIZE` writes 4096 to 32768.
    #[must_use]
    pub const fn with_page_size(mut self, page_size: u32) -> Self {
        self.page_size = page_size;
        self
    }

    /// Return the limits that the plan enforces.
    #[must_use]
    pub const fn limits(self) -> MsfLimits {
        self.limits
    }

    /// Set the limits that the plan enforces.
    ///
    /// The plan rejects a stream directory that is larger than
    /// [`MsfLimits::max_directory_bytes`]. A reader with the same limits can then open the
    /// output.
    #[must_use]
    pub const fn with_limits(mut self, limits: MsfLimits) -> Self {
        self.limits = limits;
        self
    }
}

impl Default for WriteOptions {
    /// Return options with 4096-byte pages and the default [`MsfLimits`].
    fn default() -> Self {
        Self {
            page_size: DEFAULT_PAGE_SIZE,
            limits: MsfLimits::default(),
        }
    }
}

/// A source of stream bytes for a [`WritePlan`].
trait StreamSource {
    /// Fill `buffer` with the bytes of stream `id` that start at `offset`.
    fn read_stream(&self, id: u32, offset: u64, buffer: &mut [u8]) -> Result<()>;
}

/// The frozen layout of a new container and the source of its stream bytes.
///
/// The plan borrows its source. The source cannot change while the plan exists.
///
/// # Write operations
///
/// Three operations write a container. Each operation has a different guarantee:
///
/// | Operation | Guarantee | Not guaranteed |
/// | --- | --- | --- |
/// | [`copy_exact`] | Every source byte, including free pages and padding | Any validation of the source |
/// | [`Msf::repack`] | Stream IDs, nil and empty streams, sizes and all stream bytes | Page positions, free page bytes and padding of the source |
/// | [`MsfBuilder`] | A consistent container with the given streams | PDB semantic validity of the stream contents |
///
/// A repack keeps the stream IDs, so stream references in the stream contents (for example
/// named streams and DBI module streams) stay valid. [`OldDirectoryPolicy`] selects the
/// contents of stream 0.
///
/// # Layout
///
/// [`Msf::repack`] and [`MsfBuilder::plan`] make a plan. The plan computes and checks the full
/// layout before it writes a byte:
///
/// 1. Page 0 is the header. Pages 1 and 2 of each interval of `page_size` pages are FPM1 and
///    FPM2. If the last page is the first page of an interval, the plan adds the FPM pages of
///    that interval.
/// 2. The other pages have ascending page numbers in this order: the stream pages in stream ID
///    order, the stream directory pages and the page map pages.
/// 3. The active free page map is FPM1. It marks the pages of stream 0 and the bits after the
///    last page as free. It marks all other pages as allocated. The bytes after the bitmap are
///    `0xFF`.
/// 4. The inactive free page map FPM2 has `0xFF` in all bytes.
/// 5. The unused bytes of the header page and of the last page of each stream, the directory
///    and the page map are zero.
///
/// The same streams and [`WriteOptions`] thus give the same bytes.
///
/// # Limits
///
/// Planning returns [`ErrorKind::UnsupportedVersion`] for 65536-byte pages, because DIA, LLVM
/// and `link.exe` reject them (see [`WriteOptions::with_page_size`]). It writes 512 to 32768.
///
/// Planning returns [`ErrorKind::LimitExceeded`] if a stream has `0xFFFF_FFFF` bytes or more,
/// the directory is larger than [`MsfLimits::max_directory_bytes`] or `u32`, the page map array
/// does not fit in the header page, or the page count does not fit in `u32`.
///
/// # Errors and partial output
///
/// A planning error writes nothing. [`WritePlan::write_to`] and [`copy_exact`] write in file
/// order. A read error or an output error can leave a partial output in the writer. Under
/// `std`, `WritePlan::write_file` and `copy_exact_file` write a temporary file and rename it.
/// On error, they delete the temporary file and do not change the destination.
///
/// Writing over the source file is not supported.
pub struct WritePlan<'a> {
    layout: Layout,
    sizes: Vec<u32>,
    source: &'a dyn StreamSource,
}

impl<'a> WritePlan<'a> {
    /// Make a plan for streams of the sizes `sizes`.
    fn new(options: WriteOptions, sizes: Vec<u32>, source: &'a dyn StreamSource) -> Result<Self> {
        let layout = Layout::new(options.page_size, &sizes, options.limits)?;
        Ok(Self {
            layout,
            sizes,
            source,
        })
    }

    /// Return the page size in bytes.
    #[must_use]
    pub const fn page_size(&self) -> u32 {
        self.layout.page_size
    }

    /// Return the number of pages in the output.
    #[must_use]
    pub const fn page_count(&self) -> u32 {
        self.layout.page_count
    }

    /// Return the number of streams, including nil streams.
    #[must_use]
    pub const fn stream_count(&self) -> u32 {
        self.layout.stream_count
    }

    /// Return the size of the stream directory in bytes.
    #[must_use]
    pub const fn directory_size(&self) -> u32 {
        self.layout.directory_size
    }

    /// Return the number of page map pages that the header names.
    #[must_use]
    pub const fn map_pages(&self) -> u32 {
        self.layout.map_pages
    }

    /// Return the size of the output in bytes.
    #[must_use]
    pub fn file_size(&self) -> u64 {
        self.layout.file_size()
    }

    /// Write the container to `output` in page order.
    ///
    /// The function keeps one page in memory. It reads the stream bytes one page at a time.
    ///
    /// # Errors
    ///
    /// - An error from the source if a stream read fails.
    /// - [`ErrorKind::Io`] if `output` fails. The error offset is the file offset of the page.
    /// - [`ErrorKind::LimitExceeded`] if the page buffer allocation fails.
    ///
    /// After an error, `output` can hold a part of the container.
    pub fn write_to<W: Write + ?Sized>(&self, output: &mut W) -> Result<()> {
        emit::write(self.layout, &self.sizes, self.source, output)
    }

    /// Write the container to a new vector.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`] if the file size does not fit in memory or the
    ///   allocation fails. The function checks the size before it writes.
    /// - The errors of [`WritePlan::write_to`].
    pub fn to_vec(&self) -> Result<Vec<u8>> {
        let size = usize::try_from(self.file_size()).map_err(|_| output_too_large())?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| output_too_large())?;
        self.write_to(&mut bytes)?;
        Ok(bytes)
    }
}

impl core::fmt::Debug for WritePlan<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("WritePlan")
            .field("page_size", &self.layout.page_size)
            .field("page_count", &self.layout.page_count)
            .field("stream_count", &self.layout.stream_count)
            .field("directory_size", &self.layout.directory_size)
            .finish_non_exhaustive()
    }
}

impl<S: ReadAt> Msf<S> {
    /// Plan a new container with the streams of this container.
    ///
    /// The plan keeps the stream IDs, the nil streams, the empty streams, the stream sizes and
    /// the stream bytes. `old_directory` selects the contents of stream 0. The plan reads the
    /// stream bytes from this container when it writes.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedVersion`] if the page size in `options` is 65536 (see
    ///   [`WriteOptions::with_page_size`]).
    /// - [`ErrorKind::InvalidFormat`] if the page size in `options` is not valid, or if
    ///   `old_directory` is [`OldDirectoryPolicy::Preserve`], stream 0 has bytes and the page
    ///   size in `options` is not [`Msf::block_size`].
    /// - [`ErrorKind::LimitExceeded`] if the new layout exceeds a limit (see [`WritePlan`]) or
    ///   an allocation fails.
    pub fn repack(
        &self,
        options: WriteOptions,
        old_directory: OldDirectoryPolicy,
    ) -> Result<WritePlan<'_>> {
        check_page_size(options.page_size)?;
        let mut sizes = Vec::new();
        sizes
            .try_reserve_exact(self.streams.len())
            .map_err(|_| plan_allocation_failed())?;
        sizes.extend(self.streams.iter().map(|entry| entry.size));
        match (old_directory, sizes.first_mut()) {
            (OldDirectoryPolicy::Empty, Some(first)) => *first = 0,
            (OldDirectoryPolicy::Preserve, Some(&mut size))
                if size != NIL_STREAM_SIZE
                    && size != 0
                    && options.page_size != self.geometry.block_size =>
            {
                return Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("a preserved stream 0 needs the page size of the source")
                    .with_stream(0, 0));
            }
            _ => {}
        }
        WritePlan::new(options, sizes, self)
    }
}

impl<S: ReadAt> StreamSource for Msf<S> {
    fn read_stream(&self, id: u32, offset: u64, buffer: &mut [u8]) -> Result<()> {
        let Some(stream) = self.stream(StreamId(id))? else {
            return Err(Error::new(ErrorKind::MissingStream, None)
                .with_context("planned stream is nil in the source")
                .with_stream(id, offset));
        };
        stream.read_exact_at(offset, buffer)
    }
}

/// Copy all bytes of `source` to `output` without a change.
///
/// The function does not check that `source` is a valid container. It copies through one
/// buffer of at most 64 KiB and returns the number of bytes.
///
/// # Errors
///
/// - An error from `source` if a read fails.
/// - [`ErrorKind::Io`] if `output` fails. The error offset is the file offset of the buffer.
/// - [`ErrorKind::LimitExceeded`] if the buffer allocation fails.
///
/// After an error, `output` can hold a part of the source.
pub fn copy_exact<S: ReadAt + ?Sized, W: Write + ?Sized>(
    source: &S,
    output: &mut W,
) -> Result<u64> {
    let length = source.len()?;
    let capacity =
        usize::try_from(length).map_or(COPY_BUFFER_SIZE, |length| length.min(COPY_BUFFER_SIZE));
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(capacity).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None).with_context("copy buffer allocation failed")
    })?;
    buffer.resize(capacity, 0);
    let mut offset = 0_u64;
    while offset < length {
        // The count is at most the buffer length, so it fits in `usize`.
        let count = usize::try_from(length - offset).map_or(capacity, |rest| rest.min(capacity));
        let chunk = &mut buffer[..count];
        source.read_exact_at(offset, chunk)?;
        output
            .write_all(chunk)
            .map_err(|error| Error::from_io(error, Some(offset)))?;
        // `count` is at most 64 KiB.
        offset += u64::try_from(count).unwrap_or(0);
    }
    Ok(length)
}

/// Return the error for an output that does not fit in memory.
fn output_too_large() -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context("output does not fit in memory")
}

/// Return the error for a failed plan allocation.
fn plan_allocation_failed() -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context("stream size list allocation failed")
}
