//! Behavior tests for Big MSF 7.00 container writing.
//!
//! Each test writes a container and opens the output with the reader. The reader checks the
//! structure, and `validate_free_page_map` checks the active free page map.

use pdbrs::io::ReadAt;
use pdbrs::io::copy_exact;
use pdbrs::native::msf::{Msf, MsfBuilder, MsfLimits, OldDirectoryPolicy, StreamId, WriteOptions};
use pdbrs::{Error, ErrorKind, Result};

/// The page sizes that the writer writes. The reader also accepts 65536.
const PAGE_SIZES: [u32; 7] = [512, 1024, 2048, 4096, 8192, 16_384, 32_768];

/// A source of `len` bytes with the value of `offset % 251` at each offset.
///
/// The source does not allocate memory for its bytes.
#[derive(Clone, Copy, Debug)]
struct Pattern(u64);

impl ReadAt for Pattern {
    fn len(&self) -> Result<u64> {
        Ok(self.0)
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        let end = offset + u64::try_from(buffer.len()).unwrap();
        if end > self.0 {
            return Err(Error::new(ErrorKind::UnexpectedEof, Some(self.0)));
        }
        for (position, byte) in (offset..).zip(buffer.iter_mut()) {
            *byte = u8::try_from(position % 251).unwrap();
        }
        Ok(())
    }
}

/// A source with a length whose reads fail.
#[derive(Clone, Copy, Debug)]
struct Failing(u64);

impl ReadAt for Failing {
    fn len(&self) -> Result<u64> {
        Ok(self.0)
    }

    fn read_exact_at(&self, offset: u64, _buffer: &mut [u8]) -> Result<()> {
        Err(Error::new(ErrorKind::Io, Some(offset)))
    }
}

fn bytes(length: usize, seed: u8) -> Vec<u8> {
    (0..length)
        .map(|index| (index ^ (index >> 8)).to_le_bytes()[0] ^ seed)
        .collect()
}

fn options(page_size: u32) -> WriteOptions {
    WriteOptions::default().with_page_size(page_size)
}

/// Return streams with nil, empty, short, page-sized and multi-page contents.
fn sample_streams(page_size: u32) -> Vec<Option<Vec<u8>>> {
    let page = usize::try_from(page_size).unwrap();
    vec![
        Some(bytes(100, 0)),
        Some(Vec::new()),
        None,
        Some(bytes(1, 1)),
        Some(bytes(page, 2)),
        Some(bytes(page * 2 + 7, 3)),
        None,
        Some(Vec::new()),
    ]
}

fn builder(streams: &[Option<Vec<u8>>]) -> MsfBuilder<&[u8]> {
    let mut builder = MsfBuilder::new();
    for (index, stream) in streams.iter().enumerate() {
        let id = match stream {
            Some(data) => builder.push(data.as_slice()).unwrap(),
            None => builder.push_nil().unwrap(),
        };
        assert_eq!(id, StreamId(u32::try_from(index).unwrap()));
    }
    builder
}

/// Open `bytes` and compare the streams with `expected`.
fn check_streams<'a>(bytes: &'a [u8], expected: &[Option<Vec<u8>>]) -> Msf<&'a [u8]> {
    let msf = Msf::read(bytes).unwrap();
    assert_eq!(msf.stream_count(), u32::try_from(expected.len()).unwrap());
    for (index, content) in expected.iter().enumerate() {
        let stream = msf.stream(StreamId(u32::try_from(index).unwrap())).unwrap();
        match (stream, content) {
            (None, None) => {}
            (Some(stream), Some(content)) => {
                assert_eq!(
                    stream.read_all(usize::MAX).unwrap(),
                    *content,
                    "stream {index}"
                );
            }
            (stream, content) => panic!("stream {index}: {stream:?} is not {content:?}"),
        }
    }
    msf
}

/// Check the free page map and the file size of a written container.
fn check_layout(bytes: &[u8], msf: &Msf<&[u8]>, old_pages: u32) {
    let page_size = msf.block_size();
    let page_count = msf.page_count();
    assert_eq!(
        u64::try_from(bytes.len()).unwrap(),
        u64::from(page_count) * u64::from(page_size)
    );
    assert_eq!(msf.active_free_page_map(), 1);
    assert!(
        !matches!(page_count % page_size, 1 | 2),
        "the last interval has its FPM pages"
    );
    let report = msf.validate_free_page_map().unwrap();
    assert_eq!(report.unreferenced_allocated_count(), 0);
    assert_eq!(report.old_directory_free_pages(), old_pages);
    assert_eq!(report.old_directory_allocated_pages(), 0);

    let page = usize::try_from(page_size).unwrap();
    let bitmap = usize::try_from(page_count.div_ceil(8)).unwrap();
    for interval in 0..usize::try_from(page_count.div_ceil(page_size)).unwrap() {
        let fpm1 = (interval * page + 1) * page;
        let fpm2 = (interval * page + 2) * page;
        assert!(bytes[fpm2..fpm2 + page].iter().all(|&byte| byte == 0xFF));
        let used = bitmap.saturating_sub(interval * page).min(page);
        assert!(
            bytes[fpm1 + used..fpm1 + page]
                .iter()
                .all(|&byte| byte == 0xFF)
        );
    }
}

#[test]
fn every_page_size_writes_a_container_that_reads_back() {
    for page_size in PAGE_SIZES {
        let streams = sample_streams(page_size);
        let source = builder(&streams);
        let plan = source.plan(options(page_size)).unwrap();
        assert_eq!(plan.page_size(), page_size);
        assert_eq!(plan.stream_count(), 8);
        let bytes = plan.to_vec().unwrap();
        assert_eq!(u64::try_from(bytes.len()).unwrap(), plan.file_size());
        let msf = check_streams(&bytes, &streams);
        assert_eq!(msf.page_count(), plan.page_count());
        check_layout(&bytes, &msf, 1);
    }
}

#[test]
fn identical_input_and_options_give_identical_bytes() {
    for page_size in PAGE_SIZES {
        let streams = sample_streams(page_size);
        let first_builder = builder(&streams);
        let first = first_builder.plan(options(page_size)).unwrap();
        let mut written = Vec::new();
        first.write_to(&mut written).unwrap();
        assert_eq!(first.to_vec().unwrap(), written);
        let second_builder = builder(&streams);
        let second = second_builder.plan(options(page_size)).unwrap();
        assert_eq!(second.to_vec().unwrap(), written);
    }
}

#[test]
fn unused_bytes_are_zero() {
    let streams = vec![Some(Vec::new()), Some(vec![0xAA; 10])];
    let bytes = builder(&streams)
        .plan(options(512))
        .unwrap()
        .to_vec()
        .unwrap();
    // Page 3 holds stream 1, page 4 the directory and page 5 the page map.
    assert_eq!(bytes.len(), 6 * 512);
    assert!(
        bytes[3 * 512..3 * 512 + 10]
            .iter()
            .all(|&byte| byte == 0xAA)
    );
    assert!(bytes[3 * 512 + 10..4 * 512].iter().all(|&byte| byte == 0));
    // The directory has 4 words: the count, two sizes and one page number.
    assert_eq!(bytes[4 * 512 + 12..4 * 512 + 16], 3_u32.to_le_bytes());
    assert!(bytes[4 * 512 + 16..5 * 512].iter().all(|&byte| byte == 0));
    assert_eq!(bytes[5 * 512..5 * 512 + 4], 4_u32.to_le_bytes());
    assert!(bytes[5 * 512 + 4..6 * 512].iter().all(|&byte| byte == 0));
    // The header page map array has one entry.
    assert_eq!(bytes[52..56], 5_u32.to_le_bytes());
    assert!(bytes[56..512].iter().all(|&byte| byte == 0));
}

#[test]
fn pages_are_assigned_in_ascending_order() {
    let streams = sample_streams(1024);
    let bytes = builder(&streams)
        .plan(options(1024))
        .unwrap()
        .to_vec()
        .unwrap();
    let msf = check_streams(&bytes, &streams);
    // Stream 5 has three pages. They follow stream 0, stream 3 and stream 4.
    let stream = msf.stream(StreamId(5)).unwrap().unwrap();
    let mut first = [0; 4];
    stream.read_exact_at(0, &mut first).unwrap();
    assert_eq!(bytes[6 * 1024..6 * 1024 + 4], first);
}

#[test]
fn multi_interval_files_keep_fpm_pages_in_each_interval() {
    for page_size in [512_u32, 1024] {
        let size = u64::from(page_size) * u64::from(page_size) * 5 / 2;
        let mut builder = MsfBuilder::new();
        builder.push(Pattern(0)).unwrap();
        builder.push(Pattern(size)).unwrap();
        builder.push_nil().unwrap();
        builder.push(Pattern(12_345)).unwrap();
        let plan = builder.plan(options(page_size)).unwrap();
        assert!(plan.page_count() > 2 * page_size);
        let bytes = plan.to_vec().unwrap();
        let msf = Msf::read(bytes.as_slice()).unwrap();
        check_layout(&bytes, &msf, 0);
        for (id, source) in [(1, Pattern(size)), (3, Pattern(12_345))] {
            let stream = msf.stream(StreamId(id)).unwrap().unwrap();
            let length = usize::try_from(source.0).unwrap();
            let mut expected = vec![0; length];
            source.read_exact_at(0, &mut expected).unwrap();
            assert_eq!(stream.read_all(length).unwrap(), expected);
        }
    }
}

#[test]
fn a_last_page_at_an_interval_start_adds_the_fpm_pages() {
    let mut seen = false;
    for pages in 495..520_u64 {
        let mut builder = MsfBuilder::new();
        builder.push(Pattern(0)).unwrap();
        builder.push(Pattern(pages * 512)).unwrap();
        let plan = builder.plan(options(512)).unwrap();
        // The data pages of interval 0 end at page 511. Page 512 starts interval 1.
        seen |= plan.page_count() == 515;
        let bytes = plan.to_vec().unwrap();
        let msf = Msf::read(bytes.as_slice()).unwrap();
        check_layout(&bytes, &msf, 0);
    }
    assert!(seen, "one layout ends at the first page of interval 1");
}

#[test]
fn large_directories_use_several_page_map_pages() {
    // 20000 streams and 6668 stream pages need 106676 directory bytes: 209 pages of 512 bytes
    // and 2 page map pages.
    let mut streams = vec![Some(bytes(700, 9))];
    streams.extend((1..20_000).map(|index| match index % 3 {
        0 => None,
        1 => Some(Vec::new()),
        _ => Some(vec![u8::try_from(index % 256).unwrap(); 3]),
    }));
    let source = builder(&streams);
    let plan = source.plan(options(512)).unwrap();
    assert_eq!(plan.map_pages(), 2);
    let bytes = plan.to_vec().unwrap();
    let msf = check_streams(&bytes, &streams);
    check_layout(&bytes, &msf, 2);
}

#[test]
fn nil_and_empty_streams_stay_distinct() {
    let streams = vec![None, Some(Vec::new()), None, Some(Vec::new())];
    let bytes = builder(&streams)
        .plan(options(4096))
        .unwrap()
        .to_vec()
        .unwrap();
    let msf = check_streams(&bytes, &streams);
    check_layout(&bytes, &msf, 0);
}

#[test]
fn page_map_array_overflow_is_a_limit_error() {
    // 512-byte pages hold 115 page map entries in the header. Each page map page names 128
    // directory pages, so the directory can have 14720 pages. This stream needs more.
    let mut builder = MsfBuilder::new();
    builder.push(Pattern(0)).unwrap();
    builder.push(Pattern(512 * 128 * 115 * 128)).unwrap();
    let limits = MsfLimits::default().with_max_directory_bytes(u32::MAX);
    let error = builder.plan(options(512).with_limits(limits)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    assert_eq!(
        error.context(),
        Some("stream directory page map array does not fit in the header page")
    );
}

#[test]
fn directory_limit_is_enforced() {
    let streams = sample_streams(512);
    let limits = MsfLimits::default().with_max_directory_bytes(16);
    let error = builder(&streams)
        .plan(options(512).with_limits(limits))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    assert_eq!(
        error.context(),
        Some("stream directory is larger than the directory limit")
    );
}

#[test]
fn directory_size_overflow_is_a_limit_error() {
    let mut builder = MsfBuilder::new();
    for _ in 0..200 {
        builder.push(Pattern(u64::from(u32::MAX - 1))).unwrap();
    }
    let limits = MsfLimits::default().with_max_directory_bytes(u32::MAX);
    let error = builder.plan(options(512).with_limits(limits)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
}

#[test]
fn streams_that_are_too_large_are_rejected() {
    for size in [u64::from(u32::MAX), 1 << 32, u64::MAX] {
        let mut builder = MsfBuilder::new();
        builder.push(Pattern(0)).unwrap();
        builder.push(Pattern(size)).unwrap();
        let error = builder.plan(WriteOptions::default()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::LimitExceeded);
        assert_eq!(error.stream_location(), Some((StreamId(1), 0)));
    }
    let mut builder = MsfBuilder::new();
    builder.push(Pattern(0)).unwrap();
    builder.push(Pattern(u64::from(u32::MAX - 1))).unwrap();
    let plan = builder.plan(WriteOptions::default()).unwrap();
    assert_eq!(plan.directory_size(), 4 * (3 + 1_048_576));
}

#[test]
fn invalid_options_and_inputs_fail_before_output() {
    let streams = sample_streams(512);
    for page_size in [0, 256, 3000, 4095, 131_072] {
        let error = builder(&streams).plan(options(page_size)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    }
    let error = MsfBuilder::<&[u8]>::new()
        .plan(WriteOptions::default())
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
}

#[test]
fn page_size_65536_is_not_written() {
    let streams = sample_streams(512);
    let error = builder(&streams).plan(options(65_536)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedVersion);
    assert_eq!(
        error.context(),
        Some("65536-byte pages are not written: DIA, LLVM and link.exe reject them")
    );
    let source = builder(&streams)
        .plan(options(512))
        .unwrap()
        .to_vec()
        .unwrap();
    let msf = Msf::read(source.as_slice()).unwrap();
    for policy in [OldDirectoryPolicy::Empty, OldDirectoryPolicy::Preserve] {
        let error = msf.repack(options(65_536), policy).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnsupportedVersion);
    }
}

#[test]
fn source_read_errors_stop_the_write() {
    let mut builder = MsfBuilder::new();
    builder.push(Failing(0)).unwrap();
    builder.push(Failing(10)).unwrap();
    let plan = builder.plan(WriteOptions::default()).unwrap();
    let error = plan.to_vec().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
    assert_eq!(error.stream_location(), Some((StreamId(1), 0)));
}

#[test]
fn repack_preserves_stream_ids_sizes_and_bytes() {
    let mut streams = sample_streams(1024);
    streams[0] = Some(bytes(1500, 7));
    let source = builder(&streams)
        .plan(options(1024))
        .unwrap()
        .to_vec()
        .unwrap();
    let msf = Msf::read(source.as_slice()).unwrap();
    let mut expected = streams.clone();
    expected[0] = Some(Vec::new());
    for page_size in PAGE_SIZES {
        let plan = msf
            .repack(options(page_size), OldDirectoryPolicy::Empty)
            .unwrap();
        let bytes = plan.to_vec().unwrap();
        let repacked = check_streams(&bytes, &expected);
        check_layout(&bytes, &repacked, 0);
        assert_eq!(plan.to_vec().unwrap(), bytes);
    }
}

#[test]
fn repack_preserves_stream_0_with_the_source_page_size() {
    for page_size in PAGE_SIZES {
        let mut streams = sample_streams(page_size);
        streams[0] = Some(bytes(1500, 7));
        let source = builder(&streams)
            .plan(options(page_size))
            .unwrap()
            .to_vec()
            .unwrap();
        let msf = Msf::read(source.as_slice()).unwrap();
        let plan = msf
            .repack(options(page_size), OldDirectoryPolicy::Preserve)
            .unwrap();
        let bytes = plan.to_vec().unwrap();
        let repacked = check_streams(&bytes, &streams);
        check_layout(&bytes, &repacked, 1500_u32.div_ceil(page_size));
        // The builder and the repack use the same layout for the same streams.
        assert_eq!(bytes, source);
        let other = if page_size == 4096 { 8192 } else { 4096 };
        let error = msf
            .repack(options(other), OldDirectoryPolicy::Preserve)
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat);
        assert_eq!(error.stream_location(), Some((StreamId(0), 0)));
    }
}

#[test]
fn repack_preserves_a_nil_stream_0() {
    let streams = vec![None, Some(bytes(10, 1))];
    let source = builder(&streams)
        .plan(options(512))
        .unwrap()
        .to_vec()
        .unwrap();
    let msf = Msf::read(source.as_slice()).unwrap();
    // A nil stream 0 has no page lists, so the page size can change.
    let preserved = msf
        .repack(options(4096), OldDirectoryPolicy::Preserve)
        .unwrap()
        .to_vec()
        .unwrap();
    check_streams(&preserved, &streams);
    let empty = msf
        .repack(options(512), OldDirectoryPolicy::Empty)
        .unwrap()
        .to_vec()
        .unwrap();
    check_streams(&empty, &[Some(Vec::new()), Some(bytes(10, 1))]);
}

#[test]
fn repack_of_a_repack_is_identical() {
    let streams = sample_streams(4096);
    let source = builder(&streams)
        .plan(options(2048))
        .unwrap()
        .to_vec()
        .unwrap();
    let first = Msf::read(source.as_slice()).unwrap();
    for (options, policy) in [
        (WriteOptions::default(), OldDirectoryPolicy::Empty),
        (options(2048), OldDirectoryPolicy::Preserve),
    ] {
        let once = first.repack(options, policy).unwrap().to_vec().unwrap();
        let second = Msf::read(once.as_slice()).unwrap();
        let twice = second.repack(options, policy).unwrap().to_vec().unwrap();
        assert_eq!(once, twice);
    }
}

#[test]
fn exact_copy_is_byte_identical() {
    for length in [0, 1, 4096, 65_536, 200_000] {
        let source = bytes(length, 5);
        let mut output = Vec::new();
        let copied = copy_exact(source.as_slice(), &mut output).unwrap();
        assert_eq!(copied, u64::try_from(length).unwrap());
        assert_eq!(output, source);
    }
    let mut output = Vec::new();
    let error = copy_exact(&Failing(10), &mut output).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
}

#[cfg(feature = "std")]
mod file {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use pdbrs::ErrorKind;
    use pdbrs::io::FileReader;
    use pdbrs::io::copy_exact_file;
    use pdbrs::native::msf::{Msf, MsfBuilder, OldDirectoryPolicy, WriteOptions};

    use super::{Failing, Pattern, builder, sample_streams};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    /// An owned temporary directory.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            loop {
                let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
                let directory = std::env::temp_dir()
                    .join(format!("pdbrs-msf-{}-{name}-{serial}", std::process::id()));
                match fs::create_dir(&directory) {
                    Ok(()) => return Self(directory),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("cannot create test directory: {error}"),
                }
            }
        }

        fn entries(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(&self.0)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            drop(fs::remove_dir_all(&self.0));
        }
    }

    #[test]
    fn write_file_and_copy_file_give_the_planned_bytes() {
        let temp = TempDir::new("write");
        let streams = sample_streams(4096);
        let source = builder(&streams);
        let plan = source.plan(WriteOptions::default()).unwrap();
        let path = temp.0.join("new.pdb");
        plan.write_file(&path).unwrap();
        let written = fs::read(&path).unwrap();
        assert_eq!(written, plan.to_vec().unwrap());

        let copy = temp.0.join("copy.pdb");
        let source = FileReader::open(&path).unwrap();
        let length = copy_exact_file(&source, &copy).unwrap();
        assert_eq!(length, u64::try_from(written.len()).unwrap());
        assert_eq!(fs::read(&copy).unwrap(), written);

        let repacked = temp.0.join("repacked.pdb");
        let msf = Msf::read(FileReader::open(&copy).unwrap()).unwrap();
        msf.repack(WriteOptions::default(), OldDirectoryPolicy::Preserve)
            .unwrap()
            .write_file(&repacked)
            .unwrap();
        assert_eq!(fs::read(&repacked).unwrap(), written);
        assert_eq!(temp.entries(), ["copy.pdb", "new.pdb", "repacked.pdb"]);
    }

    #[test]
    fn a_failed_write_leaves_no_file_and_keeps_the_destination() {
        let temp = TempDir::new("failed");
        let path = temp.0.join("out.pdb");
        let mut builder = MsfBuilder::new();
        builder.push(Failing(0)).unwrap();
        builder.push(Failing(10)).unwrap();
        let plan = builder.plan(WriteOptions::default()).unwrap();
        assert_eq!(plan.write_file(&path).unwrap_err().kind(), ErrorKind::Io);
        assert_eq!(temp.entries(), Vec::<String>::new());

        fs::write(&path, b"previous").unwrap();
        assert_eq!(plan.write_file(&path).unwrap_err().kind(), ErrorKind::Io);
        assert_eq!(fs::read(&path).unwrap(), b"previous");
        assert_eq!(temp.entries(), ["out.pdb"]);

        let error = copy_exact_file(&Failing(10), &path).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Io);
        assert_eq!(fs::read(&path).unwrap(), b"previous");
        assert_eq!(temp.entries(), ["out.pdb"]);
    }

    #[test]
    fn a_planning_error_writes_nothing() {
        let temp = TempDir::new("planning");
        let path = temp.0.join("out.pdb");
        for (page_size, size, kind) in [
            (4096, 1 << 32, ErrorKind::LimitExceeded),
            (65_536, 10, ErrorKind::UnsupportedVersion),
        ] {
            let mut builder = MsfBuilder::new();
            builder.push(Pattern(0)).unwrap();
            builder.push(Pattern(size)).unwrap();
            let result = builder
                .plan(WriteOptions::default().with_page_size(page_size))
                .and_then(|plan| plan.write_file(&path));
            assert_eq!(result.unwrap_err().kind(), kind);
            assert_eq!(temp.entries(), Vec::<String>::new());
        }
    }
}
