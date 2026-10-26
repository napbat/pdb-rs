//! Heap, table and coded index widths at their thresholds.

use pdbrs::ErrorKind;
use pdbrs::portable::Pdb;
use pdbrs::portable::metadata::{
    CodedIndex, CustomDebugInformationRow, DocumentId, DocumentRow, LocalScopeRow, LocalVariableId,
    LocalVariableRow, MetadataToken, MethodDefId, StateMachineMethodRow, TableId,
};

use crate::builder::{Builder, row, value};
use crate::location;

#[test]
fn heap_size_flags_select_the_heap_index_widths() {
    for flags in 0..8_u8 {
        let builder = Builder {
            heap_sizes: flags,
            ..Builder::default()
        };
        let bytes = builder.build();
        let pdb = Pdb::parse(&bytes).unwrap();
        let tables = pdb.tables();
        let width = |flag: u8| if flags & flag == 0 { 2 } else { 4 };
        let (string, guid, blob) = (width(1), width(2), width(4));
        assert_eq!(tables.heap_sizes(), flags);
        assert_eq!(tables.string_index_width(), string);
        assert_eq!(tables.guid_index_width(), guid);
        assert_eq!(tables.blob_index_width(), blob);
        let size = |table| tables.row_size(table).unwrap();
        assert_eq!(size(TableId::DOCUMENT), 2 * blob + 2 * guid);
        assert_eq!(size(TableId::METHOD_DEBUG_INFORMATION), 2 + blob);
        assert_eq!(size(TableId::LOCAL_VARIABLE), 4 + string);
        assert_eq!(size(TableId::LOCAL_CONSTANT), string + blob);
        assert_eq!(size(TableId::IMPORT_SCOPE), 2 + blob);
        assert_eq!(size(TableId::CUSTOM_DEBUG_INFORMATION), 2 + guid + blob);
    }
}

#[test]
fn four_byte_heap_indexes_reach_offsets_above_64_kib() {
    let mut strings = vec![0; 0x1_0001];
    strings.extend_from_slice(b"far\0");
    let mut blobs = vec![0; 0x1_0000];
    blobs.extend_from_slice(&[2, b'h', b'i']);
    let builder = Builder {
        heap_sizes: 0x07,
        strings: Some(strings),
        blobs: Some(blobs),
        ..Builder::default()
    }
    .table(
        0x30,
        1,
        row(&[value(4, 0x1_0000), value(4, 2), value(4, 0), value(4, 1)]),
    )
    .table(
        0x33,
        1,
        row(&[value(2, 0), value(2, 7), value(4, 0x1_0001)]),
    );
    let bytes = builder.build();
    let pdb = Pdb::parse(&bytes).unwrap();
    let document: DocumentRow = pdb.row(DocumentId::new(1)).unwrap();
    assert_eq!(pdb.blob(document.name).unwrap(), b"hi");
    assert_eq!(document.hash_algorithm.get(), 2);
    let variable: LocalVariableRow = pdb.row(LocalVariableId::new(1)).unwrap();
    assert_eq!(variable.index, 7);
    assert_eq!(pdb.string(variable.name).unwrap(), b"far");
}

/// The `LocalScope` row size for the given variable index width.
fn local_scope_size(variable_width: u8) -> u8 {
    2 + 2 + variable_width + 2 + 8
}

#[test]
fn table_indexes_widen_at_65536_rows() {
    for (variables, width) in [(0xFFFF_u32, 2_u8), (0x1_0000, 4)] {
        let rows = vec![0; usize::try_from(variables).unwrap() * 6];
        // A 2-byte index cannot hold 0x10000, so the 2-byte case uses the last row.
        let last = if width == 2 { variables } else { variables + 1 };
        let scope = row(&[
            value(2, 1),
            value(2, 0),
            value(width, last),
            value(2, 0),
            value(4, 0),
            value(4, 1),
        ]);
        let builder = Builder::default()
            .table(0x32, 1, scope)
            .table(0x33, variables, rows);
        let bytes = builder.build();
        let pdb = Pdb::parse(&bytes).unwrap();
        let tables = pdb.tables();
        assert_eq!(tables.table_index_width(TableId::LOCAL_VARIABLE), width);
        assert_eq!(
            tables.row_size(TableId::LOCAL_SCOPE),
            Some(local_scope_size(width))
        );
        let scope: LocalScopeRow = pdb.rows().next().unwrap().unwrap().1;
        assert_eq!(scope.variable_list, LocalVariableId::new(last));
    }
}

#[test]
fn external_method_counts_widen_method_references() {
    for (methods, width) in [(0xFFFF_u32, 2_u8), (0x1_0000, 4)] {
        let builder = Builder {
            external: vec![(0x06, methods)],
            ..Builder::default()
        }
        .table(0x36, 1, row(&[value(width, methods), value(width, 1)]));
        let bytes = builder.build();
        let pdb = Pdb::parse(&bytes).unwrap();
        let tables = pdb.tables();
        assert_eq!(tables.width_row_count(TableId::METHOD_DEF), methods);
        assert_eq!(tables.row_count(TableId::METHOD_DEF), 0);
        assert_eq!(tables.table_index_width(TableId::METHOD_DEF), width);
        assert_eq!(
            tables.row_size(TableId::STATE_MACHINE_METHOD),
            Some(2 * width)
        );
        assert_eq!(
            tables.row_size(TableId::LOCAL_SCOPE),
            Some(width + 2 + 2 + 2 + 8)
        );
        let machine: StateMachineMethodRow = pdb.rows().next().unwrap().unwrap().1;
        assert_eq!(machine.move_next_method, MethodDefId::new(methods));
        assert_eq!(machine.kickoff_method, MethodDefId::new(1));
    }
}

/// A `CustomDebugInformation` row with a parent of `width` bytes.
fn custom(width: u8, parent: u32) -> Vec<u8> {
    row(&[value(width, parent), value(2, 1), value(2, 0)])
}

#[test]
fn external_row_counts_widen_the_has_custom_debug_information_index() {
    // TypeRef has tag 2. 2047 rows fit in 11 bits; 2048 rows do not.
    for (rows, width) in [(2047_u32, 2_u8), (2048, 4)] {
        let parent = (rows << 5) | 2;
        let builder = Builder {
            external: vec![(0x01, rows), (0x06, 1)],
            ..Builder::default()
        }
        .table(0x37, 1, custom(width, parent));
        let bytes = builder.build();
        let pdb = Pdb::parse(&bytes).unwrap();
        let tables = pdb.tables();
        assert_eq!(
            tables.coded_index_width(CodedIndex::HasCustomDebugInformation),
            width
        );
        assert_eq!(
            tables.row_size(TableId::CUSTOM_DEBUG_INFORMATION),
            Some(width + 4)
        );
        let information: CustomDebugInformationRow = pdb.rows().next().unwrap().unwrap().1;
        assert_eq!(
            information.parent,
            MetadataToken::new(TableId::TYPE_REF, rows).unwrap()
        );
    }
}

#[test]
fn local_debug_table_counts_widen_the_has_custom_debug_information_index() {
    // Document has tag 22.
    let documents = 2048_u32;
    let rows = vec![0; usize::try_from(documents).unwrap() * 8];
    let builder = Builder::default().table(0x30, documents, rows).table(
        0x37,
        1,
        custom(4, (documents << 5) | 0x16),
    );
    let bytes = builder.build();
    let pdb = Pdb::parse(&bytes).unwrap();
    assert_eq!(
        pdb.tables()
            .coded_index_width(CodedIndex::HasCustomDebugInformation),
        4
    );
    let information: CustomDebugInformationRow = pdb.rows().next().unwrap().unwrap().1;
    assert_eq!(information.parent.table(), TableId::DOCUMENT.number());
    assert_eq!(information.parent.row(), documents);
}

#[test]
fn coded_indexes_encode_and_decode_tokens() {
    let index = CodedIndex::HasCustomDebugInformation;
    assert_eq!(index.tables().len(), 27);
    assert_eq!(index.tag_bits(), 5);
    for (position, table) in index.tables().iter().enumerate() {
        let token = MetadataToken::new(*table, 9).unwrap();
        let encoded = index.encode(token).unwrap();
        assert_eq!(encoded & 0x1F, u32::try_from(position).unwrap());
        assert_eq!(index.decode(encoded), Some(token));
    }
    assert_eq!(index.decode(27), None);
    assert_eq!(index.encode(MetadataToken::from_raw(0x3600_0001)), None);
}

#[test]
fn rows_of_wide_tables_need_their_full_width() {
    // The rows of 4-byte columns end after the stream when the stream has 2-byte rows.
    let builder = Builder {
        external: vec![(0x06, 0x1_0000)],
        ..Builder::default()
    }
    .table(0x36, 1, row(&[value(2, 1), value(2, 1)]));
    let error = crate::open_error(&builder.build());
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(
        location(&error).table(),
        Some(TableId::STATE_MACHINE_METHOD)
    );
}
