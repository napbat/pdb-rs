//! Tests for the `sourcelink` and `sourcelink$<n>` named streams.

use pdbrs::ErrorKind;
use pdbrs::native::auxiliary::{
    NUMBERED_SOURCE_LINK_PREFIX, NamedStreamKind, SOURCE_LINK_STREAM_NAME, SourceLink,
};
use pdbrs::native::msf::StreamId;

/// The Source Link JSON of the `dotnet-csharp-full` fixture, shortened.
const JSON: &str =
    r#"{"documents":{"C:\\src\\*":"https://raw.githubusercontent.com/napbat/pdbrs/aa5e855/*"}}"#;

#[test]
fn classifies_the_exact_stream_name() {
    assert_eq!(SOURCE_LINK_STREAM_NAME, b"sourcelink");
    assert_eq!(
        NamedStreamKind::classify(b"sourcelink"),
        NamedStreamKind::SourceLink
    );
    for other in [&b"SourceLink"[..], b"/sourcelink", b"sourcelink\0"] {
        assert_eq!(NamedStreamKind::classify(other), NamedStreamKind::Other);
    }
}

#[test]
fn keeps_the_json_bytes() {
    let link = SourceLink::parse(JSON.as_bytes(), StreamId(9)).unwrap();
    assert_eq!(link.json(), JSON);
    // White space and a byte order mark around the object stay in the text.
    let text = format!("\u{feff} \r\n{JSON}\n");
    let link = SourceLink::parse(text.as_bytes(), StreamId(9)).unwrap();
    assert_eq!(link.json(), text);
}

#[test]
fn rejects_text_that_is_not_a_json_object() {
    for bytes in [
        &b""[..],
        b"  ",
        b"[]",
        b"{\"documents\":",
        b"\"documents\"}",
        b"{\"a\":\"\xff\"}",
    ] {
        let error = SourceLink::parse(bytes, StreamId(9)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat, "{bytes:?}");
        assert_eq!(error.stream_location(), Some((StreamId(9), 0)));
    }
}

/// `link /SOURCELINK` 14.51 writes `sourcelink$1` and the following numbers
/// (`msvc-x64-sourcelink`). Only decimal numbers from 1 without leading zeros are numbered
/// streams.
#[test]
fn classifies_the_numbered_stream_names_of_link() {
    assert_eq!(NUMBERED_SOURCE_LINK_PREFIX, b"sourcelink$");
    for (name, number) in [
        (&b"sourcelink$1"[..], 1),
        (b"sourcelink$2", 2),
        (b"sourcelink$10", 10),
        (b"sourcelink$4294967295", u32::MAX),
    ] {
        assert_eq!(
            NamedStreamKind::classify(name),
            NamedStreamKind::NumberedSourceLink { number }
        );
    }
    for other in [
        &b"sourcelink$"[..],
        b"sourcelink$0",
        b"sourcelink$01",
        b"sourcelink$1a",
        b"sourcelink$-1",
        b"sourcelink$4294967296",
        b"SourceLink$1",
    ] {
        assert_eq!(NamedStreamKind::classify(other), NamedStreamKind::Other);
    }
}

/// The last numbered stream of `link /SOURCELINK` is empty. The other
/// numbered streams hold the JSON files.
#[test]
fn numbered_streams_accept_json_and_the_empty_end_marker() {
    let link = SourceLink::parse_numbered(JSON.as_bytes(), StreamId(69)).unwrap();
    assert_eq!(link.json(), JSON);
    assert!(!link.is_empty());
    let end = SourceLink::parse_numbered(b"", StreamId(70)).unwrap();
    assert!(end.is_empty());
    let mut out = Vec::new();
    end.encode(&mut out).unwrap();
    assert_eq!(out, b"");
    for bytes in [&b"  "[..], b"[]", b"{\"a\":\"\xff\"}"] {
        let error = SourceLink::parse_numbered(bytes, StreamId(69)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat, "{bytes:?}");
    }
}
