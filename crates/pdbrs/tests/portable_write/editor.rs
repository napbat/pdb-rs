//! One editor type for builds and edits: records, the identity policy and the plan report.

use pdbrs::portable::custom::CustomDebugParent;
use pdbrs::portable::debug::{LocalVariableHandle, ValidateOptions};
use pdbrs::portable::metadata::{
    DocumentId, IlOffset, LocalScopeId, MetadataStream, MethodDebugInformationId, MethodDefId,
};
use pdbrs::portable::{
    EditOptions, Identity, IdentityPolicy, Pdb, PdbEditor, RebuildReason, StreamChange,
};

use crate::build::{document, external, method, point, sample, scope, variable, write};
use crate::edit::constants;
use crate::{ID, open};

/// The streams of the output in file order.
const STREAMS: [MetadataStream; 6] = [
    MetadataStream::Pdb,
    MetadataStream::Tables,
    MetadataStream::Strings,
    MetadataStream::UserStrings,
    MetadataStream::Guid,
    MetadataStream::Blob,
];

/// Return the report of `change` for each stream, with `change` for each stream that is not
/// in `others`.
fn report(
    change: StreamChange,
    others: &[(MetadataStream, StreamChange)],
) -> Vec<(MetadataStream, StreamChange)> {
    STREAMS
        .into_iter()
        .map(|stream| {
            let change = others
                .iter()
                .find_map(|&(name, other)| (name == stream).then_some(other))
                .unwrap_or(change);
            (stream, change)
        })
        .collect()
}

/// Return the names of the documents in row order.
fn names(pdb: &Pdb<&[u8]>) -> Vec<Vec<u8>> {
    pdb.documents()
        .map(|document| document.unwrap().1.name.into_vec())
        .collect()
}

/// Return the method, the start, the length and the variable names of each scope in row
/// order.
fn scopes(pdb: &Pdb<&[u8]>) -> Vec<(u32, u32, u32, Vec<Vec<u8>>)> {
    pdb.local_scopes()
        .map(|scope| {
            let scope = scope.unwrap().1;
            let names = scope
                .variables
                .iter()
                .map(|variable| variable.name.to_vec())
                .collect();
            (scope.method.0, scope.start_offset.0, scope.length, names)
        })
        .collect()
}

#[test]
fn one_editor_builds_and_then_edits_a_pdb() {
    let mut build = PdbEditor::new(external(2), IdentityPolicy::ContentDerived);
    let id = build
        .documents_mut()
        .unwrap()
        .push(document(b"/src/A.cs", None))
        .unwrap();
    build
        .methods_mut()
        .unwrap()
        .insert(MethodDefId(1), method(0, &[point(0, 1, 3)]));
    let mut root = scope(1, 0, 10);
    root.variables.push(variable(b"a", 0));
    build.local_scopes_mut().unwrap().push(root).unwrap();
    let bytes = write(&build);
    let source = open(&bytes);
    assert_eq!(names(&source), [b"/src/A.cs".to_vec()]);
    assert_eq!(scopes(&source), [(1, 0, 10, vec![b"a".to_vec()])]);

    let mut editor = source.edit(IdentityPolicy::ContentDerived);
    editor.documents_mut().unwrap().get_mut(id).unwrap().name = b"/src/B.cs".into();
    let mut nested = scope(1, 2, 4);
    nested.variables.push(variable(b"x", 1));
    editor.local_scopes_mut().unwrap().push(nested).unwrap();
    let output = write(&editor);
    let edited = open(&output);
    edited.validate_debug(ValidateOptions::default()).unwrap();
    assert_eq!(names(&edited), [b"/src/B.cs".to_vec()]);
    assert_eq!(
        scopes(&edited),
        [
            (1, 0, 10, vec![b"a".to_vec()]),
            (1, 2, 4, vec![b"x".to_vec()])
        ]
    );
    let scope = edited.local_scope(LocalScopeId(2)).unwrap();
    assert_eq!(scope.end_offset(), Some(IlOffset(6)));
    assert_eq!(scope.variables[0].index, 1);
    let method = edited
        .method_debug_information(MethodDebugInformationId(1))
        .unwrap();
    assert_eq!(method.document, DocumentId(1));
    let points: Vec<_> = method
        .decode()
        .unwrap()
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(points, [point(0, 1, 3)]);
}

#[test]
fn a_build_takes_the_identity_policy_at_the_entry() {
    let id = Identity::from_bytes(ID);
    let mut build = PdbEditor::new(external(1), IdentityPolicy::Explicit(id));
    build
        .documents_mut()
        .unwrap()
        .push(document(b"/src/A.cs", None))
        .unwrap();
    let plan = build.plan(EditOptions::default()).unwrap();
    assert_eq!(plan.identity(), id);
    let bytes = plan.to_vec().unwrap();
    let pdb = open(&bytes);
    assert_eq!(pdb.pdb_stream().identity(), id);
    assert_eq!(pdb.pdb_stream().id(), &ID);
}

#[test]
fn an_edit_takes_the_identity_policy_at_the_entry() {
    let bytes = write(&sample());
    let source = open(&bytes);
    let identity = source.pdb_stream().identity();

    // The content-derived ID of the same content is the source ID.
    let plan = source
        .edit(IdentityPolicy::ContentDerived)
        .plan(EditOptions::default())
        .unwrap();
    assert_eq!(plan.identity(), identity);
    assert_eq!(plan.to_vec().unwrap(), bytes);

    // An explicit other ID changes only the ID in `#Pdb`.
    let other = Identity::from_bytes(ID);
    assert_ne!(other, identity);
    let plan = source
        .edit(IdentityPolicy::Explicit(other))
        .plan(EditOptions::default())
        .unwrap();
    assert_eq!(plan.identity(), other);
    assert_eq!(
        plan.report().streams(),
        report(
            StreamChange::Copied,
            &[(
                MetadataStream::Pdb,
                StreamChange::Rebuilt(RebuildReason::Identity)
            )]
        )
    );
    let output = plan.to_vec().unwrap();
    assert_eq!(output.len(), bytes.len());
    assert_eq!(open(&output).pdb_stream().identity(), other);
}

#[test]
fn the_report_gives_the_change_of_each_stream() {
    // A build has no source: each stream is new.
    let plan = sample().plan(EditOptions::default()).unwrap();
    assert_eq!(plan.report().streams(), report(StreamChange::Added, &[]));
    for stream in STREAMS {
        assert_eq!(plan.report().stream(stream), Some(StreamChange::Added));
    }

    let bytes = plan.to_vec().unwrap();
    let source = open(&bytes);
    let identity = source.pdb_stream().identity();

    // An edit without a change copies each stream.
    let plan = source
        .edit(IdentityPolicy::ContentDerived)
        .plan(EditOptions::default())
        .unwrap();
    assert_eq!(plan.report().streams(), report(StreamChange::Copied, &[]));

    // A new name adds name parts to `#Blob`, and the row gets the offset of the new name blob.
    let mut editor = source.edit(IdentityPolicy::Explicit(identity));
    editor
        .documents_mut()
        .unwrap()
        .get_mut(DocumentId(2))
        .unwrap()
        .name = b"/src/Renamed.cs".into();
    let plan = editor.plan(EditOptions::default()).unwrap();
    assert_eq!(
        plan.report().streams(),
        report(
            StreamChange::Copied,
            &[
                (MetadataStream::Tables, StreamChange::Edited),
                (MetadataStream::Blob, StreamChange::Edited)
            ]
        )
    );

    // With the content-derived ID, the new content also gives a new ID.
    let mut derived = source.edit(IdentityPolicy::ContentDerived);
    derived
        .documents_mut()
        .unwrap()
        .get_mut(DocumentId(2))
        .unwrap()
        .name = b"/src/Renamed.cs".into();
    let plan = derived.plan(EditOptions::default()).unwrap();
    assert_eq!(
        plan.report().stream(MetadataStream::Pdb),
        Some(StreamChange::Rebuilt(RebuildReason::Identity))
    );
    assert_ne!(plan.identity(), identity);
}

#[test]
fn an_edit_keeps_the_tables_that_it_does_not_decode() {
    let bytes = write(&sample());
    let source = open(&bytes);
    let mut editor = source.edit(IdentityPolicy::ContentDerived);
    // The first call decodes the documents of the source.
    let documents = editor.documents_mut().unwrap();
    assert_eq!(documents.len(), 2);
    assert_eq!(
        documents.get(DocumentId(1)).unwrap().name,
        b"C:\\src\\Program.cs"
    );
    documents.get_mut(DocumentId(1)).unwrap().name = b"C:\\src\\Main.cs".into();
    let output = write(&editor);
    let edited = open(&output);
    assert_eq!(
        names(&edited),
        [b"C:\\src\\Main.cs".to_vec(), b"/src/Other.cs".to_vec()]
    );

    // The tables without a `*_mut` call keep their records.
    assert_eq!(scopes(&edited), scopes(&source));
    assert_eq!(constants(&edited), constants(&source));
    let methods = |pdb: &Pdb<&[u8]>| -> Vec<_> {
        pdb.method_debug_informations()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(methods(&edited), methods(&source));
    let imports =
        |pdb: &Pdb<&[u8]>| -> Vec<_> { pdb.import_scopes().map(Result::unwrap).collect() };
    assert_eq!(imports(&edited), imports(&source));
    let machines =
        |pdb: &Pdb<&[u8]>| -> Vec<_> { pdb.state_machine_methods().map(Result::unwrap).collect() };
    assert_eq!(machines(&edited), machines(&source));
    let custom = |pdb: &Pdb<&[u8]>| -> Vec<_> {
        pdb.custom_debug_informations()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(custom(&edited), custom(&source));
}

#[test]
fn the_reader_gives_the_handle_of_a_local_variable_parent() {
    let bytes = write(&sample());
    let source = open(&bytes);
    // The sample gives the variable "inner" the custom debug information value 1.
    let (_, information) = source
        .custom_debug_informations()
        .map(Result::unwrap)
        .find(|(_, information)| information.value == [1])
        .unwrap();
    let CustomDebugParent::LocalVariable(handle) = information.parent else {
        panic!("unexpected parent {:?}", information.parent);
    };
    assert_eq!(handle.index, 0);

    // The editor of the source holds the scopes in row order, so the handle names the scope
    // that holds the variable.
    let mut editor = source.edit(IdentityPolicy::ContentDerived);
    let scopes = editor.local_scopes_mut().unwrap();
    let (scope, _) = scopes
        .iter()
        .find(|(_, scope)| {
            scope
                .variables
                .iter()
                .any(|variable| variable.name == b"inner")
        })
        .unwrap();
    assert_eq!(handle, LocalVariableHandle { scope, index: 0 });
    let held = scopes.get(handle.scope).unwrap();
    assert_eq!(held.variables[0].name, b"inner");
    assert_eq!(held.start_offset, IlOffset(2));
}
