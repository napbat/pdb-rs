//! Stream directory parsing for Big MSF 7.00.
//!
//! The header holds the page numbers of the page map. The page map holds the page numbers of
//! the stream directory. The stream directory holds the stream sizes and the stream page lists.

use alloc::vec::Vec;

use super::{Entry, Geometry, MsfLimits, NIL_STREAM_SIZE};
use crate::io::ReadAt;
use crate::{Error, ErrorKind, Result};

/// The file offset of the stream directory size field.
const DIRECTORY_SIZE_OFFSET: u64 = 44;

/// The file offset of the page map array in the header.
const PAGE_MAP_OFFSET: u32 = 52;

/// The size of one directory word in bytes.
const WORD: u32 = 4;

/// The smallest directory: the stream count and the size of stream 0.
const MIN_DIRECTORY_SIZE: u32 = 2 * WORD;

/// The decoded stream directory.
pub(super) struct Directory {
    /// The page numbers of the page map, from the header.
    pub(super) maps: Vec<u32>,
    /// The page numbers of the stream directory, from the page map.
    pub(super) dirs: Vec<u32>,
    /// The entry of each stream, in stream ID order.
    pub(super) streams: Vec<Entry>,
    /// The concatenated page lists of all streams.
    pub(super) pages: Vec<u32>,
}

/// The sizes of the paged directory structures.
#[derive(Clone, Copy)]
struct Shape {
    /// The size of the stream directory in bytes.
    size: u32,
    /// The number of pages that hold the stream directory.
    directory_pages: u32,
    /// The size of the page map in bytes.
    map_bytes: u32,
    /// The number of pages that hold the page map.
    map_pages: u32,
}

impl Shape {
    /// Read the directory size from the header and derive the page counts.
    fn read<S: ReadAt>(source: &S, block_size: u32, limits: MsfLimits) -> Result<Self> {
        let size = source.read_u32_at(DIRECTORY_SIZE_OFFSET)?;
        if !size.is_multiple_of(WORD) {
            return Err(invalid_size("stream directory size is not a multiple of 4"));
        }
        if size < MIN_DIRECTORY_SIZE {
            return Err(invalid_size("stream directory is too small for stream 0"));
        }
        if size > limits.max_directory_bytes {
            return Err(
                Error::new(ErrorKind::LimitExceeded, Some(DIRECTORY_SIZE_OFFSET))
                    .with_context("stream directory is larger than the directory limit"),
            );
        }
        let directory_pages = size.div_ceil(block_size);
        // The page size is at least 512, so the page map size fits in `u32`.
        let map_bytes = directory_pages
            .checked_mul(WORD)
            .ok_or_else(|| invalid_size("stream directory page map size overflows"))?;
        let map_pages = map_bytes.div_ceil(block_size);
        if u64::from(PAGE_MAP_OFFSET) + u64::from(map_pages) * u64::from(WORD)
            > u64::from(block_size)
        {
            return Err(invalid_size(
                "stream directory page map array does not fit in the header page",
            ));
        }
        Ok(Self {
            size,
            directory_pages,
            map_bytes,
            map_pages,
        })
    }
}

/// Read and validate the page map and the stream directory.
pub(super) fn read<S: ReadAt>(
    source: &S,
    geometry: Geometry,
    limits: MsfLimits,
) -> Result<Directory> {
    let shape = Shape::read(source, geometry.block_size, limits)?;
    let page_len = to_usize(geometry.block_size)?;
    let mut buffer = allocate::<u8>(page_len)?;
    buffer.resize(page_len, 0);

    let maps = read_page_map(source, geometry, shape.map_pages, &mut buffer)?;
    let mut dirs = allocate(to_usize(shape.directory_pages)?)?;
    let mut words = PagedWords::new(source, geometry, &maps, shape.map_bytes, &mut buffer);
    for _ in 0..shape.directory_pages {
        let (page, at) = words.read_word()?;
        geometry.check_page(page, at)?;
        dirs.push(page);
    }

    let mut layout = Layout {
        geometry,
        maps: &maps,
        dirs: &dirs,
        stream_count: 0,
    };
    let mut keys = allocate(maps.len() + dirs.len())?;
    for (sequence, &page) in (0..).zip(maps.iter().chain(&dirs)) {
        keys.push(key(page, sequence));
    }
    check_unique(&mut keys, &layout)?;

    let mut words = PagedWords::new(source, geometry, &dirs, shape.size, &mut buffer);
    let (streams, page_total) = read_sizes(&mut words, shape.size, geometry.block_size)?;
    // The directory holds one size for each stream, so the count fits in `u32`.
    layout.stream_count = u32::try_from(streams.len()).map_err(|_| truncated())?;

    let mut pages = allocate(to_usize(page_total)?)?;
    keys.try_reserve_exact(to_usize(page_total)?)
        .map_err(|_| allocation_failed())?;
    let base = shape.map_pages + shape.directory_pages;
    for index in 0..page_total {
        let (page, at) = words.read_word()?;
        geometry.check_page(page, at)?;
        pages.push(page);
        keys.push(key(page, base + index));
    }
    check_unique(&mut keys, &layout)?;
    Ok(Directory {
        maps,
        dirs,
        streams,
        pages,
    })
}

/// Read the stream count and the stream sizes.
///
/// Return the stream entries and the total number of stream pages. Make sure that the page
/// lists fill the rest of the directory exactly.
fn read_sizes<S: ReadAt>(
    words: &mut PagedWords<'_, S>,
    size: u32,
    block_size: u32,
) -> Result<(Vec<Entry>, u32)> {
    let (stream_count, at) = words.read_word()?;
    if stream_count == 0 {
        return Err(Error::new(ErrorKind::InvalidFormat, Some(at))
            .with_context("stream directory has no streams"));
    }
    let total_words = size / WORD;
    if stream_count >= total_words {
        return Err(Error::new(ErrorKind::InvalidFormat, Some(at))
            .with_context("stream sizes extend past the stream directory"));
    }
    let page_words = total_words - 1 - stream_count;

    let mut streams = allocate(to_usize(stream_count)?)?;
    let mut page_total = 0_u32;
    for _ in 0..stream_count {
        let (stream_size, at) = words.read_word()?;
        streams.push(Entry {
            size: stream_size,
            first: page_total,
        });
        let count = if stream_size == NIL_STREAM_SIZE {
            0
        } else {
            stream_size.div_ceil(block_size)
        };
        page_total = page_total
            .checked_add(count)
            .filter(|&total| total <= page_words)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidFormat, Some(at))
                    .with_context("stream page lists extend past the stream directory")
            })?;
    }
    if page_total != page_words {
        return Err(invalid_size(
            "stream directory size does not match the stream page lists",
        ));
    }
    Ok((streams, page_total))
}

/// Return an error for the directory size field.
fn invalid_size(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, Some(DIRECTORY_SIZE_OFFSET)).with_context(context)
}

/// Read and check the page numbers of the page map from the header page.
fn read_page_map<S: ReadAt>(
    source: &S,
    geometry: Geometry,
    map_pages: u32,
    buffer: &mut [u8],
) -> Result<Vec<u32>> {
    let offset = u64::from(PAGE_MAP_OFFSET);
    let length = to_usize(map_pages * WORD)?;
    let bytes = buffer.get_mut(..length).ok_or_else(truncated)?;
    source.read_exact_at(offset, bytes)?;
    let mut maps = allocate(to_usize(map_pages)?)?;
    for (index, chunk) in (0_u64..).zip(bytes.as_chunks::<4>().0) {
        let page = u32::from_le_bytes(*chunk);
        geometry.check_page(page, offset + index * u64::from(WORD))?;
        maps.push(page);
    }
    Ok(maps)
}

/// The positions of all page references, for error offsets.
struct Layout<'a> {
    geometry: Geometry,
    maps: &'a [u32],
    dirs: &'a [u32],
    stream_count: u32,
}

impl Layout<'_> {
    /// Return the file offset of the page reference with the sequence number `sequence`.
    ///
    /// Sequence numbers count the page map entries in the header, then the directory page
    /// numbers in the page map, then the stream page numbers in the directory.
    fn locate(&self, sequence: u32) -> Option<u64> {
        let map_count = u32::try_from(self.maps.len()).ok()?;
        let dir_count = u32::try_from(self.dirs.len()).ok()?;
        if sequence < map_count {
            return Some(u64::from(PAGE_MAP_OFFSET) + u64::from(sequence) * u64::from(WORD));
        }
        let index = sequence - map_count;
        if index < dir_count {
            return word_offset(self.geometry, self.maps, index);
        }
        let word = (index - dir_count)
            .checked_add(self.stream_count)?
            .checked_add(1)?;
        word_offset(self.geometry, self.dirs, word)
    }
}

/// Return the file offset of the word `word` in a structure that `pages` holds.
fn word_offset(geometry: Geometry, pages: &[u32], word: u32) -> Option<u64> {
    let byte = u64::from(word) * u64::from(WORD);
    let block_size = u64::from(geometry.block_size);
    let page = *pages.get(usize::try_from(byte / block_size).ok()?)?;
    Some(geometry.offset(page) + byte % block_size)
}

/// Return a sort key for a page reference.
fn key(page: u32, sequence: u32) -> u64 {
    (u64::from(page) << 32) | u64::from(sequence)
}

/// Make sure that no page occurs in more than one key.
///
/// The error offset is the location of the later reference.
fn check_unique(keys: &mut [u64], layout: &Layout<'_>) -> Result<()> {
    keys.sort_unstable();
    for (&first, &second) in keys.iter().zip(keys.iter().skip(1)) {
        if first >> 32 == second >> 32 {
            let sequence = u32::try_from(second & u64::from(u32::MAX)).ok();
            return Err(Error::new(
                ErrorKind::InvalidReference,
                sequence.and_then(|sequence| layout.locate(sequence)),
            )
            .with_context("page number is used more than once"));
        }
    }
    Ok(())
}

/// A sequential reader of little-endian words that a list of pages holds.
struct PagedWords<'a, S> {
    source: &'a S,
    geometry: Geometry,
    pages: &'a [u32],
    /// The size of the structure in bytes. It is a multiple of [`WORD`].
    length: u32,
    /// The logical offset of the next word.
    position: u32,
    buffer: &'a mut [u8],
    /// The file offset of the page in `buffer`.
    page_offset: u64,
    /// The number of valid bytes in `buffer`.
    filled: usize,
    /// The offset of the next word in `buffer`.
    cursor: usize,
}

impl<'a, S: ReadAt> PagedWords<'a, S> {
    /// Create a reader for a structure of `length` bytes in `pages`.
    fn new(
        source: &'a S,
        geometry: Geometry,
        pages: &'a [u32],
        length: u32,
        buffer: &'a mut [u8],
    ) -> Self {
        Self {
            source,
            geometry,
            pages,
            length,
            position: 0,
            buffer,
            page_offset: 0,
            filled: 0,
            cursor: 0,
        }
    }

    /// Return the next word and its file offset.
    fn read_word(&mut self) -> Result<(u32, u64)> {
        if self.position >= self.length {
            return Err(truncated());
        }
        if self.cursor == self.filled {
            self.load()?;
        }
        let end = self.cursor + 4;
        let bytes = self
            .buffer
            .get(self.cursor..end)
            .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
            .ok_or_else(truncated)?;
        let at = self.page_offset + u64::try_from(self.cursor).map_err(|_| truncated())?;
        self.cursor = end;
        self.position += WORD;
        Ok((u32::from_le_bytes(bytes), at))
    }

    /// Read the page that holds the next word into the buffer.
    ///
    /// The function reads only the bytes of the page that belong to the structure.
    fn load(&mut self) -> Result<()> {
        let block_size = self.geometry.block_size;
        let index = to_usize(self.position / block_size)?;
        let page = *self.pages.get(index).ok_or_else(truncated)?;
        let count = to_usize((self.length - self.position).min(block_size))?;
        let bytes = self.buffer.get_mut(..count).ok_or_else(truncated)?;
        let offset = self.geometry.offset(page);
        self.source.read_exact_at(offset, bytes)?;
        self.page_offset = offset;
        self.filled = count;
        self.cursor = 0;
        Ok(())
    }
}

/// Allocate an empty vector with room for `count` values.
fn allocate<T>(count: usize) -> Result<Vec<T>> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| allocation_failed())?;
    Ok(values)
}

/// Return the error for a failed directory allocation.
fn allocation_failed() -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context("stream directory allocation failed")
}

/// Return the error for a structure that ends before a word that the reader expects.
fn truncated() -> Error {
    Error::new(ErrorKind::InvalidFormat, None)
        .with_context("paged structure ends before its declared size")
}

/// Convert a directory count to `usize`.
fn to_usize(value: u32) -> Result<usize> {
    usize::try_from(value).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("stream directory count does not fit in memory")
    })
}
