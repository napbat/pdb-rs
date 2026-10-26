//! Tests of the chunk and fragment assignment without stream bytes.

use alloc::vec::Vec;

use super::layout::{Fragment, Layout, Span};
use super::{MAX_CHUNK_SIZE, MIN_CHUNK_SIZE, WriteOptions};
use crate::ErrorKind;
use crate::native::msfz::MsfzLimits;

/// Return the fragments of each stream of `layout`.
fn fragments(layout: &Layout) -> Vec<Option<Vec<Fragment>>> {
    layout
        .streams
        .iter()
        .map(|stream| stream.map(|span| layout.fragments(span).collect()))
        .collect()
}

#[test]
fn a_stream_larger_than_4_gib_has_one_fragment_for_each_chunk() {
    let size = (5_u64 << 30) + 7;
    let options = WriteOptions::default().with_max_chunk_size(MAX_CHUNK_SIZE);
    let layout = Layout::new(&[Some(0), Some(size), None, Some(3)], &options).unwrap();
    assert_eq!(layout.chunk_count(), 6);
    let all = fragments(&layout);
    assert_eq!(all[0], Some(Vec::new()));
    assert_eq!(all[2], None);
    let large = all[1].as_ref().unwrap();
    assert_eq!(large.len(), 6);
    assert!(
        large[..5]
            .iter()
            .all(|fragment| fragment.size == MAX_CHUNK_SIZE && fragment.offset == 0)
    );
    assert_eq!(
        large[5],
        Fragment {
            chunk: 5,
            offset: 0,
            size: 7
        }
    );
    assert_eq!(large.iter().map(|f| u64::from(f.size)).sum::<u64>(), size);
    // Stream 3 shares the last chunk with the end of stream 1.
    assert_eq!(
        all[3],
        Some(alloc::vec![Fragment {
            chunk: 5,
            offset: 7,
            size: 3
        }])
    );
    let directory = layout.encode_directory(u32::MAX).unwrap();
    // Stream 0: end marker. Stream 1: 6 records and the marker. Stream 2: nil. Stream 3: 1.
    assert_eq!(directory.len(), 4 + (6 * 12 + 4) + 4 + (12 + 4));
    assert_eq!(layout.fragment_count(), 7);
}

#[test]
fn one_stream_per_chunk_starts_each_stream_at_offset_0() {
    let options = WriteOptions::default()
        .with_max_chunk_size(MIN_CHUNK_SIZE)
        .with_one_stream_per_chunk(true);
    let layout = Layout::new(&[Some(10), Some(0), None, Some(5000), Some(1)], &options).unwrap();
    let sizes: Vec<u32> = layout.chunks.iter().map(|chunk| chunk.size).collect();
    assert_eq!(sizes, [10, 4096, 904, 1]);
    for span in layout.streams.iter().flatten() {
        if let Some(first) = layout.fragments(*span).next() {
            assert_eq!(first.offset, 0);
        }
    }
    assert_eq!(
        layout.streams[3],
        Some(Span {
            start: 10,
            size: 5000
        })
    );
}

#[test]
fn an_empty_stream_inside_a_chunk_has_no_fragments() {
    let options = WriteOptions::default().with_max_chunk_size(MIN_CHUNK_SIZE);
    let layout = Layout::new(&[Some(100), Some(0), Some(100)], &options).unwrap();
    assert_eq!(layout.chunk_count(), 1);
    assert_eq!(fragments(&layout)[1], Some(Vec::new()));
    assert_eq!(layout.fragment_count(), 2);
}

#[test]
fn the_chunk_count_is_limited_before_allocation() {
    // 2^40 bytes in 4 KiB chunks need 2^28 chunks: more than a 64 MiB Chunk Table holds.
    let options = WriteOptions::default().with_max_chunk_size(MIN_CHUNK_SIZE);
    let error = Layout::new(&[Some(1 << 40)], &options).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);

    // With a larger Chunk Table limit, the 31-bit chunk index is the limit.
    let limits = MsfzLimits::default().with_max_chunk_table_bytes(u32::MAX);
    let options = options.with_limits(limits);
    let error = Layout::new(&[Some(1 << 44)], &options).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
}

#[test]
fn the_total_size_must_fit_in_u64() {
    let options = WriteOptions::default().with_max_chunk_size(MAX_CHUNK_SIZE);
    let limits = MsfzLimits::default().with_max_chunk_table_bytes(u32::MAX);
    let error = Layout::new(&[Some(u64::MAX), Some(1)], &options.with_limits(limits)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
}

#[test]
fn the_directory_size_is_checked_before_allocation() {
    let options = WriteOptions::default().with_max_chunk_size(MIN_CHUNK_SIZE);
    let layout = Layout::new(&[Some(1 << 20), None], &options).unwrap();
    // 256 fragments of 12 bytes, the end marker and the nil record.
    let size = 256 * 12 + 4 + 4;
    assert_eq!(layout.encode_directory(size).unwrap().len(), 3080);
    let error = layout.encode_directory(size - 1).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
}
