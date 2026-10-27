//! The editor: unchanged output, kept heap entries, edits and unknown custom debug information.

use pdb_rs::ErrorKind;
use pdb_rs::io::ReadAt;
use pdb_rs::portable::custom::{CustomDebugKind, CustomDebugParent};
use pdb_rs::portable::debug::{
    LocalConstantHandle, LocalScopeHandle, SequencePoint, SequencePoints,
};
use pdb_rs::portable::metadata::{
    CustomDebugInformationRow, DocumentId, LocalConstantId, MetadataStream,
    MethodDebugInformationId, MethodDefId, TableId,
};
use pdb_rs::portable::{EditOptions, IdentityPolicy, Pdb, PdbEditor};

use crate::build::{UNKNOWN, constant, information, point, sample, scope, write};
use crate::{location, open};

/// Return the handle of the first local scope in row order of an editor.
fn first_scope<S: ReadAt>(editor: &mut PdbEditor<'_, S>) -> LocalScopeHandle {
    editor
        .local_scopes_mut()
        .unwrap()
        .iter()
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
pub(crate) fn constants(pdb: &Pdb<&[u8]>) -> Vec<Vec<Vec<u8>>> {
    pdb.local_scopes()
        .map(|scope| {
            scope
                .unwrap()
                .1
                .constants
                .iter()
                .map(|constant| constant.name.to_vec())
                .collect()
        })
        .collect()
}

/// Return the value of each custom debug information record in row order.
fn values(pdb: &Pdb<&[u8]>) -> Vec<Vec<u8>> {
    pdb.custom_debug_informations()
        .map(|information| information.unwrap().1.value.into_vec())
        .collect()
}

#[test]
fn edit_without_a_change_keeps_the_bytes() {
    let bytes = write(&sample());
    let source = Pdb::parse(&bytes).unwrap();
    let output = write(&source.edit(IdentityPolicy::ContentDerived));
    assert_eq!(output, bytes);
}

#[test]
fn edit_keeps_the_source_heap_entries() {
    let bytes = write(&sample());
    let source = Pdb::parse(&bytes).unwrap();
    let mut editor = source.edit(IdentityPolicy::ContentDerived);
    editor
        .documents_mut()
        .unwrap()
        .get_mut(DocumentId(2))
        .unwrap()
        .name = b"/src/Renamed.cs".into();
    let (_, last) = editor
        .local_scopes_mut()
        .unwrap()
        .iter_mut()
        .find(|(_, scope)| scope.method == MethodDefId(3))
        .unwrap();
    last.constants.push(constant(b"Added", 7));
    for information in editor.methods_mut().unwrap().values_mut() {
        let decoded = information.decode().unwrap().unwrap();
        let (signature, form) = (decoded.local_signature(), decoded.local_signature_form());
        let mut points: Vec<SequencePoint> = decoded.map(Result::unwrap).collect();
        for point in &mut points {
            if let Some(span) = &mut point.span {
                span.start_line += 100;
                span.end_line += 100;
            }
        }
        *information = SequencePoints::encode_parts(signature, form, &points).unwrap();
    }
    let output = write(&editor);
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
        edited.document(DocumentId(2)).unwrap().name,
        b"/src/Renamed.cs"
    );
    assert_eq!(
        constants(&edited),
        [vec![], vec![b"Pi".to_vec()], vec![b"Added".to_vec()]]
    );
    let method = edited
        .method_debug_information(MethodDebugInformationId(1))
        .unwrap();
    let points: Vec<_> = method
        .decode()
        .unwrap()
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        points,
        [point(0, 1, 110), point(4, 2, 103), point(9, 1, 112)]
    );
    assert_eq!(values(&edited).len(), 7);
}

#[test]
fn edit_that_moves_rows_fails_with_unknown_custom_debug_information() {
    let bytes = write(&sample());
    let source = Pdb::parse(&bytes).unwrap();
    let mut editor = source.edit(IdentityPolicy::ContentDerived);
    let outer = first_scope(&mut editor);
    // The new constant comes before "Pi", so "Pi" moves from row 1 to row 2.
    editor
        .local_scopes_mut()
        .unwrap()
        .get_mut(outer)
        .unwrap()
        .constants
        .push(constant(b"First", 1));
    let error = editor.plan(EditOptions::default()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
    let location = location(&error);
    assert_eq!(location.table(), Some(TableId::CUSTOM_DEBUG_INFORMATION));
    assert_eq!(location.column(), Some("Value"));

    // Without the unknown values, the edit succeeds.
    let custom = editor.custom_debug_information_mut().unwrap();
    let handles: Vec<_> = custom.iter().map(|(handle, _)| handle).collect();
    for handle in handles {
        custom.remove(handle).unwrap();
    }
    assert!(custom.is_empty());
    let output = write(&editor);
    let edited = open(&output);
    assert_eq!(
        constants(&edited),
        [vec![b"First".to_vec()], vec![b"Pi".to_vec()], vec![]]
    );
    assert_eq!(edited.custom_debug_informations().count(), 0);
}

/// Return the parent token of the first `CustomDebugInformation` row.
fn first_parent(pdb: &Pdb<&[u8]>) -> pdb_rs::portable::metadata::MetadataToken {
    pdb.rows::<CustomDebugInformationRow>()
        .next()
        .unwrap()
        .unwrap()
        .1
        .parent
}

#[test]
fn edit_that_moves_rows_rewrites_known_custom_debug_information_parents() {
    // A source with the constant "Pi" (row 1) in the scope of method 2, and its dynamic
    // local flags: a known kind without row references in its value.
    let mut build = PdbEditor::new(*sample().external(), IdentityPolicy::ContentDerived);
    let scopes = build.local_scopes_mut().unwrap();
    scopes.push(scope(1, 0, 10)).unwrap();
    let mut later = scope(2, 0, 10);
    later.constants.push(constant(b"Pi", 3));
    let later = scopes.push(later).unwrap();
    build
        .custom_debug_information_mut()
        .unwrap()
        .push(information(
            CustomDebugParent::LocalConstant(LocalConstantHandle {
                scope: later,
                index: 0,
            }),
            CustomDebugKind::DYNAMIC_LOCAL_VARIABLES,
            vec![0x01],
        ))
        .unwrap();
    let bytes = write(&build);
    assert_eq!(
        first_parent(&open(&bytes)),
        LocalConstantId(1).token().unwrap()
    );

    // A constant in the scope of method 1 moves "Pi" to row 2. The parent follows it.
    let source = Pdb::parse(&bytes).unwrap();
    let mut editor = source.edit(IdentityPolicy::ContentDerived);
    let first = first_scope(&mut editor);
    editor
        .local_scopes_mut()
        .unwrap()
        .get_mut(first)
        .unwrap()
        .constants
        .push(constant(b"First", 1));
    let output = write(&editor);
    let edited = open(&output);
    assert_eq!(first_parent(&edited), LocalConstantId(2).token().unwrap());
    assert_eq!(values(&edited), [vec![0x01]]);
    assert_eq!(
        edited.local_constant(LocalConstantId(2)).unwrap().name,
        b"Pi"
    );
}

#[test]
fn edit_that_keeps_rows_keeps_unknown_custom_debug_information() {
    let bytes = write(&sample());
    let source = Pdb::parse(&bytes).unwrap();
    let mut editor = source.edit(IdentityPolicy::ContentDerived);
    // A scope of method 4 comes after all source scopes, so no source row moves.
    let mut added = scope(4, 0, 2);
    added.constants.push(constant(b"Last", 9));
    let added = editor.local_scopes_mut().unwrap().push(added).unwrap();
    editor
        .custom_debug_information_mut()
        .unwrap()
        .push(information(
            CustomDebugParent::LocalScope(added),
            UNKNOWN,
            b"new".to_vec(),
        ))
        .unwrap();
    let output = write(&editor);
    let edited = open(&output);
    let mut values = values(&edited);
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
