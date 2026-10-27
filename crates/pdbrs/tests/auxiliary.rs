//! Behavior tests for the `/LinkInfo`, `/UDTSRCLINEUNDONE` and `sourcelink` streams and
//! named-stream kinds.
//!
//! `tests/tm_cache.rs` tests the `/TMCache` stream. `tests/srcsrv.rs` tests the `srcsrv` stream.

use pdbrs::ErrorKind;
use pdbrs::native::auxiliary::{
    LINK_INFO_STREAM_NAME, LINK_INFO_VERSION_ONE, LINK_INFO_VERSION_TWO, LinkInfo, NamedStreamKind,
    SOURCE_SERVER_STREAM_NAME, SourceLink, TM_CACHE_STREAM_NAME,
    UDT_SOURCE_LINE_UNDONE_STREAM_NAME, UdtSourceLineUndone,
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
        (4, 3, ErrorKind::UnsupportedVersion),
        (4, 0, ErrorKind::UnsupportedVersion),
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
        assert_eq!(error.stream_location().unwrap().0, STREAM);
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
    assert_eq!(error.stream_location(), Some((StreamId(82), 4)));
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
                virtual_file_name: b"c:\\a.natvis".into(),
            },
        ),
        (SOURCE_SERVER_STREAM_NAME, NamedStreamKind::SourceServer),
        (b"SRCSRV", NamedStreamKind::Other),
        (b"/linkinfo", NamedStreamKind::Other),
        (
            b"sourcelink$1",
            NamedStreamKind::NumberedSourceLink { number: 1 },
        ),
        (b"sourcelink$01", NamedStreamKind::Other),
    ];
    for (name, expected) in cases {
        assert_eq!(NamedStreamKind::classify(name), expected);
    }
}

/// The arguments of [`LinkInfo::new`] without the libraries, and the expected error.
type NewCase = (u32, &'static [u8], &'static [u8], u32, ErrorKind);

fn encode_link_info(info: &LinkInfo<'_>) -> pdbrs::Result<Vec<u8>> {
    let mut out = Vec::new();
    info.encode(&mut out)?;
    Ok(out)
}

#[test]
fn link_info_encodes_both_versions_and_trailing_bytes() {
    let mut bytes = link_info();
    let info = LinkInfo::parse(&bytes, STREAM).unwrap().unwrap();
    assert_eq!(info.version, LINK_INFO_VERSION_TWO);
    assert_eq!(encode_link_info(&info).unwrap(), bytes);

    // Version 1 has the same layout.
    put(&mut bytes, 4, LINK_INFO_VERSION_ONE);
    bytes.extend_from_slice(&[0; 9]);
    let info = LinkInfo::parse(&bytes, STREAM).unwrap().unwrap();
    assert_eq!(info.version, LINK_INFO_VERSION_ONE);
    assert_eq!(info.trailing, [0; 9]);
    assert_eq!(info.output_file(), b"app.exe");
    assert_eq!(encode_link_info(&info).unwrap(), bytes);
}

#[test]
fn link_info_new_calculates_the_record_size() {
    let info = LinkInfo::new(LINK_INFO_VERSION_TWO, DIRECTORY, COMMAND, 14, LIBRARIES).unwrap();
    assert_eq!(encode_link_info(&info).unwrap(), link_info());
    assert_eq!(info.size, to_u32(link_info().len()));

    let empty = LinkInfo::new(LINK_INFO_VERSION_ONE, b"", b"", 0, b"").unwrap();
    let bytes = encode_link_info(&empty).unwrap();
    assert_eq!(bytes.len(), 26);
    let decoded = LinkInfo::parse(&bytes, STREAM).unwrap().unwrap();
    assert_eq!(decoded, empty);

    let cases: [NewCase; 4] = [
        (3, DIRECTORY, COMMAND, 0, ErrorKind::UnsupportedVersion),
        (2, b"a\0b", COMMAND, 0, ErrorKind::InvalidFormat),
        (2, DIRECTORY, b"link\0", 0, ErrorKind::InvalidFormat),
        (2, DIRECTORY, COMMAND, 22, ErrorKind::InvalidFormat),
    ];
    for (version, directory, command, output, kind) in cases {
        let error = LinkInfo::new(version, directory, command, output, b"").unwrap_err();
        assert_eq!(error.kind(), kind);
    }
}

#[test]
fn link_info_encoder_checks_the_record_size() {
    let bytes = link_info();
    let mut info = LinkInfo::parse(&bytes, STREAM).unwrap().unwrap();
    info.size += 1;
    assert_eq!(
        encode_link_info(&info).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    info.size -= 1;
    info.output_file_offset = u32::MAX;
    assert_eq!(
        encode_link_info(&info).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    info.output_file_offset = 0;
    info.version = 7;
    assert_eq!(
        encode_link_info(&info).unwrap_err().kind(),
        ErrorKind::UnsupportedVersion
    );
}

#[test]
fn udt_source_line_undone_and_source_link_encode_their_bytes() {
    let bytes = [0x72, 0x10, 0, 0, 0x2e, 0x30, 0, 0];
    let list = UdtSourceLineUndone::parse(&bytes, StreamId(82)).unwrap();
    let mut out = Vec::new();
    list.encode(&mut out).unwrap();
    assert_eq!(out, bytes);
    let mut built = Vec::new();
    UdtSourceLineUndone::encode_parts(list.type_indices(), &mut built).unwrap();
    assert_eq!(built, bytes);
    let mut none = Vec::new();
    UdtSourceLineUndone::encode_parts([], &mut none).unwrap();
    assert_eq!(none, b"");

    let json = b"\xef\xbb\xbf {\"documents\":{\"C:\\\\*\":\"https://example.test/*\"}}\r\n";
    let link = SourceLink::parse(json, StreamId(9)).unwrap();
    let mut out = Vec::new();
    link.encode(&mut out).unwrap();
    assert_eq!(out, json);
    for bad in [&b""[..], b"[]", b"{\xff}"] {
        let error = SourceLink::parse(bad, StreamId(9)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    }
}
