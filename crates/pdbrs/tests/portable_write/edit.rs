//! The editor: unchanged output, kept heap entries, edits and unknown custom debug information.

use pdbrs::ErrorKind;
use pdbrs::portable::build::{CustomDebugParent, LocalScopeHandle};
use pdbrs::portable::custom::CustomDebugKind;
use pdbrs::portable::metadata::{
    DocumentId, LocalConstantId, MetadataStream, MethodDefId, TableId,
};
use pdbrs::portable::{IdentityPolicy, Pdb, PdbBuilder};

use crate::build::{UNKNOWN, constant, information, point, sample, scope};
use crate::{location, open};

/// Write a builder with the content-derived ID.
fn write(builder: &PdbBuilder) -> Vec<u8> {
    builder
        .plan(IdentityPolicy::ContentDerived)
        .unwrap()
        .to_vec()
        .unwrap()
}

/// Return the handle of the first local scope in row order of an editor.
fn first_scope(builder: &PdbBuilder) -> LocalScopeHandle {
    builder
        .local_scopes()
        .min_by_key(|(_, scope)| {
            (
                scope.method,
                scope.start_offset,
                core::cmp::Reverse(scope.length),
            )
        })
        .unwrap()
        .0
}

/// Return the names of the constants of each local scope in row order.
fn constants(pdb: &Pdb<&[u8]>) -> Vec<Vec<Vec<u8>>> {
    pdb.local_scopes()
        .map(|scope| {
            scope
                .unwrap()
                .constants()
                .map(|id| pdb.local_constant(id).unwrap().name().to_vec())
                .collect()
        })
        .collect()
}

#[test]
fn edit_without_a_change_keeps_the_bytes() {
    let bytes = write(&sample());
    let editor = Pdb::parse(&bytes).unwrap().edit().unwrap();
    let output = editor
        .plan(IdentityPolicy::ContentDerived)
        .unwrap()
        .to_vec()
        .unwrap();
    assert_eq!(output, bytes);
}

#[test]
fn edit_keeps_the_source_heap_entries() {
    let bytes = write(&sample());
    let source = Pdb::parse(&bytes).unwrap();
    let mut editor = source.edit().unwrap();
    editor.document_mut(DocumentId(2)).unwrap().name = b"/src/Renamed.cs".to_vec();
    let last = editor
        .local_scopes()
        .find(|(_, scope)| scope.method == MethodDefId(3))
        .unwrap()
        .0;
    editor
        .add_local_constant(last, constant(b"Added", 7))
        .unwrap();
    for (_, method) in editor.methods_mut() {
        for point in &mut method.sequence_points {
            if let Some(span) = &mut point.span {
                span.start_line += 100;
                span.end_line += 100;
            }
        }
    }
    let output = editor
        .plan(IdentityPolicy::ContentDerived)
        .unwrap()
        .to_vec()
        .unwrap();
    let edited = open(&output);
    for stream in [
        MetadataStream::Strings,
        MetadataStream::Blob,
        MetadataStream::Guid,
        MetadataStream::UserStrings,
    ] {
        let old = source.stream_bytes(stream).unwrap().unwrap();
        let new = edited.stream_bytes(stream).unwrap().unwrap();
        assert!(
            new.starts_with(&old),
            "{stream:?} does not start with the source heap"
        );
    }
    assert_eq!(
        edited.document(DocumentId(2)).unwrap().name(),
        b"/src/Renamed.cs"
    );
    assert_eq!(
        constants(&edited),
        [vec![], vec![b"Pi".to_vec()], vec![b"Added".to_vec()]]
    );
    let method = edited
        .method_debug_information(pdbrs::portable::metadata::MethodDebugInformationId(1))
        .unwrap();
    let points: Vec<_> = method
        .sequence_points()
        .unwrap()
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        points,
        [point(0, 1, 110), point(4, 2, 103), point(9, 1, 112)]
    );
    let values: Vec<Vec<u8>> = edited
        .custom_debug_informations()
        .map(|row| row.unwrap().value().to_vec())
        .collect();
    assert_eq!(values.len(), 7);
}

#[test]
fn edit_that_moves_rows_fails_with_unknown_custom_debug_information() {
    let bytes = write(&sample());
    let mut editor = Pdb::parse(&bytes).unwrap().edit().unwrap();
    let outer = first_scope(&editor);
    // The new constant comes before "Pi", so "Pi" moves from row 1 to row 2.
    editor
        .add_local_constant(outer, constant(b"First", 1))
        .unwrap();
    let error = editor.plan(IdentityPolicy::ContentDerived).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
    let location = location(&error);
    assert_eq!(location.table(), Some(TableId::CUSTOM_DEBUG_INFORMATION));
    assert_eq!(location.column(), Some("Value"));

    // Without the unknown values, the edit succeeds.
    let handles: Vec<_> = editor
        .custom_debug_informations()
        .map(|(handle, _)| handle)
        .collect();
    for handle in handles {
        editor.remove_custom_debug_information(handle).unwrap();
    }
    let output = editor
        .plan(IdentityPolicy::ContentDerived)
        .unwrap()
        .to_vec()
        .unwrap();
    let edited = open(&output);
    assert_eq!(
        constants(&edited),
        [vec![b"First".to_vec()], vec![b"Pi".to_vec()], vec![]]
    );
    assert_eq!(edited.custom_debug_informations().count(), 0);
}

#[test]
fn edit_that_moves_rows_rewrites_known_custom_debug_information_parents() {
    // A source with the constant "Pi" (row 1) in the scope of method 2, and its dynamic
    // local flags: a known kind without row references in its value.
    let mut source = PdbBuilder::new(*sample().external());
    source.add_local_scope(scope(1, 0, 10)).unwrap();
    let later = source.add_local_scope(scope(2, 0, 10)).unwrap();
    let pi = source
        .add_local_constant(later, constant(b"Pi", 3))
        .unwrap();
    source
        .add_custom_debug_information(information(
            CustomDebugParent::LocalConstant(pi),
            CustomDebugKind::DYNAMIC_LOCAL_VARIABLES,
            vec![0x01],
        ))
        .unwrap();
    let bytes = write(&source);
    assert_eq!(
        open(&bytes)
            .custom_debug_informations()
            .next()
            .unwrap()
            .unwrap()
            .parent(),
        LocalConstantId(1).token().unwrap()
    );

    // A constant in the scope of method 1 moves "Pi" to row 2. The parent follows it.
    let mut editor = Pdb::parse(&bytes).unwrap().edit().unwrap();
    let first = first_scope(&editor);
    editor
        .add_local_constant(first, constant(b"First", 1))
        .unwrap();
    let output = editor
        .plan(IdentityPolicy::ContentDerived)
        .unwrap()
        .to_vec()
        .unwrap();
    let edited = open(&output);
    let row = edited.custom_debug_informations().next().unwrap().unwrap();
    assert_eq!(row.parent(), LocalConstantId(2).token().unwrap());
    assert_eq!(row.value(), [0x01]);
    assert_eq!(
        edited.local_constant(LocalConstantId(2)).unwrap().name(),
        b"Pi"
    );
}

#[test]
fn edit_that_keeps_rows_keeps_unknown_custom_debug_information() {
    let bytes = write(&sample());
    let mut editor = Pdb::parse(&bytes).unwrap().edit().unwrap();
    // A scope of method 4 comes after all source scopes, so no source row moves.
    let added = editor.add_local_scope(scope(4, 0, 2)).unwrap();
    editor
        .add_local_constant(added, constant(b"Last", 9))
        .unwrap();
    editor
        .add_custom_debug_information(information(
            CustomDebugParent::LocalScope(added),
            UNKNOWN,
            b"new".to_vec(),
        ))
        .unwrap();
    let output = editor
        .plan(IdentityPolicy::ContentDerived)
        .unwrap()
        .to_vec()
        .unwrap();
    let edited = open(&output);
    let mut values: Vec<Vec<u8>> = edited
        .custom_debug_informations()
        .map(|row| row.unwrap().value().to_vec())
        .collect();
    values.sort();
    let mut expected: Vec<Vec<u8>> = (1_u8..=7).map(|value| vec![value]).collect();
    expected.push(b"new".to_vec());
    expected.sort();
    assert_eq!(values, expected);
    assert_eq!(
        constants(&edited),
        [vec![], vec![b"Pi".to_vec()], vec![], vec![b"Last".to_vec()]]
    );
}
