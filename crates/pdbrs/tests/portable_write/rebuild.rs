//! Exact copies and the rebuild of an existing Portable PDB with `Pdb::to_parts`.

use pdbrs::ErrorKind;
use pdbrs::portable::Pdb;
use pdbrs::portable::metadata::{
    CustomDebugInformationRow, LocalVariableRow, MetadataStream, MethodDefId, TableId,
};
use pdbrs::portable::write::IdPolicy;

use crate::builder::{Builder, row, value};
use crate::{location, open};

/// The `Document` table number.
const DOCUMENT: u8 = 0x30;
/// The `LocalVariable` table number.
const LOCAL_VARIABLE: u8 = 0x33;
/// The `CustomDebugInformation` table number.
const CUSTOM_DEBUG_INFORMATION: u8 = 0x37;

/// A source whose `#Blob` heap has the blob "a" twice, at offsets 1 and 3, and whose
/// `#Strings` heap is not in the writer order.
fn source() -> Builder {
    let mut builder = Builder {
        strings: Some(b"\0zeta\0alpha\0".to_vec()),
        blobs: Some(vec![0, 1, b'a', 1, b'a']),
        ..Builder::default()
    };
    // Parent MethodDef row 2 (tag 0), kind GUID 2, value at offset 3.
    let information = row(&[value(2, 2 << 5), value(2, 2), value(2, 3)]);
    builder = builder.table(CUSTOM_DEBUG_INFORMATION, 1, information);
    let variables = [
        row(&[value(2, 0), value(2, 1), value(2, 1)]),
        row(&[value(2, 1), value(2, 0), value(2, 6)]),
    ]
    .concat();
    builder.table(LOCAL_VARIABLE, 2, variables)
}

#[test]
fn exact_copy_keeps_every_byte() {
    let bytes = source().build();
    let pdb = Pdb::parse(&bytes).unwrap();
    let mut copy = Vec::new();
    assert_eq!(
        pdb.copy_exact(&mut copy).unwrap(),
        u64::try_from(bytes.len()).unwrap()
    );
    assert_eq!(copy, bytes);
}

#[test]
fn rebuild_keeps_rows_and_values() {
    let bytes = source().build();
    let pdb = Pdb::parse(&bytes).unwrap();
    let parts = pdb.to_parts().unwrap();
    // The duplicate blob gets the offset of its first copy.
    assert_eq!(parts.tables.custom_debug_information[0].value.get(), 1);
    let output = parts
        .plan(IdPolicy::ContentDerived)
        .unwrap()
        .to_vec()
        .unwrap();
    let rebuilt = open(&output);
    assert_eq!(
        rebuilt.pdb_stream().entry_point(),
        Some(MethodDefId::new(1))
    );
    assert_eq!(
        rebuilt.pdb_stream().external_row_count(TableId::METHOD_DEF),
        3
    );
    assert_eq!(rebuilt.pdb_stream().external_row_count(TableId::MODULE), 1);
    let names: Vec<Vec<u8>> = rebuilt
        .rows::<LocalVariableRow>()
        .map(|row| rebuilt.string(row.unwrap().1.name).unwrap())
        .collect();
    assert_eq!(names, [b"zeta".to_vec(), b"alpha".to_vec()]);
    let (_, information) = rebuilt
        .rows::<CustomDebugInformationRow>()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(rebuilt.blob(information.value).unwrap(), b"a");
    assert_eq!(information.parent.row(), 2);
    let original = pdb.guid(
        pdb.rows::<CustomDebugInformationRow>()
            .next()
            .unwrap()
            .unwrap()
            .1
            .kind,
    );
    assert_eq!(rebuilt.guid(information.kind).unwrap(), original.unwrap());

    // A rebuild of the rebuilt file gives the same bytes.
    let again = rebuilt
        .to_parts()
        .and_then(|parts| parts.plan(IdPolicy::ContentDerived))
        .and_then(|plan| plan.to_vec())
        .unwrap();
    assert_eq!(again, output);
}

#[test]
fn rebuild_fails_if_a_document_blob_offset_could_change() {
    // Name at offset 3, the duplicate copy of "a".
    let document = row(&[value(2, 3), value(2, 0), value(2, 0), value(2, 0)]);
    let bytes = source().table(DOCUMENT, 1, document).build();
    let error = Pdb::parse(&bytes).unwrap().to_parts().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
    assert_eq!(location(&error).stream(), Some(MetadataStream::Blob));
}
