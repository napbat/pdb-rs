//! An independent parser of the MSF 7.0 stream directory: the page size and
//! the page list of each stream.
//!
//! The tests use it to map a file offset to a stream page, and a stream
//! offset to a file offset. It does not use the reader of the library.

use std::collections::BTreeMap;

/// The file offset of the page size field.
const PAGE_SIZE_OFFSET: usize = 32;
/// The file offset of the stream directory size field.
const DIRECTORY_SIZE_OFFSET: usize = 44;
/// The file offset of the list of the pages that hold the directory page
/// numbers.
const MAP_PAGES_OFFSET: usize = 52;
/// The size value of a nil stream.
const NIL_STREAM_SIZE: u32 = u32::MAX;

/// Return the little-endian `u32` at `offset` of `bytes`.
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    let field = bytes
        .get(offset..offset + 4)
        .expect("the field is inside the input");
    u32::from_le_bytes(field.try_into().expect("four bytes"))
}

/// Return the number of pages of `page_size` bytes for `size` bytes.
fn page_count(size: usize, page_size: usize) -> usize {
    size.div_ceil(page_size)
}

/// One stream of the directory.
#[derive(Debug)]
pub struct StreamPages {
    /// The stream size in bytes.
    pub size: usize,
    /// The page numbers, in stream order.
    pub pages: Vec<u32>,
}

/// The stream directory of an MSF 7.0 file.
#[derive(Debug)]
pub struct Layout {
    /// The page size in bytes.
    pub page_size: usize,
    /// The streams in stream ID order. `None` is a nil stream.
    pub streams: Vec<Option<StreamPages>>,
    /// The stream and the page position in that stream of each stream page.
    owners: BTreeMap<u32, (u32, usize)>,
}

impl Layout {
    /// Parse the superblock and the stream directory of `bytes`.
    pub fn parse(bytes: &[u8]) -> Self {
        let page_size = usize::try_from(u32_at(bytes, PAGE_SIZE_OFFSET)).expect("page size");
        let directory_size =
            usize::try_from(u32_at(bytes, DIRECTORY_SIZE_OFFSET)).expect("directory size");
        let directory_pages = page_count(directory_size, page_size);
        let map_pages = page_count(directory_pages * 4, page_size);
        let mut page_list = Vec::new();
        for map in 0..map_pages {
            let map_page = u32_at(bytes, MAP_PAGES_OFFSET + map * 4) as usize;
            let entries = (directory_pages - map * page_size / 4).min(page_size / 4);
            page_list
                .extend((0..entries).map(|entry| u32_at(bytes, map_page * page_size + entry * 4)));
        }
        let mut directory = Vec::new();
        for &page in &page_list {
            let start = page as usize * page_size;
            directory.extend_from_slice(&bytes[start..start + page_size]);
        }
        directory.truncate(directory_size);
        let count = u32_at(&directory, 0) as usize;
        let mut cursor = 4 + count * 4;
        let mut streams = Vec::new();
        let mut owners = BTreeMap::new();
        for id in 0..count {
            let size = u32_at(&directory, 4 + id * 4);
            if size == NIL_STREAM_SIZE {
                streams.push(None);
                continue;
            }
            let size = size as usize;
            let pages: Vec<u32> = (0..page_count(size, page_size))
                .map(|index| u32_at(&directory, cursor + index * 4))
                .collect();
            cursor += pages.len() * 4;
            let stream = u32::try_from(id).expect("stream ID");
            for (position, &page) in pages.iter().enumerate() {
                let previous = owners.insert(page, (stream, position));
                assert_eq!(previous, None, "page {page} belongs to two streams");
            }
            streams.push(Some(StreamPages { size, pages }));
        }
        assert_eq!(cursor, directory_size, "the directory has no extra bytes");
        Self {
            page_size,
            streams,
            owners,
        }
    }

    /// Return the stream `id`, or `None` for a nil or absent stream.
    pub fn stream(&self, id: u32) -> Option<&StreamPages> {
        self.streams.get(id as usize).and_then(Option::as_ref)
    }

    /// Return the stream and the page position of the page `page`, or `None`
    /// for a page that no stream holds.
    pub fn owner(&self, page: u32) -> Option<(u32, usize)> {
        self.owners.get(&page).copied()
    }

    /// Return the file offset of the byte at `offset` of the stream `id`.
    pub fn file_offset(&self, id: u32, offset: usize) -> usize {
        let stream = self.stream(id).expect("stream present");
        assert!(offset < stream.size, "offset is inside the stream");
        stream.pages[offset / self.page_size] as usize * self.page_size + offset % self.page_size
    }

    /// Call `visit` with the stream, the page position and the stream
    /// offsets of each part of the file range `offset..offset + length`.
    ///
    /// A part outside each stream gives `None`.
    pub fn split(
        &self,
        offset: u64,
        length: usize,
        mut visit: impl FnMut(Option<(u32, usize)>, core::ops::Range<usize>),
    ) {
        let mut at = usize::try_from(offset).expect("offset fits in usize");
        let end = at + length;
        while at < end {
            let page = at / self.page_size;
            let inner = at % self.page_size;
            let next = ((page + 1) * self.page_size).min(end);
            let owner = self.owner(u32::try_from(page).expect("page number"));
            let range = owner.map_or(0..0, |(_, position)| {
                let base = position * self.page_size;
                base + inner..base + inner + (next - at)
            });
            visit(owner, range);
            at = next;
        }
    }
}
