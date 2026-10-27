//! Tests for .NET document records in `/src/files/` streams (compression byte 101).

use std::borrow::Cow;

use pdb_rs::ErrorKind;
use pdb_rs::native::codeview::index::NameOffset;
use pdb_rs::native::hash::hash_buffer_v8;
use pdb_rs::native::injected_source::{
    CHECKSUM_SHA1, CHECKSUM_SHA256, ChecksumCheck, DOCUMENT_TYPE_TEXT, DotNetDocument,
    InjectedSource, LANGUAGE_CSHARP, SourceCompression, VENDOR_MICROSOFT,
};
use pdb_rs::native::msf::StreamId;
use pdb_rs::portable::embedded::{EmbeddedFormat, EmbeddedSource};

/// The source text of the tests.
const SOURCE: &[u8] = b"abc";

/// The SHA-256 digest of `abc` (FIPS 180-4 example).
const SOURCE_SHA256: [u8; 32] = [
    0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
    0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
];

/// `abc` as a raw DEFLATE stream with one fixed-Huffman block (Python `zlib`, `wbits=-15`).
const SOURCE_DEFLATE: [u8; 5] = [0x4b, 0x4c, 0x4a, 0x06, 0x00];

/// The stream ID in the tests.
const STREAM: StreamId = StreamId(15);

/// Serialize a document record.
fn record(algorithm: [u8; 16], checksum: &[u8], embedded: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for guid in [
        LANGUAGE_CSHARP,
        VENDOR_MICROSOFT,
        DOCUMENT_TYPE_TEXT,
        algorithm,
    ] {
        bytes.extend_from_slice(&guid);
    }
    bytes.extend_from_slice(&u32::try_from(checksum.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(embedded.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(checksum);
    bytes.extend_from_slice(embedded);
    bytes
}

/// Return an embedded-source blob with the format field `format`.
fn blob(format: u32, data: &[u8]) -> Vec<u8> {
    let mut bytes = format.to_le_bytes().to_vec();
    bytes.extend_from_slice(data);
    bytes
}

/// Return a header entry that describes `content`.
fn entry(content: &[u8], compression: SourceCompression) -> InjectedSource {
    let mut source = InjectedSource::default();
    source.crc = hash_buffer_v8(content);
    source.file_size = u32::try_from(content.len()).unwrap();
    source.file_name_offset = NameOffset(1);
    source.object_name_offset = NameOffset(0);
    source.virtual_file_name_offset = NameOffset(1);
    source.compression = compression;
    source.is_virtual = false;
    source.reserved = [0; 8];
    source
}

/// Verify `content` with a correct entry and return the record.
fn document(content: &[u8]) -> DotNetDocument<'_> {
    entry(content, SourceCompression::DOT_NET)
        .verify_document(content, STREAM)
        .unwrap()
}

#[test]
fn compression_byte_101_is_a_dotnet_document() {
    assert_eq!(SourceCompression(101), SourceCompression::DOT_NET);
    assert_ne!(SourceCompression(100), SourceCompression::DOT_NET);
    assert_ne!(SourceCompression(102), SourceCompression::DOT_NET);
}

#[test]
fn decodes_checksum_only_records() {
    let content = record(CHECKSUM_SHA256, &SOURCE_SHA256, &[]);
    assert_eq!(content.len(), 104);
    let document = document(&content);
    assert_eq!(document.language, LANGUAGE_CSHARP);
    assert_eq!(document.language_vendor, VENDOR_MICROSOFT);
    assert_eq!(document.document_type, DOCUMENT_TYPE_TEXT);
    assert_eq!(document.checksum_algorithm, CHECKSUM_SHA256);
    assert_eq!(document.checksum, SOURCE_SHA256);
    assert_eq!(document.embedded, None);
    // The caller can check the checksum against a file on disk.
    assert_eq!(
        document.verify_checksum(SOURCE).unwrap(),
        ChecksumCheck::Verified
    );
}

#[test]
fn decodes_raw_embedded_sources() {
    let content = record(CHECKSUM_SHA256, &SOURCE_SHA256, &blob(0, SOURCE));
    let document = document(&content);
    let embedded = document.embedded.clone().unwrap();
    assert_eq!(embedded, EmbeddedSource::Raw(SOURCE.into()));
    assert_eq!(embedded.format(), EmbeddedFormat::Raw);
    let source = embedded.decode(SOURCE.len()).unwrap();
    assert!(matches!(source, Cow::Borrowed(_)));
    assert_eq!(
        document.verify_checksum(&source).unwrap(),
        ChecksumCheck::Verified
    );
}

#[test]
fn decodes_deflate_embedded_sources() {
    let content = record(CHECKSUM_SHA256, &SOURCE_SHA256, &blob(3, &SOURCE_DEFLATE));
    let document = document(&content);
    let embedded = document.embedded.clone().unwrap();
    assert_eq!(embedded.format(), EmbeddedFormat::Deflate);
    assert_eq!(embedded.decoded_size(), 3);
    let source = embedded.decode(3).unwrap();
    assert_eq!(&*source, SOURCE);
    assert_eq!(
        document.verify_checksum(&source).unwrap(),
        ChecksumCheck::Verified
    );
}

#[test]
fn reports_other_algorithms_as_not_verified() {
    let content = record(CHECKSUM_SHA1, &[0xa9; 20], &blob(0, SOURCE));
    let sha1 = document(&content);
    assert_eq!(sha1.checksum.len(), 20);
    assert_eq!(
        sha1.verify_checksum(SOURCE).unwrap(),
        ChecksumCheck::NotVerified
    );
    // An unknown algorithm GUID is also not an error.
    let content = record([0x55; 16], &[], &[]);
    assert_eq!(
        document(&content).verify_checksum(b"").unwrap(),
        ChecksumCheck::NotVerified
    );
}

#[test]
fn rejects_wrong_sha256_checksums() {
    let mut checksum = SOURCE_SHA256;
    checksum[31] ^= 1;
    let content = record(CHECKSUM_SHA256, &checksum, &blob(0, SOURCE));
    let error = document(&content).verify_checksum(SOURCE).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    // A SHA-256 checksum of the wrong size is also wrong.
    let content = record(CHECKSUM_SHA256, &SOURCE_SHA256[..20], &[]);
    let error = document(&content).verify_checksum(SOURCE).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
}

#[test]
fn rejects_wrong_entry_crc_and_size() {
    let content = record(CHECKSUM_SHA256, &SOURCE_SHA256, &[]);
    let mut source = entry(&content, SourceCompression::DOT_NET);
    source.crc ^= 1;
    let error = source.verify_document(&content, STREAM).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(error.stream_location(), Some((STREAM, 0)));

    for size in [103, 105, 0] {
        let mut source = entry(&content, SourceCompression::DOT_NET);
        source.file_size = size;
        let error = source.verify_document(&content, STREAM).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat, "size {size}");
    }
}

#[test]
fn separates_document_records_from_file_contents() {
    let content = record(CHECKSUM_SHA256, &SOURCE_SHA256, &[]);
    let dotnet = entry(&content, SourceCompression::DOT_NET);
    let error = dotnet.verify_content(&content, STREAM).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedVersion);
    for compression in [SourceCompression::NONE, SourceCompression::LZ] {
        let error = entry(&content, compression)
            .verify_document(&content, STREAM)
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnsupportedVersion);
        assert_eq!(error.stream_location(), Some((STREAM, 0)));
    }
}

/// Decode `content` with a correct entry and return the error kind.
fn record_error(content: &[u8]) -> ErrorKind {
    let error = entry(content, SourceCompression::DOT_NET)
        .verify_document(content, STREAM)
        .unwrap_err();
    assert_eq!(error.stream_location(), Some((STREAM, 0)));
    error.kind()
}

#[test]
fn rejects_records_with_wrong_sizes() {
    let content = record(CHECKSUM_SHA256, &SOURCE_SHA256, &blob(0, SOURCE));
    // Each truncation is too short for a size field.
    for length in 0..content.len() {
        assert_eq!(
            record_error(&content[..length]),
            ErrorKind::InvalidFormat,
            "length {length}"
        );
    }
    // A byte after the embedded source.
    let mut longer = content.clone();
    longer.push(0);
    assert_eq!(record_error(&longer), ErrorKind::InvalidFormat);
    // Bytes after the checksum of a record without an embedded source.
    let mut longer = record(CHECKSUM_SHA256, &SOURCE_SHA256, &[]);
    longer.extend_from_slice(&[0; 4]);
    assert_eq!(record_error(&longer), ErrorKind::InvalidFormat);
    // Sizes past the end of the stream.
    for (checksum_size, source_size) in [(u32::MAX, 0), (32, u32::MAX), (33, 7)] {
        let mut bad = content.clone();
        bad[64..68].copy_from_slice(&checksum_size.to_le_bytes());
        bad[68..72].copy_from_slice(&source_size.to_le_bytes());
        assert_eq!(record_error(&bad), ErrorKind::InvalidFormat);
    }
}

#[test]
fn rejects_embedded_sources_shorter_than_the_format_field() {
    // The embedded size is 3: shorter than the 4-byte format field.
    let content = record(CHECKSUM_SHA256, &SOURCE_SHA256, &[0, 0, 0]);
    assert_eq!(record_error(&content), ErrorKind::UnexpectedEof);
    // A negative format is reserved.
    let content = record(CHECKSUM_SHA256, &SOURCE_SHA256, &blob(0x8000_0000, SOURCE));
    assert_eq!(record_error(&content), ErrorKind::UnsupportedVersion);
}

#[test]
fn reports_embedded_source_decode_errors() {
    // The declared size is larger than the stream gives.
    let content = record(CHECKSUM_SHA256, &SOURCE_SHA256, &blob(4, &SOURCE_DEFLATE));
    let embedded = document(&content).embedded.unwrap();
    assert_eq!(
        embedded.decode(16).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    // The declared size is larger than the limit.
    assert_eq!(
        embedded.decode(3).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
}
