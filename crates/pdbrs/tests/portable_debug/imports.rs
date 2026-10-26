//! Import blobs: every import kind, unknown kinds, truncation and the encoder.

use pdbrs::ErrorKind;
use pdbrs::portable::debug::{Import, Imports, encode_imports};
use pdbrs::portable::metadata::{AssemblyRefId, BlobOffset, MetadataToken, TableId};

use crate::{every_prefix_fails, expect};

/// One import of each kind and its encoding.
fn every_kind() -> [(Import, &'static [u8]); 9] {
    let blob = BlobOffset::new;
    let assembly = AssemblyRefId::new(2);
    let target_type = MetadataToken::new(TableId::TYPE_REF, 7).unwrap();
    [
        (Import::Namespace { namespace: blob(5) }, &[0x01, 0x05]),
        (
            Import::AssemblyNamespace {
                assembly,
                namespace: blob(5),
            },
            &[0x02, 0x02, 0x05],
        ),
        (Import::Type { target_type }, &[0x03, 0x1d]),
        (
            Import::XmlNamespace {
                alias: blob(0),
                namespace: blob(0x80),
            },
            &[0x04, 0x00, 0x80, 0x80],
        ),
        (
            Import::AssemblyReferenceAlias { alias: blob(9) },
            &[0x05, 0x09],
        ),
        (
            Import::AliasAssemblyReference {
                alias: blob(9),
                assembly,
            },
            &[0x06, 0x09, 0x02],
        ),
        (
            Import::AliasNamespace {
                alias: blob(9),
                namespace: blob(5),
            },
            &[0x07, 0x09, 0x05],
        ),
        (
            Import::AliasAssemblyNamespace {
                alias: blob(9),
                assembly,
                namespace: blob(5),
            },
            &[0x08, 0x09, 0x02, 0x05],
        ),
        (
            Import::AliasType {
                alias: blob(9),
                target_type,
            },
            &[0x09, 0x09, 0x1d],
        ),
    ]
}

#[test]
fn every_import_kind_round_trips() {
    for (index, (import, bytes)) in every_kind().into_iter().enumerate() {
        assert_eq!(import.kind(), u32::try_from(index).unwrap() + 1);
        let decoded: Vec<Import> = Imports::decode(bytes).collect::<Result<_, _>>().unwrap();
        assert_eq!(decoded, [import]);
        assert_eq!(encode_imports(&[import]).unwrap(), bytes);
        // The empty prefix is a valid blob without imports, so require exactly one import.
        every_prefix_fails(bytes, |blob| {
            let imports = Imports::decode(blob).collect::<pdbrs::Result<Vec<_>>>()?;
            if imports.len() == 1 {
                Ok(())
            } else {
                Err(pdbrs::Error::new(ErrorKind::UnexpectedEof, None))
            }
        });
    }
    let all: Vec<Import> = every_kind().into_iter().map(|(import, _)| import).collect();
    let bytes: Vec<u8> = every_kind()
        .into_iter()
        .flat_map(|(_, bytes)| bytes.to_vec())
        .collect();
    assert_eq!(encode_imports(&all).unwrap(), bytes);
    let decoded: Vec<Import> = Imports::decode(&bytes).collect::<Result<_, _>>().unwrap();
    assert_eq!(decoded, all);
}

#[test]
fn accessors_give_the_fields_of_each_kind() {
    let [namespace, _, type_import, xml, ..] = every_kind().map(|(import, _)| import);
    assert_eq!(namespace.namespace(), Some(BlobOffset::new(5)));
    assert_eq!(namespace.alias(), None);
    assert_eq!(namespace.assembly(), None);
    assert_eq!(type_import.target_type().map(MetadataToken::row), Some(7));
    assert_eq!(xml.alias(), Some(BlobOffset::new(0)));
    assert_eq!(xml.namespace(), Some(BlobOffset::new(0x80)));
}

#[test]
fn an_unknown_kind_stops_its_blob() {
    // A namespace import, an import of kind 10, then a namespace import.
    let mut imports = Imports::decode(&[0x01, 0x05, 0x0a, 0x01, 0x01, 0x05]);
    assert!(imports.next().unwrap().is_ok());
    expect(
        &imports.next().unwrap().unwrap_err(),
        ErrorKind::InvalidFormat,
        "unknown kind",
    );
    assert!(imports.next().is_none());
    for kind in [0x00, 0x0a, 0x7f] {
        assert!(Imports::decode(&[kind, 0x01]).next().unwrap().is_err());
    }
}

#[test]
fn invalid_targets_give_errors() {
    expect(
        &Imports::decode(&[0x02, 0x00, 0x05])
            .next()
            .unwrap()
            .unwrap_err(),
        ErrorKind::InvalidFormat,
        "assembly is nil",
    );
    expect(
        &Imports::decode(&[0x03, 0x03]).next().unwrap().unwrap_err(),
        ErrorKind::InvalidFormat,
        "tag 3",
    );
    let nil = Import::AliasAssemblyReference {
        alias: BlobOffset::new(1),
        assembly: AssemblyRefId::new(0),
    };
    expect(
        &encode_imports(&[nil]).unwrap_err(),
        ErrorKind::InvalidFormat,
        "assembly is nil",
    );
    assert!(Imports::decode(&[]).next().is_none());
}
