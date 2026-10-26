//! Tests for the `sourcelink` named stream.

use pdbrs::ErrorKind;
use pdbrs::native::auxiliary::{NamedStreamKind, SOURCE_LINK_STREAM_NAME, SourceLink};
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
        assert_eq!(error.stream_location(), Some((9, 0)));
    }
}
