//! Behavior tests for the `/LinkInfo`, `/TMCache` and `/UDTSRCLINEUNDONE` streams.

use pdbrs::ErrorKind;
use pdbrs::native::auxiliary::{
    LINK_INFO_STREAM_NAME, LINK_INFO_VERSION_TWO, LinkInfo, NamedStreamKind, TM_CACHE_STREAM_NAME,
    TmCache, UDT_SOURCE_LINE_UNDONE_STREAM_NAME, UdtSourceLineUndone,
};
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::msf::StreamId;

const STREAM: StreamId = StreamId(5);
const DIRECTORY: &[u8] = b"C:\\build";
const COMMAND: &[u8] = b"link.exe -out:app.exe";
const LIBRARIES: &[u8] = b"a.lib\0b.lib\0";

fn put(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap()
}

/// Build a version 2 record with the strings directly after the fixed fields.
fn link_info() -> Vec<u8> {
    let command = 24 + DIRECTORY.len() + 1;
    let libraries = command + COMMAND.len() + 1;
    let size = libraries + LIBRARIES.len();
    let mut bytes = vec![0; 24];
    for (offset, value) in [
        (0, to_u32(size)),
        (4, LINK_INFO_VERSION_TWO),
        (8, 24),
        (12, to_u32(command)),
        (16, 14),
        (20, to_u32(libraries)),
    ] {
        put(&mut bytes, offset, value);
    }
    for part in [DIRECTORY, b"\0", COMMAND, b"\0", LIBRARIES] {
        bytes.extend_from_slice(part);
    }
    bytes
}

#[test]
fn link_info_decodes_strings_libraries_and_trailing_bytes() {
    let mut bytes = link_info();
    let info = LinkInfo::parse(&bytes, STREAM).unwrap().unwrap();
    assert_eq!(info.size, to_u32(bytes.len()));
    assert_eq!(info.current_directory, DIRECTORY);
    assert_eq!(info.command, COMMAND);
    assert_eq!(info.output_file_offset, 14);
    assert_eq!(info.output_file(), b"app.exe");
    assert_eq!(info.libraries, LIBRARIES);
    assert_eq!(info.trailing, b"");

    bytes.extend_from_slice(&[0xcd; 7]);
    let info = LinkInfo::parse(&bytes, STREAM).unwrap().unwrap();
    assert_eq!(info.trailing, [0xcd; 7]);
    assert_eq!(info.libraries, LIBRARIES);
}

#[test]
fn an_empty_link_info_stream_has_no_record() {
    assert_eq!(LinkInfo::parse(&[], STREAM).unwrap(), None);
}

#[test]
fn link_info_layout_rules_are_enforced() {
    let base = link_info();
    let size = to_u32(base.len());
    let cases = [
        (4, 1, ErrorKind::UnsupportedVersion),
        (0, 23, ErrorKind::InvalidFormat),
        (0, size + 1, ErrorKind::UnexpectedEof),
        (8, 25, ErrorKind::InvalidFormat),
        (12, 34, ErrorKind::InvalidFormat),
        (20, size, ErrorKind::InvalidFormat),
        (16, to_u32(COMMAND.len()) + 1, ErrorKind::InvalidFormat),
    ];
    for (offset, value, expected) in cases {
        let mut bytes = base.clone();
        put(&mut bytes, offset, value);
        let error = LinkInfo::parse(&bytes, STREAM).unwrap_err();
        assert_eq!(error.kind(), expected, "offset {offset} value {value}");
        assert_eq!(error.stream_location().unwrap().0, STREAM.0);
        assert_eq!(error.offset(), None);
    }
    // A command without a NUL inside the record.
    let mut bytes = base.clone();
    let libraries = 24 + DIRECTORY.len() + 1 + COMMAND.len() + 1;
    put(&mut bytes, 0, to_u32(libraries - 1));
    let error = LinkInfo::parse(&bytes, STREAM).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);

    let error = LinkInfo::parse(&base[..20], STREAM).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
}

#[test]
fn tm_cache_accepts_only_an_empty_stream() {
    assert_eq!(TmCache::parse(&[], StreamId(6)).unwrap(), TmCache::Empty);
    let error = TmCache::parse(&[0x29, 0x02, 0x20, 0x20], StreamId(6)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedVersion);
    assert_eq!(error.stream_location(), Some((6, 0)));
}

#[test]
fn udt_source_line_undone_lists_type_indices() {
    let bytes = [0x72, 0x10, 0, 0, 0x2e, 0x30, 0, 0];
    let list = UdtSourceLineUndone::parse(&bytes, StreamId(82)).unwrap();
    assert_eq!(list.len(), 2);
    assert!(!list.is_empty());
    let indices: Vec<_> = list.type_indices().collect();
    assert_eq!(indices, [TypeIndex(0x1072), TypeIndex(0x302e)]);
    assert!(
        UdtSourceLineUndone::parse(&[], StreamId(82))
            .unwrap()
            .is_empty()
    );

    let error = UdtSourceLineUndone::parse(&bytes[..6], StreamId(82)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(error.stream_location(), Some((82, 4)));
}

#[test]
fn named_streams_are_classified_by_exact_name_bytes() {
    let cases = [
        (&b"/names"[..], NamedStreamKind::Names),
        (LINK_INFO_STREAM_NAME, NamedStreamKind::LinkInfo),
        (TM_CACHE_STREAM_NAME, NamedStreamKind::TmCache),
        (
            UDT_SOURCE_LINE_UNDONE_STREAM_NAME,
            NamedStreamKind::UdtSourceLineUndone,
        ),
        (b"/src/headerblock", NamedStreamKind::SourceHeaderBlock),
        (
            b"/src/files/c:\\a.natvis",
            NamedStreamKind::SourceFile {
                virtual_file_name: b"c:\\a.natvis",
            },
        ),
        (b"/linkinfo", NamedStreamKind::Other),
        (b"sourcelink$1", NamedStreamKind::Other),
    ];
    for (name, expected) in cases {
        assert_eq!(NamedStreamKind::classify(name), expected);
    }
}
