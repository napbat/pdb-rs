//! The editing tasks `set_source_link`, `set_source_server`,
//! `remove_named_stream` and `rename_symbols`.

use pdbrs::io::ReadAt;
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::index::ModuleIndex;
use pdbrs::native::codeview::symbol::{
    ModuleSymbolTarget, ProcedureReference, S_PROCREF, Symbol, SymbolData,
};
use pdbrs::native::editor::ModuleSymbol;
use pdbrs::native::msf::StreamId;
use pdbrs::native::{EditOptions, IdentityPolicy, Pdb, PdbEditor};
use pdbrs::{Bytes, ErrorKind};

use crate::{build, output, program, stream, synthetic};

/// A Source Link file.
const SOURCE_LINK: &[u8] = br#"{"documents":{"C:\\src\\*":"https://example.invalid/src/*"}}"#;
/// A `srcsrv` stream.
const SOURCE_SERVER: &[u8] = b"SRCSRV: ini ------------------------------------------------\r\n\
VERSION=2\r\n\
SRCSRV: variables ------------------------------------------\r\n\
SRCSRVTRG=%targ%\\%var2%\r\n\
SRCSRV: source files ---------------------------------------\r\n\
C:\\src\\sum.cpp*sum.cpp\r\n\
SRCSRV: end ------------------------------------------------\r\n";

/// Return the bytes of the named stream `name` of `pdb`.
fn named<S: ReadAt>(pdb: &Pdb<S>, name: &[u8]) -> Option<Vec<u8>> {
    let id = pdb.info().named_stream(name)?;
    stream(pdb, id.0)
}

/// Return the name of each named record of the symbol record stream, and of
/// each procedure of module 0.
fn names<S: ReadAt>(pdb: &Pdb<S>) -> Vec<Vec<u8>> {
    let records = pdb
        .symbol_records()
        .expect("symbol records")
        .expect("present");
    let mut names: Vec<Vec<u8>> = Records::new(records)
        .filter_map(|record| {
            let symbol = Symbol::decode(record.expect("framed")).expect("decoded");
            symbol.name().map(|name| name.to_vec())
        })
        .collect();
    let module = pdb
        .module(ModuleIndex(0))
        .expect("module")
        .expect("module stream");
    for record in Records::new(module.symbol_data()) {
        let symbol = Symbol::decode(record.expect("framed")).expect("decoded");
        if matches!(symbol.data(), SymbolData::Procedure(_)) {
            names.push(symbol.name().expect("procedure name").to_vec());
        }
    }
    names
}

/// Check that each `S_PROCREF` record of `pdb` names a procedure with its
/// own name. Return the names.
fn reference_names<S: ReadAt>(pdb: &Pdb<S>) -> Vec<Vec<u8>> {
    let records = pdb
        .symbol_records()
        .expect("symbol records")
        .expect("present");
    let mut found = Vec::new();
    for record in Records::new(records) {
        let record = record.expect("framed");
        if record.kind != S_PROCREF.0 {
            continue;
        }
        let symbol = Symbol::decode(record).expect("decoded");
        let SymbolData::ProcedureReference(reference) = symbol.data() else {
            panic!("S_PROCREF without its fields");
        };
        let ModuleSymbolTarget::Position {
            module: Some(module),
            offset,
        } = reference.target
        else {
            panic!("S_PROCREF without a module");
        };
        let stream = pdb.module(module).expect("module").expect("module stream");
        let base = u32::try_from(stream.symbol_offset()).expect("offset");
        let target = Records::new(stream.symbol_data())
            .map(|record| record.expect("framed"))
            .find(|record| record.offset + base == offset.0)
            .expect("the reference names a record");
        let target = Symbol::decode(target).expect("decoded");
        assert_eq!(
            target.name(),
            Some(&reference.name),
            "reference and procedure"
        );
        found.push(reference.name.to_vec());
    }
    found
}

#[test]
fn named_stream_tasks_add_replace_and_remove_streams() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor
        .set_source_link(SOURCE_LINK.to_vec())
        .expect("Source Link");
    editor
        .set_source_server(SOURCE_SERVER.to_vec())
        .expect("srcsrv");
    let added = output(&editor);
    let added = Pdb::parse(added.as_slice()).expect("output parses");
    assert_eq!(named(&added, b"sourcelink").as_deref(), Some(SOURCE_LINK));
    assert_eq!(named(&added, b"srcsrv").as_deref(), Some(SOURCE_SERVER));
    assert_eq!(added.info().identity(), pdb.info().identity());

    let mut editor = added.edit(IdentityPolicy::Preserve);
    let link = b"{\"documents\":{}}".to_vec();
    editor.set_source_link(link.clone()).expect("replace");
    editor.remove_named_stream(b"srcsrv").expect("remove");
    let plan = editor.plan(EditOptions::default()).expect("plan");
    let source_link = added.info().named_stream(b"sourcelink").expect("stream");
    let server = added.info().named_stream(b"srcsrv").expect("stream");
    let mut changed: Vec<StreamId> = plan.report().changed().collect();
    changed.sort_unstable();
    let mut expected = vec![StreamId(1), source_link, server];
    expected.sort_unstable();
    assert_eq!(changed, expected);
    let edited = plan.to_vec().expect("output");
    let edited = Pdb::parse(edited.as_slice()).expect("output parses");
    assert_eq!(named(&edited, b"sourcelink"), Some(link));
    assert_eq!(edited.info().named_stream(b"srcsrv"), None);
    // The removed stream is a nil stream, so the stream IDs stay.
    assert_eq!(stream(&edited, server.0), None);
}

#[test]
fn rename_symbols_renames_a_procedure_and_its_reference_records() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let mut seen = Vec::new();
    let renamed = editor
        .rename_symbols(|name| {
            seen.push(name.to_vec());
            (name == b"sum").then(|| b"sum_renamed".to_vec())
        })
        .expect("rename");
    // The procedure, its S_PROCREF record and the public symbol.
    assert_eq!(renamed, 3);
    // Each distinct name once, in byte order.
    let mut sorted = seen.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(seen, sorted);
    assert!(seen.contains(&b"sum".to_vec()) && seen.contains(&b"g_value".to_vec()));

    let edited = output(&editor);
    let edited = Pdb::parse(edited.as_slice()).expect("output parses");
    let all = names(&edited);
    assert_eq!(all.iter().filter(|name| *name == b"sum_renamed").count(), 3);
    assert!(!all.contains(&b"sum".to_vec()));
    assert!(reference_names(&edited).contains(&b"sum_renamed".to_vec()));
}

#[test]
fn rename_symbols_follows_handles_of_a_build_and_a_rebuild() {
    let mut new = program::build().expect("the synthetic program builds");
    assert_eq!(
        new.rename_symbols(|name| (name == b"main").then(|| b"start".to_vec()))
            .expect("rename"),
        3
    );
    let first = output(&new);
    let first = Pdb::parse(first.as_slice()).expect("output parses");
    assert!(reference_names(&first).contains(&b"start".to_vec()));

    let mut decoded = first.edit(IdentityPolicy::IncrementAge);
    decoded.decode_all().expect("decode all");
    decoded
        .rename_symbols(|name| (name == b"start").then(|| b"main".to_vec()))
        .expect("rename");
    let second = output(&decoded);
    let second = Pdb::parse(second.as_slice()).expect("output parses");
    assert!(reference_names(&second).contains(&b"main".to_vec()));
    assert!(!names(&second).contains(&b"start".to_vec()));
}

#[test]
fn plan_accepts_a_reference_rename_only_with_its_procedure() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let mut globals = editor.globals_mut().expect("globals");
    let handle = globals.find(b"sum").expect("find").expect("S_PROCREF sum");
    *globals
        .get_mut(handle)
        .expect("record")
        .data_mut()
        .name_mut()
        .expect("name") = Bytes::from(b"total".to_vec());
    let error = editor.plan(EditOptions::default()).expect_err("rejected");
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit, "{error}");

    let procedure = editor
        .module_mut(ModuleIndex(0))
        .expect("module")
        .procedure_mut(b"sum")
        .expect("procedure");
    *procedure.opener.data_mut().name_mut().expect("name") = Bytes::from(b"total".to_vec());
    let edited = output(&editor);
    let edited = Pdb::parse(edited.as_slice()).expect("output parses");
    assert!(reference_names(&edited).contains(&b"total".to_vec()));
}

/// Build the synthetic program with a second `S_PROCREF` record of `sum` that
/// has the name `sum_alias`.
fn with_alias() -> Vec<u8> {
    build(|editor| {
        let module = editor.module_mut(ModuleIndex(0))?;
        let handle = module
            .symbols()?
            .iter()
            .find_map(|symbol| match symbol {
                ModuleSymbol::Scope(scope)
                    if scope.opener.data().name().map(Bytes::as_bytes) == Some(b"sum") =>
                {
                    module.handle(&scope.opener)
                }
                _ => None,
            })
            .expect("handle of sum");
        let mut reference = ProcedureReference::default();
        reference.target = ModuleSymbolTarget::Handle(handle);
        reference.name = Bytes::from(b"sum_alias".to_vec());
        editor
            .globals_mut()?
            .push(S_PROCREF, SymbolData::ProcedureReference(reference), 1)?;
        Ok(())
    })
}

/// Apply `task` to an editor with a Source Link change. Check that it fails
/// with `kind` and that the output stays the same.
fn rejected<'p>(
    pdb: &'p Pdb<&'p [u8]>,
    kind: ErrorKind,
    task: impl FnOnce(&mut PdbEditor<'p, &'p [u8]>) -> pdbrs::Result<()>,
) -> pdbrs::Error {
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .set_source_link(SOURCE_LINK.to_vec())
        .expect("Source Link");
    let before = output(&editor);
    let error = task(&mut editor).expect_err("rejected task");
    assert_eq!(error.kind(), kind, "{error}");
    assert_eq!(output(&editor), before, "the model changed: {error}");
    error
}

#[test]
fn a_task_with_invalid_input_leaves_the_model_unchanged() {
    let source = with_alias();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    rejected(&pdb, ErrorKind::InvalidFormat, |editor| {
        editor.set_source_link(b"[\"not an object\"]".to_vec())
    });
    rejected(&pdb, ErrorKind::InvalidFormat, |editor| {
        editor.set_source_link(vec![b'{', 0xFF, b'}'])
    });
    rejected(&pdb, ErrorKind::InvalidFormat, |editor| {
        editor.set_source_server(b"SRCSRV: ini --\r\nno end marker\r\n".to_vec())
    });
    rejected(&pdb, ErrorKind::MissingStream, |editor| {
        editor.remove_named_stream(b"srcsrv")
    });
    rejected(&pdb, ErrorKind::InvalidFormat, |editor| {
        editor.remove_named_stream(b"/names")
    });
    rejected(&pdb, ErrorKind::InvalidFormat, |editor| {
        editor
            .rename_symbols(|name| (name == b"g_value").then(|| b"g\0value".to_vec()))
            .map(|_| ())
    });
    rejected(&pdb, ErrorKind::LimitExceeded, |editor| {
        editor
            .rename_symbols(|name| (name == b"main").then(|| vec![b'm'; 0x1_0000]))
            .map(|_| ())
    });
    let symbol_stream = pdb
        .debug_info()
        .expect("DBI")
        .expect("DBI present")
        .header()
        .symbol_stream
        .expect("symbol record stream");
    // The alias names the procedure `sum`, which keeps its name.
    let error = rejected(&pdb, ErrorKind::UnsupportedEdit, |editor| {
        editor
            .rename_symbols(|name| (name == b"sum_alias").then(|| b"alias".to_vec()))
            .map(|_| ())
    });
    assert_eq!(
        error.stream_location().map(|(stream, _)| stream),
        Some(symbol_stream)
    );

    // A rename that gives the alias and the procedure the same new name
    // succeeds: the procedure, both references and the public symbol.
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let renamed = editor
        .rename_symbols(|name| match name {
            b"sum" | b"sum_alias" => Some(b"total".to_vec()),
            _ => None,
        })
        .expect("rename");
    assert_eq!(renamed, 4);
    let edited = output(&editor);
    let edited = Pdb::parse(edited.as_slice()).expect("output parses");
    let references = reference_names(&edited);
    assert_eq!(
        references
            .iter()
            .filter(|name| name.as_slice() == b"total")
            .count(),
        2
    );
}
