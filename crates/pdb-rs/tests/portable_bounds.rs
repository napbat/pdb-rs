//! Integer boundaries of the Portable PDB metadata root: stream ranges and the stream count.

use pdb_rs::ErrorKind;
use pdb_rs::portable::metadata::{MetadataRoot, MetadataStream};

/// The size of the root header with one `#Blob` stream header.
const HEADER_LEN: u32 = 48;
/// The size of the input.
const INPUT_LEN: usize = 64;

/// Build a metadata root with one `#Blob` stream header and zero bytes up to [`INPUT_LEN`].
fn root(offset: u32, size: u32) -> Vec<u8> {
    let mut bytes = b"BSJB".to_vec();
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&12_u32.to_le_bytes());
    bytes.extend_from_slice(b"PDB v1.0\0\0\0\0");
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&offset.to_le_bytes());
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(b"#Blob\0\0\0");
    assert_eq!(bytes.len(), usize::try_from(HEADER_LEN).unwrap());
    bytes.resize(INPUT_LEN, 0);
    bytes
}

#[test]
fn stream_ranges_near_the_u32_limit_end_after_the_input() {
    let length = u64::try_from(INPUT_LEN).unwrap();
    for (offset, size) in [
        (0xffff_fffc, 4),
        (HEADER_LEN, 0xffff_fffc),
        (0xffff_fffc, 0xffff_fffc),
    ] {
        let bytes = root(offset, size);
        let error = MetadataRoot::read(bytes.as_slice(), 4).unwrap_err();
        assert_eq!(
            error.kind(),
            ErrorKind::UnexpectedEof,
            "{offset:#x} {size:#x}"
        );
        assert_eq!(error.offset(), Some(length));
        assert_eq!(
            error.portable_location().unwrap().stream(),
            Some(MetadataStream::Blob)
        );
    }
}

#[test]
fn a_stream_may_end_at_the_input_end() {
    let size = u32::try_from(INPUT_LEN).unwrap() - HEADER_LEN;
    let bytes = root(HEADER_LEN, size);
    let parsed = MetadataRoot::read(bytes.as_slice(), 4).unwrap();
    let header = parsed.stream(MetadataStream::Blob).unwrap();
    assert_eq!((header.offset(), header.size()), (HEADER_LEN, size));

    let bytes = root(HEADER_LEN, size + 4);
    let error = MetadataRoot::read(bytes.as_slice(), 4).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
}

#[test]
fn the_stream_count_may_equal_the_limit() {
    let bytes = root(HEADER_LEN, 0);
    assert_eq!(
        MetadataRoot::read(bytes.as_slice(), 1)
            .unwrap()
            .streams()
            .len(),
        1
    );
    let error = MetadataRoot::read(bytes.as_slice(), 0).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    // The stream count follows the version string and the flags.
    assert_eq!(error.offset(), Some(30));
}
