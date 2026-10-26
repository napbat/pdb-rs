//! Behavior tests for the active free page map check.
//!
//! Each image has 512-byte pages, the page map in page 3 and the stream directory in page 4.

use core::ops::Range;

use pdbrs::ContainerKind;
use pdbrs::ErrorKind;
use pdbrs::native::msf::Msf;

const PAGE: usize = 512;

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).expect("test value fits in u32")
}

fn to_usize(value: u32) -> usize {
    usize::try_from(value).expect("test value fits in usize")
}

fn put(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn is_reserved(page: u32) -> bool {
    page == 0 || matches!(page % 512, 1 | 2)
}

/// A container with a consistent active free page map.
struct Image {
    bytes: Vec<u8>,
    active: u32,
}

impl Image {
    /// Build an image. `streams` holds the size and the pages of each stream.
    fn new(page_count: u32, active: u32, streams: &[(u32, Vec<u32>)]) -> Self {
        let mut bytes = vec![0; to_usize(page_count) * PAGE];
        let signature = ContainerKind::NativeMsf7.signature();
        bytes[..signature.len()].copy_from_slice(signature);
        let words: Vec<u32> = core::iter::once(to_u32(streams.len()))
            .chain(streams.iter().map(|(size, _)| *size))
            .chain(streams.iter().flat_map(|(_, pages)| pages.iter().copied()))
            .collect();
        assert!(words.len() * 4 <= PAGE, "directory fits in one page");
        for (offset, value) in [
            (32, 512),
            (36, active),
            (40, page_count),
            (44, to_u32(words.len() * 4)),
            (52, 3),
        ] {
            put(&mut bytes, offset, value);
        }
        put(&mut bytes, 3 * PAGE, 4);
        for (index, &word) in words.iter().enumerate() {
            put(&mut bytes, 4 * PAGE + index * 4, word);
        }
        let mut image = Self { bytes, active };
        // Fill the inactive map with allocated bits; the reader must ignore it.
        let mut fpm_page = 3 - active;
        while fpm_page < page_count {
            let start = to_usize(fpm_page) * PAGE;
            image.bytes[start..start + PAGE].fill(0);
            fpm_page += 512;
        }
        let mut used = vec![false; to_usize(page_count)];
        for page in [3, 4].into_iter().chain(
            streams
                .iter()
                .skip(1)
                .flat_map(|(_, pages)| pages.iter().copied()),
        ) {
            used[to_usize(page)] = true;
        }
        for page in 0..page_count {
            image.set_free(page, !is_reserved(page) && !used[to_usize(page)]);
        }
        image
    }

    /// Return the file offset of the bitmap byte of `page` in the active map.
    fn fpm_offset(&self, page: u32) -> usize {
        let byte = to_usize(page / 8);
        (to_usize(self.active) + 512 * (byte / PAGE)) * PAGE + byte % PAGE
    }

    fn set_free(&mut self, page: u32, free: bool) {
        let offset = self.fpm_offset(page);
        let mask = 1 << (page % 8);
        if free {
            self.bytes[offset] |= mask;
        } else {
            self.bytes[offset] &= !mask;
        }
    }

    fn msf(&self) -> Msf<&[u8]> {
        Msf::read(self.bytes.as_slice()).expect("valid image opens")
    }
}

/// Three streams: stream 0 in pages 5 and 6, stream 1 in pages 7 and 9, stream 2 in page 10.
fn basic(active: u32) -> Image {
    let streams = [(600, vec![5, 6]), (1000, vec![7, 9]), (20, vec![10])];
    Image::new(16, active, &streams)
}

#[test]
fn consistent_maps_report_free_old_directory_pages_and_no_leaks() {
    for active in [1, 2] {
        let image = basic(active);
        let report = image.msf().check_free_page_map().unwrap();
        assert_eq!(report.active(), active);
        assert_eq!(report.old_directory_free_pages(), 2);
        assert_eq!(report.old_directory_allocated_pages(), 0);
        assert_eq!(report.unreferenced_allocated_count(), 0);
    }
}

#[test]
fn allocated_old_directory_pages_are_accepted_and_counted() {
    let mut image = basic(1);
    image.set_free(6, false);
    let report = image.msf().check_free_page_map().unwrap();
    assert_eq!(report.old_directory_free_pages(), 1);
    assert_eq!(report.old_directory_allocated_pages(), 1);
}

#[test]
fn allocated_pages_without_a_reference_are_reported_as_runs() {
    let mut image = basic(2);
    for page in [8, 11, 12, 13, 15] {
        image.set_free(page, false);
    }
    let report = image.msf().check_free_page_map().unwrap();
    let expected: [Range<u32>; 3] = [8..9, 11..14, 15..16];
    assert_eq!(report.unreferenced_allocated(), &expected);
    assert_eq!(report.unreferenced_allocated_count(), 5);
}

#[test]
fn free_referenced_pages_are_rejected_at_their_bitmap_byte() {
    // The header, both map pages, the page map, the directory and the stream pages.
    for page in [0, 1, 2, 3, 4, 7, 9, 10] {
        let mut image = basic(1);
        image.set_free(page, true);
        let error = image.msf().check_free_page_map().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat, "page {page}");
        let offset = u64::try_from(image.fpm_offset(page)).unwrap();
        assert_eq!(error.offset(), Some(offset), "page {page}");
    }
}

#[test]
fn bits_after_the_declared_pages_are_ignored() {
    let mut image = basic(1);
    // With 13 pages, bits 5 to 7 of bitmap byte 1 describe no page.
    put(&mut image.bytes, 40, 13);
    image.bytes.truncate(13 * PAGE);
    let offset = image.fpm_offset(13);
    image.bytes[offset] &= 0b0001_1111;
    let report = image.msf().check_free_page_map().unwrap();
    assert_eq!(report.unreferenced_allocated_count(), 0);
}

#[test]
fn the_bitmap_continues_in_the_map_page_of_the_next_interval() {
    // 512-byte map pages describe 4096 pages, so pages from 4096 use the map in page 513.
    let page_count = 4096 + 600;
    let streams = [(0, Vec::new()), (1024, vec![4100, 4101])];
    let mut image = Image::new(page_count, 1, &streams);
    let report = image.msf().check_free_page_map().unwrap();
    assert_eq!(report.unreferenced_allocated_count(), 0);
    assert_eq!(image.fpm_offset(4100), 513 * PAGE);

    image.set_free(4101, true);
    let error = image.msf().check_free_page_map().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(error.offset(), Some(u64::try_from(513 * PAGE).unwrap()));

    let mut image = Image::new(page_count, 1, &streams);
    for reserved in [513, 514, 4097, 4098] {
        assert!(is_reserved(reserved));
    }
    image.set_free(4098, true);
    let error = image.msf().check_free_page_map().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
}
