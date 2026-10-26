//! Big MSF 7.00 container access.
//!
//! An MSF container stores numbered streams in fixed-size pages. [`Msf`] reads
//! the file header and the stream directory. It does not read stream payloads.
//! [`Stream`] reads stream bytes on demand through [`ReadAt`].
//!
//! [`copy_exact`], [`Msf::repack`] and [`MsfBuilder`] write containers. The documentation of
//! [`WritePlan`] gives the layout and the guarantee of each operation.
//!
//! # Validation
//!
//! [`Msf::open`] and [`Msf::with_limits`] check these structural rules:
//!
//! - The page size is a power of two from 512 to 65536.
//! - The active free page map is 1 or 2.
//! - The declared pages fit in the input. Bytes after the declared pages are ignored.
//! - The page map array fits in the header page, and the directory size matches its contents.
//! - Each page reference is inside the declared pages.
//! - No page reference names the header page or a free page map page. Pages 1 and 2 of each
//!   interval are free page map pages.
//! - No page occurs more than once in the page map, the stream directory and all stream pages.
//!
//! Opening does not read the free page maps. [`Msf::check_free_page_map`] reads the active
//! free page map and compares it with the page references. The reader does not check unused
//! bytes after the end of a structure.
//!
//! Stream 0 holds the previous stream directory. Opening does not interpret its contents.
//! [`OldDirectory`] decodes the contents as a snapshot and does not follow its pages.
//!
//! The legacy small MSF 2.00 container is recognized and rejected with
//! [`ErrorKind::UnsupportedVersion`].
//!
//! # Offsets in errors
//!
//! Errors in the header, page map and stream directory have a physical file offset.
//! Errors from a [`Stream`] have a stream ID and a logical offset in the stream.

use alloc::vec::Vec;
use core::fmt;

use crate::format::{ContainerKind, probe};
use crate::io::ReadAt;
use crate::{Error, ErrorKind, Result};

mod directory;
mod free_page_map;
mod old_directory;
mod write;

pub use free_page_map::FreePageMapReport;
pub use old_directory::{OldDirectory, OldStream, OldStreams};
#[cfg(feature = "std")]
pub use write::copy_exact_file;
pub use write::{MsfBuilder, OldDirectoryPolicy, WriteOptions, WritePlan, copy_exact};
pub(crate) use write::{StreamSource, plan_sizes};

/// The file offset of the page size field.
const PAGE_SIZE_OFFSET: u64 = 32;

/// The file offset of the active free page map field.
const ACTIVE_FPM_OFFSET: u64 = 36;

/// The file offset of the page count field.
const PAGE_COUNT_OFFSET: u64 = 40;

/// The size value that marks a nil stream.
const NIL_STREAM_SIZE: u32 = u32::MAX;

/// The smallest valid page size.
const MIN_PAGE_SIZE: u32 = 512;

/// The largest valid page size.
const MAX_PAGE_SIZE: u32 = 65_536;

/// The default value of [`MsfLimits::max_directory_bytes`] (64 MiB).
const DEFAULT_MAX_DIRECTORY_BYTES: u32 = 64 << 20;

/// The index of a stream in the stream directory.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StreamId(pub u32);

/// Resource limits for [`Msf::with_limits`].
///
/// The memory that the stream directory uses is proportional to its size. The reader keeps
/// about 2 bytes for each directory byte that describes a stream. It keeps about 3 bytes for
/// each directory byte that holds a page number. It also uses one page-sized buffer.
///
/// The reader does not allocate memory in proportion to the declared page count.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MsfLimits {
    max_directory_bytes: u32,
}

impl MsfLimits {
    /// Return the maximum size of the stream directory in bytes.
    #[must_use]
    pub const fn max_directory_bytes(self) -> u32 {
        self.max_directory_bytes
    }

    /// Set the maximum size of the stream directory in bytes.
    ///
    /// The reader checks this limit before it allocates memory for the directory.
    #[must_use]
    pub const fn with_max_directory_bytes(mut self, bytes: u32) -> Self {
        self.max_directory_bytes = bytes;
        self
    }
}

impl Default for MsfLimits {
    /// Return limits with a maximum stream directory size of 64 MiB.
    ///
    /// This size can describe about 16 million pages.
    fn default() -> Self {
        Self {
            max_directory_bytes: DEFAULT_MAX_DIRECTORY_BYTES,
        }
    }
}

/// The page layout of a container.
#[derive(Clone, Copy, Debug)]
struct Geometry {
    /// The size of each page in bytes.
    block_size: u32,
    /// The number of pages that the header declares.
    page_count: u32,
}

impl Geometry {
    /// Return the file offset of `page`.
    fn offset(self, page: u32) -> u64 {
        u64::from(page) * u64::from(self.block_size)
    }

    /// Make sure that `page` is a declared page that a stream can use.
    ///
    /// `at` is the file offset of the page reference.
    fn check_page(self, page: u32, at: u64) -> Result<()> {
        if page >= self.page_count {
            return Err(Error::new(ErrorKind::InvalidReference, Some(at))
                .with_context("page number is outside the declared pages"));
        }
        if is_reserved_page(page, self.block_size) {
            return Err(Error::new(ErrorKind::InvalidReference, Some(at))
                .with_context("page number is the header page or a free page map page"));
        }
        Ok(())
    }
}

/// Return `true` if `size` is a power of two from 512 to 65536.
const fn is_valid_page_size(size: u32) -> bool {
    size.is_power_of_two() && size >= MIN_PAGE_SIZE && size <= MAX_PAGE_SIZE
}

/// Return `true` if `page` is the header page or a free page map page.
///
/// Each interval has `block_size` pages. Pages 1 and 2 of each interval hold FPM1 and FPM2.
const fn is_reserved_page(page: u32, block_size: u32) -> bool {
    page == 0 || matches!(page % block_size, 1 | 2)
}

/// The directory entry of one stream.
#[derive(Clone, Copy, Debug)]
struct Entry {
    /// The stream size in bytes, or [`NIL_STREAM_SIZE`].
    size: u32,
    /// The index of the first page of the stream in the page list.
    first: u32,
}

/// A Big MSF 7.00 container.
///
/// The value owns its source and keeps the stream directory in memory.
/// Stream payloads stay in the source until a caller reads them.
pub struct Msf<S> {
    source: S,
    geometry: Geometry,
    active_fpm: u32,
    map_pages: Vec<u32>,
    directory_pages: Vec<u32>,
    streams: Vec<Entry>,
    pages: Vec<u32>,
}

impl<S: ReadAt> Msf<S> {
    /// Open a container with the default [`MsfLimits`].
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Msf::with_limits`].
    pub fn open(source: S) -> Result<Self> {
        Self::with_limits(source, MsfLimits::default())
    }

    /// Open a container and read its stream directory.
    ///
    /// The function reads the header, the page map and the stream directory.
    /// It does not read stream payloads.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidSignature`] if the input is not an MSF 7.00 container.
    /// - [`ErrorKind::UnsupportedVersion`] if the input is a small MSF 2.00 container.
    /// - [`ErrorKind::UnexpectedEof`] if the header is incomplete or the declared pages extend
    ///   past the end of the input.
    /// - [`ErrorKind::InvalidFormat`] if a header field or the stream directory layout is not
    ///   valid.
    /// - [`ErrorKind::InvalidReference`] if a page reference is out of range, names a reserved
    ///   page or names a page that another reference also names.
    /// - [`ErrorKind::LimitExceeded`] if the stream directory exceeds `limits` or an allocation
    ///   fails.
    /// - An error from `source` if a read fails.
    pub fn with_limits(source: S, limits: MsfLimits) -> Result<Self> {
        match probe(&source)? {
            ContainerKind::NativeMsf7 => {}
            ContainerKind::LegacyMsf2 => {
                return Err(Error::new(ErrorKind::UnsupportedVersion, Some(0))
                    .with_context("small MSF 2.00 container"));
            }
            _ => {
                return Err(Error::new(ErrorKind::InvalidSignature, Some(0))
                    .with_context("not an MSF 7.00 container"));
            }
        }
        let length = source.len()?;
        let block_size = source.read_u32_at(PAGE_SIZE_OFFSET)?;
        if !is_valid_page_size(block_size) {
            return Err(Error::new(ErrorKind::InvalidFormat, Some(PAGE_SIZE_OFFSET))
                .with_context("page size is not a power of two from 512 to 65536"));
        }
        let active_fpm = source.read_u32_at(ACTIVE_FPM_OFFSET)?;
        if !matches!(active_fpm, 1 | 2) {
            return Err(
                Error::new(ErrorKind::InvalidFormat, Some(ACTIVE_FPM_OFFSET))
                    .with_context("active free page map is not 1 or 2"),
            );
        }
        let page_count = source.read_u32_at(PAGE_COUNT_OFFSET)?;
        let geometry = Geometry {
            block_size,
            page_count,
        };
        // Both factors are `u32` values, so the product fits in `u64`.
        if geometry.offset(page_count) > length {
            return Err(Error::new(ErrorKind::UnexpectedEof, Some(length))
                .with_context("declared pages extend past the end of the input"));
        }
        let directory = directory::read(&source, geometry, limits)?;
        Ok(Self {
            source,
            geometry,
            active_fpm,
            map_pages: directory.maps,
            directory_pages: directory.dirs,
            streams: directory.streams,
            pages: directory.pages,
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
    /// [`Msf::stream_count`].
    pub fn stream(&self, id: StreamId) -> Result<Option<Stream<'_, S>>> {
        let Some(entry) = usize::try_from(id.0)
            .ok()
            .and_then(|index| self.streams.get(index))
        else {
            return Err(Error::new(ErrorKind::InvalidStreamIndex, None)
                .with_context("stream index is outside the stream directory")
                .with_stream(id.0, 0));
        };
        if entry.size == NIL_STREAM_SIZE {
            return Ok(None);
        }
        let pages = usize::try_from(entry.first)
            .ok()
            .and_then(|first| {
                let count = usize::try_from(entry.size.div_ceil(self.geometry.block_size)).ok()?;
                self.pages.get(first..first.checked_add(count)?)
            })
            .ok_or_else(|| {
                Error::new(ErrorKind::OffsetOverflow, None)
                    .with_context("stream page list is not addressable")
                    .with_stream(id.0, 0)
            })?;
        Ok(Some(Stream {
            source: &self.source,
            block_size: self.geometry.block_size,
            id,
            size: entry.size,
            pages,
        }))
    }
}

impl<S> Msf<S> {
    /// Return the number of streams in the stream directory, including nil streams.
    #[must_use]
    pub fn stream_count(&self) -> u32 {
        // The directory holds one `u32` size for each stream, so the count fits in `u32`.
        u32::try_from(self.streams.len()).unwrap_or(u32::MAX)
    }

    /// Return the page size in bytes.
    #[must_use]
    pub const fn block_size(&self) -> u32 {
        self.geometry.block_size
    }

    /// Return the number of pages that the header declares.
    #[must_use]
    pub const fn page_count(&self) -> u32 {
        self.geometry.page_count
    }

    /// Return the active free page map, 1 or 2.
    #[must_use]
    pub const fn active_free_page_map(&self) -> u32 {
        self.active_fpm
    }
}

impl<S> fmt::Debug for Msf<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Msf")
            .field("block_size", &self.geometry.block_size)
            .field("page_count", &self.geometry.page_count)
            .field("stream_count", &self.streams.len())
            .finish_non_exhaustive()
    }
}

/// A reader for the bytes of one stream.
///
/// Offsets for [`ReadAt`] are logical offsets in the stream. A read copies only the requested
/// bytes from the pages that hold them. It reads adjacent pages with one source read.
///
/// Errors that the stream detects have no physical offset. They have the stream ID and the
/// logical offset. Errors from the source keep their physical offset and get the stream ID and
/// the logical offset of the failed fragment.
pub struct Stream<'a, S> {
    source: &'a S,
    block_size: u32,
    id: StreamId,
    size: u32,
    pages: &'a [u32],
}

impl<S> Stream<'_, S> {
    /// Return the ID of the stream.
    #[must_use]
    pub const fn id(&self) -> StreamId {
        self.id
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
        Ok(u64::from(self.size))
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        let stream = self.id.0;
        let end = u64::try_from(buffer.len())
            .ok()
            .and_then(|count| offset.checked_add(count))
            .ok_or_else(|| unaddressable(stream, offset))?;
        let size = u64::from(self.size);
        if end > size {
            return Err(
                Error::new(ErrorKind::UnexpectedEof, None).with_stream(stream, offset.max(size))
            );
        }
        let block_size = u64::from(self.block_size);
        let page_len =
            usize::try_from(self.block_size).map_err(|_| unaddressable(stream, offset))?;
        let mut logical = offset;
        let mut rest = buffer;
        while !rest.is_empty() {
            let index = usize::try_from(logical / block_size)
                .map_err(|_| unaddressable(stream, logical))?;
            // The remainder is less than the page size, so it fits in `usize`.
            let inner = usize::try_from(logical % block_size)
                .map_err(|_| unaddressable(stream, logical))?;
            let first = *self
                .pages
                .get(index)
                .ok_or_else(|| unaddressable(stream, logical))?;
            let mut available = page_len - inner;
            let mut last = first;
            for &next in self.pages.get(index + 1..).unwrap_or_default() {
                if available >= rest.len() || last.checked_add(1) != Some(next) {
                    break;
                }
                available += page_len;
                last = next;
            }
            let count = available.min(rest.len());
            let advance = u64::try_from(count).map_err(|_| unaddressable(stream, logical))?;
            let (head, tail) = rest.split_at_mut(count);
            let physical = u64::from(first) * block_size + (logical % block_size);
            self.source
                .read_exact_at(physical, head)
                .map_err(|error| error.with_stream(stream, logical))?;
            rest = tail;
            logical += advance;
        }
        Ok(())
    }
}

/// Return the error for a stream position that the reader cannot address.
fn unaddressable(stream: u32, offset: u64) -> Error {
    Error::new(ErrorKind::OffsetOverflow, None)
        .with_context("stream position is not addressable")
        .with_stream(stream, offset)
}

impl<S> fmt::Debug for Stream<'_, S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Stream")
            .field("id", &self.id)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}
