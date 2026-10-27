//! Name lookups of the editor (#48): the source hash buckets and the
//! records with a changed name.

use pdbrs::native::codeview::symbol::{Public, SymbolData};
use pdbrs::native::{IdentityPolicy, Pdb, PdbEditor};
use pdbrs::{Bytes, ErrorKind};

use crate::synthetic;

/// Give the record `name` of the public index the name `new`.
fn rename(editor: &mut PdbEditor<'_, &[u8]>, name: &[u8], new: &[u8]) {
    let mut publics = editor.publics_mut().expect("publics");
    let handle = publics.find(name).expect("find").expect("record");
    *publics
        .get_mut(handle)
        .expect("record")
        .data_mut()
        .name_mut()
        .expect("named record") = Bytes::from(new.to_vec());
}

/// Return a public symbol with the name `name`.
fn named_public(name: &'static [u8]) -> Public<'static> {
    let mut public = Public::default();
    public.name = name.into();
    public
}

#[test]
fn find_uses_the_source_buckets_and_the_changed_names() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);

    // Source records: each index finds its own record of a shared name.
    let public = editor
        .publics_mut()
        .expect("publics")
        .find(b"g_value")
        .expect("find")
        .expect("public g_value");
    let global = editor
        .globals_mut()
        .expect("globals")
        .find(b"g_value")
        .expect("find")
        .expect("global g_value");
    assert_ne!(public, global);
    let publics = editor.publics_mut().expect("publics");
    assert!(matches!(
        publics
            .get(public)
            .map(pdbrs::native::editor::SymbolRecord::data),
        Some(SymbolData::Public(_))
    ));
    assert_eq!(publics.find(b"missing").expect("find"), None);

    // A new name: found at once, and also after the next change.
    rename(&mut editor, b"g_value", b"renamed");
    let mut publics = editor.publics_mut().expect("publics");
    assert_eq!(publics.find(b"renamed").expect("find"), Some(public));
    assert_eq!(publics.find(b"g_value").expect("find"), None);
    let main = publics.find(b"main").expect("find").expect("public main");
    assert!(publics.get_mut(main).is_some());
    assert_eq!(publics.find(b"renamed").expect("find"), Some(public));
    assert_eq!(publics.find(b"main").expect("find"), Some(main));
    let globals = editor.globals_mut().expect("globals");
    assert_eq!(globals.find(b"renamed").expect("find"), None);
    assert_eq!(globals.find(b"g_value").expect("find"), Some(global));

    // A pushed record: found in its own index only.
    let added = editor
        .publics_mut()
        .expect("publics")
        .push(named_public(b"added"))
        .expect("push");
    let found = editor.publics_mut().expect("publics").find(b"added");
    assert_eq!(found.expect("find"), Some(added));
    let found = editor.globals_mut().expect("globals").find(b"added");
    assert_eq!(found.expect("find"), None);

    // A removed record is not found.
    let mut publics = editor.publics_mut().expect("publics");
    publics.remove(public).expect("remove");
    assert_eq!(publics.find(b"renamed").expect("find"), None);

    // Two records with the same name.
    publics.push(named_public(b"twin")).expect("push");
    publics.push(named_public(b"twin")).expect("push");
    let error = publics.find(b"twin").expect_err("two records");
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
}

#[test]
fn find_after_decode_all_and_without_a_source() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor.decode_all().expect("decode all");
    let main = editor
        .publics_mut()
        .expect("publics")
        .find(b"main")
        .expect("find")
        .expect("public main");
    rename(&mut editor, b"main", b"main_renamed");
    let publics = editor.publics_mut().expect("publics");
    assert_eq!(publics.find(b"main").expect("find"), None);
    assert_eq!(publics.find(b"main_renamed").expect("find"), Some(main));

    let mut built = crate::program::build().expect("the synthetic program builds");
    let reference = built
        .globals_mut()
        .expect("globals")
        .find(b"sum")
        .expect("find")
        .expect("the procedure reference sum");
    let found = built.publics_mut().expect("publics").find(b"sum");
    let public = found.expect("find").expect("the public sum");
    assert_ne!(public, reference);
}
