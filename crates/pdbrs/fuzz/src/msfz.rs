//! The MSFZ container: the header, the stream directory, the Chunk Table and chunk decoding.

use pdbrs::format::ContainerKind;
use pdbrs::io::ReadAt;
use pdbrs::native::container::Container;
use pdbrs::native::msf::{MsfLimits, StreamId};
use pdbrs::native::msfz::{Msfz, MsfzLimits};

use crate::input::Input;
use crate::{MAX_STREAM, ok};

/// The largest decompressed chunk that a target decodes (16 MiB).
const MAX_CHUNK: u32 = 16 << 20;

/// The size of the chunk cache of a target (32 MiB).
const MAX_CACHE: usize = 32 << 20;

/// The bytes that a target reads at the start, the middle and the end of a large stream.
const WINDOW: usize = 4096;

/// Return the limits of the targets.
#[must_use]
pub fn limits() -> MsfzLimits {
    MsfzLimits::default()
        .with_max_chunk_bytes(MAX_CHUNK)
        .with_max_cache_bytes(MAX_CACHE)
}

/// Read every stream of an MSFZ container, with and without the chunk cache.
///
/// # Panics
///
/// Panics if a container opens with other limits and gives another stream count, if the
/// stream readers of `Msfz` and `Container` disagree, or if a read with the cache gives another
/// result than a read without the cache.
pub fn read(data: &[u8]) {
    let _ = pdbrs::format::probe(data);
    let Some(msfz) = ok(Msfz::with_limits(data, limits())) else {
        return;
    };
    let uncached = Msfz::with_limits(data, limits().with_max_cache_bytes(0))
        .expect("the container opens with another cache size");
    let container = Container::with_limits(data, MsfLimits::default(), limits())
        .expect("the container opens through Container");
    assert_eq!(container.kind(), ContainerKind::NativeMsfz);
    assert_eq!(container.stream_count(), msfz.stream_count());
    for id in (0..msfz.stream_count()).map(StreamId) {
        let first = streams(&msfz, id);
        assert_eq!(first, streams(&msfz, id), "a second read differs");
        let wrapped = container
            .stream(id)
            .expect("the stream index is valid")
            .map(|stream| {
                let length = stream.len().expect("a stream has a length");
                (length, ok(stream.read_all(MAX_STREAM)))
            });
        let expected = first
            .as_ref()
            .map(|(length, bytes, _)| (*length, bytes.clone()));
        assert_eq!(wrapped, expected, "Container and Msfz disagree");
        assert_eq!(first, streams(&uncached, id), "the cache changes a read");
    }
    let small = MsfzLimits::default().with_max_directory_bytes(4096);
    if let Some(small) = ok(Msfz::with_limits(data, small)) {
        assert_eq!(small.stream_count(), msfz.stream_count());
    }
}

/// The length, the bytes (up to [`MAX_STREAM`]) and three windows of a stream, or `None` for
/// a nil stream. A failed read is `None`.
type StreamRead = Option<(u64, Option<Vec<u8>>, [Option<Vec<u8>>; 3])>;

/// Read stream `id` of `msfz`.
fn streams(msfz: &Msfz<&[u8]>, id: StreamId) -> StreamRead {
    let stream = msfz.stream(id).expect("the stream index is valid")?;
    assert_eq!(stream.id(), id);
    let length = stream.len().expect("a stream has a length");
    let bytes = ok(stream.read_all(MAX_STREAM));
    if let Some(bytes) = &bytes {
        assert_eq!(u64::try_from(bytes.len()).ok(), Some(length));
    }
    let window = u64::try_from(WINDOW).unwrap_or(u64::MAX);
    let windows = [0, length / 2, length.saturating_sub(window)].map(|offset| {
        let count = usize::try_from((length - offset).min(window)).unwrap_or(WINDOW);
        let mut buffer = vec![0; count];
        ok(stream.read_exact_at(offset, &mut buffer)).map(|()| buffer)
    });
    Some((length, bytes, windows))
}

/// The input layout of [`chunks`].
struct Chunks<'a> {
    codes: [u32; 2],
    sizes: [u32; 2],
    offset: u32,
    stored: [&'a [u8]; 2],
    two: bool,
}

impl<'a> Chunks<'a> {
    /// Split a [`chunks`] input.
    fn parse(data: &'a [u8]) -> Self {
        let mut input = Input::new(data);
        let selector = input.u8();
        let sizes = [input.u32(), input.u32()];
        let offset = input.u32().checked_rem(sizes[0]).unwrap_or(0);
        let first = input.part();
        let second = input.rest();
        Self {
            codes: [
                u32::from(selector & 3) % 3,
                u32::from((selector >> 2) & 3) % 3,
            ],
            sizes,
            offset,
            stored: [first, second],
            two: selector & 0x10 != 0,
        }
    }
}

/// Return a [`chunks`] input with one chunk: the compression code `code`, the declared size
/// `size` and the stored bytes `stored`.
///
/// # Panics
///
/// Panics if `stored` is larger than 4 GiB.
#[must_use]
pub fn chunk_input(code: u8, size: u32, stored: &[u8]) -> Vec<u8> {
    let mut input = vec![code & 3];
    input.extend_from_slice(&size.to_le_bytes());
    input.extend_from_slice(&[0; 8]);
    let length = u32::try_from(stored.len()).expect("a chunk is smaller than 4 GiB");
    input.extend_from_slice(&length.to_le_bytes());
    input.extend_from_slice(stored);
    input
}

/// Return the container that [`chunks`] builds from `data`.
#[must_use]
pub fn chunk_container(data: &[u8]) -> Vec<u8> {
    container(&Chunks::parse(data))
}

/// Build an MSFZ container around one or two fuzzed chunks and read its stream.
///
/// The input is a selector byte (bits 0 to 1 and 2 to 3: the compression codes, mod 3; bit 4:
/// two chunks), the declared sizes of the chunks (two `u32`), the offset of the fragment in the
/// first chunk (`u32`, mod the first size), the stored bytes of the first chunk (a part with a
/// 32-bit length prefix) and the stored bytes of the second chunk (the rest). Stream 0 is one
/// fragment from the offset to the end of the last chunk.
///
/// # Panics
///
/// Panics if a read with the cache gives another result than a read without the cache, or if a
/// read of a stored chunk gives other bytes.
pub fn chunks(data: &[u8]) {
    let chunks = Chunks::parse(data);
    let bytes = container(&chunks);
    let Some(msfz) = ok(Msfz::with_limits(bytes.as_slice(), limits())) else {
        return;
    };
    let uncached = Msfz::with_limits(bytes.as_slice(), limits().with_max_cache_bytes(0))
        .expect("the container opens with another cache size");
    let read = streams(&msfz, StreamId(0));
    assert_eq!(
        read,
        streams(&uncached, StreamId(0)),
        "the cache changes a read"
    );
    let Some((_, Some(bytes), _)) = read else {
        return;
    };
    if chunks.codes[0] == 0 && (!chunks.two || chunks.codes[1] == 0) {
        let stored = if chunks.two {
            [chunks.stored[0], chunks.stored[1]].concat()
        } else {
            chunks.stored[0].to_vec()
        };
        let start = usize::try_from(chunks.offset).unwrap_or(usize::MAX);
        assert_eq!(stored.get(start..), Some(bytes.as_slice()));
    }
}

/// Return the container bytes of `chunks`.
fn container(chunks: &Chunks<'_>) -> Vec<u8> {
    let count = if chunks.two { 2 } else { 1 };
    let mut file = vec![0_u8; 80];
    let mut table = Vec::new();
    for index in 0..count {
        let stored = chunks.stored[index];
        table.extend_from_slice(&u64::try_from(file.len()).unwrap_or(0).to_le_bytes());
        table.extend_from_slice(&chunks.codes[index].to_le_bytes());
        table.extend_from_slice(&u32::try_from(stored.len()).unwrap_or(0).to_le_bytes());
        table.extend_from_slice(&chunks.sizes[index].to_le_bytes());
        file.extend_from_slice(stored);
    }
    let total: u64 = chunks.sizes[..count]
        .iter()
        .map(|&size| u64::from(size))
        .sum();
    let size = u32::try_from(total - u64::from(chunks.offset)).unwrap_or(u32::MAX);
    let mut directory = Vec::new();
    if size != 0 {
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&((1_u64 << 63) | u64::from(chunks.offset)).to_le_bytes());
    }
    directory.extend_from_slice(&0_u32.to_le_bytes());
    let directory_offset = u64::try_from(file.len()).unwrap_or(0);
    file.extend_from_slice(&directory);
    let table_offset = u64::try_from(file.len()).unwrap_or(0);
    file.extend_from_slice(&table);
    let directory_size = u32::try_from(directory.len()).unwrap_or(0).to_le_bytes();
    let header = [
        ContainerKind::NativeMsfz.signature(),
        &0_u64.to_le_bytes(),
        &directory_offset.to_le_bytes(),
        &table_offset.to_le_bytes(),
        &1_u32.to_le_bytes(),
        &0_u32.to_le_bytes(),
        &directory_size,
        &directory_size,
        &u32::try_from(count).unwrap_or(0).to_le_bytes(),
        &u32::try_from(table.len()).unwrap_or(0).to_le_bytes(),
    ]
    .concat();
    file[..80].copy_from_slice(&header);
    file
}
