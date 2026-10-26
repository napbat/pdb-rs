//! Exact copies and the rebuild of an existing Portable PDB with `Pdb::to_parts`.

use pdbrs::io::copy_exact;
use pdbrs::portable::debug::Import;
use pdbrs::portable::metadata::{
    CustomDebugInformationRow, DocumentId, ImportScopeId, LocalVariableRow, MethodDefId, TableId,
};
use pdbrs::portable::{IdentityPolicy, Pdb};

use crate::builder::{Builder, row, value};
use crate::{location, open};

/// The `Document` table number.
const DOCUMENT: u8 = 0x30;
/// The `LocalVariable` table number.
const LOCAL_VARIABLE: u8 = 0x33;
/// The `ImportScope` table number.
const IMPORT_SCOPE: u8 = 0x35;
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
        copy_exact(pdb.source(), &mut copy).unwrap(),
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
        .plan(IdentityPolicy::ContentDerived)
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
        .and_then(|parts| parts.plan(IdentityPolicy::ContentDerived))
        .and_then(|plan| plan.to_vec())
        .unwrap();
    assert_eq!(again, output);
}

/// A source whose `#Blob` heap has "x" twice (offsets 1 and 3), a document name `x/x` at
/// offset 5 that uses both copies and an imports blob at offset 9 with the namespace at
/// offset 3.
fn duplicate_parts() -> Builder {
    let builder = Builder {
        blobs: Some(vec![0, 1, b'x', 1, b'x', 3, b'/', 1, 3, 2, 1, 3]),
        ..Builder::default()
    };
    let document = row(&[value(2, 5), value(2, 0), value(2, 0), value(2, 0)]);
    let import_scope = row(&[value(2, 0), value(2, 9)]);
    builder
        .table(DOCUMENT, 1, document)
        .table(IMPORT_SCOPE, 1, import_scope)
}

#[test]
fn rebuild_rewrites_document_names_and_imports_if_blob_offsets_change() {
    let bytes = duplicate_parts().build();
    let pdb = Pdb::parse(&bytes).unwrap();
    assert_eq!(pdb.document(DocumentId::new(1)).unwrap().name(), b"x/x");
    let parts = pdb.to_parts().unwrap();
    // The duplicate "x" is gone, so the name and the imports move. The rebuild adds the
    // encoded name ("/", 1, 1) and imports (1, 1) with the new offset of "x".
    assert_ne!(parts.tables.documents[0].name.get(), 5);
    let output = parts
        .plan(IdentityPolicy::ContentDerived)
        .unwrap()
        .to_vec()
        .unwrap();
    let rebuilt = open(&output);
    let document = rebuilt.document(DocumentId::new(1)).unwrap();
    assert_eq!(document.name(), b"x/x");
    let scope = rebuilt.import_scope(ImportScopeId::new(1)).unwrap();
    let imports: Vec<Import> = scope.imports().map(Result::unwrap).collect();
    let [Import::Namespace { namespace }] = imports[..] else {
        panic!("unexpected imports {imports:?}");
    };
    assert_eq!(namespace.get(), 1);
    assert_eq!(rebuilt.blob(namespace).unwrap(), b"x");
}

#[test]
fn rebuild_fails_if_a_moved_document_name_does_not_decode() {
    // The document name is the duplicate blob "x": its separator has no part.
    let document = row(&[value(2, 3), value(2, 0), value(2, 0), value(2, 0)]);
    let bytes = Builder {
        blobs: Some(vec![0, 1, b'x', 1, b'x']),
        ..Builder::default()
    }
    .table(DOCUMENT, 1, document)
    .build();
    let error = Pdb::parse(&bytes).unwrap().to_parts().unwrap_err();
    assert_eq!(location(&error).table(), Some(TableId::DOCUMENT));
    assert_eq!(location(&error).column(), Some("Name"));
}
