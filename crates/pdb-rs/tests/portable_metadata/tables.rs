//! The `#~` header, the table schema checks, row access and heap access.

use pdb_rs::ErrorKind;
use pdb_rs::portable::metadata::{
    BlobOffset, CustomDebugInformationRow, DocumentId, DocumentRow, GuidIndex, ImportScopeId,
    LocalConstantId, LocalScopeRow, LocalVariableId, MetadataStream, MethodDebugInformationRow,
    MethodDefId, StringOffset, TableId, UserStringOffset,
};
use pdb_rs::portable::{Pdb, ReadOptions};

use crate::builder::{Builder, row, value};
use crate::{expect, location, open_error};

/// A `Document` row with 2-byte heap indexes.
fn document(name: u32, hash_algorithm: u32, hash: u32, language: u32) -> Vec<u8> {
    row(&[
        value(2, name),
        value(2, hash_algorithm),
        value(2, hash),
        value(2, language),
    ])
}

#[test]
fn table_stream_version_and_heap_flags_are_checked() {
    let old = Builder {
        tables_version: (1, 0),
        ..Builder::default()
    };
    expect(
        &open_error(&old.build()),
        ErrorKind::UnsupportedVersion,
        MetadataStream::Tables,
    );
    for flags in [0x08, 0x20, 0x40, 0x80] {
        let delta = Builder {
            heap_sizes: flags,
            ..Builder::default()
        };
        expect(
            &open_error(&delta.build()),
            ErrorKind::UnsupportedVersion,
            MetadataStream::Tables,
        );
    }
}

/// Assemble the default streams with other `#~` bytes.
fn with_table_stream(builder: &Builder, bytes: Vec<u8>) -> Vec<u8> {
    let mut streams = builder.streams();
    streams[1].bytes = bytes;
    builder.assemble(&streams)
}

#[test]
fn truncated_table_stream_parts_are_rejected() {
    let builder = Builder::default().table(0x30, 2, document(0, 0, 0, 0).repeat(2));
    let full = builder.table_stream();
    // The header has 24 bytes, one row count and two rows of 8 bytes.
    for (length, table) in [(20, None), (24, None), (36, Some(TableId::DOCUMENT))] {
        let error = open_error(&with_table_stream(&builder, full[..length].to_vec()));
        expect(&error, ErrorKind::InvalidFormat, MetadataStream::Tables);
        assert_eq!(location(&error).table(), table, "length {length}: {error}");
    }
}

#[test]
fn tables_outside_the_debug_schema_cannot_have_rows() {
    for table in [0x02, 0x2C, 0x2D, 0x38, 0x3F] {
        let builder = Builder::default().table(table, 1, vec![0; 4]);
        let error = open_error(&builder.build());
        expect(&error, ErrorKind::InvalidFormat, MetadataStream::Tables);
        assert_eq!(location(&error).table(), TableId::from_raw(table));
        assert!(error.context().unwrap().contains("row width is unknown"));
    }
    let empty = Builder::default().table(0x02, 0, Vec::new());
    let bytes = empty.build();
    let pdb = Pdb::parse(&bytes).unwrap();
    assert_eq!(pdb.tables().valid_mask(), 1 << 0x02);
    assert_eq!(pdb.tables().row_count(TableId::TYPE_DEF), 0);
}

#[test]
fn row_counts_must_fit_in_a_token() {
    let builder = Builder::default().table(0x30, 0x0100_0000, Vec::new());
    let error = open_error(&builder.build());
    expect(&error, ErrorKind::InvalidFormat, MetadataStream::Tables);
    assert_eq!(location(&error).table(), Some(TableId::DOCUMENT));
}

#[test]
fn rows_and_heap_values_are_read_with_checks() {
    let builder = Builder {
        sorted: 0x00c4_0000_0000_0000,
        ..Builder::default()
    }
    .table(
        0x30,
        2,
        [document(1, 1, 0, 2), document(0, 0, 0, 0)].concat(),
    )
    .table(0x31, 1, row(&[value(2, 2), value(2, 1)]));
    let bytes = builder.build();
    let pdb = Pdb::parse(&bytes).unwrap();
    assert_eq!(pdb.tables().sorted_mask(), 0x00c4_0000_0000_0000);
    assert!(pdb.tables().table_offset(TableId::LOCAL_SCOPE).is_some());
    assert_eq!(pdb.tables().table_offset(TableId::METHOD_DEF), None);

    let first: DocumentRow = pdb.row(DocumentId(1)).unwrap();
    assert_eq!(first.name, BlobOffset(1));
    assert_eq!(first.hash_algorithm, GuidIndex(1));
    assert_eq!(pdb.blob(first.name).unwrap(), b"abc");
    assert_eq!(pdb.blob(first.hash).unwrap(), b"");
    assert_eq!(
        pdb.guid(first.hash_algorithm).unwrap(),
        Some(core::array::from_fn(|i| u8::try_from(i).unwrap() + 1))
    );
    assert_eq!(pdb.guid(first.language).unwrap().unwrap()[0], 17);
    assert_eq!(pdb.guid(GuidIndex(0)).unwrap(), None);

    let ids: Vec<u32> = pdb
        .rows::<DocumentRow>()
        .map(|row| row.unwrap().0.0)
        .collect();
    assert_eq!(ids, [1, 2]);
    assert_eq!(pdb.rows::<LocalScopeRow>().len(), 0);
    let method: MethodDebugInformationRow = pdb.rows().next().unwrap().unwrap().1;
    assert_eq!(method.document, DocumentId(2));

    for id in [0, 3] {
        let error = pdb.row::<DocumentRow>(DocumentId(id)).unwrap_err();
        expect(&error, ErrorKind::InvalidReference, MetadataStream::Tables);
        assert_eq!(location(&error).row(), Some(id));
    }
}

#[test]
fn row_columns_outside_their_heap_or_table_are_invalid_references() {
    let cases = [
        (document(8, 0, 0, 0), "Name", Some(8)),
        (document(0, 3, 0, 0), "HashAlgorithm", Some(3)),
        (document(0, 0, 0, 3), "Language", Some(3)),
    ];
    for (bytes, column, heap_offset) in cases {
        let builder = Builder::default().table(0x30, 1, bytes);
        let bytes = builder.build();
        let pdb = Pdb::parse(&bytes).unwrap();
        let error = pdb.row::<DocumentRow>(DocumentId(1)).unwrap_err();
        expect(&error, ErrorKind::InvalidReference, MetadataStream::Tables);
        let location = location(&error);
        assert_eq!(location.table(), Some(TableId::DOCUMENT));
        assert_eq!(location.row(), Some(1));
        assert_eq!(location.column(), Some(column));
        assert_eq!(location.heap_offset(), heap_offset);
    }

    let builder = Builder::default().table(0x31, 1, row(&[value(2, 1), value(2, 0)]));
    let bytes = builder.build();
    let pdb = Pdb::parse(&bytes).unwrap();
    let error = pdb
        .rows::<MethodDebugInformationRow>()
        .next()
        .unwrap()
        .unwrap_err();
    assert_eq!(location(&error).column(), Some("Document"));
}

/// A `LocalScope` row with 2-byte indexes.
fn scope(method: u32, import: u32, variables: u32, constants: u32) -> Vec<u8> {
    row(&[
        value(2, method),
        value(2, import),
        value(2, variables),
        value(2, constants),
        value(4, 0x10),
        value(4, 0x20),
    ])
}

#[test]
fn list_columns_may_end_one_after_the_last_row() {
    let builder = Builder::default()
        .table(0x32, 2, [scope(3, 0, 1, 0), scope(3, 0, 2, 1)].concat())
        .table(0x33, 1, row(&[value(2, 1), value(2, 0), value(2, 1)]));
    let bytes = builder.build();
    let pdb = Pdb::parse(&bytes).unwrap();
    let rows: Vec<LocalScopeRow> = pdb.rows().map(|row| row.unwrap().1).collect();
    assert_eq!(rows[0].method, MethodDefId(3));
    assert_eq!(rows[0].import_scope, ImportScopeId(0));
    assert_eq!(rows[1].variable_list, LocalVariableId(2));
    assert_eq!(rows[0].constant_list, LocalConstantId(0));
    assert_eq!(rows[1].start_offset.0, 0x10);
    assert_eq!(rows[1].length, 0x20);

    for (bytes, column) in [
        (scope(3, 0, 3, 0), "VariableList"),
        (scope(3, 0, 1, 2), "ConstantList"),
        (scope(4, 0, 1, 0), "Method"),
        (scope(3, 1, 1, 0), "ImportScope"),
    ] {
        let builder = Builder::default().table(0x32, 1, bytes).table(
            0x33,
            1,
            row(&[value(2, 1), value(2, 0), value(2, 1)]),
        );
        let bytes = builder.build();
        let pdb = Pdb::parse(&bytes).unwrap();
        let error = pdb.rows::<LocalScopeRow>().next().unwrap().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidReference);
        assert_eq!(location(&error).column(), Some(column));
    }
}

#[test]
fn custom_debug_information_parents_are_checked_tokens() {
    let cdi = |parent: u32| row(&[value(2, parent), value(2, 1), value(2, 1)]);
    let builder = Builder::default().table(0x37, 1, cdi((1 << 5) | 7));
    let bytes = builder.build();
    let pdb = Pdb::parse(&bytes).unwrap();
    let row: CustomDebugInformationRow = pdb.rows().next().unwrap().unwrap().1;
    assert_eq!(row.parent.raw(), 0x0000_0001);
    assert_eq!(row.kind, GuidIndex(1));
    assert_eq!(row.value, BlobOffset(1));

    // Tag 27 is not in the list; Module (tag 7) has one row; TypeDef (tag 3) has no rows.
    for parent in [27, (2 << 5) | 7, (1 << 5) | 3] {
        let builder = Builder::default().table(0x37, 1, cdi(parent));
        let bytes = builder.build();
        let pdb = Pdb::parse(&bytes).unwrap();
        let error = pdb
            .rows::<CustomDebugInformationRow>()
            .next()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidReference);
        assert_eq!(location(&error).column(), Some("Parent"));
    }
}

#[test]
fn heap_values_check_termination_lengths_and_limits() {
    let builder = Builder {
        strings: Some(b"\0abc".to_vec()),
        blobs: Some(vec![0, 7, b'a', b'b', 0, 0, 0xE0, 0xC0]),
        user_strings: Some(vec![0, 3, b'a', 0, 1, 0, 0, 0]),
        ..Builder::default()
    };
    let bytes = builder.build();
    let pdb = Pdb::parse(&bytes).unwrap();
    assert_eq!(pdb.string(StringOffset(0)).unwrap(), b"");
    let error = pdb.string(StringOffset(1)).unwrap_err();
    expect(&error, ErrorKind::InvalidFormat, MetadataStream::Strings);
    assert_eq!(location(&error).heap_offset(), Some(1));
    expect(
        &pdb.string(StringOffset(4)).unwrap_err(),
        ErrorKind::InvalidReference,
        MetadataStream::Strings,
    );

    let error = pdb.blob(BlobOffset(1)).unwrap_err();
    expect(&error, ErrorKind::InvalidFormat, MetadataStream::Blob);
    assert_eq!(error.context(), Some("blob ends after the heap"));
    let error = pdb.blob(BlobOffset(6)).unwrap_err();
    assert_eq!(
        error.context(),
        Some("blob length prefix is not a compressed integer")
    );
    let error = pdb.blob(BlobOffset(7)).unwrap_err();
    assert_eq!(
        error.context(),
        Some("blob length prefix ends after the heap")
    );
    assert_eq!(pdb.user_string(UserStringOffset(1)).unwrap(), [b'a', 0, 1]);
    expect(
        &pdb.guid(GuidIndex(3)).unwrap_err(),
        ErrorKind::InvalidReference,
        MetadataStream::Guid,
    );

    let bytes = Builder::default().build();
    let options = ReadOptions::default().with_max_value_bytes(2);
    let pdb = Pdb::with_options(bytes.as_slice(), options).unwrap();
    assert_eq!(
        pdb.blob(BlobOffset(1)).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    assert_eq!(
        pdb.string(StringOffset(1)).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );

    let options = ReadOptions::default().with_max_stream_bytes(4);
    let pdb = Pdb::with_options(bytes.as_slice(), options).unwrap();
    assert_eq!(
        pdb.stream_bytes(MetadataStream::Pdb).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
}

#[test]
fn long_strings_are_read_across_chunks() {
    let mut strings = vec![0];
    strings.extend(core::iter::repeat_n(b's', 200));
    strings.push(0);
    let builder = Builder {
        strings: Some(strings),
        ..Builder::default()
    };
    let bytes = builder.build();
    let pdb = Pdb::parse(&bytes).unwrap();
    assert_eq!(pdb.string(StringOffset(1)).unwrap(), vec![b's'; 200]);
    assert_eq!(pdb.string(StringOffset(150)).unwrap(), vec![b's'; 51]);
}
