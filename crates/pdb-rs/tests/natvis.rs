//! Behavior tests for Natvis files in injected sources.

use pdb_rs::ErrorKind;
use pdb_rs::native::codeview::index::NameOffset;
use pdb_rs::native::hash::hash_buffer_v8;
use pdb_rs::native::injected_source::{
    InjectedSource, InjectedSourceKind, Natvis, SourceCompression, is_natvis_name,
};
use pdb_rs::native::msf::StreamId;

const XML: &[u8] = b"<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n\
<!-- Visualizers -->\r\n\
<!DOCTYPE AutoVisualizer>\r\n\
<AutoVisualizer xmlns=\"http://schemas.microsoft.com/vstudio/debugger/natvis/2010\">\r\n\
</AutoVisualizer>\r\n";

fn source(content: &[u8], compression: SourceCompression) -> InjectedSource {
    let mut source = InjectedSource::default();
    source.crc = hash_buffer_v8(content);
    source.file_size = u32::try_from(content.len()).unwrap();
    source.file_name_offset = NameOffset(1);
    source.object_name_offset = NameOffset(0);
    source.virtual_file_name_offset = NameOffset(2);
    source.compression = compression;
    source.is_virtual = false;
    source.reserved = [0; 8];
    source
}

#[test]
fn natvis_names_end_with_the_extension_in_any_case() {
    assert!(is_natvis_name(b"c:\\x\\libcore.natvis"));
    assert!(is_natvis_name(b"C:\\X\\A.NATVIS"));
    assert!(is_natvis_name(b".natvis"));
    assert!(!is_natvis_name(b"natvis"));
    assert!(!is_natvis_name(b"a.natvis.bak"));
    assert!(!is_natvis_name(b""));
    assert!(!is_natvis_name(b"a.natvi\0s"));
}

#[test]
fn entries_are_classified_by_compression_and_virtual_name() {
    let file = source(XML, SourceCompression::NONE);
    assert_eq!(file.kind(b"c:\\a.natvis"), InjectedSourceKind::Natvis);
    assert_eq!(file.kind(b"c:\\a.cpp"), InjectedSourceKind::File);
    let document = source(XML, SourceCompression::DOT_NET);
    assert_eq!(
        document.kind(b"c:\\a.natvis"),
        InjectedSourceKind::DotNetDocument
    );
}

#[test]
fn natvis_text_has_an_auto_visualizer_root() {
    let natvis = Natvis::parse(XML).unwrap();
    assert!(!natvis.has_byte_order_mark());
    assert_eq!(natvis.xml().as_bytes(), XML);

    let mut bom = b"\xef\xbb\xbf".to_vec();
    bom.extend_from_slice(XML);
    let natvis = Natvis::parse(&bom).unwrap();
    assert!(natvis.has_byte_order_mark());
    assert_eq!(natvis.xml().as_bytes(), XML);

    for minimal in [
        &b"<AutoVisualizer/>"[..],
        b"  \n<AutoVisualizer\t>",
        b"<AutoVisualizer>",
    ] {
        Natvis::parse(minimal).unwrap();
    }
}

#[test]
fn other_text_is_rejected() {
    let cases: [&[u8]; 9] = [
        b"",
        b"\xef\xbb\xbf",
        b"<AutoVisualizerX/>",
        b"<Root><AutoVisualizer/></Root>",
        b"<?xml version=\"1.0\"",
        b"<!-- open comment <AutoVisualizer/>",
        b"<AutoVisualizer/>\xff",
        b"\xff\xfe<\0A\0",
        b"text <AutoVisualizer/>",
    ];
    for bytes in cases {
        let error = Natvis::parse(bytes).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat, "{bytes:?}");
        assert_eq!(error.stream_location(), None);
    }
}

#[test]
fn verify_natvis_checks_the_entry_and_the_text() {
    let stream = StreamId(85);
    let file = source(XML, SourceCompression::NONE);
    assert_eq!(
        file.verify_natvis(XML, stream).unwrap().xml().as_bytes(),
        XML
    );

    // The entry check comes first: a size or CRC mismatch is reported before the text.
    let error = file.verify_natvis(&XML[1..], stream).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(error.stream_location(), Some((StreamId(85), 0)));

    let text = b"<NotNatvis/>";
    let error = source(text, SourceCompression::NONE)
        .verify_natvis(text, stream)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(error.stream_location(), Some((StreamId(85), 0)));

    let error = source(XML, SourceCompression::LZ)
        .verify_natvis(XML, stream)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedVersion);
}
