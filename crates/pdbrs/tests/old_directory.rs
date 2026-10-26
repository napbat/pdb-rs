//! Behavior tests for the previous stream directory snapshot in stream 0.

use pdbrs::ErrorKind;
use pdbrs::native::msf::{OldDirectory, StreamId};

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

#[test]
fn decodes_sizes_and_page_lists_without_following_pages() {
    // The layout of a 48-byte stream 0: six streams, five pages.
    let bytes = words(&[6, 40, 60, 56, 64, 56, 0, 8, 5, 3, 6, 4]);
    let directory = OldDirectory::parse(&bytes, 4096).unwrap().unwrap();
    assert_eq!(directory.stream_count(), 6);
    assert_eq!(directory.page_count(), 5);
    let streams: Vec<(Option<u32>, Vec<u32>)> = directory
        .streams()
        .map(|stream| (stream.size(), stream.pages().collect()))
        .collect();
    assert_eq!(
        streams,
        [
            (Some(40), vec![8]),
            (Some(60), vec![5]),
            (Some(56), vec![3]),
            (Some(64), vec![6]),
            (Some(56), vec![4]),
            (Some(0), vec![]),
        ]
    );
    assert_eq!(directory.streams().len(), 6);
}

#[test]
fn nil_streams_have_no_size_and_no_pages() {
    // Page numbers are not checked: 0 and 1 are reserved pages in a container.
    let bytes = words(&[3, u32::MAX, 1025, u32::MAX, 0, 1, 2]);
    let directory = OldDirectory::parse(&bytes, 512).unwrap().unwrap();
    let streams: Vec<(Option<u32>, Vec<u32>)> = directory
        .streams()
        .map(|stream| (stream.size(), stream.pages().collect()))
        .collect();
    assert_eq!(
        streams,
        [(None, vec![]), (Some(1025), vec![0, 1, 2]), (None, vec![])]
    );
}

#[test]
fn an_empty_stream_zero_has_no_snapshot() {
    assert!(OldDirectory::parse(&[], 4096).unwrap().is_none());
}

#[test]
fn malformed_snapshots_are_rejected_in_stream_zero() {
    let cases: [(Vec<u8>, ErrorKind, u64); 6] = [
        (vec![1, 0, 0], ErrorKind::InvalidFormat, 3),
        (words(&[0]), ErrorKind::InvalidFormat, 0),
        (words(&[3, 1]), ErrorKind::UnexpectedEof, 4),
        (words(&[1, 100]), ErrorKind::InvalidFormat, 8),
        (words(&[1, 100, 7, 8]), ErrorKind::InvalidFormat, 8),
        (words(&[2, 0, 513, 9]), ErrorKind::InvalidFormat, 12),
    ];
    for (bytes, kind, offset) in cases {
        let error = OldDirectory::parse(&bytes, 512).unwrap_err();
        assert_eq!(error.kind(), kind, "{bytes:?}");
        assert_eq!(
            error.stream_location(),
            Some((StreamId(0), offset)),
            "{bytes:?}"
        );
        assert_eq!(error.offset(), None);
    }
}

#[test]
fn page_sizes_outside_the_valid_powers_of_two_are_rejected() {
    let bytes = words(&[1, 0]);
    for block_size in [0, 256, 768, 131_072] {
        let error = OldDirectory::parse(&bytes, block_size).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    }
    assert!(OldDirectory::parse(&bytes, 65_536).unwrap().is_some());
}
