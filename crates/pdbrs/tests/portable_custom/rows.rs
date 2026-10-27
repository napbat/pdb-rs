//! Tests for the `CustomDebugInformation` rows of a synthetic Portable PDB.

use pdbrs::portable::Pdb;
use pdbrs::portable::custom::{CustomDebugKind, CustomDebugParent, CustomDebugValue};
use pdbrs::portable::metadata::{
    CustomDebugInformationId, CustomDebugInformationRow, MetadataToken, TableId,
};
use pdbrs::{Bytes, ErrorKind};

use super::builder::{Builder, row, value};

/// The `CustomDebugInformation` table number.
const CUSTOM_DEBUG_INFORMATION: u8 = 0x37;

/// A `CustomDebugInformation` row with 2-byte columns.
fn cdi_row(parent: u32, kind: u32, blob: u32) -> Vec<u8> {
    row(&[value(2, parent), value(2, kind), value(2, blob)])
}

/// A Portable PDB with four `CustomDebugInformation` rows:
///
/// 1. `MethodDef` 1, the default namespace `abc`.
/// 2. `Module` 1, type definition documents with the nil row 0 (malformed).
/// 3. `Module` 1, an unknown kind with the blob `abc`.
/// 4. `Module` 1, the nil kind with an empty blob.
fn pdb_bytes() -> Vec<u8> {
    let mut guids = CustomDebugKind::DEFAULT_NAMESPACE.0.to_vec();
    guids.extend_from_slice(&CustomDebugKind::TYPE_DEFINITION_DOCUMENTS.0);
    guids.extend([0x55; 16]);
    let rows = [
        cdi_row(1 << 5, 1, 1),
        cdi_row((1 << 5) | 7, 2, 5),
        cdi_row((1 << 5) | 7, 3, 1),
        cdi_row((1 << 5) | 7, 0, 0),
    ]
    .concat();
    Builder {
        guids: Some(guids),
        blobs: Some(vec![0, 3, b'a', b'b', b'c', 2, 0x01, 0x00]),
        ..Builder::default()
    }
    .table(CUSTOM_DEBUG_INFORMATION, 4, rows)
    .build()
}

#[test]
fn reads_and_decodes_custom_debug_information_rows() {
    let bytes = pdb_bytes();
    let pdb = Pdb::parse(&bytes).unwrap();
    let rows = pdb.custom_debug_informations();
    assert_eq!(rows.len(), 4);
    let rows: Vec<_> = rows.collect();
    let raw: Vec<CustomDebugInformationRow> = pdb
        .rows::<CustomDebugInformationRow>()
        .map(|row| row.unwrap().1)
        .collect();

    let (id, first) = rows[0].as_ref().unwrap();
    assert_eq!(*id, CustomDebugInformationId(1));
    assert_eq!(
        first.parent,
        CustomDebugParent::Metadata(MetadataToken::new(TableId::METHOD_DEF, 1).unwrap())
    );
    assert_eq!(first.kind, CustomDebugKind::DEFAULT_NAMESPACE);
    assert_eq!(raw[0].kind.0, 1);
    assert_eq!(raw[0].value.0, 1);
    assert_eq!(first.value, b"abc");
    let CustomDebugValue::DefaultNamespace(name) = first.decode().unwrap() else {
        panic!("expected a default namespace");
    };
    assert_eq!(name.as_bytes(), b"abc");

    // A malformed value gives an error without a location. The raw row has the blob offset.
    let (_, second) = rows[1].as_ref().unwrap();
    assert_eq!(
        second.parent,
        CustomDebugParent::Metadata(MetadataToken::new(TableId::MODULE, 1).unwrap())
    );
    let error = second.decode().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert!(error.portable_location().is_none());
    assert_eq!(raw[1].value.0, 5);

    // Unknown kinds and the nil kind keep their raw bytes.
    let (_, third) = rows[2].as_ref().unwrap();
    assert_eq!(
        third.decode().unwrap(),
        CustomDebugValue::Unknown {
            kind: CustomDebugKind([0x55; 16]),
            value: Bytes::new(b"abc")
        }
    );
    let (_, fourth) = rows[3].as_ref().unwrap();
    assert_eq!(fourth.kind, CustomDebugKind([0; 16]));
    assert_eq!(
        fourth.decode().unwrap(),
        CustomDebugValue::Unknown {
            kind: CustomDebugKind([0; 16]),
            value: Bytes::new(&[])
        }
    );
}

#[test]
fn reports_row_errors_with_their_column() {
    let bytes = pdb_bytes();
    let pdb = Pdb::parse(&bytes).unwrap();
    let error = pdb
        .custom_debug_information(CustomDebugInformationId(5))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);

    // A GUID index after the `#GUID` heap.
    let bytes = Builder::default()
        .table(CUSTOM_DEBUG_INFORMATION, 1, cdi_row(1 << 5, 9, 1))
        .build();
    let pdb = Pdb::parse(&bytes).unwrap();
    let error = pdb.custom_debug_informations().next().unwrap().unwrap_err();
    let location = error.portable_location().unwrap();
    assert_eq!(location.row(), Some(1));
    assert_eq!(location.column(), Some("Kind"));
}
