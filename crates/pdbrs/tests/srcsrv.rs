//! Behavior tests for the `srcsrv` named stream.

use pdbrs::ErrorKind;
use pdbrs::native::auxiliary::{SourceServer, SourceServerSection};
use pdbrs::native::msf::StreamId;

const STREAM: StreamId = StreamId(74);

/// A stream with CR LF line ends, like the files that indexing scripts write.
const SAMPLE: &[u8] = b"SRCSRV: ini ------------------------------------------------\r\n\
VERSION=2\r\n\
VERCTRL=http\r\n\
SRCSRV: variables ------------------------------------------\r\n\
SRCSRVTRG=https://example.test/%var2%\r\n\
EMPTY=\r\n\
\r\n\
SRCSRV: source files ---------------------------------------\r\n\
c:\\src\\main.cpp*src/main.cpp\r\n\
c:\\src\\a*b.cpp**x\r\n\
SRCSRV: end ------------------------------------------------\r\n";

fn parse(bytes: &[u8]) -> pdbrs::Result<SourceServer<'_>> {
    SourceServer::parse(bytes, STREAM)
}

fn assert_invalid(bytes: &[u8], offset: usize) {
    let error = parse(bytes).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat, "{error}");
    assert_eq!(
        error.stream_location(),
        Some((STREAM, u64::try_from(offset).unwrap())),
        "{error}"
    );
}

#[test]
fn sections_and_lines_are_framed_with_raw_bytes() {
    let server = parse(SAMPLE).unwrap();
    let ini: Vec<_> = server.ini().map(|v| (v.key, v.value)).collect();
    assert_eq!(ini, [(&b"VERSION"[..], &b"2"[..]), (b"VERCTRL", b"http")]);
    let variables: Vec<_> = server.variables().map(|v| (v.key, v.value)).collect();
    assert_eq!(
        variables,
        [
            (&b"SRCSRVTRG"[..], &b"https://example.test/%var2%"[..]),
            (b"EMPTY", b""),
        ]
    );
    let files: Vec<_> = server.source_files().collect();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path(), b"c:\\src\\main.cpp");
    assert_eq!(
        files[0].fields().collect::<Vec<_>>(),
        [&b"c:\\src\\main.cpp"[..], b"src/main.cpp"]
    );
    assert_eq!(
        files[1].fields().collect::<Vec<_>>(),
        [&b"c:\\src\\a"[..], b"b.cpp", b"", b"x"]
    );
    assert_eq!(server.trailing(), b"");
    for section in [
        SourceServerSection::Ini,
        SourceServerSection::Variables,
        SourceServerSection::SourceFiles,
        SourceServerSection::End,
    ] {
        assert!(server.has_section(section));
    }
    let mut out = Vec::new();
    server.encode(&mut out).unwrap();
    assert_eq!(out, SAMPLE);
    assert_eq!(server.bytes(), SAMPLE);
}

#[test]
fn optional_sections_bom_case_and_trailing_bytes_are_kept() {
    let bytes = b"\xef\xbb\xbfsrcsrv: INI\nVERSION=1\nSRCSRV: end\n\0\0junk";
    let server = parse(bytes).unwrap();
    assert!(!server.has_section(SourceServerSection::Variables));
    assert!(!server.has_section(SourceServerSection::SourceFiles));
    assert_eq!(server.variables().count(), 0);
    assert_eq!(server.source_files().count(), 0);
    assert_eq!(server.ini().count(), 1);
    assert_eq!(server.trailing(), b"\0\0junk");
    let mut out = Vec::new();
    server.encode(&mut out).unwrap();
    assert_eq!(out, bytes);

    // The end marker can be the last line without a line end.
    let last = parse(b"SRCSRV: ini\nSRCSRV: end").unwrap();
    assert_eq!(last.ini().count(), 0);
    assert_eq!(last.trailing(), b"");
}

#[test]
fn streams_without_ini_start_or_end_marker_are_rejected() {
    assert_invalid(b"", 0);
    assert_invalid(b"\xef\xbb\xbf", 3);
    assert_invalid(b"VERSION=2\nSRCSRV: ini\nSRCSRV: end\n", 0);
    assert_invalid(b"SRCSRV: variables\nSRCSRV: end\n", 0);
    // A truncated stream has no end marker.
    let end = SAMPLE
        .windows(11)
        .position(|window| window == b"SRCSRV: end")
        .unwrap();
    assert_invalid(&SAMPLE[..end], end);
    assert_invalid(b"SRCSRV: ini\n", 12);
}

#[test]
fn sections_out_of_order_unknown_or_repeated_are_rejected() {
    assert_invalid(
        b"SRCSRV: ini\nSRCSRV: source files\nSRCSRV: variables\n",
        33,
    );
    assert_invalid(b"SRCSRV: ini\nSRCSRV: ini\nSRCSRV: end\n", 12);
    assert_invalid(b"SRCSRV: ini\nSRCSRV: other\nSRCSRV: end\n", 12);
    assert_invalid(b"SRCSRV: ini\nSRCSRV: endx\n", 12);
    assert_invalid(b"SRCSRV: ini\nSRCSRV:\n", 12);
}

#[test]
fn ini_and_variable_lines_need_a_key() {
    assert_invalid(b"SRCSRV: ini\nVERSION\nSRCSRV: end\n", 12);
    assert_invalid(b"SRCSRV: ini\nSRCSRV: variables\n=value\nSRCSRV: end\n", 30);
    // A NUL byte is data: the decoder keeps it in the value.
    let server = parse(b"SRCSRV: ini\nKEY=a\0b\nSRCSRV: end\n").unwrap();
    assert_eq!(server.ini().next().unwrap().value, b"a\0b");
    // Blank and white-space lines are allowed in each section.
    parse(b"SRCSRV: ini\n  \t\nSRCSRV: source files\n\nSRCSRV: end\n").unwrap();
}
