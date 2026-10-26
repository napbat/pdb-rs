//! Sequential output of a planned Big MSF 7.00 container.
//!
//! The writer emits each page once, in page order. It keeps one page buffer and reads stream
//! payloads one page at a time.

use alloc::vec::Vec;

use super::super::directory::{DIRECTORY_SIZE_OFFSET, PAGE_MAP_OFFSET, WORD};
use super::super::{ACTIVE_FPM_OFFSET, PAGE_COUNT_OFFSET, PAGE_SIZE_OFFSET};
use super::StreamSource;
use super::layout::{Layout, pages_of};
use crate::ContainerKind;
use crate::io::Write;
use crate::{Error, ErrorKind, Result};

/// The active free page map of a new container.
const ACTIVE_FPM: u32 = 1;

/// The FPM byte that marks eight pages as free.
const ALL_FREE: u8 = 0xFF;

/// Write all pages of `layout` to `output`.
///
/// `sizes` holds the size of each stream. `source` supplies the stream bytes.
pub(super) fn write<W: Write + ?Sized>(
    layout: Layout,
    sizes: &[u32],
    source: &dyn StreamSource,
    output: &mut W,
) -> Result<()> {
    let page_len = usize::try_from(layout.page_size).map_err(|_| allocation_failed())?;
    let mut page = Vec::new();
    page.try_reserve_exact(page_len)
        .map_err(|_| allocation_failed())?;
    page.resize(page_len, 0);
    let mut streams = StreamCursor {
        sizes,
        page_size: layout.page_size,
        stream: 0,
        done: 0,
    };
    let mut data = 0_u32;
    for number in 0..layout.page_count {
        if number == 0 {
            header(layout, &mut page);
        } else if number % layout.page_size == 1 {
            free_page_map(layout, number / layout.page_size, &mut page);
        } else if number % layout.page_size == 2 {
            // The inactive map FPM2 marks all pages as free.
            page.fill(ALL_FREE);
        } else {
            if data < layout.stream_pages {
                streams.next_page(source, &mut page)?;
            } else if data - layout.stream_pages < layout.directory_pages {
                directory(layout, sizes, data - layout.stream_pages, &mut page);
            } else {
                page_map(
                    layout,
                    data - layout.stream_pages - layout.directory_pages,
                    &mut page,
                );
            }
            data += 1;
        }
        let offset = u64::from(number) * u64::from(layout.page_size);
        output
            .write_all(&page)
            .map_err(|error| Error::from_io(error, Some(offset)))?;
    }
    Ok(())
}

/// Fill `page` with the header page.
fn header(layout: Layout, page: &mut [u8]) {
    page.fill(0);
    let signature = ContainerKind::NativeMsf7.signature();
    page[..signature.len()].copy_from_slice(signature);
    for (offset, value) in [
        (PAGE_SIZE_OFFSET, layout.page_size),
        (ACTIVE_FPM_OFFSET, ACTIVE_FPM),
        (PAGE_COUNT_OFFSET, layout.page_count),
        (DIRECTORY_SIZE_OFFSET, layout.directory_size),
    ] {
        put_word(page, offset, value);
    }
    let first = layout.stream_pages + layout.directory_pages;
    for (index, slot) in (0..layout.map_pages).zip(
        page[to_usize(PAGE_MAP_OFFSET)..]
            .as_chunks_mut::<4>()
            .0
            .iter_mut(),
    ) {
        *slot = layout.page(first + index).to_le_bytes();
    }
}

/// Fill `page` with the FPM1 page of the interval `interval`.
///
/// The page holds bytes `interval * page_size..(interval + 1) * page_size` of the bitmap.
fn free_page_map(layout: Layout, interval: u32, page: &mut [u8]) {
    let first_byte = u64::from(interval) * u64::from(layout.page_size);
    for (byte, slot) in (first_byte..).zip(page.iter_mut()) {
        let first_page = byte * 8;
        *slot = if first_page >= u64::from(layout.page_count) {
            ALL_FREE
        } else {
            // The page is below the page count, so the eight pages fit in `u32`.
            let first_page = u32::try_from(first_page).unwrap_or(u32::MAX);
            (0..8_u32).fold(0, |bits, bit| {
                let free = first_page
                    .checked_add(bit)
                    .is_none_or(|number| layout.is_free(number));
                bits | (u8::from(free) << bit)
            })
        };
    }
}

/// Fill `page` with the directory page `index`.
///
/// The directory holds the stream count, the stream sizes and the page lists. The page lists
/// are the data pages `0..stream_pages`.
fn directory(layout: Layout, sizes: &[u32], index: u32, page: &mut [u8]) {
    let words_per_page = layout.page_size / WORD;
    let total = layout.directory_size / WORD;
    let first = index * words_per_page;
    for (word, slot) in (first..).zip(page.as_chunks_mut::<4>().0.iter_mut()) {
        let value = if word >= total {
            0
        } else if word == 0 {
            layout.stream_count
        } else if word <= layout.stream_count {
            sizes.get(to_usize(word - 1)).copied().unwrap_or(0)
        } else {
            layout.page(word - 1 - layout.stream_count)
        };
        *slot = value.to_le_bytes();
    }
}

/// Fill `page` with the page map page `index`.
///
/// The page map holds the page numbers of the directory pages.
fn page_map(layout: Layout, index: u32, page: &mut [u8]) {
    let words_per_page = layout.page_size / WORD;
    let first = index * words_per_page;
    for (entry, slot) in (first..).zip(page.as_chunks_mut::<4>().0.iter_mut()) {
        let value = if entry < layout.directory_pages {
            layout.page(layout.stream_pages + entry)
        } else {
            0
        };
        *slot = value.to_le_bytes();
    }
}

/// The position of the next stream page in stream ID order.
struct StreamCursor<'a> {
    sizes: &'a [u32],
    page_size: u32,
    /// The stream of the next page.
    stream: u32,
    /// The number of pages of `stream` that the cursor wrote.
    done: u32,
}

impl StreamCursor<'_> {
    /// Fill `page` with the next stream page and zero the bytes after the stream end.
    fn next_page(&mut self, source: &dyn StreamSource, page: &mut [u8]) -> Result<()> {
        let size = loop {
            let size = self
                .sizes
                .get(to_usize(self.stream))
                .copied()
                .ok_or_else(|| {
                    Error::new(ErrorKind::InvalidFormat, None)
                        .with_context("stream pages extend past the last stream")
                })?;
            if self.done < pages_of(size, self.page_size) {
                break size;
            }
            self.stream += 1;
            self.done = 0;
        };
        let start = self.done * self.page_size;
        let count = to_usize((size - start).min(self.page_size));
        let (head, tail) = page.split_at_mut(count);
        source.read_stream(self.stream, u64::from(start), head)?;
        tail.fill(0);
        self.done += 1;
        Ok(())
    }
}

/// Store `value` as a little-endian word at `offset` in `page`.
fn put_word(page: &mut [u8], offset: u64, value: u32) {
    let start = usize::try_from(offset).unwrap_or(usize::MAX);
    if let Some(slot) = page.get_mut(start..start.saturating_add(4)) {
        slot.copy_from_slice(&value.to_le_bytes());
    }
}

/// Convert a value that is at most a page size or a stream index to `usize`.
fn to_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// Return the error for a failed page buffer allocation.
fn allocation_failed() -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context("page buffer allocation failed")
}
