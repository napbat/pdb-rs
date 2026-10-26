//! Tests for the embedded source of `Document` rows and its check against the document hash.

use pdbrs::ErrorKind;
use pdbrs::portable::Pdb;
use pdbrs::portable::compressed::encode_unsigned;
use pdbrs::portable::custom::CustomDebugKind;
use pdbrs::portable::debug::HashAlgorithm;
use pdbrs::portable::embedded::{EmbeddedFormat, SourceCheck};
use pdbrs::portable::metadata::{CustomDebugInformationId, DocumentId, MetadataStream, TableId};

use super::builder::{Builder, row, value};

/// The source text of the tests.
const SOURCE: &[u8] = b"abc";

/// The SHA-256 digest of [`SOURCE`] (FIPS 180-2, appendix B.1).
const SOURCE_SHA256: [u8; 32] = [
    0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
    0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
];

/// [`SOURCE`] as a raw DEFLATE stream with one fixed-Huffman block.
const SOURCE_DEFLATE: [u8; 5] = [0x4b, 0x4c, 0x4a, 0x06, 0x00];

/// The `#GUID` indexes of the tests.
const KIND_EMBEDDED_SOURCE: u32 = 1;
/// The SHA-256 hash algorithm.
const SHA256: u32 = 2;
/// The SHA-1 hash algorithm.
const SHA1: u32 = 3;
/// A kind that is not the embedded source.
const KIND_OTHER: u32 = 4;

/// The `HasCustomDebugInformation` tag of the `Document` table.
const DOCUMENT_TAG: u32 = 22;

/// The heaps and rows of a synthetic Portable PDB with documents and custom debug information.
struct Sources {
    blobs: Vec<u8>,
    /// `Document`: name, hash algorithm, hash, language.
    documents: Vec<[u32; 4]>,
    /// `CustomDebugInformation`: document row, kind, value.
    custom: Vec<[u32; 3]>,
}

impl Sources {
    fn new() -> Self {
        Self {
            blobs: vec![0],
            documents: Vec::new(),
            custom: Vec::new(),
        }
    }

    /// Add a blob and return its offset.
    fn blob(&mut self, bytes: &[u8]) -> u32 {
        let offset = u32::try_from(self.blobs.len()).unwrap();
        let length = encode_unsigned(u32::try_from(bytes.len()).unwrap()).unwrap();
        self.blobs.extend_from_slice(length.as_bytes());
        self.blobs.extend_from_slice(bytes);
        offset
    }

    /// Add a document with `algorithm` and `hash` and return its row ID.
    fn document(&mut self, algorithm: u32, hash: &[u8]) -> u32 {
        let part = self.blob(b"a.cs");
        let mut name = vec![0];
        name.extend_from_slice(encode_unsigned(part).unwrap().as_bytes());
        let name = self.blob(&name);
        let hash = self.blob(hash);
        self.documents.push([name, algorithm, hash, 0]);
        u32::try_from(self.documents.len()).unwrap()
    }

    /// Add a custom debug information row for `document`.
    fn custom(&mut self, document: u32, kind: u32, value: &[u8]) {
        let value = self.blob(value);
        self.custom.push([document, kind, value]);
    }

    /// Assemble the metadata.
    fn bytes(&self) -> Vec<u8> {
        let mut guids = CustomDebugKind::EMBEDDED_SOURCE.0.to_vec();
        guids.extend_from_slice(&HashAlgorithm::Sha256.guid());
        guids.extend_from_slice(&HashAlgorithm::Sha1.guid());
        guids.extend_from_slice(&CustomDebugKind::SOURCE_LINK.0);
        let documents = self
            .documents
            .iter()
            .flat_map(|columns| row(&columns.map(|column| value(2, column))))
            .collect();
        let custom = self
            .custom
            .iter()
            .flat_map(|&[document, kind, blob]| {
                row(&[
                    value(2, (document << 5) | DOCUMENT_TAG),
                    value(2, kind),
                    value(2, blob),
                ])
            })
            .collect();
        let count = |rows: usize| u32::try_from(rows).unwrap();
        Builder {
            guids: Some(guids),
            blobs: Some(self.blobs.clone()),
            ..Builder::default()
        }
        .table(0x30, count(self.documents.len()), documents)
        .table(0x37, count(self.custom.len()), custom)
        .build()
    }
}

/// Return an embedded-source blob with the format field `format`.
fn blob(format: u32, data: &[u8]) -> Vec<u8> {
    let mut bytes = format.to_le_bytes().to_vec();
    bytes.extend_from_slice(data);
    bytes
}

/// Make the PDB of the tests:
///
/// 1. SHA-256, a raw source.
/// 2. SHA-256, a DEFLATE source.
/// 3. SHA-256, a raw source that does not match the hash.
/// 4. SHA-1, a raw source.
/// 5. No hash algorithm, no embedded source, but another kind of custom debug information.
fn sources() -> Sources {
    let mut pdb = Sources::new();
    let raw = pdb.document(SHA256, &SOURCE_SHA256);
    let deflate = pdb.document(SHA256, &SOURCE_SHA256);
    let mismatch = pdb.document(SHA256, &SOURCE_SHA256);
    let sha1 = pdb.document(SHA1, &[0x11; 20]);
    let none = pdb.document(0, &[]);
    pdb.custom(raw, KIND_EMBEDDED_SOURCE, &blob(0, SOURCE));
    pdb.custom(deflate, KIND_EMBEDDED_SOURCE, &blob(3, &SOURCE_DEFLATE));
    pdb.custom(mismatch, KIND_EMBEDDED_SOURCE, &blob(0, b"abd"));
    pdb.custom(sha1, KIND_EMBEDDED_SOURCE, &blob(0, SOURCE));
    pdb.custom(none, KIND_OTHER, b"{}");
    pdb
}

#[test]
fn decodes_and_checks_embedded_sources() {
    let bytes = sources().bytes();
    let pdb = Pdb::parse(&bytes).unwrap();
    let expected = [
        Some((EmbeddedFormat::Raw, SOURCE, SourceCheck::Verified)),
        Some((EmbeddedFormat::Deflate, SOURCE, SourceCheck::Verified)),
        Some((EmbeddedFormat::Raw, &b"abd"[..], SourceCheck::Mismatch)),
        Some((EmbeddedFormat::Raw, SOURCE, SourceCheck::NotVerified)),
        None,
    ];
    for (index, expected) in expected.into_iter().enumerate() {
        let row = u32::try_from(index + 1).unwrap();
        let document = pdb.document(DocumentId::new(row)).unwrap();
        let source = pdb.document_source(&document, SOURCE.len()).unwrap();
        let actual = source
            .as_ref()
            .map(|source| (source.format(), source.content(), source.check()));
        assert_eq!(actual, expected, "document {row}");
        if let Some(source) = source {
            assert_eq!(
                source.custom_debug_information(),
                CustomDebugInformationId::new(row)
            );
            assert_eq!(document.verify_source(source.content()), source.check());
            assert_eq!(source.into_content().len(), 3);
        }
    }
}

#[test]
fn checks_sources_from_other_places() {
    let bytes = sources().bytes();
    let pdb = Pdb::parse(&bytes).unwrap();
    let sha256 = pdb.document(DocumentId::new(1)).unwrap();
    assert_eq!(sha256.verify_source(SOURCE), SourceCheck::Verified);
    assert_eq!(sha256.verify_source(b""), SourceCheck::Mismatch);
    let sha1 = pdb.document(DocumentId::new(4)).unwrap();
    assert_eq!(sha1.verify_source(SOURCE), SourceCheck::NotVerified);
    let none = pdb.document(DocumentId::new(5)).unwrap();
    assert_eq!(none.verify_source(SOURCE), SourceCheck::NotVerified);
}

#[test]
fn applies_the_limit_to_raw_and_deflate_sources() {
    let bytes = sources().bytes();
    let pdb = Pdb::parse(&bytes).unwrap();
    for row in [1, 2] {
        let document = pdb.document(DocumentId::new(row)).unwrap();
        let error = pdb.document_source(&document, 2).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::LimitExceeded, "document {row}");
        let location = error.portable_location().unwrap();
        assert_eq!(location.stream(), Some(MetadataStream::Tables));
        assert_eq!(location.table(), Some(TableId::CUSTOM_DEBUG_INFORMATION));
        assert_eq!(location.row(), Some(row));
        assert_eq!(location.column(), Some("Value"));
        assert!(location.heap_offset().is_some());
    }
}

#[test]
fn rejects_bad_embedded_sources() {
    let mut pdb = Sources::new();
    let document = pdb.document(SHA256, &SOURCE_SHA256);
    // The DEFLATE stream gives fewer bytes than the declared size.
    pdb.custom(document, KIND_EMBEDDED_SOURCE, &blob(4, &SOURCE_DEFLATE));
    let short = pdb.document(SHA256, &SOURCE_SHA256);
    // The blob is shorter than its format field.
    pdb.custom(short, KIND_EMBEDDED_SOURCE, &[0, 0]);
    let bytes = pdb.bytes();
    let pdb = Pdb::parse(&bytes).unwrap();
    for (row, kind) in [(1, ErrorKind::InvalidFormat), (2, ErrorKind::UnexpectedEof)] {
        let document = pdb.document(DocumentId::new(row)).unwrap();
        let error = pdb.document_source(&document, 1024).unwrap_err();
        assert_eq!(error.kind(), kind, "document {row}");
        assert_eq!(error.portable_location().unwrap().row(), Some(row));
    }
}

#[test]
fn rejects_two_embedded_sources_for_one_document() {
    let mut pdb = Sources::new();
    let document = pdb.document(SHA256, &SOURCE_SHA256);
    pdb.custom(document, KIND_EMBEDDED_SOURCE, &blob(0, SOURCE));
    pdb.custom(document, KIND_EMBEDDED_SOURCE, &blob(0, SOURCE));
    let bytes = pdb.bytes();
    let pdb = Pdb::parse(&bytes).unwrap();
    let document = pdb.document(DocumentId::new(1)).unwrap();
    let error = pdb.document_source(&document, 1024).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(error.portable_location().unwrap().row(), Some(2));
}
