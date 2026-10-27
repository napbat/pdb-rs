//! Tests of the verification additions of `native::PdbEditor` (stage 6c of
//! #42):
//!
//! - `read_count.rs`: the reads of a named stream change and its output.
//! - `patched.rs`: the reads of a patched stream and its output (#48).
//! - `names.rs`: the name lookups of the editor (#48).
//! - `opaque.rs`: a module whose symbols do not form a scope tree.
//! - `rejected.rs`: the rejected edits and the editor after a rejection.
//! - `determinism.rs`: the output depends only on the final model.
//! - `views.rs`: borrowed and copied stream views (#48).
//!
//! The source PDB is the synthetic program of the `native_synthetic`
//! example. `layout.rs` parses the MSF stream directory without the reader
//! of the library.

#[path = "../../examples/native_synthetic/program.rs"]
mod program;

mod determinism;
mod layout;
mod names;
mod opaque;
mod patched;
mod read_count;
mod rejected;
mod views;

use pdbrs::Bytes;
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::codeview::numeric::{Integer, Numeric};
use pdbrs::native::codeview::type_record::{Class, ClassKind, ClassOptions, TypeLeaf};
use pdbrs::native::msf::StreamId;
use pdbrs::native::{EditOptions, Pdb, PdbEditor};

/// Build the synthetic program with the changes of `extra`.
fn build(extra: impl FnOnce(&mut PdbEditor<'static>) -> pdbrs::Result<()>) -> Vec<u8> {
    let mut editor = program::build().expect("the synthetic program builds");
    extra(&mut editor).expect("the extra records build");
    editor
        .plan(EditOptions::default())
        .and_then(|plan| plan.to_vec())
        .expect("the synthetic program plans")
}

/// Build the synthetic program.
fn synthetic() -> Vec<u8> {
    build(|_| Ok(()))
}

/// Return the bytes of stream `id`, or `None` for a nil or absent stream.
fn stream<S: pdbrs::io::ReadAt>(pdb: &Pdb<S>, id: u32) -> Option<Vec<u8>> {
    if id >= pdb.container().stream_count() {
        return None;
    }
    pdb.container()
        .stream(StreamId(id))
        .expect("stream directory")
        .map(|reader| reader.read_all(usize::MAX).expect("stream bytes"))
}

/// Plan the edits of `editor` and return the output.
fn output<S: pdbrs::io::ReadAt>(editor: &PdbEditor<'_, S>) -> Vec<u8> {
    editor
        .plan(EditOptions::default())
        .expect("plan")
        .to_vec()
        .expect("output")
}

/// Give the public symbol `old` the name `new`.
fn rename_public<S: pdbrs::io::ReadAt>(editor: &mut PdbEditor<'_, S>, old: &[u8], new: &[u8]) {
    let mut publics = editor.publics_mut().expect("publics");
    let handle = publics.find(old).expect("find").expect("public present");
    *publics
        .get_mut(handle)
        .expect("record of the handle")
        .data_mut()
        .name_mut()
        .expect("named record") = Bytes::from(new.to_vec());
}

/// Return a forward reference to a structure with the name `name`.
fn forward(name: &[u8]) -> TypeLeaf<'_> {
    let mut class = Class::new(ClassKind::Structure);
    class.member_count = 0;
    class.options = ClassOptions::FORWARD_REFERENCE;
    class.field_list = TypeIndex::NONE;
    class.derivation_list = TypeIndex::NONE;
    class.vtable_shape = TypeIndex::NONE;
    class.size = Numeric::from_integer(Integer::Unsigned(0));
    class.name = Bytes::new(name);
    class.unique_name = None;
    TypeLeaf::Class(class)
}
