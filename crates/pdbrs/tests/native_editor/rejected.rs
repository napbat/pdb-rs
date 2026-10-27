//! Rejected edits.
//!
//! Each rejected plan writes nothing. After the rejection, the editor stays
//! usable: when the test undoes the change, the plan succeeds.

use pdbrs::io::ReadAt;
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::id_record::{FunctionId, IdRecord, StringId};
use pdbrs::native::codeview::index::{IdIndex, ModuleIndex, TypeIndex};
use pdbrs::native::codeview::symbol::{S_PROCREF, Symbol};
use pdbrs::native::codeview::type_record::{ArgumentList, TypeLeaf};
use pdbrs::native::editor::ModuleSymbol;
use pdbrs::native::msf::StreamId;
use pdbrs::native::{EditOptions, IdentityPolicy, IndexSpace, Pdb, PdbEditor};
use pdbrs::{Bytes, Error, ErrorKind};

use crate::{build, forward, output, rename_public, stream, synthetic};

/// The `T_INT4` primitive type.
const T_INT4: TypeIndex = TypeIndex(0x74);
/// The bytes of a stream that no other stream names.
const OPAQUE: &[u8] = b"opaque data";

/// Plan `editor` and return the error. Under `std`, also check that a plan
/// and a write to the file `name` write no file.
fn rejected<S: ReadAt>(editor: &PdbEditor<'_, S>, name: &str) -> Error {
    let error = editor
        .plan(EditOptions::default())
        .expect_err("rejected plan");
    #[cfg(feature = "std")]
    {
        let path = std::env::temp_dir().join(format!(
            "pdbrs-native-editor-{}-{name}.pdb",
            std::process::id()
        ));
        let result = editor
            .plan(EditOptions::default())
            .and_then(|plan| plan.write_file(&path));
        assert_eq!(result.expect_err("rejected write").kind(), error.kind());
        assert!(!path.exists());
    }
    #[cfg(not(feature = "std"))]
    let _ = name;
    error
}

/// Return the stream of the location of `error`.
fn stream_of(error: &Error) -> Option<StreamId> {
    error.stream_location().map(|(stream, _)| stream)
}

#[test]
fn opaque_stream_rejects_moved_symbol_records() {
    let source = build(|editor| {
        editor
            .unnamed_streams_mut()?
            .push(OPAQUE.to_vec())
            .map(|_| ())
    });
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    // The new layout writes the streams without a name last.
    let opaque = pdb.container().stream_count() - 1;
    assert_eq!(stream(&pdb, opaque), Some(OPAQUE.to_vec()));
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    // A longer name moves the records after the public symbol.
    rename_public(&mut editor, b"sum", b"sum_with_a_longer_name");
    let error = rejected(&editor, "opaque");
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit, "{error}");
    assert_eq!(error.index_space(), Some(IndexSpace::SymbolOffset));
    assert_eq!(stream_of(&error), Some(StreamId(opaque)));

    rename_public(&mut editor, b"sum_with_a_longer_name", b"sum");
    let edited = output(&editor);
    let edited = Pdb::parse(edited.as_slice()).expect("output parses");
    assert_eq!(stream(&edited, opaque), Some(OPAQUE.to_vec()));
}

#[test]
fn removed_procedure_of_a_reference_record_is_rejected() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let symbol_stream = pdb
        .debug_info()
        .expect("DBI")
        .expect("DBI present")
        .header()
        .symbol_stream
        .expect("symbol record stream");
    let records = pdb
        .symbol_records()
        .expect("symbol records")
        .expect("present");
    let is_sum = |name: Option<&Bytes<'_>>| name.map(Bytes::as_bytes) == Some(b"sum".as_slice());
    let reference = Records::new(records)
        .map(|record| record.expect("framed record"))
        .find(|record| {
            record.kind == S_PROCREF.0
                && is_sum(Symbol::decode(*record).expect("decoded").data().name())
        })
        .expect("S_PROCREF record of sum")
        .offset;

    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    let symbols = editor
        .module_mut(ModuleIndex(0))
        .expect("module")
        .symbols_mut()
        .expect("scope tree");
    let position = symbols
        .iter()
        .position(|symbol| match symbol {
            ModuleSymbol::Scope(scope) => is_sum(scope.opener.data().name()),
            ModuleSymbol::Record(_) => false,
        })
        .expect("procedure sum");
    let removed = symbols.remove(position);
    let error = rejected(&editor, "reference");
    assert_eq!(error.kind(), ErrorKind::InvalidReference, "{error}");
    assert_eq!(
        error.stream_location(),
        Some((symbol_stream, u64::from(reference)))
    );

    editor
        .module_mut(ModuleIndex(0))
        .expect("module")
        .symbols_mut()
        .expect("scope tree")
        .insert(position, removed);
    let plan = editor
        .plan(EditOptions::default())
        .expect("plan after undo");
    assert_eq!(plan.report().changed().count(), 0);
}

/// Return an `LF_ARGLIST` record with the one argument `argument`.
fn argument_list(argument: TypeIndex) -> TypeLeaf<'static> {
    TypeLeaf::ArgumentList(
        ArgumentList::parse(argument.0.to_le_bytes().to_vec()).expect("argument list"),
    )
}

/// Check that `error` rejects a forward reference in `space`.
fn assert_forward(error: &Error, space: IndexSpace) {
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit, "{error}");
    assert_eq!(error.index_space(), Some(space));
}

#[test]
fn forward_type_references_are_rejected() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");

    // A record that refers to a later record.
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let types = editor.types_mut().expect("types");
    let end = types.end().expect("end");
    let first = types
        .push(argument_list(TypeIndex(end.0 + 1)))
        .expect("first");
    types.push(forward(b"Later")).expect("second");
    assert_forward(&rejected(&editor, "later-type"), IndexSpace::TypeIndex);
    *editor
        .types_mut()
        .expect("types")
        .get_mut(first)
        .expect("first record") = argument_list(T_INT4);
    output(&editor);

    // A record that refers to itself.
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let types = editor.types_mut().expect("types");
    let end = types.end().expect("end");
    let record = types.push(argument_list(end)).expect("record");
    assert_eq!(record, end);
    assert_forward(&rejected(&editor, "self-type"), IndexSpace::TypeIndex);
    *editor
        .types_mut()
        .expect("types")
        .get_mut(record)
        .expect("record") = argument_list(T_INT4);
    output(&editor);
}

/// Return an `LF_FUNC_ID` record of `sum` with the scope `scope`.
fn function_id(scope: Option<IdIndex>) -> IdRecord<'static> {
    let mut function = FunctionId::default();
    function.scope = scope;
    // The `LF_PROCEDURE` record of `sum`.
    function.function_type = TypeIndex(0x1003);
    function.name = Bytes::new(b"scoped");
    function.decorated_name_hash = None;
    IdRecord::Function(function)
}

#[test]
fn forward_id_references_are_rejected() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let ids = editor.ids_mut().expect("IDs");
    let end = ids.end().expect("end");
    let function = ids
        .push(function_id(Some(IdIndex(end.0 + 1))))
        .expect("function");
    let mut string = StringId::default();
    string.substrings = None;
    string.string = Bytes::new(b"scope");
    ids.push(IdRecord::String(string)).expect("string");
    assert_forward(&rejected(&editor, "later-id"), IndexSpace::IdIndex);
    *editor
        .ids_mut()
        .expect("IDs")
        .get_mut(function)
        .expect("function record") = function_id(None);
    output(&editor);
}
