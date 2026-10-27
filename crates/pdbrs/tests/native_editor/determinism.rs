//! The output depends only on the final model (S6c-4 of #42): the same
//! edits give the same bytes, and two call sequences with the same final
//! model give the same bytes.

use pdbrs::Bytes;
use pdbrs::io::ReadAt;
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::codeview::symbol::{S_UDT, SymbolData, UserDefinedType};
use pdbrs::native::{EditOptions, IdentityPolicy, Pdb, PdbEditor};

use crate::{forward, output, program, rename_public, synthetic};

/// Apply a set of edits to `editor`: a rename that moves records, a type
/// with a global record that names it, and a named stream.
fn edit<S: ReadAt>(editor: &mut PdbEditor<'_, S>) {
    rename_public(editor, b"sum", b"sum_with_a_longer_name");
    let added = editor
        .types_mut()
        .expect("types")
        .push(forward(b"Added"))
        .expect("type");
    let mut udt = UserDefinedType::default();
    udt.type_index = added;
    udt.name = Bytes::new(b"Added");
    editor
        .globals_mut()
        .expect("globals")
        .push(S_UDT, SymbolData::UserDefinedType(udt), 1)
        .expect("global");
    editor
        .named_streams_mut()
        .expect("named streams")
        .insert(b"extra", b"extra bytes".to_vec())
        .expect("named stream");
}

#[test]
fn same_edits_give_the_same_bytes() {
    // Two builds of the synthetic program.
    let source = synthetic();
    assert_eq!(synthetic(), source);
    let build = program::build().expect("build");
    let plan = build.plan(EditOptions::default()).expect("plan");
    assert_eq!(plan.to_vec().expect("output"), source);
    assert_eq!(plan.to_vec().expect("second output"), source);

    // The same edits on two editors of one source.
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut first = pdb.edit(IdentityPolicy::IncrementAge);
    edit(&mut first);
    let mut second = pdb.edit(IdentityPolicy::IncrementAge);
    edit(&mut second);
    let edited = output(&first);
    assert_ne!(edited, source);
    assert_eq!(output(&second), edited);
    // A second plan of the same model.
    assert_eq!(output(&first), edited);
}

#[test]
fn rename_in_one_step_or_two_gives_the_same_bytes() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut one = pdb.edit(IdentityPolicy::IncrementAge);
    rename_public(&mut one, b"sum", b"sum_c");
    let mut two = pdb.edit(IdentityPolicy::IncrementAge);
    rename_public(&mut two, b"sum", b"sum_b_with_a_longer_name");
    rename_public(&mut two, b"sum_b_with_a_longer_name", b"sum_c");
    assert_eq!(output(&two), output(&one));
}

#[test]
fn type_push_and_rename_commute() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let push = |editor: &mut PdbEditor<'_, &[u8]>| -> TypeIndex {
        editor
            .types_mut()
            .expect("types")
            .push(forward(b"Added"))
            .expect("type")
    };
    let mut first = pdb.edit(IdentityPolicy::IncrementAge);
    let pushed = push(&mut first);
    rename_public(&mut first, b"main", b"main_with_a_longer_name");
    let mut second = pdb.edit(IdentityPolicy::IncrementAge);
    rename_public(&mut second, b"main", b"main_with_a_longer_name");
    assert_eq!(push(&mut second), pushed);
    assert_eq!(output(&second), output(&first));
}

#[test]
fn named_stream_insert_change_and_removal_is_no_change() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut changed = pdb.edit(IdentityPolicy::Preserve);
    let named = changed.named_streams_mut().expect("named streams");
    named.insert(b"extra", b"first".to_vec()).expect("insert");
    named.insert(b"extra", b"second".to_vec()).expect("change");
    named.remove(b"extra").expect("remove");
    assert!(!named.contains(b"extra"));
    let unchanged = pdb.edit(IdentityPolicy::Preserve);
    assert_eq!(output(&changed), output(&unchanged));
}

#[test]
fn rebuild_twice_gives_the_same_bytes() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let rebuild = || {
        let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
        editor.decode_all().expect("decode_all");
        assert!(editor.is_new_layout());
        output(&editor)
    };
    let first = rebuild();
    assert_eq!(rebuild(), first);
    Pdb::parse(first.as_slice()).expect("rebuild parses");
}
