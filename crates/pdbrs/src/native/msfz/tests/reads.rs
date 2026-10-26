//! Stream reads: fragment kinds, chunk boundaries, nil and empty streams, the cache and limits.

use alloc::vec;
use alloc::vec::Vec;

use super::{DEFLATE, Image, Part, STORED, Spec, ZSTD, expect_error, pattern, read};
use crate::ErrorKind;
use crate::io::ReadAt;
use crate::native::msf::StreamId;
use crate::native::msfz::{Msfz, MsfzLimits};

/// Return a container with each fragment kind and each chunk compression code.
fn mixed() -> (Image, Vec<Vec<u8>>) {
    let zstd_chunk = pattern(1, 300);
    let deflate_chunk = pattern(2, 200);
    let stored_chunk = pattern(3, 100);
    let file = pattern(4, 50);
    let streams = vec![
        Spec::Parts(Vec::new()),
        Spec::Parts(vec![Part::File(file.clone())]),
        Spec::Nil,
        Spec::Parts(vec![Part::Chunk {
            chunk: 0,
            offset: 10,
            size: 100,
        }]),
        Spec::Parts(vec![Part::Chunk {
            chunk: 1,
            offset: 0,
            size: 200,
        }]),
        Spec::Parts(vec![
            Part::File(file[..20].to_vec()),
            Part::Chunk {
                chunk: 2,
                offset: 90,
                size: 10,
            },
            Part::Chunk {
                chunk: 0,
                offset: 0,
                size: 5,
            },
        ]),
    ];
    let expected = vec![
        Vec::new(),
        file.clone(),
        Vec::new(),
        zstd_chunk[10..110].to_vec(),
        deflate_chunk.clone(),
        [&file[..20], &stored_chunk[90..], &zstd_chunk[..5]].concat(),
    ];
    let image = Image::new(
        &[
            (ZSTD, &zstd_chunk),
            (DEFLATE, &deflate_chunk),
            (STORED, &stored_chunk),
        ],
        &streams,
    );
    (image, expected)
}

#[test]
fn reads_each_fragment_kind_and_compression_code() {
    let (image, expected) = mixed();
    let msfz = image.open().unwrap();
    assert_eq!(msfz.stream_count(), 6);
    assert_eq!(msfz.chunk_count(), 3);
    assert_eq!(msfz.fragment_count(), 6);
    assert!(!msfz.is_directory_compressed());
    for (id, bytes) in expected.iter().enumerate() {
        let id = u32::try_from(id).unwrap();
        if id == 2 {
            assert_eq!(read(&msfz, id).unwrap(), None, "stream 2 is nil");
        } else {
            assert_eq!(
                read(&msfz, id).unwrap().as_ref(),
                Some(bytes),
                "stream {id}"
            );
        }
    }
}

#[test]
fn nil_and_empty_streams_stay_distinct() {
    let (image, _) = mixed();
    let msfz = image.open().unwrap();
    let empty = msfz.stream(StreamId(0)).unwrap().unwrap();
    assert_eq!(empty.len().unwrap(), 0);
    assert_eq!(empty.fragment_count(), 0);
    empty.read_exact_at(0, &mut []).unwrap();
    expect_unexpected_eof(&empty, 0, 1);
    assert!(msfz.stream(StreamId(2)).unwrap().is_none());
    let error = msfz.stream(StreamId(6)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidStreamIndex);
}

/// Check that a read of `count` bytes at `offset` fails at the end of the stream.
#[track_caller]
fn expect_unexpected_eof<S: ReadAt>(
    stream: &crate::native::msfz::Stream<'_, S>,
    offset: u64,
    count: usize,
) {
    let mut buffer = vec![0; count];
    let error = stream.read_exact_at(offset, &mut buffer).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    let size = stream.len().unwrap();
    assert_eq!(
        error.stream_location(),
        Some((stream.id().0, offset.max(size)))
    );
}

#[test]
fn partial_reads_cross_fragment_boundaries() {
    let (image, expected) = mixed();
    let msfz = image.open().unwrap();
    let stream = msfz.stream(StreamId(5)).unwrap().unwrap();
    let bytes = &expected[5];
    assert_eq!(stream.len().unwrap(), 35);
    for start in 0..bytes.len() {
        for end in start..=bytes.len() {
            let mut buffer = vec![0; end - start];
            stream
                .read_exact_at(u64::try_from(start).unwrap(), &mut buffer)
                .unwrap();
            assert_eq!(buffer, bytes[start..end], "{start}..{end}");
        }
    }
    expect_unexpected_eof(&stream, 30, 6);
    expect_unexpected_eof(&stream, 40, 0);
    let error = stream.read_exact_at(u64::MAX, &mut [0]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::OffsetOverflow);
}

#[test]
fn compressed_fragments_cross_chunk_boundaries() {
    let chunks = [pattern(10, 100), pattern(20, 50), pattern(30, 30)];
    let virtual_bytes = chunks.concat();
    let image = Image::new(
        &[
            (ZSTD, &chunks[0]),
            (DEFLATE, &chunks[1]),
            (STORED, &chunks[2]),
        ],
        &[
            // From chunk 0 through chunk 1 into chunk 2.
            Spec::Parts(vec![Part::Chunk {
                chunk: 0,
                offset: 90,
                size: 80,
            }]),
            // To the last byte of the last chunk.
            Spec::Parts(vec![Part::Chunk {
                chunk: 1,
                offset: 49,
                size: 31,
            }]),
        ],
    );
    let msfz = image.open().unwrap();
    assert_eq!(read(&msfz, 0).unwrap().unwrap(), virtual_bytes[90..170]);
    assert_eq!(read(&msfz, 1).unwrap().unwrap(), virtual_bytes[149..]);
    let stream = msfz.stream(StreamId(0)).unwrap().unwrap();
    for start in 0..80 {
        let mut buffer = vec![0; 80 - start];
        stream
            .read_exact_at(u64::try_from(start).unwrap(), &mut buffer)
            .unwrap();
        assert_eq!(buffer, virtual_bytes[90 + start..170]);
    }
}

#[test]
fn compressed_stream_directories_decode() {
    let chunk = pattern(5, 64);
    let streams = [
        Spec::Parts(Vec::new()),
        Spec::Nil,
        Spec::Parts(vec![
            Part::Chunk {
                chunk: 0,
                offset: 0,
                size: 64,
            },
            Part::File(pattern(6, 9)),
        ]),
    ];
    for code in [ZSTD, DEFLATE] {
        let image = Image::with_directory(&[(ZSTD, &chunk)], &streams, code);
        let msfz = image.open().unwrap();
        assert!(msfz.is_directory_compressed());
        assert_eq!(read(&msfz, 0).unwrap(), Some(Vec::new()));
        assert_eq!(read(&msfz, 1).unwrap(), None);
        assert_eq!(
            read(&msfz, 2).unwrap().unwrap(),
            [chunk.as_slice(), &pattern(6, 9)].concat()
        );
    }
}

#[test]
fn streams_larger_than_4_gib_use_u64_sizes() {
    // Two chunks that declare almost 4 GiB each. Opening does not decompress them.
    let size = u32::MAX - 15;
    let mut image = Image::new(
        &[(STORED, &[0; 8]), (STORED, &[0; 8])],
        &[Spec::Parts(vec![
            Part::Chunk {
                chunk: 0,
                offset: 0,
                size,
            },
            Part::Chunk {
                chunk: 1,
                offset: 0,
                size,
            },
            // A fragment that crosses from chunk 0 into chunk 1.
            Part::Chunk {
                chunk: 0,
                offset: size - 4,
                size: 8,
            },
        ])],
    );
    for chunk in 0..2 {
        let entry = image.entry(chunk, 8);
        image.set_u32(entry, ZSTD);
        let entry = image.entry(chunk, 16);
        image.set_u32(entry, size);
    }
    let msfz = image.open().unwrap();
    let stream = msfz.stream(StreamId(0)).unwrap().unwrap();
    let expected = 2 * u64::from(size) + 8;
    assert_eq!(stream.len().unwrap(), expected);
    let error = stream.read_all(64 << 20).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    // A read past 4 GiB needs chunk 1, which is larger than the default chunk limit.
    let error = stream
        .read_exact_at(u64::from(size) + 10, &mut [0; 4])
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    assert_eq!(
        error.context(),
        Some("MSFZ chunk is larger than the chunk limit")
    );
    assert_eq!(error.stream_location(), Some((0, u64::from(size) + 10)));
}

#[test]
fn chunk_limit_applies_when_a_read_needs_the_chunk() {
    let chunk = pattern(7, 4096);
    let image = Image::new(
        &[(ZSTD, &chunk)],
        &[
            Spec::Parts(vec![Part::File(pattern(8, 4))]),
            Spec::Parts(vec![Part::Chunk {
                chunk: 0,
                offset: 0,
                size: 4096,
            }]),
        ],
    );
    let msfz = image
        .open_with(MsfzLimits::default().with_max_chunk_bytes(4095))
        .unwrap();
    assert_eq!(read(&msfz, 0).unwrap().unwrap(), pattern(8, 4));
    let error = read(&msfz, 1).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    let msfz = image
        .open_with(MsfzLimits::default().with_max_chunk_bytes(4096))
        .unwrap();
    assert_eq!(read(&msfz, 1).unwrap().unwrap(), chunk);
    let stream = msfz.stream(StreamId(1)).unwrap().unwrap();
    let error = stream.read_all(4095).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
}

/// Return a container with four chunks of 100 bytes and one stream for each chunk.
fn four_chunks() -> (Image, Vec<Vec<u8>>) {
    let chunks: Vec<Vec<u8>> = (0..4).map(|seed| pattern(seed * 40, 100)).collect();
    let specs: Vec<(u32, &[u8])> = chunks
        .iter()
        .map(|chunk| (ZSTD, chunk.as_slice()))
        .collect();
    let streams: Vec<Spec> = (0..4)
        .map(|chunk| {
            Spec::Parts(vec![Part::Chunk {
                chunk,
                offset: 0,
                size: 100,
            }])
        })
        .collect();
    (Image::new(&specs, &streams), chunks)
}

/// Read one byte of stream `id`.
fn touch(msfz: &Msfz<&[u8]>, id: u32) {
    let stream = msfz.stream(StreamId(id)).unwrap().unwrap();
    stream.read_exact_at(0, &mut [0]).unwrap();
}

#[test]
fn cache_evicts_the_least_recently_used_chunk() {
    let (image, chunks) = four_chunks();
    let msfz = image
        .open_with(MsfzLimits::default().with_max_cache_bytes(250))
        .unwrap();
    assert_eq!(msfz.cache.cached(), Vec::<u32>::new());
    touch(&msfz, 0);
    touch(&msfz, 1);
    assert_eq!(msfz.cache.cached(), [0, 1]);
    touch(&msfz, 2);
    assert_eq!(msfz.cache.cached(), [1, 2]);
    touch(&msfz, 1);
    touch(&msfz, 3);
    assert_eq!(msfz.cache.cached(), [1, 3]);
    for (id, chunk) in chunks.iter().enumerate() {
        assert_eq!(
            &read(&msfz, u32::try_from(id).unwrap()).unwrap().unwrap(),
            chunk
        );
    }
    assert_eq!(msfz.cache.cached(), [2, 3]);
}

#[test]
fn chunks_larger_than_the_cache_are_not_kept() {
    let (image, chunks) = four_chunks();
    for capacity in [0, 99] {
        let msfz = image
            .open_with(MsfzLimits::default().with_max_cache_bytes(capacity))
            .unwrap();
        for (id, chunk) in chunks.iter().enumerate() {
            assert_eq!(
                &read(&msfz, u32::try_from(id).unwrap()).unwrap().unwrap(),
                chunk
            );
        }
        assert_eq!(msfz.cache.cached(), Vec::<u32>::new());
    }
}

#[test]
fn corrupt_chunks_fail_on_read_with_the_stream_location() {
    let chunk = pattern(9, 500);
    for code in [ZSTD, DEFLATE] {
        let mut image = Image::new(
            &[(code, &chunk)],
            &[
                Spec::Parts(vec![Part::File(pattern(1, 3))]),
                Spec::Parts(vec![
                    Part::File(pattern(2, 7)),
                    Part::Chunk {
                        chunk: 0,
                        offset: 0,
                        size: 500,
                    },
                ]),
            ],
        );
        let start = usize::try_from(image.chunks[0]).unwrap();
        for byte in &mut image.bytes[start + 2..start + 12] {
            *byte ^= 0x5a;
        }
        let msfz = image.open().unwrap();
        assert_eq!(read(&msfz, 0).unwrap().unwrap(), pattern(1, 3));
        let error = read(&msfz, 1).unwrap_err();
        assert!(
            matches!(
                error.kind(),
                ErrorKind::InvalidFormat | ErrorKind::UnexpectedEof
            ),
            "{error:?}"
        );
        assert_eq!(error.stream_location(), Some((1, 7)));
    }
}

#[test]
fn chunk_size_mismatches_fail_on_read() {
    let chunk = pattern(11, 300);
    for code in [ZSTD, DEFLATE] {
        for declared in [299, 301] {
            let mut image = Image::new(
                &[(code, &chunk)],
                &[Spec::Parts(vec![Part::Chunk {
                    chunk: 0,
                    offset: 0,
                    size: 200,
                }])],
            );
            let entry = image.entry(0, 16);
            image.set_u32(entry, declared);
            let msfz = image.open().unwrap();
            let error = read(&msfz, 0).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidFormat, "{error:?}");
        }
    }
}

#[cfg(feature = "std")]
#[test]
fn containers_and_streams_are_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<Msfz<&[u8]>>();
    check::<crate::native::msfz::Stream<'_, &[u8]>>();
    check::<crate::native::container::Container<&[u8]>>();
}

#[test]
fn error_kinds_in_this_module_match_the_docs() {
    let (image, _) = mixed();
    let msfz = image.open().unwrap();
    let error = expect_error(
        msfz.stream(StreamId(100)).map(|_| ()),
        ErrorKind::InvalidStreamIndex,
        "stream index is outside the stream directory",
    );
    assert_eq!(error.stream_location(), Some((100, 0)));
}
