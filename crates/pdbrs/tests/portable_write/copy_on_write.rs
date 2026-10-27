//! Copy-on-write edits (#49): the plan keeps the raw rows of a table that no `*_mut` call
//! decoded, and encodes them with the index widths of the output.

use pdbrs::portable::custom::{CustomDebugKind, CustomDebugParent};
use pdbrs::portable::debug::{DocumentName, ValidateOptions};
use pdbrs::portable::metadata::{
    CodedIndex, CustomDebugInformationRow, DocumentId, DocumentRow, ImportScopeRow,
    LocalConstantRow, MetadataStream, MethodDebugInformationId, MethodDebugInformationRow,
    MethodDefId, TableId, TableRow,
};
use pdbrs::portable::{Identity, IdentityPolicy, Pdb};

use crate::build::{document, method, point, sample, write as write_editor};
use crate::{ID, information, open, parts_with_methods, write};

/// The debug tables of the output in table order.
const TABLES: [TableId; 8] = TableId::DEBUG_TABLES;

/// Return the bytes of the rows of `table` in the Portable PDB `bytes`.
fn table_bytes(bytes: &[u8], table: TableId) -> Vec<u8> {
    let pdb = open(bytes);
    let tables = pdb.tables();
    let start = usize::try_from(tables.table_offset(table).unwrap()).unwrap();
    let size = usize::from(tables.row_size(table).unwrap());
    let count = usize::try_from(tables.row_count(table)).unwrap();
    bytes[start..start + size * count].to_vec()
}

/// Return the bytes of a stream of the Portable PDB `bytes`.
fn stream(bytes: &[u8], stream: MetadataStream) -> Vec<u8> {
    open(bytes).stream_bytes(stream).unwrap().unwrap()
}

/// Return all rows of the table of `T`.
fn rows<T: TableRow>(bytes: &[u8]) -> Vec<T> {
    open(bytes).rows::<T>().map(|row| row.unwrap().1).collect()
}

/// Return the row size of `table` in the Portable PDB `bytes`.
fn row_size(bytes: &[u8], table: TableId) -> u8 {
    open(bytes).tables().row_size(table).unwrap()
}

/// Make a source with the table layer: a document name blob without a separator (Roslyn
/// splits the name at `/`), two methods, and custom debug information of the document.
fn source() -> Vec<u8> {
    let mut parts = parts_with_methods(2);
    let part = parts.blobs.add(b"/src/A.cs").unwrap();
    let mut name = Vec::new();
    DocumentName::encode_parts(None, &[part], &mut name).unwrap();
    let mut row = DocumentRow::default();
    row.name = parts.blobs.add(&name).unwrap();
    parts.tables.documents.push(row);
    let points = method(0, &[point(0, 1, 10)]);
    let mut first = MethodDebugInformationRow::default();
    first.document = DocumentId(1);
    first.sequence_points = parts.blobs.add(&points.sequence_points).unwrap();
    parts.tables.method_debug_information = vec![first, MethodDebugInformationRow::default()];
    let value = information(&mut parts, (TableId::DOCUMENT, 1), b"value");
    parts.tables.custom_debug_information.push(value);
    write(parts)
}

#[test]
fn an_edit_of_one_table_keeps_the_bytes_of_the_other_tables() {
    let bytes = source();
    let pdb = open(&bytes);
    let mut editor = pdb.edit(IdentityPolicy::Explicit(Identity::from_bytes(ID)));
    *editor
        .methods_mut()
        .unwrap()
        .get_mut(&MethodDefId(1))
        .unwrap() = method(0, &[point(0, 1, 11)]);
    let output = write_editor(&editor);

    for table in TABLES {
        let same = table_bytes(&output, table) == table_bytes(&bytes, table);
        assert_eq!(
            same,
            table != TableId::METHOD_DEBUG_INFORMATION,
            "table {table:?}"
        );
    }
    for heap in [
        MetadataStream::Strings,
        MetadataStream::UserStrings,
        MetadataStream::Guid,
    ] {
        assert_eq!(stream(&output, heap), stream(&bytes, heap), "{heap:?}");
    }
    let blobs = stream(&output, MetadataStream::Blob);
    assert!(blobs.starts_with(&stream(&bytes, MetadataStream::Blob)));
    let edited = open(&output);
    let document = edited.documents().next().unwrap().unwrap().1;
    assert_eq!(document.name, b"/src/A.cs");
    let information = edited
        .method_debug_information(MethodDebugInformationId(1))
        .unwrap();
    let points: Vec<_> = information
        .decode()
        .unwrap()
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(points, [point(0, 1, 11)]);
}

#[test]
fn an_edit_without_a_change_keeps_the_bytes_of_any_producer() {
    let bytes = source();
    let pdb = open(&bytes);
    let editor = pdb.edit(IdentityPolicy::Explicit(Identity::from_bytes(ID)));
    assert_eq!(write_editor(&editor), bytes);
}

#[test]
fn an_edit_that_crosses_the_blob_width_encodes_the_other_tables_again() {
    let bytes = write_editor(&sample());
    assert_eq!(open(&bytes).tables().blob_index_width(), 2);
    let pdb = open(&bytes);
    let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
    let mut large = document(b"/src/Large.cs", None);
    large.hash_algorithm = Some([3; 16]);
    large.hash = vec![0x11; 70_000].into();
    editor.documents_mut().unwrap().push(large).unwrap();
    let output = write_editor(&editor);

    let edited = open(&output);
    assert_eq!(edited.tables().blob_index_width(), 4);
    edited.validate_debug(ValidateOptions::default()).unwrap();
    for table in [
        TableId::METHOD_DEBUG_INFORMATION,
        TableId::LOCAL_CONSTANT,
        TableId::IMPORT_SCOPE,
        TableId::CUSTOM_DEBUG_INFORMATION,
    ] {
        assert_eq!(
            row_size(&output, table),
            row_size(&bytes, table) + 2,
            "table {table:?}"
        );
    }
    assert_eq!(
        rows::<MethodDebugInformationRow>(&output),
        rows::<MethodDebugInformationRow>(&bytes)
    );
    assert_eq!(
        rows::<LocalConstantRow>(&output),
        rows::<LocalConstantRow>(&bytes)
    );
    assert_eq!(
        rows::<ImportScopeRow>(&output),
        rows::<ImportScopeRow>(&bytes)
    );
    assert_eq!(
        rows::<CustomDebugInformationRow>(&output),
        rows::<CustomDebugInformationRow>(&bytes)
    );
}

#[test]
fn an_edit_that_crosses_a_coded_index_limit_encodes_the_custom_debug_information_again() {
    let bytes = write_editor(&sample());
    let coded = |bytes: &[u8]| {
        open(bytes)
            .tables()
            .coded_index_width(CodedIndex::HasCustomDebugInformation)
    };
    assert_eq!(coded(&bytes), 2);
    let pdb = open(&bytes);
    let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
    let documents = editor.documents_mut().unwrap();
    // A `HasCustomDebugInformation` index has 5 tag bits: 2048 rows need 4 bytes.
    while documents.len() < 2048 {
        let name = format!("/src/More{}.cs", documents.len());
        documents.push(document(name.as_bytes(), None)).unwrap();
    }
    let output = write_editor(&editor);

    assert_eq!(coded(&output), 4);
    open(&output)
        .validate_debug(ValidateOptions::default())
        .unwrap();
    let table = TableId::CUSTOM_DEBUG_INFORMATION;
    assert_eq!(row_size(&output, table), row_size(&bytes, table) + 2);
    assert_eq!(
        rows::<CustomDebugInformationRow>(&output),
        rows::<CustomDebugInformationRow>(&bytes)
    );
    for table in [TableId::LOCAL_SCOPE, TableId::LOCAL_VARIABLE] {
        assert_eq!(
            table_bytes(&output, table),
            table_bytes(&bytes, table),
            "table {table:?}"
        );
    }
}

#[test]
fn a_new_method_def_count_decodes_the_method_table() {
    let bytes = write_editor(&sample());
    let pdb = open(&bytes);
    let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
    editor.external_mut().set(TableId::METHOD_DEF, 5);
    let output = write_editor(&editor);

    let edited = open(&output);
    assert_eq!(
        edited.tables().row_count(TableId::METHOD_DEBUG_INFORMATION),
        5
    );
    let mut expected = rows::<MethodDebugInformationRow>(&bytes);
    expected.push(MethodDebugInformationRow::default());
    assert_eq!(rows::<MethodDebugInformationRow>(&output), expected);
}

#[test]
fn custom_debug_information_with_source_local_scopes_keeps_its_parents() {
    let bytes = write_editor(&sample());
    let pdb: Pdb<&[u8]> = open(&bytes);
    let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
    let custom = editor.custom_debug_information_mut().unwrap();
    let (handle, parent) = custom
        .iter()
        .map(|(handle, information)| (handle, information.parent))
        .find(|(_, parent)| matches!(parent, CustomDebugParent::LocalVariable(_)))
        .unwrap();
    custom.get_mut(handle).unwrap().value = b"changed".to_vec().into();
    let output = write_editor(&editor);

    let parents = |bytes: &[u8]| {
        rows::<CustomDebugInformationRow>(bytes)
            .into_iter()
            .map(|row| row.parent)
            .collect::<Vec<_>>()
    };
    assert_eq!(parents(&output), parents(&bytes));
    for table in [
        TableId::LOCAL_SCOPE,
        TableId::LOCAL_VARIABLE,
        TableId::LOCAL_CONSTANT,
    ] {
        assert_eq!(
            table_bytes(&output, table),
            table_bytes(&bytes, table),
            "table {table:?}"
        );
    }
    let edited = open(&output);
    let changed = edited
        .custom_debug_informations()
        .map(|information| information.unwrap().1)
        .find(|information| information.parent == parent)
        .unwrap();
    assert_eq!(changed.value, b"changed");
    assert_ne!(changed.kind, CustomDebugKind::EMBEDDED_SOURCE);
}
