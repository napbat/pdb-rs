//! Page assignment for a new Big MSF 7.00 container.
//!
//! The writer gives each page that is not reserved a data page index in ascending page order.
//! The stream pages use the first data pages in stream ID order. The stream directory uses the
//! next data pages, and the page map uses the data pages after the directory. Thus the
//! concatenated page lists of the directory are data pages `0..stream_pages`.

use super::super::directory::{PAGE_MAP_OFFSET, WORD};
use super::super::{MsfLimits, NIL_STREAM_SIZE, is_reserved_page, is_valid_page_size};
use crate::{Error, ErrorKind, Result};

/// The first page after the header page and the FPM pages of interval 0.
const FIRST_DATA_PAGE: u32 = 3;

/// The largest page size that the writer writes.
///
/// The reader accepts 65536-byte pages, but independent consumers reject them: DIA fails with
/// `HRESULT 806D000C`, LLVM `isValidBlockSize` reports "Unsupported block size" and
/// `link.exe /PDBPAGESIZE:65536` fails with `LNK1399`.
pub(super) const MAX_WRITE_PAGE_SIZE: u32 = 32_768;

/// Make sure that the writer can write pages of `page_size` bytes.
///
/// # Errors
///
/// - [`ErrorKind::InvalidFormat`] if `page_size` is not a power of two from 512 to 65536.
/// - [`ErrorKind::UnsupportedVersion`] if `page_size` is larger than [`MAX_WRITE_PAGE_SIZE`].
pub(super) fn check_page_size(page_size: u32) -> Result<()> {
    if !is_valid_page_size(page_size) {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("page size is not a power of two from 512 to 65536"));
    }
    if page_size > MAX_WRITE_PAGE_SIZE {
        return Err(Error::new(ErrorKind::UnsupportedVersion, None)
            .with_context("65536-byte pages are not written: DIA, LLVM and link.exe reject them"));
    }
    Ok(())
}

/// The validated page layout of a new container.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Layout {
    /// The size of each page in bytes.
    pub(super) page_size: u32,
    /// The number of streams, including nil streams.
    pub(super) stream_count: u32,
    /// The number of pages of all streams, in data pages `0..stream_pages`.
    pub(super) stream_pages: u32,
    /// The size of the stream directory in bytes.
    pub(super) directory_size: u32,
    /// The number of directory pages, after the stream pages.
    pub(super) directory_pages: u32,
    /// The number of page map pages, after the directory pages.
    pub(super) map_pages: u32,
    /// The number of pages in the file.
    pub(super) page_count: u32,
    /// The page after the last page of stream 0, or [`FIRST_DATA_PAGE`] if stream 0 has no
    /// pages. The free page map marks the stream 0 pages as free.
    old_end: u32,
}

impl Layout {
    /// Compute and check the layout for streams of the sizes `sizes`.
    ///
    /// A size of [`NIL_STREAM_SIZE`] marks a nil stream.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `page_size` is not valid or `sizes` is empty.
    /// - [`ErrorKind::UnsupportedVersion`] if `page_size` is larger than
    ///   [`MAX_WRITE_PAGE_SIZE`].
    /// - [`ErrorKind::LimitExceeded`] if the directory is larger than `limits` or `u32`, the page
    ///   map array does not fit in the header page or the page count does not fit in `u32`.
    pub(super) fn new(page_size: u32, sizes: &[u32], limits: MsfLimits) -> Result<Self> {
        check_page_size(page_size)?;
        let Some(&old_size) = sizes.first() else {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("stream directory has no streams"));
        };
        let stream_count = u32::try_from(sizes.len())
            .map_err(|_| limit("stream count does not fit in the stream directory"))?;
        let stream_pages: u64 = sizes
            .iter()
            .map(|&size| u64::from(pages_of(size, page_size)))
            .sum();
        let words = 1 + u64::from(stream_count) + stream_pages;
        let directory_size = u32::try_from(words * u64::from(WORD))
            .map_err(|_| limit("stream directory size does not fit in u32"))?;
        if directory_size > limits.max_directory_bytes() {
            return Err(limit("stream directory is larger than the directory limit"));
        }
        // The directory size fits in `u32`, so its page count does too.
        let stream_pages = u32::try_from(stream_pages)
            .map_err(|_| limit("stream page count does not fit in u32"))?;
        let directory_pages = directory_size.div_ceil(page_size);
        // The page size is at least 512, so the page map size fits in `u32`.
        let map_pages = (directory_pages * WORD).div_ceil(page_size);
        if u64::from(PAGE_MAP_OFFSET) + u64::from(map_pages) * u64::from(WORD)
            > u64::from(page_size)
        {
            return Err(limit(
                "stream directory page map array does not fit in the header page",
            ));
        }
        let data_pages =
            u64::from(stream_pages) + u64::from(directory_pages) + u64::from(map_pages);
        // The directory has at least one page, so `data_pages` is at least 2.
        let last = data_page(data_pages - 1, page_size);
        // Every interval in the file keeps its two FPM pages.
        let end = if last.is_multiple_of(u64::from(page_size)) {
            last + 3
        } else {
            last + 1
        };
        let page_count = u32::try_from(end).map_err(|_| limit("page count does not fit in u32"))?;
        let old_pages = pages_of(old_size, page_size);
        let old_end = if old_pages == 0 {
            FIRST_DATA_PAGE
        } else {
            // The stream 0 pages are data pages, so their page numbers fit in `u32`.
            u32::try_from(data_page(u64::from(old_pages) - 1, page_size) + 1)
                .map_err(|_| limit("page count does not fit in u32"))?
        };
        Ok(Self {
            page_size,
            stream_count,
            stream_pages,
            directory_size,
            directory_pages,
            map_pages,
            page_count,
            old_end,
        })
    }

    /// Return the size of the file in bytes.
    pub(super) fn file_size(self) -> u64 {
        u64::from(self.page_count) * u64::from(self.page_size)
    }

    /// Return the page number of the data page `index`.
    ///
    /// The layout makes sure that each data page that it uses has a page number below
    /// [`Layout::page_count`].
    pub(super) fn page(self, index: u32) -> u32 {
        // `Layout::new` checked that the page numbers of all data pages fit in `u32`.
        u32::try_from(data_page(u64::from(index), self.page_size)).unwrap_or(u32::MAX)
    }

    /// Return `true` if the active free page map marks `page` as free.
    ///
    /// The pages of stream 0 and the bits after the last page are free.
    pub(super) fn is_free(self, page: u32) -> bool {
        page >= self.page_count
            || ((FIRST_DATA_PAGE..self.old_end).contains(&page)
                && !is_reserved_page(page, self.page_size))
    }
}

/// Return the number of pages of a stream of `size` bytes.
pub(super) const fn pages_of(size: u32, page_size: u32) -> u32 {
    if size == NIL_STREAM_SIZE {
        0
    } else {
        size.div_ceil(page_size)
    }
}

/// Return the page number of the data page `index`.
///
/// Interval 0 has the data pages `3..page_size`. Each later interval `k` has the data pages
/// `k * page_size` and `k * page_size + 3..(k + 1) * page_size`.
fn data_page(index: u64, page_size: u32) -> u64 {
    let size = u64::from(page_size);
    let first = size - u64::from(FIRST_DATA_PAGE);
    if index < first {
        return index + u64::from(FIRST_DATA_PAGE);
    }
    let rest = index - first;
    let per_interval = size - 2;
    let interval = 1 + rest / per_interval;
    let position = rest % per_interval;
    interval * size + if position == 0 { 0 } else { position + 2 }
}

/// Return a [`ErrorKind::LimitExceeded`] error.
fn limit(context: &'static str) -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context(context)
}

#[cfg(test)]
mod tests {
    use super::{data_page, is_reserved_page};

    #[test]
    fn data_pages_skip_reserved_pages_in_order() {
        for page_size in [512_u32, 1024, 4096] {
            let mut expected = (0..u64::from(page_size) * 3)
                .filter(|&page| !is_reserved_page(u32::try_from(page).unwrap_or(0), page_size));
            for index in 0..u64::from(page_size) * 3 - 8 {
                assert_eq!(Some(data_page(index, page_size)), expected.next());
            }
        }
    }
}
