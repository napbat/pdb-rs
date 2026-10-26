//! Behavior tests for Big MSF 7.00 container reading.
//!
//! Each fixture is a valid container. Corruption tests change one field of a valid fixture.

use core::cell::RefCell;
use core::fmt::Debug;

use pdbrs::ContainerKind;
use pdbrs::io::ReadAt;
use pdbrs::native::msf::{Msf, MsfLimits, StreamId};
use pdbrs::{Error, ErrorKind, Result};

const PAGE_SIZES: [u32; 8] = [512, 1024, 2048, 4096, 8192, 16_384, 32_768, 65_536];

/// The page allocation order of a fixture.
#[derive(Clone, Copy, Debug)]
enum Order {
    Ascending,
    Descending,
    Interleaved,
}

/// A valid container and the location of each page reference in it.
struct Fixture {
    bytes: Vec<u8>,
    block_size: u32,
    page_count: u32,
    maps: Vec<u32>,
    dirs: Vec<u32>,
    stream_pages: Vec<Vec<u32>>,
    contents: Vec<Option<Vec<u8>>>,
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).expect("test value fits in u32")
}

fn to_usize(value: u32) -> usize {
    usize::try_from(value).expect("test value fits in usize")
}

fn to_u64(value: usize) -> u64 {
    u64::try_from(value).expect("test value fits in u64")
}

fn put(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn is_reserved(page: u32, block_size: u32) -> bool {
    page == 0 || matches!(page % block_size, 1 | 2)
}

/// Return bytes that differ between pages for each page size.
fn pattern(length: usize, seed: u8) -> Vec<u8> {
    (0..length)
        .map(|index| (index ^ (index >> 8) ^ (index >> 16)).to_le_bytes()[0] ^ seed)
        .collect()
}

impl Fixture {
    fn build(block_size: u32, contents: Vec<Option<Vec<u8>>>, order: Order) -> Self {
        let page_len = to_usize(block_size);
        let counts: Vec<usize> = contents
            .iter()
            .map(|content| {
                content
                    .as_ref()
                    .map_or(0, |data| data.len().div_ceil(page_len))
            })
            .collect();
        let total: usize = counts.iter().sum();
        let size = 4 * (1 + contents.len() + total);
        let dir_count = size.div_ceil(page_len);
        let map_count = (dir_count * 4).div_ceil(page_len);
        let mut free: Vec<u32> = (3..)
            .filter(|&page| !is_reserved(page, block_size))
            .take(map_count + dir_count + total)
            .collect();
        let page_count = free.last().map_or(3, |&page| page + 1);
        match order {
            Order::Ascending => {}
            Order::Descending => free.reverse(),
            Order::Interleaved => {
                let even = free.iter().step_by(2);
                let odd = free.iter().skip(1).step_by(2);
                free = even.chain(odd).copied().collect();
            }
        }
        let mut free = free.into_iter();
        let maps: Vec<u32> = free.by_ref().take(map_count).collect();
        let dirs: Vec<u32> = free.by_ref().take(dir_count).collect();
        let stream_pages: Vec<Vec<u32>> = counts
            .iter()
            .map(|&count| free.by_ref().take(count).collect())
            .collect();

        let mut fixture = Self {
            bytes: vec![0; to_usize(page_count) * page_len],
            block_size,
            page_count,
            maps,
            dirs,
            stream_pages,
            contents,
        };
        fixture.write_structures(to_u32(size));
        fixture
    }

    fn write_structures(&mut self, size: u32) {
        let signature = ContainerKind::NativeMsf7.signature();
        self.bytes[..signature.len()].copy_from_slice(signature);
        for (offset, value) in [
            (32, self.block_size),
            (36, 1),
            (40, self.page_count),
            (44, size),
        ] {
            put(&mut self.bytes, offset, value);
        }
        for (index, &page) in self.maps.iter().enumerate() {
            put(&mut self.bytes, 52 + 4 * index, page);
        }
        for (index, &page) in self.dirs.iter().enumerate() {
            let offset = self.word_offset(&self.maps, index);
            put(&mut self.bytes, offset, page);
        }
        let sizes = self
            .contents
            .iter()
            .map(|content| content.as_ref().map_or(u32::MAX, |data| to_u32(data.len())));
        let words: Vec<u32> = core::iter::once(to_u32(self.contents.len()))
            .chain(sizes)
            .chain(self.stream_pages.iter().flatten().copied())
            .collect();
        for (index, &word) in words.iter().enumerate() {
            let offset = self.word_offset(&self.dirs, index);
            put(&mut self.bytes, offset, word);
        }
        let page_len = to_usize(self.block_size);
        for (pages, content) in self.stream_pages.iter().zip(&self.contents) {
            let data = content.as_deref().unwrap_or_default();
            for (&page, chunk) in pages.iter().zip(data.chunks(page_len)) {
                let start = to_usize(page) * page_len;
                self.bytes[start..start + chunk.len()].copy_from_slice(chunk);
            }
        }
        self.write_free_page_maps();
    }

    /// Write the active free page map 1 and fill the inactive map 2 with free bits.
    ///
    /// The active map marks the stream 0 pages and the bits after the declared pages as free.
    /// Every other declared page is allocated. The reader must ignore the inactive map.
    fn write_free_page_maps(&mut self) {
        let page_len = to_usize(self.block_size);
        let bitmap_len = to_usize(self.page_count.div_ceil(8));
        let old_pages = self.stream_pages.first().cloned().unwrap_or_default();
        for page in old_pages
            .into_iter()
            .chain(self.page_count..to_u32(bitmap_len * 8))
        {
            let byte = to_usize(page / 8);
            let fpm_page = 1 + to_usize(self.block_size) * (byte / page_len);
            self.bytes[fpm_page * page_len + byte % page_len] |= 1 << (page % 8);
        }
        for chunk in 0..bitmap_len.div_ceil(page_len) {
            let start = (2 + chunk * page_len) * page_len;
            self.bytes[start..start + page_len].fill(0xff);
        }
    }

    fn word_offset(&self, pages: &[u32], index: usize) -> usize {
        let page_len = to_usize(self.block_size);
        let byte = index * 4;
        to_usize(pages[byte / page_len]) * page_len + byte % page_len
    }

    /// Return the file offset of the entry for directory page `index` in the page map.
    fn map_entry(&self, index: usize) -> usize {
        self.word_offset(&self.maps, index)
    }

    /// Return the file offset of the size of `stream` in the directory.
    fn stream_size(&self, stream: usize) -> usize {
        self.word_offset(&self.dirs, 1 + stream)
    }

    /// Return the file offset of page number `page` of `stream` in the directory.
    fn stream_page(&self, stream: usize, page: usize) -> usize {
        let before: usize = self.stream_pages[..stream].iter().map(Vec::len).sum();
        self.word_offset(&self.dirs, 1 + self.contents.len() + before + page)
    }

    fn with(&self, offset: usize, value: u32) -> Vec<u8> {
        let mut bytes = self.bytes.clone();
        put(&mut bytes, offset, value);
        bytes
    }

    fn open(&self) -> Msf<&[u8]> {
        Msf::read(self.bytes.as_slice()).expect("valid fixture opens")
    }
}

/// A container with two page map pages and data that crosses into the second interval.
fn wide_fixture(order: Order) -> Fixture {
    let mut contents: Vec<Option<Vec<u8>>> = (0..16_500)
        .map(|index: usize| (!index.is_multiple_of(3)).then(Vec::new))
        .collect();
    contents[0] = Some(pattern(100, 7));
    contents[1] = Some(pattern(600 * 512 + 17, 1));
    contents[16_499] = Some(pattern(1500, 2));
    let fixture = Fixture::build(512, contents, order);
    assert!(
        fixture.maps.len() >= 2,
        "fixture needs several page map pages"
    );
    fixture
}

/// A container with a small stream 0 and one data stream.
fn two_stream_fixture(block_size: u32) -> Fixture {
    let contents = vec![Some(Vec::new()), Some(pattern(10, 1))];
    Fixture::build(block_size, contents, Order::Ascending)
}

/// A small container with a two-page directory.
fn small_fixture() -> Fixture {
    let mut contents: Vec<Option<Vec<u8>>> = vec![Some(Vec::new()); 200];
    contents[0] = Some(pattern(40, 9));
    contents[1] = Some(pattern(1100, 1));
    contents[2] = Some(pattern(300, 2));
    contents[3] = None;
    contents[199] = Some(pattern(700, 3));
    let fixture = Fixture::build(512, contents, Order::Interleaved);
    assert!(
        fixture.dirs.len() >= 2,
        "fixture needs several directory pages"
    );
    fixture
}

fn expect_error<T: Debug>(
    result: Result<T>,
    kind: ErrorKind,
    offset: Option<u64>,
    case: &str,
) -> Error {
    match result {
        Ok(value) => panic!("{case}: expected {kind:?}, got {value:?}"),
        Err(error) => {
            assert_eq!(error.kind(), kind, "{case}: {error}");
            assert_eq!(error.offset(), offset, "{case}: {error}");
            error
        }
    }
}

fn open_bytes(bytes: &[u8]) -> Result<Msf<&[u8]>> {
    Msf::read(bytes)
}

/// Check every stream of `msf` against the fixture contents.
fn check_contents(fixture: &Fixture, msf: &Msf<&[u8]>) {
    assert_eq!(msf.stream_count(), to_u32(fixture.contents.len()));
    for (index, expected) in fixture.contents.iter().enumerate() {
        let id = StreamId(to_u32(index));
        let stream = msf.stream(id).expect("stream ID is in range");
        match (expected, stream) {
            (None, None) => {}
            (Some(expected), Some(stream)) => {
                assert_eq!(stream.id(), id);
                assert_eq!(stream.len().unwrap(), to_u64(expected.len()));
                let bytes = stream.read_all(expected.len()).unwrap();
                assert_eq!(&bytes, expected, "stream {index}");
            }
            (expected, stream) => panic!("stream {index}: expected {expected:?}, got {stream:?}"),
        }
    }
}

#[test]
fn every_page_size_reads_fragmented_streams() {
    for block_size in PAGE_SIZES {
        for order in [Order::Ascending, Order::Descending, Order::Interleaved] {
            let page = to_usize(block_size);
            let contents = vec![
                Some(pattern(13, 5)),
                Some(pattern(page * 3 + page / 2, 1)),
                None,
                Some(Vec::new()),
                Some(pattern(page, 3)),
            ];
            let fixture = Fixture::build(block_size, contents, order);
            let msf = fixture.open();
            assert_eq!(msf.block_size(), block_size);
            check_contents(&fixture, &msf);

            let stream = msf.stream(StreamId(1)).unwrap().unwrap();
            let expected = fixture.contents[1].as_deref().unwrap();
            for boundary in [page, 2 * page, 3 * page] {
                for start in boundary - 7..boundary {
                    let offset = to_u64(start);
                    let case = format!("{block_size} {order:?} offset {start}");
                    let bytes: [u8; 8] = expected[start..start + 8].try_into().unwrap();
                    let long = u64::from_le_bytes(bytes);
                    assert_eq!(stream.read_u64_at(offset).unwrap(), long, "{case}");
                    let word = u32::from_le_bytes(bytes[..4].try_into().unwrap());
                    assert_eq!(stream.read_u32_at(offset).unwrap(), word, "{case}");
                    let half = u16::from_le_bytes(bytes[..2].try_into().unwrap());
                    assert_eq!(stream.read_u16_at(offset).unwrap(), half, "{case}");
                }
            }
            let mut span = vec![0; 2 * page + 3];
            stream.read_exact_at(to_u64(page / 2), &mut span).unwrap();
            assert_eq!(span, expected[page / 2..page / 2 + span.len()]);
        }
    }
}

#[test]
fn page_sizes_outside_the_valid_powers_of_two_are_rejected() {
    let fixture = two_stream_fixture(4096);
    for value in [
        0,
        1,
        256,
        511,
        513,
        3 * 1024,
        4095,
        131_072,
        1 << 31,
        u32::MAX,
    ] {
        let bytes = fixture.with(32, value);
        let case = format!("page size {value}");
        expect_error(
            open_bytes(&bytes),
            ErrorKind::InvalidFormat,
            Some(32),
            &case,
        );
    }
}

#[test]
fn active_free_page_map_must_be_one_or_two() {
    let fixture = two_stream_fixture(4096);
    let bytes = fixture.with(36, 2);
    open_bytes(&bytes).expect("free page map 2 is valid");
    for value in [0, 3, 4096, u32::MAX] {
        let bytes = fixture.with(36, value);
        let case = format!("free page map {value}");
        expect_error(
            open_bytes(&bytes),
            ErrorKind::InvalidFormat,
            Some(36),
            &case,
        );
    }
}

#[test]
fn other_signatures_are_rejected_and_small_msf_is_unsupported() {
    let mut legacy = ContainerKind::LegacyMsf2.signature().to_vec();
    legacy.resize(1024, 0);
    expect_error(
        open_bytes(&legacy),
        ErrorKind::UnsupportedVersion,
        Some(0),
        "small MSF",
    );

    let mut metadata = ContainerKind::CliMetadata.signature().to_vec();
    metadata.resize(1024, 0);
    expect_error(
        open_bytes(&metadata),
        ErrorKind::InvalidSignature,
        Some(0),
        "CLI metadata",
    );

    let fixture = Fixture::build(512, vec![Some(Vec::new())], Order::Ascending);
    let mut bytes = fixture.bytes.clone();
    bytes[24] = b'X';
    expect_error(
        open_bytes(&bytes),
        ErrorKind::InvalidSignature,
        Some(0),
        "changed signature",
    );
}

#[test]
fn truncated_input_is_rejected_and_trailing_bytes_are_ignored() {
    let contents = vec![Some(pattern(5, 1)), Some(pattern(3000, 2))];
    let fixture = Fixture::build(1024, contents, Order::Descending);
    let length = fixture.bytes.len();
    for (end, case) in [
        (length - 1, "last byte removed"),
        (40, "short header"),
        (20, "short signature"),
    ] {
        let result = open_bytes(&fixture.bytes[..end]);
        expect_error(result, ErrorKind::UnexpectedEof, Some(to_u64(end)), case);
    }

    let mut long = fixture.bytes.clone();
    long.extend_from_slice(&[0xA5; 100]);
    let msf = open_bytes(&long).expect("trailing bytes are ignored");
    check_contents(&fixture, &msf);
}

#[test]
fn directory_spans_several_page_map_pages() {
    for order in [Order::Ascending, Order::Descending] {
        let fixture = wide_fixture(order);
        let msf = fixture.open();
        check_contents(&fixture, &msf);
    }
}

#[test]
fn stream_data_crosses_reserved_pages_of_a_later_interval() {
    let fixture = wide_fixture(Order::Ascending);
    let pages = &fixture.stream_pages[1];
    let gap = pages
        .iter()
        .position(|&page| page == 512)
        .expect("stream uses page 512");
    assert_eq!(pages[gap + 1], 515, "pages 513 and 514 are reserved");
    let msf = fixture.open();
    let stream = msf.stream(StreamId(1)).unwrap().unwrap();
    let expected = fixture.contents[1].as_deref().unwrap();
    let start = gap * 512 - 3;
    let mut bytes = vec![0; 2 * 512 + 6];
    stream.read_exact_at(to_u64(start), &mut bytes).unwrap();
    assert_eq!(bytes, expected[start..start + bytes.len()]);
}

#[test]
fn stream_ids_outside_the_directory_are_rejected() {
    let fixture = small_fixture();
    let msf = fixture.open();
    let count = msf.stream_count();
    msf.stream(StreamId(count - 1))
        .expect("last stream is valid");
    for id in [count, count + 1, u32::MAX] {
        let result = msf.stream(StreamId(id));
        let error = expect_error(result, ErrorKind::InvalidStreamIndex, None, "stream index");
        assert_eq!(error.stream_location(), Some((StreamId(id), 0)));
    }
}

#[test]
fn nil_and_empty_streams_stay_distinct() {
    let fixture = small_fixture();
    let msf = fixture.open();
    assert!(
        msf.stream(StreamId(3)).unwrap().is_none(),
        "stream 3 is nil"
    );
    let empty = msf
        .stream(StreamId(4))
        .unwrap()
        .expect("stream 4 is empty, not nil");
    assert_eq!(empty.len().unwrap(), 0);
    assert_eq!(empty.read_all(0).unwrap(), [] as [u8; 0]);
}

#[test]
fn stream_zero_may_be_nil_or_empty_and_its_contents_are_not_a_directory() {
    let mut directory_like = Vec::new();
    for word in [u32::MAX, 0, 7, u32::MAX] {
        directory_like.extend_from_slice(&word.to_le_bytes());
    }
    for first in [None, Some(Vec::new()), Some(directory_like)] {
        let contents = vec![first, Some(pattern(600, 1))];
        let fixture = Fixture::build(512, contents, Order::Descending);
        let msf = fixture.open();
        check_contents(&fixture, &msf);
    }
}

#[test]
fn stream_reads_stop_at_the_stream_end() {
    let fixture = small_fixture();
    let msf = fixture.open();
    let stream = msf.stream(StreamId(1)).unwrap().unwrap();
    let length = stream.len().unwrap();
    stream
        .read_exact_at(length, &mut [])
        .expect("empty read at the end is valid");

    let cases: [(u64, usize, ErrorKind, u64); 3] = [
        (length + 1, 0, ErrorKind::UnexpectedEof, length + 1),
        (length - 3, 4, ErrorKind::UnexpectedEof, length),
        (u64::MAX, 1, ErrorKind::OffsetOverflow, u64::MAX),
    ];
    for (offset, count, kind, location) in cases {
        let mut bytes = vec![0; count];
        let case = format!("{count} bytes at {offset}");
        let error = expect_error(stream.read_exact_at(offset, &mut bytes), kind, None, &case);
        assert_eq!(
            error.stream_location(),
            Some((StreamId(1), location)),
            "{case}"
        );
    }

    let size = fixture.contents[1].as_ref().unwrap().len();
    let result = stream.read_all(size - 1);
    let error = expect_error(result, ErrorKind::LimitExceeded, None, "read limit");
    assert_eq!(error.stream_location(), Some((StreamId(1), 0)));
    assert_eq!(stream.read_all(size).unwrap().len(), size);
}

#[test]
fn aliased_stream_pages_are_rejected_at_the_later_reference() {
    let fixture = small_fixture();
    let first = fixture.stream_pages[1][0];
    let stream_page = fixture.stream_page(1, 0);
    let cases = [
        (
            fixture.stream_page(1, 1),
            first,
            None,
            "page twice in one stream",
        ),
        (
            fixture.stream_page(2, 0),
            first,
            None,
            "page in two streams",
        ),
        (
            stream_page,
            fixture.dirs[0],
            None,
            "stream page is a directory page",
        ),
        (
            stream_page,
            fixture.maps[0],
            None,
            "stream page is a page map page",
        ),
        (
            fixture.stream_page(0, 0),
            fixture.stream_pages[199][1],
            Some(fixture.stream_page(199, 1)),
            "old directory page in a later stream",
        ),
    ];
    for (offset, page, later, case) in cases {
        let bytes = fixture.with(offset, page);
        let expected = {
            let offset = later.unwrap_or(offset);
            Some(to_u64(offset))
        };
        let error = expect_error(
            open_bytes(&bytes),
            ErrorKind::InvalidReference,
            expected,
            case,
        );
        assert_eq!(
            error.context(),
            Some("page number is used more than once"),
            "{case}"
        );
    }
}

#[test]
fn aliased_directory_pages_are_rejected_before_the_directory_is_read() {
    let fixture = small_fixture();
    let cases = [
        (
            fixture.map_entry(1),
            fixture.dirs[0],
            "directory page twice",
        ),
        (
            fixture.map_entry(0),
            fixture.maps[0],
            "directory page is a page map page",
        ),
    ];
    for (offset, page, case) in cases {
        let bytes = fixture.with(offset, page);
        let error = expect_error(
            open_bytes(&bytes),
            ErrorKind::InvalidReference,
            Some(to_u64(offset)),
            case,
        );
        assert_eq!(
            error.context(),
            Some("page number is used more than once"),
            "{case}"
        );
    }

    // Ascending allocation puts the page map pages first, so their duplicate sorts first.
    let wide = wide_fixture(Order::Ascending);
    let bytes = wide.with(56, wide.maps[0]);
    expect_error(
        open_bytes(&bytes),
        ErrorKind::InvalidReference,
        Some(56),
        "page map page twice",
    );
}

#[test]
fn reserved_and_undeclared_pages_are_rejected() {
    let fixture = wide_fixture(Order::Ascending);
    assert!(fixture.page_count > 515, "fixture covers a second interval");
    let reserved = Some("page number is the header page or a free page map page");
    let outside = Some("page number is outside the declared pages");
    let locations = [
        (52, "page map entry"),
        (fixture.map_entry(1), "directory page"),
        (fixture.stream_page(1, 3), "stream page"),
        (fixture.stream_page(0, 0), "old directory page"),
    ];
    for (offset, role) in locations {
        let pages = [0, 1, 2, 513, 514].map(|page| (page, reserved));
        let undeclared = [fixture.page_count, u32::MAX].map(|page| (page, outside));
        for (page, context) in pages.into_iter().chain(undeclared) {
            let bytes = fixture.with(offset, page);
            let case = format!("{role} {page}");
            let result = open_bytes(&bytes);
            let error = expect_error(
                result,
                ErrorKind::InvalidReference,
                Some(to_u64(offset)),
                &case,
            );
            assert_eq!(error.context(), context, "{case}");
        }
    }

    let small = small_fixture();
    let last = small.page_count - 1;
    let (stream, index) = small
        .stream_pages
        .iter()
        .enumerate()
        .find_map(|(stream, pages)| Some((stream, pages.iter().position(|&page| page == last)?)))
        .expect("a stream uses the last page");
    let bytes = small.with(40, last);
    let expected = {
        let offset = small.stream_page(stream, index);
        Some(to_u64(offset))
    };
    let result = open_bytes(&bytes);
    let error = expect_error(result, ErrorKind::InvalidReference, expected, "fewer pages");
    assert_eq!(error.context(), outside);
}

#[test]
fn directory_size_must_match_the_directory_contents() {
    let fixture = small_fixture();
    let page_total: usize = fixture.stream_pages.iter().map(Vec::len).sum();
    let size = 4 * (1 + fixture.contents.len() + page_total);
    assert!(
        !size.is_multiple_of(512),
        "a larger size stays in the same directory pages"
    );
    let size = to_u32(size);
    for (value, case) in [
        (size + 2, "size not a multiple of 4"),
        (0, "empty directory"),
        (4, "directory without stream 0"),
        (size + 4, "unused directory word"),
    ] {
        let bytes = fixture.with(44, value);
        expect_error(open_bytes(&bytes), ErrorKind::InvalidFormat, Some(44), case);
    }

    let count_offset = fixture.word_offset(&fixture.dirs, 0);
    for (value, case) in [(0, "no streams"), (size / 4, "stream sizes past the end")] {
        let bytes = fixture.with(count_offset, value);
        expect_error(
            open_bytes(&bytes),
            ErrorKind::InvalidFormat,
            Some(to_u64(count_offset)),
            case,
        );
    }

    let last = fixture.contents.len() - 1;
    let grown = to_u32(fixture.contents[last].as_ref().unwrap().len()) + 512;
    let offset = fixture.stream_size(last);
    let bytes = fixture.with(offset, grown);
    expect_error(
        open_bytes(&bytes),
        ErrorKind::InvalidFormat,
        Some(to_u64(offset)),
        "one more page",
    );
}

#[test]
fn page_map_array_must_fit_in_the_header_page() {
    let fixture = two_stream_fixture(512);
    // At 512 bytes, the header page holds 115 page map entries, and each entry covers 128 pages.
    let largest = 115 * 128 * 512;
    let bytes = fixture.with(44, largest + 4);
    expect_error(
        open_bytes(&bytes),
        ErrorKind::InvalidFormat,
        Some(44),
        "116 page map pages",
    );
    let bytes = fixture.with(44, largest);
    let result = open_bytes(&bytes);
    let error = expect_error(
        result,
        ErrorKind::InvalidReference,
        Some(56),
        "115 page map pages",
    );
    assert_eq!(
        error.context(),
        Some("page number is the header page or a free page map page")
    );
}

/// A source that records each read and can fail reads that touch one byte.
struct Recorder<'a> {
    bytes: &'a [u8],
    reads: RefCell<Vec<(u64, usize)>>,
    fail_at: Option<u64>,
}

impl<'a> Recorder<'a> {
    fn new(bytes: &'a [u8], fail_at: Option<u64>) -> Self {
        Self {
            bytes,
            reads: RefCell::new(Vec::new()),
            fail_at,
        }
    }

    fn take_reads(&self) -> Vec<(u64, usize)> {
        self.reads.take()
    }
}

impl ReadAt for Recorder<'_> {
    fn len(&self) -> Result<u64> {
        ReadAt::len(self.bytes)
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        self.reads.borrow_mut().push((offset, buffer.len()));
        let end = offset + to_u64(buffer.len());
        if let Some(fail) = self.fail_at.filter(|fail| (offset..end).contains(fail)) {
            return Err(Error::new(ErrorKind::Io, Some(fail)));
        }
        self.bytes.read_exact_at(offset, buffer)
    }
}

#[test]
fn directory_limit_is_checked_before_allocation() {
    let fixture = small_fixture();
    let size = fixture.bytes.as_slice().read_u32_at(44).unwrap();
    let limits = MsfLimits::default().with_max_directory_bytes(size - 4);
    let result = Msf::with_limits(fixture.bytes.as_slice(), limits);
    expect_error(result, ErrorKind::LimitExceeded, Some(44), "limit");
    let limits = MsfLimits::default().with_max_directory_bytes(size);
    Msf::with_limits(fixture.bytes.as_slice(), limits).expect("directory at the limit");

    // This size needs only 4 page map entries at 65536 bytes per page.
    let large = Fixture::build(65_536, vec![Some(Vec::new())], Order::Ascending);
    let bytes = large.with(44, u32::MAX - 3);
    let source = Recorder::new(&bytes, None);
    expect_error(
        Msf::read(&source),
        ErrorKind::LimitExceeded,
        Some(44),
        "default limit",
    );
    let end = source
        .take_reads()
        .iter()
        .map(|&(offset, count)| offset + to_u64(count))
        .max();
    assert_eq!(end, Some(48), "only the header fields are read");
}

#[test]
fn opening_reads_no_stream_payload_and_reads_copy_only_requested_bytes() {
    let fixture = small_fixture();
    let page_len = to_u64(to_usize(fixture.block_size));
    let source = Recorder::new(&fixture.bytes, None);
    let msf = Msf::read(&source).unwrap();
    let payload: Vec<u64> = fixture
        .stream_pages
        .iter()
        .flatten()
        .map(|&page| u64::from(page) * page_len)
        .collect();
    for (offset, count) in source.take_reads() {
        let end = offset + to_u64(count);
        let touches = payload
            .iter()
            .any(|&start| start < end && offset < start + page_len);
        assert!(!touches, "open read {offset:#x}+{count} touches a payload");
    }

    let stream = msf.stream(StreamId(1)).unwrap().unwrap();
    let mut bytes = [0; 600];
    stream.read_exact_at(100, &mut bytes).unwrap();
    let reads = source.take_reads();
    assert_eq!(
        reads.iter().map(|&(_, count)| count).sum::<usize>(),
        bytes.len()
    );
    let pages = &fixture.stream_pages[1];
    let first = u64::from(pages[0]) * page_len + 100;
    assert_eq!(reads.first(), Some(&(first, 412)));
}

#[test]
fn source_errors_keep_the_physical_offset_and_add_the_stream_location() {
    let fixture = small_fixture();
    let page = fixture.stream_pages[1][1];
    let failing = u64::from(page) * 512 + 20;
    let source = Recorder::new(&fixture.bytes, Some(failing));
    let msf = Msf::read(&source).unwrap();
    let stream = msf.stream(StreamId(1)).unwrap().unwrap();
    let mut bytes = [0; 100];
    let result = stream.read_exact_at(500, &mut bytes);
    let error = expect_error(result, ErrorKind::Io, Some(failing), "failed page");
    assert_eq!(error.stream_location(), Some((StreamId(1), 512)));
}

#[test]
fn free_page_maps_of_all_fixtures_agree_with_the_directory() {
    let mut fixtures: Vec<Fixture> = PAGE_SIZES.into_iter().map(two_stream_fixture).collect();
    fixtures.push(small_fixture());
    fixtures.push(wide_fixture(Order::Ascending));
    fixtures.push(wide_fixture(Order::Interleaved));
    for fixture in &fixtures {
        let msf = fixture.open();
        assert_eq!(msf.page_count(), fixture.page_count);
        assert_eq!(msf.active_free_page_map(), 1);
        let report = msf.check_free_page_map().unwrap();
        assert_eq!(report.active(), 1);
        assert_eq!(
            report.unreferenced_allocated(),
            &[] as &[core::ops::Range<u32>]
        );
        assert_eq!(report.unreferenced_allocated_count(), 0);
        let old_pages = to_u32(fixture.stream_pages[0].len());
        assert_eq!(report.old_directory_free_pages(), old_pages);
        assert_eq!(report.old_directory_allocated_pages(), 0);
    }
}
