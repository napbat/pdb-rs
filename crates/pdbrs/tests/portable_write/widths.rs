//! Index widths that change during a build: heap sizes, local row counts, external row counts
//! and coded indexes. Each test plans parts just below and at a threshold, reads the output
//! with the reader and checks the widths and the values that need the wide index.

use pdbrs::portable::metadata::{
    CodedIndex, CustomDebugInformationId, CustomDebugInformationRow, DocumentRow, IlOffset,
    LocalScopeId, LocalScopeRow, LocalVariableId, LocalVariableRow, MetadataToken, MethodDefId,
    StateMachineMethodId, StateMachineMethodRow, TableId,
};
use pdbrs::portable::write::{PdbParts, StringHeapBuilder};

use crate::{document, information, open, parts_with_methods, write};

/// Make parts with local variables named `a` × `length` and `b`.
fn string_parts(length: usize) -> PdbParts {
    let long = vec![b'a'; length];
    let mut builder = StringHeapBuilder::new();
    builder.add(&long).unwrap();
    builder.add(b"b").unwrap();
    let mut parts = PdbParts::default();
    parts.strings = builder.finish().unwrap();
    for name in [&long[..], b"b"] {
        let mut row = LocalVariableRow::default();
        row.name = parts.strings.offset(name).unwrap();
        parts.tables.local_variables.push(row);
    }
    parts
}

#[test]
fn string_index_widens_after_0xffff_heap_bytes() {
    // The heap has 1 + (length + 1) + 2 bytes. "b" is the last string.
    for (length, width) in [(0xFFFB, 2), (0xFFFC, 4)] {
        let parts = string_parts(length);
        assert_eq!(parts.strings.size() > 0xFFFF, width == 4);
        let bytes = write(parts);
        let pdb = open(&bytes);
        assert_eq!(pdb.tables().string_index_width(), width);
        let last = pdb.row::<LocalVariableRow>(LocalVariableId(2)).unwrap();
        assert_eq!(pdb.string(last.name).unwrap(), b"b");
        let first = pdb.row::<LocalVariableRow>(LocalVariableId(1)).unwrap();
        assert_eq!(pdb.string(first.name).unwrap().len(), length);
    }
}

#[test]
fn blob_index_widens_after_0xffff_heap_bytes() {
    // The heap has 1 + (4 + length) + 2 bytes. The value "z" is the last blob.
    for (length, width) in [(0xFFF8, 2), (0xFFF9, 4)] {
        let mut parts = parts_with_methods(1);
        let long = information(&mut parts, (TableId::METHOD_DEF, 1), &vec![0x11; length]);
        let short = information(&mut parts, (TableId::METHOD_DEF, 1), b"z");
        parts.tables.custom_debug_information = vec![long, short];
        assert_eq!(parts.blobs.size() > 0xFFFF, width == 4);
        let bytes = write(parts);
        let pdb = open(&bytes);
        assert_eq!(pdb.tables().blob_index_width(), width);
        let row = pdb
            .row::<CustomDebugInformationRow>(CustomDebugInformationId(2))
            .unwrap();
        assert_eq!(pdb.blob(row.value).unwrap(), b"z");
    }
}

#[test]
fn guid_index_widens_after_0xffff_heap_bytes() {
    // The kind GUID of `information` is the last of `count` GUIDs.
    for (count, width) in [(4095_u32, 2), (4096, 4)] {
        let mut parts = parts_with_methods(1);
        for index in 1..count {
            let mut guid = [0; 16];
            guid[..4].copy_from_slice(&index.to_le_bytes());
            parts.guids.add(guid).unwrap();
        }
        let row = information(&mut parts, (TableId::METHOD_DEF, 1), b"v");
        assert_eq!((row.kind.0, parts.guids.count()), (count, count));
        parts.tables.custom_debug_information.push(row);
        let bytes = write(parts);
        let pdb = open(&bytes);
        assert_eq!(pdb.tables().guid_index_width(), width);
        let row = pdb
            .row::<CustomDebugInformationRow>(CustomDebugInformationId(1))
            .unwrap();
        assert_eq!(pdb.guid(row.kind).unwrap(), Some([7; 16]));
    }
}

#[test]
fn local_table_index_widens_at_0x10000_rows() {
    for (count, width) in [(0xFFFF_u32, 2), (0x1_0000, 4)] {
        let mut parts = parts_with_methods(1);
        let variable = LocalVariableRow::default();
        parts.tables.local_variables = vec![variable; count as usize];
        parts.tables.local_scopes = vec![scope(1, 0, 10, 1), scope(1, 2, 4, count)];
        let bytes = write(parts);
        let pdb = open(&bytes);
        assert_eq!(
            pdb.tables().table_index_width(TableId::LOCAL_VARIABLE),
            width
        );
        let row = pdb.row::<LocalScopeRow>(LocalScopeId(2)).unwrap();
        assert_eq!(row.variable_list.0, count);
        assert_eq!(row.length, 4);
    }
}

/// Make a local scope row of `method` with a variable list start.
fn scope(method: u32, start: u32, length: u32, variables: u32) -> LocalScopeRow {
    let mut row = LocalScopeRow::default();
    row.method = MethodDefId(method);
    row.variable_list = LocalVariableId(variables);
    row.start_offset = IlOffset(start);
    row.length = length;
    row
}

#[test]
fn external_method_count_widens_method_references() {
    for (methods, width) in [(0xFFFF_u32, 2), (0x1_0000, 4)] {
        let mut parts = parts_with_methods(methods);
        parts.entry_point = Some(MethodDefId(methods));
        parts.tables.local_scopes = vec![scope(methods, 0, 8, 0)];
        let mut state_machine = StateMachineMethodRow::default();
        state_machine.move_next_method = MethodDefId(methods);
        state_machine.kickoff_method = MethodDefId(methods - 1);
        parts.tables.state_machine_methods = vec![state_machine];
        let bytes = write(parts);
        let pdb = open(&bytes);
        assert_eq!(pdb.tables().table_index_width(TableId::METHOD_DEF), width);
        assert_eq!(pdb.pdb_stream().entry_point(), Some(MethodDefId(methods)));
        let row = pdb.row::<LocalScopeRow>(LocalScopeId(1)).unwrap();
        assert_eq!(row.method.0, methods);
        let row = pdb
            .row::<StateMachineMethodRow>(StateMachineMethodId(1))
            .unwrap();
        assert_eq!(row.kickoff_method.0, methods - 1);
    }
}

#[test]
fn external_type_count_widens_the_coded_parent() {
    // HasCustomDebugInformation has 5 tag bits: 2 bytes for fewer than 0x800 rows.
    for (types, width) in [(0x7FF_u32, 2), (0x800, 4)] {
        let mut parts = parts_with_methods(1);
        parts.external.set(TableId::TYPE_DEF, types);
        let row = information(&mut parts, (TableId::TYPE_DEF, types), b"t");
        parts.tables.custom_debug_information.push(row);
        let bytes = write(parts);
        let pdb = open(&bytes);
        let coded = pdb
            .tables()
            .coded_index_width(CodedIndex::HasCustomDebugInformation);
        assert_eq!(coded, width);
        let row = pdb
            .row::<CustomDebugInformationRow>(CustomDebugInformationId(1))
            .unwrap();
        assert_eq!(
            row.parent,
            MetadataToken::new(TableId::TYPE_DEF, types).unwrap()
        );
    }
}

#[test]
fn local_document_count_widens_the_coded_parent() {
    for (count, width) in [(0x7FF_u32, 2), (0x800, 4)] {
        let mut parts = parts_with_methods(1);
        let row: DocumentRow = document(&mut parts, b"\x2fa");
        parts.tables.documents = vec![row; count as usize];
        let row = information(&mut parts, (TableId::DOCUMENT, count), b"d");
        parts.tables.custom_debug_information.push(row);
        let bytes = write(parts);
        let pdb = open(&bytes);
        let coded = pdb
            .tables()
            .coded_index_width(CodedIndex::HasCustomDebugInformation);
        assert_eq!(coded, width);
        let row = pdb
            .row::<CustomDebugInformationRow>(CustomDebugInformationId(1))
            .unwrap();
        assert_eq!(row.parent.row(), count);
        assert_eq!(pdb.blob(row.value).unwrap(), b"d");
    }
}
