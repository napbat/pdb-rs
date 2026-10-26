//! The previous stream directory in stream 0.
//!
//! At each commit, the Microsoft writer stores the stream directory that was in effect before
//! the commit in stream 0 (`MSF_HB` in `msf.cpp`; Microsoft PDB documentation `msf.md`). The
//! contents use the Big MSF stream directory layout:
//!
//! 1. `stream_count: u32`.
//! 2. `stream_sizes: [u32; stream_count]`. The size `0xffff_ffff` marks a nil stream.
//! 3. The page lists of all streams, concatenated in stream order.
//!
//! The pages of a previous directory can belong to current streams or be free. This module
//! decodes the snapshot only. It does not check or follow the page numbers. `msf.md` permits an
//! empty or invalid stream 0, so a decode error does not make the container invalid.

use super::{MAX_PAGE_SIZE, MIN_PAGE_SIZE, NIL_STREAM_SIZE, StreamId};
use crate::native::hash_table::Cursor;
use crate::{ErrorKind, Result};

/// The size of one directory word in bytes.
const WORD: usize = 4;

/// The stream that holds the previous stream directory.
const OLD_DIRECTORY_STREAM: StreamId = StreamId(0);

/// A decoded previous stream directory, borrowed from the bytes of stream 0.
#[derive(Clone, Copy, Debug)]
pub struct OldDirectory<'a> {
    block_size: u32,
    sizes: &'a [u8],
    pages: &'a [u8],
}

impl<'a> OldDirectory<'a> {
    /// Decode the contents of stream 0.
    ///
    /// `block_size` is the page size of the container. Returns `Ok(None)` for an empty
    /// stream 0.
    ///
    /// # Errors
    ///
    /// Errors have stream 0 and a logical offset, and no physical offset.
    ///
    /// - [`ErrorKind::InvalidFormat`] if `block_size` is not a valid page size, the size is not
    ///   a multiple of 4, the stream count is zero, or the page lists do not fill the rest of
    ///   the stream exactly.
    /// - [`ErrorKind::UnexpectedEof`] if the stream sizes extend past the end.
    pub fn parse(bytes: &'a [u8], block_size: u32) -> Result<Option<Self>> {
        let mut cursor = Cursor::new(bytes, OLD_DIRECTORY_STREAM, "previous stream directory");
        if !block_size.is_power_of_two() || !(MIN_PAGE_SIZE..=MAX_PAGE_SIZE).contains(&block_size) {
            return Err(cursor.fail(
                ErrorKind::InvalidFormat,
                0,
                "page size is not a power of two from 512 to 65536",
            ));
        }
        if bytes.is_empty() {
            return Ok(None);
        }
        if !bytes.len().is_multiple_of(WORD) {
            return Err(cursor.fail(
                ErrorKind::InvalidFormat,
                bytes.len(),
                "previous stream directory size is not a multiple of 4",
            ));
        }
        let stream_count = cursor.u32()?;
        if stream_count == 0 {
            return Err(cursor.fail(
                ErrorKind::InvalidFormat,
                0,
                "previous stream directory has no streams",
            ));
        }
        let length = usize::try_from(stream_count)
            .ok()
            .and_then(|count| count.checked_mul(WORD))
            .ok_or_else(|| cursor.fail(ErrorKind::UnexpectedEof, WORD, "stream sizes"))?;
        let sizes = cursor.take(length)?;
        let pages_at = cursor.position();
        let pages = cursor.rest();
        let mut total = 0_u64;
        for size in words(sizes) {
            total += u64::from(page_count(size, block_size));
        }
        if total * 4 != u64::try_from(pages.len()).unwrap_or(u64::MAX) {
            return Err(cursor.fail(
                ErrorKind::InvalidFormat,
                pages_at,
                "previous stream page lists do not match the stream sizes",
            ));
        }
        Ok(Some(Self {
            block_size,
            sizes,
            pages,
        }))
    }

    /// Return the number of streams in the snapshot, including nil streams.
    #[must_use]
    pub fn stream_count(&self) -> u32 {
        // `parse` read the count from a `u32`, and each stream has one size word.
        u32::try_from(self.sizes.len() / WORD).unwrap_or(u32::MAX)
    }

    /// Return the number of page numbers in all page lists.
    #[must_use]
    pub fn page_count(&self) -> u32 {
        u32::try_from(self.pages.len() / WORD).unwrap_or(u32::MAX)
    }

    /// Iterate the streams of the snapshot in stream order.
    #[must_use]
    pub fn streams(&self) -> OldStreams<'a> {
        OldStreams {
            block_size: self.block_size,
            sizes: self.sizes,
            pages: self.pages,
        }
    }
}

/// One stream of a previous stream directory.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OldStream<'a> {
    size: u32,
    pages: &'a [u8],
}

impl OldStream<'_> {
    /// Return the stream size in bytes, or `None` for a nil stream.
    #[must_use]
    pub const fn size(&self) -> Option<u32> {
        if self.size == NIL_STREAM_SIZE {
            None
        } else {
            Some(self.size)
        }
    }

    /// Iterate the page numbers of the stream. The pages are not checked or read.
    pub fn pages(&self) -> impl ExactSizeIterator<Item = u32> + '_ {
        self.pages
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| u32::from_le_bytes(*word))
    }
}

/// An iterator over the streams of a previous stream directory.
#[derive(Clone, Debug)]
pub struct OldStreams<'a> {
    block_size: u32,
    sizes: &'a [u8],
    pages: &'a [u8],
}

impl<'a> Iterator for OldStreams<'a> {
    type Item = OldStream<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let (size, rest) = self.sizes.split_first_chunk::<4>()?;
        self.sizes = rest;
        let size = u32::from_le_bytes(*size);
        let length = usize::try_from(page_count(size, self.block_size))
            .ok()?
            .checked_mul(WORD)?;
        // `OldDirectory::parse` checked that the page lists fill the stream exactly.
        let (pages, rest) = self.pages.split_at_checked(length)?;
        self.pages = rest;
        Some(OldStream { size, pages })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let count = self.sizes.len() / WORD;
        (count, Some(count))
    }
}

impl ExactSizeIterator for OldStreams<'_> {}

/// Return the number of pages of a stream of `size` bytes.
fn page_count(size: u32, block_size: u32) -> u32 {
    if size == NIL_STREAM_SIZE {
        0
    } else {
        size.div_ceil(block_size)
    }
}

/// Iterate little-endian words.
fn words(bytes: &[u8]) -> impl Iterator<Item = u32> + '_ {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_le_bytes(*word))
}
