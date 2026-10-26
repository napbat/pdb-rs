//! Behavior tests for MSFZ container writing.
//!
//! Each test writes a container and opens the output with the reader, which checks the
//! structure. The helpers decode the header and the Chunk Table to check the layout.

use std::cell::Cell;

use pdbrs::io::ReadAt;
use pdbrs::native::msf::StreamId;
use pdbrs::native::msfz::{
    Codec, DEFAULT_MAX_CHUNK_SIZE, MAX_CHUNK_SIZE, MIN_CHUNK_SIZE, Msfz, MsfzBuilder, MsfzLimits,
    MsfzWriteOptions, PAD_16K_SIZE,
};
use pdbrs::{Error, ErrorKind, Result};

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

/// A source that gives zero bytes until the flag is set, then pattern bytes.
struct Changing<'a> {
    changed: &'a Cell<bool>,
    len: u64,
}

impl ReadAt for Changing<'_> {
    fn len(&self) -> Result<u64> {
        Ok(self.len)
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        if self.changed.get() {
            Pattern(self.len).read_exact_at(offset, buffer)
        } else {
            buffer.fill(0);
            Ok(())
        }
    }
}

/// Return `length` bytes that compress a little.
fn bytes(length: usize, seed: u8) -> Vec<u8> {
    (0..length)
        .map(|index| (index ^ (index >> 8)).to_le_bytes()[0] ^ seed)
        .collect()
}

/// Return `length` bytes that do not compress.
fn noise(length: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[0]
        })
        .collect()
}

/// Streams with a nil stream, empty streams and sizes around the 4 KiB chunk size.
fn sample_streams() -> Vec<Option<Vec<u8>>> {
    vec![
        Some(Vec::new()),
        Some(bytes(100, 1)),
        None,
        Some(bytes(4096, 2)),
        Some(Vec::new()),
        Some(bytes(4097, 3)),
        Some(bytes(4095, 4)),
        None,
        Some(bytes(20_000, 5)),
        Some(bytes(1, 6)),
    ]
}

fn builder(streams: &[Option<Vec<u8>>]) -> MsfzBuilder<&[u8]> {
    let mut builder = MsfzBuilder::new();
    for stream in streams {
        match stream {
            Some(data) => builder.push(data.as_slice()).unwrap(),
            None => builder.push_nil().unwrap(),
        };
    }
    builder
}

/// Write `streams` with `options`.
fn write(streams: &[Option<Vec<u8>>], options: MsfzWriteOptions) -> Vec<u8> {
    let builder = builder(streams);
    let plan = builder.plan(options).unwrap();
    let bytes = plan.to_vec().unwrap();
    assert_eq!(u64::try_from(bytes.len()).unwrap(), plan.file_size());
    assert_eq!(plan.stream_count(), u32::try_from(streams.len()).unwrap());
    bytes
}

/// Open `bytes` and check that the streams are `expected`.
fn check_streams<'a>(bytes: &'a [u8], expected: &[Option<Vec<u8>>]) -> Msfz<&'a [u8]> {
    let msfz = Msfz::read(bytes).unwrap();
    assert_eq!(msfz.stream_count(), u32::try_from(expected.len()).unwrap());
    for (id, expected) in (0..).zip(expected) {
        let actual = msfz
            .stream(StreamId(id))
            .unwrap()
            .map(|stream| stream.read_all(usize::MAX).unwrap());
        assert_eq!(&actual, expected, "stream {id}");
    }
    msfz
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

/// One Chunk Table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    offset: u64,
    code: u32,
    stored: u32,
    size: u32,
}

/// The header fields that the layout checks use.
#[derive(Debug)]
struct Header {
    directory_offset: usize,
    table_offset: usize,
    directory_code: u32,
    directory_stored: usize,
    chunks: Vec<Entry>,
}

fn header(bytes: &[u8]) -> Header {
    let table_offset = usize::try_from(u64_at(bytes, 48)).unwrap();
    let count = usize::try_from(u32_at(bytes, 72)).unwrap();
    assert_eq!(u32_at(bytes, 76), u32::try_from(count * 20).unwrap());
    Header {
        directory_offset: usize::try_from(u64_at(bytes, 40)).unwrap(),
        table_offset,
        directory_code: u32_at(bytes, 60),
        directory_stored: usize::try_from(u32_at(bytes, 64)).unwrap(),
        chunks: (0..count)
            .map(|index| {
                let at = table_offset + index * 20;
                Entry {
                    offset: u64_at(bytes, at),
                    code: u32_at(bytes, at + 8),
                    stored: u32_at(bytes, at + 12),
                    size: u32_at(bytes, at + 16),
                }
            })
            .collect(),
    }
}

/// Check the file order, the alignment and the zero gaps of a written container.
fn check_layout(bytes: &[u8], code: u32) -> Header {
    let header = header(bytes);
    assert_eq!(u64_at(bytes, 32), 0, "version");
    let mut next = 80_usize;
    for entry in &header.chunks {
        assert_eq!(entry.offset, u64::try_from(next).unwrap());
        assert_eq!(entry.code, code);
        next += usize::try_from(entry.stored).unwrap();
    }
    assert_eq!(header.directory_offset, next.next_multiple_of(16));
    assert!(bytes[next..header.directory_offset].iter().all(|&b| b == 0));
    let directory_end = header.directory_offset + header.directory_stored;
    assert_eq!(header.table_offset, directory_end.next_multiple_of(16));
    assert!(
        bytes[directory_end..header.table_offset]
            .iter()
            .all(|&b| b == 0)
    );
    let end = header.table_offset + header.chunks.len() * 20;
    assert!(bytes[end..].iter().all(|&b| b == 0));
    header
}

#[test]
fn written_streams_read_back_with_each_codec() {
    let streams = sample_streams();
    for codec in [Codec::Zstd, Codec::Deflate] {
        let options = MsfzWriteOptions::default()
            .with_max_chunk_size(MIN_CHUNK_SIZE)
            .with_codec(codec);
        let bytes = write(&streams, options);
        check_streams(&bytes, &streams);
        check_layout(&bytes, codec.code());
    }
}

#[test]
fn nil_and_empty_streams_stay_distinct() {
    let streams = vec![None, Some(Vec::new()), None, Some(Vec::new())];
    let bytes = write(&streams, MsfzWriteOptions::default());
    let msfz = check_streams(&bytes, &streams);
    assert_eq!(msfz.chunk_count(), 0);
    assert_eq!(msfz.fragment_count(), 0);
    assert!(msfz.stream(StreamId(0)).unwrap().is_none());
    assert_eq!(msfz.stream(StreamId(1)).unwrap().unwrap().len().unwrap(), 0);
}

#[test]
fn identical_input_and_options_give_identical_bytes() {
    let streams = sample_streams();
    let options = MsfzWriteOptions::default()
        .with_max_chunk_size(MIN_CHUNK_SIZE)
        .with_directory_codec(Some(Codec::Deflate));
    let source = builder(&streams);
    let plan = source.plan(options).unwrap();
    let first = plan.to_vec().unwrap();
    let mut second = Vec::new();
    plan.write_to(&mut second).unwrap();
    assert_eq!(first, second);
    assert_eq!(first, write(&streams, options));
}

#[test]
fn chunks_have_the_maximum_size_except_the_last() {
    let streams = sample_streams();
    let total: usize = streams.iter().flatten().map(Vec::len).sum();
    let bytes = write(
        &streams,
        MsfzWriteOptions::default().with_max_chunk_size(MIN_CHUNK_SIZE),
    );
    let header = check_layout(&bytes, 1);
    let max = usize::try_from(MIN_CHUNK_SIZE).unwrap();
    assert_eq!(header.chunks.len(), total.div_ceil(max));
    let (last, full) = header.chunks.split_last().unwrap();
    assert!(full.iter().all(|entry| entry.size == MIN_CHUNK_SIZE));
    assert_eq!(
        usize::try_from(last.size).unwrap(),
        total - full.len() * max
    );
}

#[test]
fn one_stream_per_chunk_starts_a_chunk_for_each_stream() {
    let streams = sample_streams();
    let bytes = write(
        &streams,
        MsfzWriteOptions::default()
            .with_max_chunk_size(MIN_CHUNK_SIZE)
            .with_one_stream_per_chunk(true),
    );
    let msfz = check_streams(&bytes, &streams);
    let header = check_layout(&bytes, 1);
    let max = usize::try_from(MIN_CHUNK_SIZE).unwrap();
    let mut expected = Vec::new();
    for stream in streams.iter().flatten() {
        let mut rest = stream.len();
        while rest != 0 {
            expected.push(u32::try_from(rest.min(max)).unwrap());
            rest -= rest.min(max);
        }
    }
    let sizes: Vec<u32> = header.chunks.iter().map(|entry| entry.size).collect();
    assert_eq!(sizes, expected);
    // Each stream has one fragment for each of its chunks.
    assert_eq!(msfz.fragment_count(), expected.len());
}

#[test]
fn fragments_end_at_chunk_boundaries() {
    // Streams of 3000 bytes with 4096-byte chunks: stream 1 crosses the first boundary.
    let streams = vec![
        Some(bytes(3000, 1)),
        Some(bytes(3000, 2)),
        Some(bytes(3000, 3)),
    ];
    let bytes = write(
        &streams,
        MsfzWriteOptions::default().with_max_chunk_size(MIN_CHUNK_SIZE),
    );
    let msfz = check_streams(&bytes, &streams);
    assert_eq!(msfz.chunk_count(), 3);
    assert_eq!(
        (0..3)
            .map(|id| msfz.stream(StreamId(id)).unwrap().unwrap().fragment_count())
            .collect::<Vec<_>>(),
        [1, 2, 2]
    );
    let header = header(&bytes);
    let directory = &bytes[header.directory_offset..][..header.directory_stored];
    // Stream 1: 1096 bytes at chunk 0 offset 3000, then 1904 bytes at chunk 1 offset 0.
    let compressed = 1_u64 << 63;
    assert_eq!(u32_at(directory, 16), 1096);
    assert_eq!(u64_at(directory, 20), compressed + 3000);
    assert_eq!(u32_at(directory, 28), 1904);
    assert_eq!(u64_at(directory, 32), compressed + (1 << 32));
    assert_eq!(u32_at(directory, 40), 0);
}

#[test]
fn the_stream_directory_can_be_compressed() {
    let streams = sample_streams();
    for codec in [Codec::Zstd, Codec::Deflate] {
        let options = MsfzWriteOptions::default().with_directory_codec(Some(codec));
        let bytes = write(&streams, options);
        let msfz = check_streams(&bytes, &streams);
        assert!(msfz.is_directory_compressed());
        assert_eq!(check_layout(&bytes, 1).directory_code, codec.code());
    }
    let bytes = write(&streams, MsfzWriteOptions::default());
    assert!(
        !Msfz::read(bytes.as_slice())
            .unwrap()
            .is_directory_compressed()
    );
    assert_eq!(header(&bytes).directory_code, 0);
}

#[test]
fn pad_16k_pads_only_small_files() {
    let small = vec![Some(Vec::new()), Some(bytes(10, 1))];
    let bytes = write(&small, MsfzWriteOptions::default().with_pad_16k(true));
    assert_eq!(u64::try_from(bytes.len()).unwrap(), PAD_16K_SIZE);
    check_streams(&bytes, &small);
    check_layout(&bytes, 1);

    let large = vec![Some(Vec::new()), Some(noise(1 << 16))];
    let padded = write(&large, MsfzWriteOptions::default().with_pad_16k(true));
    assert_eq!(padded, write(&large, MsfzWriteOptions::default()));
    assert!(u64::try_from(padded.len()).unwrap() > PAD_16K_SIZE);
}

#[test]
fn the_defaults_match_pdz_encode() {
    let options = MsfzWriteOptions::default();
    assert_eq!(options.max_chunk_size(), DEFAULT_MAX_CHUNK_SIZE);
    assert_eq!(DEFAULT_MAX_CHUNK_SIZE, 1 << 20);
    assert!(!options.one_stream_per_chunk());
    assert_eq!(options.codec(), Codec::Zstd);
    assert_eq!(options.directory_codec(), None);
    assert!(!options.pad_16k());
    assert_eq!(options.limits(), MsfzLimits::default());
}

#[test]
fn invalid_options_and_inputs_fail_before_output() {
    let streams = sample_streams();
    let source = builder(&streams);
    for size in [0, MIN_CHUNK_SIZE - 1, MAX_CHUNK_SIZE + 1, u32::MAX] {
        let error = source
            .plan(MsfzWriteOptions::default().with_max_chunk_size(size))
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat, "{size}");
    }
    let empty: MsfzBuilder<&[u8]> = MsfzBuilder::new();
    assert_eq!(
        empty.plan(MsfzWriteOptions::default()).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    let small_chunks = MsfzLimits::default().with_max_chunk_bytes(MIN_CHUNK_SIZE);
    let options = MsfzWriteOptions::default().with_limits(small_chunks);
    assert_eq!(
        source.plan(options).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
}

#[test]
fn limits_of_the_options_are_enforced() {
    let streams = sample_streams();
    let source = builder(&streams);
    let options = MsfzWriteOptions::default().with_max_chunk_size(MIN_CHUNK_SIZE);
    let plan = source.plan(options).unwrap();
    let directory = plan.directory_size();
    let table = plan.chunk_count() * 20;

    let limits = MsfzLimits::default().with_max_directory_bytes(directory - 1);
    let error = source.plan(options.with_limits(limits)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    let limits = MsfzLimits::default().with_max_directory_bytes(directory);
    source.plan(options.with_limits(limits)).unwrap();

    let limits = MsfzLimits::default().with_max_chunk_table_bytes(table - 1);
    let error = source.plan(options.with_limits(limits)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    let limits = MsfzLimits::default().with_max_chunk_table_bytes(table);
    let bytes = source
        .plan(options.with_limits(limits))
        .unwrap()
        .to_vec()
        .unwrap();
    Msfz::with_limits(bytes.as_slice(), limits).unwrap();

    // Incompressible chunks are larger than their decompressed size.
    let noisy_streams = [Some(noise(4096))];
    let noisy = builder(&noisy_streams);
    let limits = MsfzLimits::default().with_max_chunk_bytes(MIN_CHUNK_SIZE);
    let error = noisy
        .plan(options.with_limits(limits).with_codec(Codec::Deflate))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
}

#[test]
fn large_streams_need_no_large_buffers() {
    // A 40 MiB stream with 4 KiB chunks: the writer keeps one 4 KiB chunk in memory.
    let mut builder = MsfzBuilder::new();
    builder.push(Pattern(0)).unwrap();
    builder.push(Pattern(40 << 20)).unwrap();
    let options = MsfzWriteOptions::default().with_max_chunk_size(MIN_CHUNK_SIZE);
    let plan = builder.plan(options).unwrap();
    assert_eq!(plan.chunk_count(), 10_240);
    assert_eq!(plan.fragment_count(), 10_240);
    let bytes = plan.to_vec().unwrap();
    let msfz = Msfz::read(bytes.as_slice()).unwrap();
    let stream = msfz.stream(StreamId(1)).unwrap().unwrap();
    assert_eq!(stream.len().unwrap(), 40 << 20);
    let mut window = vec![0; 10_000];
    let offset = (40 << 20) - 10_000;
    stream.read_exact_at(offset, &mut window).unwrap();
    let mut expected = vec![0; 10_000];
    Pattern(40 << 20)
        .read_exact_at(offset, &mut expected)
        .unwrap();
    assert_eq!(window, expected);
}

#[test]
fn streams_larger_than_4_gib_are_planned() {
    // The plan accepts the size and fails at the first read of the source.
    let mut builder = MsfzBuilder::new();
    builder.push(Failing(0)).unwrap();
    builder.push(Failing((5 << 30) + 7)).unwrap();
    // 4 KiB chunks keep the chunk buffer small: 1 310 721 chunks, a 25 MiB Chunk Table.
    let options = MsfzWriteOptions::default().with_max_chunk_size(MIN_CHUNK_SIZE);
    let error = builder.plan(options).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
    // The default chunk limit (256 MiB) rejects 1 GiB chunks before any read.
    let options = options.with_max_chunk_size(MAX_CHUNK_SIZE);
    let error = builder.plan(options).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
}

#[test]
fn source_read_errors_stop_the_plan() {
    let mut builder = MsfzBuilder::new();
    builder.push(Failing(0)).unwrap();
    builder.push(Failing(10)).unwrap();
    let error = builder.plan(MsfzWriteOptions::default()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Io);
    assert_eq!(
        error.stream_location().map(|(stream, _)| stream),
        Some(StreamId(1))
    );
}

#[test]
fn a_changed_source_stops_the_write() {
    let changed = Cell::new(false);
    let mut builder = MsfzBuilder::new();
    builder
        .push(Changing {
            changed: &changed,
            len: 50_000,
        })
        .unwrap();
    let plan = builder.plan(MsfzWriteOptions::default()).unwrap();
    changed.set(true);
    let error = plan.to_vec().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(error.offset(), Some(80));
}

#[cfg(feature = "std")]
mod file {
    use std::cell::Cell;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use pdbrs::ErrorKind;
    use pdbrs::native::msfz::{MsfzBuilder, MsfzWriteOptions};

    use super::{Changing, Pattern, builder, sample_streams};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    /// An owned temporary directory.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            loop {
                let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
                let directory = std::env::temp_dir()
                    .join(format!("pdbrs-msfz-{}-{name}-{serial}", std::process::id()));
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
    fn write_file_gives_the_planned_bytes() {
        let temp = TempDir::new("write");
        let streams = sample_streams();
        let source = builder(&streams);
        let plan = source.plan(MsfzWriteOptions::default()).unwrap();
        let path = temp.0.join("new.pdb");
        plan.write_file(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), plan.to_vec().unwrap());
        assert_eq!(temp.entries(), ["new.pdb"]);
    }

    #[test]
    fn a_failed_write_leaves_no_file_and_keeps_the_destination() {
        let temp = TempDir::new("failed");
        let path = temp.0.join("out.pdb");
        let changed = Cell::new(false);
        let mut builder = MsfzBuilder::new();
        builder
            .push(Changing {
                changed: &changed,
                len: 50_000,
            })
            .unwrap();
        let plan = builder.plan(MsfzWriteOptions::default()).unwrap();
        changed.set(true);
        let error = plan.write_file(&path).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat);
        assert_eq!(temp.entries(), Vec::<String>::new());

        fs::write(&path, b"previous").unwrap();
        assert_eq!(
            plan.write_file(&path).unwrap_err().kind(),
            ErrorKind::InvalidFormat
        );
        assert_eq!(fs::read(&path).unwrap(), b"previous");
        assert_eq!(temp.entries(), ["out.pdb"]);
    }

    #[test]
    fn a_planning_error_writes_nothing() {
        let temp = TempDir::new("planning");
        let path = temp.0.join("out.pdb");
        let mut builder = MsfzBuilder::new();
        builder.push(Pattern(10)).unwrap();
        let result = builder
            .plan(MsfzWriteOptions::default().with_max_chunk_size(100))
            .and_then(|plan| plan.write_file(&path));
        assert_eq!(result.unwrap_err().kind(), ErrorKind::InvalidFormat);
        assert_eq!(temp.entries(), Vec::<String>::new());
    }
}
