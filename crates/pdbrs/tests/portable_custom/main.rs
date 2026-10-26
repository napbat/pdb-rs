//! Tests for the custom debug information codecs of Portable PDBs.

#[path = "../portable_metadata/builder.rs"]
mod builder;
mod compilation;
mod enc;
mod locals;
mod rows;
mod sources;
mod state_machine;

use pdbrs::ErrorKind;
use pdbrs::portable::custom::{self, CustomDebugKind, CustomDebugValue, TypeDefinitionDocuments};
use pdbrs::portable::embedded::EmbeddedSource;
use pdbrs::portable::metadata::DocumentId;

/// Decode `blob` as `kind` and check that the value encodes to `blob` again.
fn round_trip(kind: CustomDebugKind, blob: &[u8]) -> CustomDebugValue<'_> {
    let value = custom::decode(kind, blob).unwrap();
    assert_eq!(value.kind(), kind);
    assert_eq!(value.encode().unwrap(), blob, "{kind}");
    value
}

/// Return the error kind of decoding `blob` as `kind`.
fn error(kind: CustomDebugKind, blob: &[u8]) -> ErrorKind {
    custom::decode(kind, blob).unwrap_err().kind()
}

#[test]
fn kind_text_matches_roslyn() {
    for (kind, text) in [
        (
            CustomDebugKind::ASYNC_METHOD_STEPPING_INFORMATION,
            "54fd2ac5-e925-401a-9c2a-f94f171072f8",
        ),
        (
            CustomDebugKind::STATE_MACHINE_HOISTED_LOCAL_SCOPES,
            "6da9a61e-f8c7-4874-be62-68bc5630df71",
        ),
        (
            CustomDebugKind::DYNAMIC_LOCAL_VARIABLES,
            "83c563c4-b4f3-47d5-b824-ba5441477ea8",
        ),
        (
            CustomDebugKind::TUPLE_ELEMENT_NAMES,
            "ed9fdf71-8879-4747-8ed3-fe5ede3ce710",
        ),
        (
            CustomDebugKind::DEFAULT_NAMESPACE,
            "58b2eab6-209f-4e4e-a22c-b2d0f910c782",
        ),
        (
            CustomDebugKind::ENC_LOCAL_SLOT_MAP,
            "755f52a8-91c5-45be-b4b8-209571e552bd",
        ),
        (
            CustomDebugKind::ENC_LAMBDA_AND_CLOSURE_MAP,
            "a643004c-0240-496f-a783-30d64f4979de",
        ),
        (
            CustomDebugKind::ENC_STATE_MACHINE_STATE_MAP,
            "8b78cd68-2ede-420b-980b-e15884b8aaa3",
        ),
        (
            CustomDebugKind::SOURCE_LINK,
            "cc110556-a091-4d38-9fec-25ab9a351a6a",
        ),
        (
            CustomDebugKind::EMBEDDED_SOURCE,
            "0e8a571b-6926-466e-b4ad-8ab04611f5fe",
        ),
        (
            CustomDebugKind::COMPILATION_METADATA_REFERENCES,
            "7e4d4708-096e-4c5c-aeda-cb10ba6a740d",
        ),
        (
            CustomDebugKind::COMPILATION_OPTIONS,
            "b5feec05-8cd0-4a83-96da-466284bb4bd8",
        ),
        (
            CustomDebugKind::TYPE_DEFINITION_DOCUMENTS,
            "932e74bc-dba9-4478-8d46-0f32a7bab3d3",
        ),
        (
            CustomDebugKind::PRIMARY_CONSTRUCTOR_INFORMATION,
            "9d40ace1-c703-4d0e-bf41-7243060a8fb5",
        ),
    ] {
        assert_eq!(kind.to_string(), text);
    }
    // The `#GUID` heap stores the first three fields little-endian.
    assert_eq!(
        CustomDebugKind::EMBEDDED_SOURCE.as_bytes(),
        &[
            0x1b, 0x57, 0x8a, 0x0e, 0x26, 0x69, 0x6e, 0x46, 0xb4, 0xad, 0x8a, 0xb0, 0x46, 0x11,
            0xf5, 0xfe
        ]
    );
}

#[test]
fn keeps_unknown_kinds_raw() {
    let kind = CustomDebugKind([7; 16]);
    for blob in [&[][..], &[0xff, 0xe0, 0x00], b"any bytes"] {
        let value = round_trip(kind, blob);
        assert_eq!(value, CustomDebugValue::Unknown { kind, value: blob });
    }
}

#[test]
fn keeps_text_as_bytes() {
    let json = br#"{"documents":{"C:\\src\\*":"https://example.com/*"}}"#;
    let CustomDebugValue::SourceLink(text) = round_trip(CustomDebugKind::SOURCE_LINK, json) else {
        panic!("expected Source Link");
    };
    assert_eq!(text.as_bytes(), json);
    assert_eq!(text.to_str().unwrap().as_bytes(), json);

    let CustomDebugValue::DefaultNamespace(name) =
        round_trip(CustomDebugKind::DEFAULT_NAMESPACE, b"Demo.App")
    else {
        panic!("expected a default namespace");
    };
    assert_eq!(name.to_str().unwrap(), "Demo.App");

    // The decoder keeps bytes that are not UTF-8. Only the text accessor fails.
    let CustomDebugValue::DefaultNamespace(name) =
        round_trip(CustomDebugKind::DEFAULT_NAMESPACE, &[0x41, 0xff])
    else {
        panic!("expected a default namespace");
    };
    assert!(name.to_str().is_err());
    assert!(
        matches!(round_trip(CustomDebugKind::SOURCE_LINK, &[]), CustomDebugValue::SourceLink(text) if text.is_empty())
    );
}

#[test]
fn decodes_embedded_source_through_the_shared_codec() {
    let raw = [0, 0, 0, 0, b'a', b'b', b'c'];
    assert_eq!(
        round_trip(CustomDebugKind::EMBEDDED_SOURCE, &raw),
        CustomDebugValue::EmbeddedSource(EmbeddedSource::Raw(b"abc"))
    );
    let deflate = [3, 0, 0, 0, 0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c'];
    let CustomDebugValue::EmbeddedSource(source) =
        round_trip(CustomDebugKind::EMBEDDED_SOURCE, &deflate)
    else {
        panic!("expected an embedded source");
    };
    assert_eq!(source.decode(16).unwrap().as_ref(), b"abc");
    assert_eq!(
        error(CustomDebugKind::EMBEDDED_SOURCE, &[0, 0, 0]),
        ErrorKind::UnexpectedEof
    );
    assert_eq!(
        error(CustomDebugKind::EMBEDDED_SOURCE, &[0xff, 0xff, 0xff, 0xff]),
        ErrorKind::UnsupportedVersion
    );
}

#[test]
fn decodes_primary_constructor_information() {
    assert_eq!(
        round_trip(CustomDebugKind::PRIMARY_CONSTRUCTOR_INFORMATION, &[]),
        CustomDebugValue::PrimaryConstructorInformation
    );
    assert_eq!(
        error(CustomDebugKind::PRIMARY_CONSTRUCTOR_INFORMATION, &[0]),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn decodes_type_definition_documents() {
    // Row IDs 1, 128 (two bytes) and 0x4000 (four bytes).
    let blob = [0x01, 0x80, 0x80, 0xc0, 0x00, 0x40, 0x00];
    let CustomDebugValue::TypeDefinitionDocuments(documents) =
        round_trip(CustomDebugKind::TYPE_DEFINITION_DOCUMENTS, &blob)
    else {
        panic!("expected type definition documents");
    };
    assert_eq!(documents.len(), 3);
    assert_eq!(
        documents.iter().map(DocumentId::get).collect::<Vec<_>>(),
        [1, 128, 0x4000]
    );
    assert_eq!(documents.as_bytes(), blob);

    assert!(TypeDefinitionDocuments::parse(&[]).unwrap().is_empty());
    let kind = CustomDebugKind::TYPE_DEFINITION_DOCUMENTS;
    assert_eq!(error(kind, &[0x01, 0x00]), ErrorKind::InvalidFormat);
    assert_eq!(error(kind, &[0x01, 0x80]), ErrorKind::UnexpectedEof);
    assert_eq!(error(kind, &[0xc0, 0x00, 0x40]), ErrorKind::UnexpectedEof);
    assert_eq!(error(kind, &[0xe0]), ErrorKind::InvalidFormat);

    assert_eq!(
        TypeDefinitionDocuments::encode([DocumentId::new(0x1fff_ffff)]).unwrap(),
        [0xdf, 0xff, 0xff, 0xff]
    );
    assert_eq!(
        TypeDefinitionDocuments::encode([DocumentId::new(0x2000_0000)])
            .unwrap_err()
            .kind(),
        ErrorKind::LimitExceeded
    );
    assert_eq!(
        TypeDefinitionDocuments::encode([DocumentId::new(1), DocumentId::default()])
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn reads_values_through_into_iterator() {
    let blob = [0x01, 0x02];
    let documents = TypeDefinitionDocuments::parse(&blob).unwrap();
    let mut rows = Vec::new();
    for row in &documents {
        rows.push(row.get());
    }
    assert_eq!(rows, [1, 2]);
    let mut iter = documents.into_iter();
    assert_eq!(iter.len(), 2);
    iter.next();
    assert_eq!(iter.size_hint(), (1, Some(1)));
    iter.next();
    assert_eq!(iter.next(), None);
    assert_eq!(iter.next(), None);
}
