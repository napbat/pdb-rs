//! Planning errors: each check fails before a plan exists, so no output is written.

use pdbrs::ErrorKind;
use pdbrs::portable::metadata::{
    BlobOffset, GuidIndex, ImportScopeId, ImportScopeRow, LocalConstantRow, LocalScopeRow,
    MetadataStream, MethodDebugInformationRow, MethodDefId, StateMachineMethodRow, StringOffset,
    TableId,
};
use pdbrs::portable::write::PdbParts;

use crate::{document, information, location, parts_with_methods, plan, plan_error};

/// Check the kind and the table, row and column of a planning error.
fn expect(parts: PdbParts, kind: ErrorKind, table: TableId, row: u32, column: &str) {
    let error = plan_error(parts);
    assert_eq!(error.kind(), kind, "{error}");
    let location = location(&error);
    assert_eq!(location.stream(), Some(MetadataStream::Tables), "{error}");
    assert_eq!(location.table(), Some(table), "{error}");
    assert_eq!(location.row(), Some(row), "{error}");
    assert_eq!(location.column(), Some(column), "{error}");
}

/// Make a local scope row.
fn scope(method: u32, start: u32, length: u32) -> LocalScopeRow {
    LocalScopeRow {
        method: MethodDefId::new(method),
        import_scope: ImportScopeId::new(0),
        variable_list: pdbrs::portable::metadata::LocalVariableId::new(1),
        constant_list: pdbrs::portable::metadata::LocalConstantId::new(1),
        start_offset: pdbrs::portable::metadata::IlOffset::new(start),
        length,
    }
}

#[test]
fn local_scopes_need_method_start_and_longest_first_order() {
    let sorted = [
        scope(1, 0, 20),
        scope(1, 0, 10),
        scope(1, 4, 2),
        scope(2, 0, 5),
    ];
    let mut parts = parts_with_methods(2);
    parts.tables.local_scopes = sorted.to_vec();
    plan(parts);
    for (rows, row) in [
        (vec![scope(2, 0, 5), scope(1, 0, 20)], 2),
        (vec![scope(1, 4, 5), scope(1, 0, 20)], 2),
        (vec![scope(1, 0, 10), scope(1, 0, 20)], 2),
    ] {
        let mut parts = parts_with_methods(2);
        parts.tables.local_scopes = rows;
        expect(
            parts,
            ErrorKind::InvalidFormat,
            TableId::LOCAL_SCOPE,
            row,
            "Method",
        );
    }
}

#[test]
fn variable_lists_cannot_decrease() {
    let mut parts = parts_with_methods(2);
    let mut second = scope(2, 0, 5);
    second.variable_list = pdbrs::portable::metadata::LocalVariableId::new(0);
    parts.tables.local_scopes = vec![scope(1, 0, 5), second];
    expect(
        parts,
        ErrorKind::InvalidFormat,
        TableId::LOCAL_SCOPE,
        2,
        "VariableList",
    );
}

#[test]
fn local_scope_method_must_be_an_external_row() {
    let mut parts = parts_with_methods(2);
    parts.tables.local_scopes = vec![scope(3, 0, 5)];
    expect(
        parts,
        ErrorKind::InvalidReference,
        TableId::LOCAL_SCOPE,
        1,
        "Method",
    );
}

#[test]
fn state_machine_methods_need_unique_sorted_methods() {
    let row = |move_next, kickoff| StateMachineMethodRow {
        move_next_method: MethodDefId::new(move_next),
        kickoff_method: MethodDefId::new(kickoff),
    };
    let mut parts = parts_with_methods(4);
    parts.tables.state_machine_methods = vec![row(2, 1), row(2, 3)];
    let table = TableId::STATE_MACHINE_METHOD;
    expect(parts, ErrorKind::InvalidFormat, table, 2, "MoveNextMethod");
    let mut parts = parts_with_methods(4);
    parts.tables.state_machine_methods = vec![row(2, 1), row(4, 1)];
    expect(parts, ErrorKind::InvalidFormat, table, 2, "KickoffMethod");
}

#[test]
fn custom_debug_information_needs_sorted_parents() {
    let mut parts = parts_with_methods(2);
    let second = information(&mut parts, (TableId::METHOD_DEF, 2), b"a");
    let first = information(&mut parts, (TableId::METHOD_DEF, 1), b"b");
    parts.tables.custom_debug_information = vec![second, first];
    let table = TableId::CUSTOM_DEBUG_INFORMATION;
    expect(parts, ErrorKind::InvalidFormat, table, 2, "Parent");
}

#[test]
fn custom_debug_information_parent_must_be_in_the_coded_index() {
    let mut parts = parts_with_methods(2);
    parts.tables.state_machine_methods = vec![StateMachineMethodRow {
        move_next_method: MethodDefId::new(2),
        kickoff_method: MethodDefId::new(1),
    }];
    let row = information(&mut parts, (TableId::STATE_MACHINE_METHOD, 1), b"a");
    parts.tables.custom_debug_information = vec![row];
    let table = TableId::CUSTOM_DEBUG_INFORMATION;
    expect(parts, ErrorKind::InvalidReference, table, 1, "Parent");

    let mut parts = parts_with_methods(2);
    let row = information(&mut parts, (TableId::METHOD_DEF, 3), b"a");
    parts.tables.custom_debug_information = vec![row];
    expect(parts, ErrorKind::InvalidReference, table, 1, "Parent");
}

#[test]
fn heap_references_must_identify_entries() {
    let mut parts = parts_with_methods(1);
    parts.blobs.add(b"abc").unwrap();
    parts.tables.local_constants = vec![LocalConstantRow {
        name: StringOffset::new(0),
        // Offset 2 is inside the blob at offset 1.
        signature: BlobOffset::new(2),
    }];
    let table = TableId::LOCAL_CONSTANT;
    expect(parts, ErrorKind::InvalidReference, table, 1, "Signature");

    let mut parts = parts_with_methods(1);
    parts.tables.local_constants = vec![LocalConstantRow {
        name: StringOffset::new(1),
        signature: BlobOffset::new(0),
    }];
    expect(parts, ErrorKind::InvalidReference, table, 1, "Name");

    let mut parts = parts_with_methods(1);
    let mut row = information(&mut parts, (TableId::METHOD_DEF, 1), b"a");
    row.kind = GuidIndex::new(2);
    parts.tables.custom_debug_information = vec![row];
    let table = TableId::CUSTOM_DEBUG_INFORMATION;
    expect(parts, ErrorKind::InvalidReference, table, 1, "Kind");
}

#[test]
fn document_names_are_not_nil() {
    let mut parts = parts_with_methods(1);
    let mut row = document(&mut parts, b"/a");
    row.name = BlobOffset::new(0);
    parts.tables.documents = vec![row];
    expect(
        parts,
        ErrorKind::InvalidReference,
        TableId::DOCUMENT,
        1,
        "Name",
    );
}

#[test]
fn method_debug_information_has_a_row_for_each_method() {
    let row = MethodDebugInformationRow {
        document: pdbrs::portable::metadata::DocumentId::new(0),
        sequence_points: BlobOffset::new(0),
    };
    let mut parts = parts_with_methods(2);
    parts.tables.method_debug_information = vec![row];
    let error = plan_error(parts);
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    let location = location(&error);
    assert_eq!(location.table(), Some(TableId::METHOD_DEBUG_INFORMATION));

    let mut parts = parts_with_methods(2);
    parts.tables.method_debug_information = vec![row, row];
    plan(parts);
}

#[test]
fn import_scope_parents_cannot_form_a_cycle() {
    let row = |parent| ImportScopeRow {
        parent: ImportScopeId::new(parent),
        imports: BlobOffset::new(0),
    };
    let mut parts = parts_with_methods(1);
    parts.tables.import_scopes = vec![row(0), row(1), row(4), row(3)];
    let table = TableId::IMPORT_SCOPE;
    expect(parts, ErrorKind::InvalidReference, table, 3, "Parent");

    let mut parts = parts_with_methods(1);
    parts.tables.import_scopes = vec![row(0), row(1), row(2), row(2)];
    plan(parts);
}

#[test]
fn pdb_stream_fields_are_checked() {
    let mut parts = parts_with_methods(2);
    parts.entry_point = Some(MethodDefId::new(3));
    let error = plan_error(parts);
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
    assert_eq!(location(&error).stream(), Some(MetadataStream::Pdb));

    let mut parts = parts_with_methods(2);
    parts.external.set(TableId::DOCUMENT, 1);
    let error = plan_error(parts);
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(location(&error).table(), Some(TableId::DOCUMENT));

    let mut parts = parts_with_methods(2);
    parts.external.set(TableId::TYPE_DEF, 0x0100_0000);
    let error = plan_error(parts);
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);

    let mut parts = parts_with_methods(2);
    parts.version = b"PDB\0v1".to_vec();
    let error = plan_error(parts);
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(location(&error).stream(), Some(MetadataStream::Root));
}
