//! Document name blobs: decoding, the Roslyn split, encoding and the joined name.

use pdbrs::ErrorKind;
use pdbrs::portable::debug::{DocumentName, HashAlgorithm, Language, split_document_name};
use pdbrs::portable::metadata::{BlobOffset, DocumentId};

use crate::synthetic::Synthetic;
use crate::{every_prefix_fails, expect};

#[test]
fn decode_gives_the_separator_and_the_part_offsets() {
    let blob = [b'/', 0x00, 0x05, 0x81, 0x02];
    let name = DocumentName::decode(&blob).unwrap();
    assert_eq!(name.separator(), Some('/'));
    let parts: Vec<u32> = name.parts().map(BlobOffset::get).collect();
    assert_eq!(parts, [0, 5, 0x102]);
}

#[test]
fn decode_accepts_the_empty_separator_and_a_multibyte_separator() {
    let empty = DocumentName::decode(&[0, 3]).unwrap();
    assert_eq!(empty.separator(), None);
    let blob = [0xc2, 0xa7, 1, 2];
    let name = DocumentName::decode(&blob).unwrap();
    assert_eq!(name.separator(), Some('\u{a7}'));
    assert_eq!(
        DocumentName::encode(Some('\u{a7}'), &[BlobOffset::new(1), BlobOffset::new(2)]).unwrap(),
        blob
    );
}

#[test]
fn decode_rejects_bad_separators_and_missing_parts() {
    expect(
        &DocumentName::decode(&[]).unwrap_err(),
        ErrorKind::UnexpectedEof,
        "no separator",
    );
    expect(
        &DocumentName::decode(b"/").unwrap_err(),
        ErrorKind::UnexpectedEof,
        "no part",
    );
    expect(
        &DocumentName::decode(&[0xff, 1]).unwrap_err(),
        ErrorKind::InvalidFormat,
        "not a UTF-8 character",
    );
    expect(
        &DocumentName::decode(&[0xc2, 0x41, 1]).unwrap_err(),
        ErrorKind::InvalidFormat,
        "not a UTF-8 character",
    );
    every_prefix_fails(&[b'\\', 0x81, 0x02], |blob| {
        DocumentName::decode(blob).map(|_| ())
    });
}

#[test]
fn split_uses_the_more_frequent_separator_like_roslyn() {
    let split = split_document_name(br"C:\src\dir/a.cs");
    assert_eq!(split.separator(), '\\');
    let parts: Vec<&[u8]> = split.collect();
    assert_eq!(parts, [&b"C:"[..], b"src", b"dir/a.cs"]);

    let split = split_document_name(b"/home/a.cs");
    assert_eq!(split.separator(), '/');
    assert_eq!(split.collect::<Vec<_>>(), [&b""[..], b"home", b"a.cs"]);

    // A tie and a name without separators use `/` and give one part.
    let split = split_document_name(b"a.cs");
    assert_eq!(split.separator(), '/');
    assert_eq!(split.collect::<Vec<_>>(), [&b"a.cs"[..]]);
}

#[test]
fn encode_and_decode_round_trip() {
    let parts = [
        BlobOffset::new(0),
        BlobOffset::new(0x7f),
        BlobOffset::new(0x4000),
    ];
    let blob = DocumentName::encode(Some('/'), &parts).unwrap();
    assert_eq!(blob, [b'/', 0x00, 0x7f, 0xc0, 0x00, 0x40, 0x00]);
    let name = DocumentName::decode(&blob).unwrap();
    assert_eq!(name.parts().collect::<Vec<_>>(), parts);
    expect(
        &DocumentName::encode(Some('/'), &[]).unwrap_err(),
        ErrorKind::InvalidFormat,
        "no part",
    );
}

#[test]
fn pdb_joins_the_parts_of_each_document_name() {
    let synthetic = Synthetic::valid();
    let bytes = synthetic.bytes();
    let pdb = pdbrs::portable::Pdb::parse(&bytes).unwrap();
    let names: Vec<Vec<u8>> = pdb
        .documents()
        .map(|document| document.unwrap().name().to_vec())
        .collect();
    assert_eq!(names, [b"/src/a.cs".to_vec(), b"/src/b.cs".to_vec()]);
    let document = pdb.document(DocumentId::new(1)).unwrap();
    assert_eq!(document.language(), Some(Language::CSharp));
    assert_eq!(document.hash_algorithm(), Some(HashAlgorithm::Sha256));
    assert_eq!(document.hash().len(), 32);
}

#[test]
fn a_bad_name_blob_has_the_location_of_its_row() {
    let mut synthetic = Synthetic::valid();
    let bad = synthetic.blob(&[0xff, 1]);
    synthetic.documents[1][0] = bad;
    let bytes = synthetic.bytes();
    let pdb = pdbrs::portable::Pdb::parse(&bytes).unwrap();
    let error = pdb.document(DocumentId::new(2)).unwrap_err();
    expect(&error, ErrorKind::InvalidFormat, "UTF-8");
    let location = error.portable_location().unwrap();
    assert_eq!(location.row(), Some(2));
    assert_eq!(location.column(), Some("Name"));
    assert_eq!(location.heap_offset(), Some(bad));
}

#[test]
fn known_guids_map_to_languages_and_hash_algorithms() {
    for language in [Language::CSharp, Language::VisualBasic, Language::FSharp] {
        assert_eq!(Language::from_guid(&language.guid()), Some(language));
    }
    let lengths: Vec<usize> = [
        HashAlgorithm::Sha1,
        HashAlgorithm::Sha256,
        HashAlgorithm::Sha384,
        HashAlgorithm::Sha512,
    ]
    .into_iter()
    .map(|algorithm| {
        assert_eq!(HashAlgorithm::from_guid(&algorithm.guid()), Some(algorithm));
        algorithm.hash_len()
    })
    .collect();
    assert_eq!(lengths, [20, 32, 48, 64]);
    // 3f5162f8-07c6-11d3-9053-00c04fa302a1 in serialized order.
    assert_eq!(
        Language::CSharp.guid(),
        [
            0xf8, 0x62, 0x51, 0x3f, 0xc6, 0x07, 0xd3, 0x11, 0x90, 0x53, 0x00, 0xc0, 0x4f, 0xa3,
            0x02, 0xa1
        ]
    );
    assert_eq!(Language::from_guid(&[0; 16]), None);
}
