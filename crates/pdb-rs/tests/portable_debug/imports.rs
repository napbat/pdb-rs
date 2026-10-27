//! Import blobs: every import kind, unknown kinds, truncation and the encoder.

use pdb_rs::ErrorKind;
use pdb_rs::portable::debug::{Imports, RawImport};
use pdb_rs::portable::metadata::{AssemblyRefId, BlobOffset, MetadataToken, TableId};

use crate::{encoded, every_prefix_fails, expect};

/// One import of each kind and its encoding.
fn every_kind() -> [(RawImport, &'static [u8]); 9] {
    let blob = BlobOffset;
    let assembly = AssemblyRefId(2);
    let target_type = MetadataToken::new(TableId::TYPE_REF, 7).unwrap();
    [
        (RawImport::Namespace { namespace: blob(5) }, &[0x01, 0x05]),
        (
            RawImport::AssemblyNamespace {
                assembly,
                namespace: blob(5),
            },
            &[0x02, 0x02, 0x05],
        ),
        (RawImport::Type { target_type }, &[0x03, 0x1d]),
        (
            RawImport::XmlNamespace {
                alias: blob(0),
                namespace: blob(0x80),
            },
            &[0x04, 0x00, 0x80, 0x80],
        ),
        (
            RawImport::AssemblyReferenceAlias { alias: blob(9) },
            &[0x05, 0x09],
        ),
        (
            RawImport::AliasAssemblyReference {
                alias: blob(9),
                assembly,
            },
            &[0x06, 0x09, 0x02],
        ),
        (
            RawImport::AliasNamespace {
                alias: blob(9),
                namespace: blob(5),
            },
            &[0x07, 0x09, 0x05],
        ),
        (
            RawImport::AliasAssemblyNamespace {
                alias: blob(9),
                assembly,
                namespace: blob(5),
            },
            &[0x08, 0x09, 0x02, 0x05],
        ),
        (
            RawImport::AliasType {
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
        let decoded: Vec<RawImport> = Imports::parse(bytes).collect::<Result<_, _>>().unwrap();
        assert_eq!(decoded, [import]);
        assert_eq!(
            encoded(|out| Imports::encode_parts(&[import], out)).unwrap(),
            bytes
        );
        // The empty prefix is a valid blob without imports, so require exactly one import.
        every_prefix_fails(bytes, |blob| {
            let imports = Imports::parse(blob).collect::<pdb_rs::Result<Vec<_>>>()?;
            if imports.len() == 1 {
                Ok(())
            } else {
                Err(pdb_rs::Error::new(ErrorKind::UnexpectedEof, None))
            }
        });
    }
    let all: Vec<RawImport> = every_kind().into_iter().map(|(import, _)| import).collect();
    let bytes: Vec<u8> = every_kind()
        .into_iter()
        .flat_map(|(_, bytes)| bytes.to_vec())
        .collect();
    assert_eq!(
        encoded(|out| Imports::encode_parts(&all, out)).unwrap(),
        bytes
    );
    let decoded: Vec<RawImport> = Imports::parse(&bytes).collect::<Result<_, _>>().unwrap();
    assert_eq!(decoded, all);
}

#[test]
fn accessors_give_the_fields_of_each_kind() {
    let [namespace, _, type_import, xml, ..] = every_kind().map(|(import, _)| import);
    assert_eq!(namespace.namespace(), Some(BlobOffset(5)));
    assert_eq!(namespace.alias(), None);
    assert_eq!(namespace.assembly(), None);
    assert_eq!(type_import.target_type().map(MetadataToken::row), Some(7));
    assert_eq!(xml.alias(), Some(BlobOffset(0)));
    assert_eq!(xml.namespace(), Some(BlobOffset(0x80)));
}

#[test]
fn an_unknown_kind_stops_its_blob() {
    // A namespace import, an import of kind 10, then a namespace import.
    let mut imports = Imports::parse(&[0x01, 0x05, 0x0a, 0x01, 0x01, 0x05]);
    assert!(imports.next().unwrap().is_ok());
    expect(
        &imports.next().unwrap().unwrap_err(),
        ErrorKind::InvalidFormat,
        "unknown kind",
    );
    assert!(imports.next().is_none());
    for kind in [0x00, 0x0a, 0x7f] {
        assert!(Imports::parse(&[kind, 0x01]).next().unwrap().is_err());
    }
}

#[test]
fn invalid_targets_give_errors() {
    expect(
        &Imports::parse(&[0x02, 0x00, 0x05])
            .next()
            .unwrap()
            .unwrap_err(),
        ErrorKind::InvalidFormat,
        "assembly is nil",
    );
    expect(
        &Imports::parse(&[0x03, 0x03]).next().unwrap().unwrap_err(),
        ErrorKind::InvalidFormat,
        "tag 3",
    );
    let nil = RawImport::AliasAssemblyReference {
        alias: BlobOffset(1),
        assembly: AssemblyRefId(0),
    };
    expect(
        &encoded(|out| Imports::encode_parts(&[nil], out)).unwrap_err(),
        ErrorKind::InvalidFormat,
        "assembly is nil",
    );
    assert!(Imports::parse(&[]).next().is_none());
}
