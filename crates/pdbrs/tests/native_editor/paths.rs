//! The source path remap of #46: `remap_source_paths`.
//!
//! The synthetic program has its paths below `C:\synthetic\`: the file checksum name in
//! `/names`, the DBI source file name, the module and object file names and `S_OBJNAME`. The
//! tests add a `srcsrv` stream with the same source file.

use pdbrs::io::ReadAt;
use pdbrs::native::codeview::index::ModuleIndex;
use pdbrs::native::lines::{LineSubsection, ModuleLines};
use pdbrs::native::msf::StreamId;
use pdbrs::native::{EditOptions, IdentityPolicy, Pdb, PdbEditor};
use pdbrs::{Error, ErrorKind};

use crate::opaque::opaque_module;
use crate::{build, output, stream};

/// The old directory.
const OLD: &[u8] = b"C:\\synthetic\\";
/// A `srcsrv` stream with the source file of the synthetic program.
const SOURCE_SERVER: &[u8] = b"SRCSRV: ini ------------------------------------------------\r\n\
VERSION=2\r\n\
SRCSRV: variables ------------------------------------------\r\n\
SRCSRVTRG=%targ%\\%var2%\r\n\
SRCSRV: source files ---------------------------------------\r\n\
C:\\synthetic\\synthetic.c*synthetic.c\r\n\
SRCSRV: end ------------------------------------------------\r\n";

/// Replace the prefix `C:\synthetic\` with `/_/`.
fn remap(path: &[u8]) -> Option<Vec<u8>> {
    path.strip_prefix(OLD)
        .map(|rest| [b"/_/".as_slice(), rest].concat())
}

/// Return the synthetic program with a `srcsrv` stream.
fn with_source_server() -> Vec<u8> {
    build(|editor| {
        editor
            .named_streams_mut()?
            .insert(b"srcsrv", SOURCE_SERVER.to_vec())
    })
}

/// Return `true` if `bytes` contains `part`.
fn contains(bytes: &[u8], part: &[u8]) -> bool {
    bytes.windows(part.len()).any(|window| window == part)
}

/// Check the paths of the remapped PDB `bytes`.
fn assert_remapped(bytes: &[u8]) {
    let pdb = Pdb::parse(bytes).expect("output parses");
    for id in 0..pdb.container().stream_count() {
        if let Some(data) = stream(&pdb, id) {
            assert!(!contains(&data, OLD), "stream {id} keeps the old directory");
        }
    }
    let names = pdb.names().expect("/names").expect("/names present");
    let module = pdb
        .module(ModuleIndex(0))
        .expect("module")
        .expect("module stream");
    let lines = ModuleLines::parse(&module).expect("lines");
    let checksums: Vec<Vec<u8>> = lines
        .subsections()
        .iter()
        .filter_map(|subsection| match &subsection.data {
            LineSubsection::FileChecksums(checksums) => Some(
                checksums
                    .iter()
                    .map(|entry| names.get(entry.name_offset).expect("name").to_vec())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(checksums, [b"/_/synthetic.c".to_vec()]);
    let dbi = pdb.debug_info().expect("DBI").expect("DBI present");
    let (_, record) = dbi.modules().next().expect("module").expect("record");
    assert_eq!(record.name, b"/_/synthetic.obj");
    assert_eq!(record.object_file, b"/_/synthetic.obj");
    let info = dbi.source_info().expect("source info").expect("present");
    let files: Vec<Vec<u8>> = info
        .file_names(&info.modules[0])
        .map(|name| name.expect("file name").to_vec())
        .collect();
    assert_eq!(files, [b"/_/synthetic.c".to_vec()]);
    let server = pdb.info().named_stream(b"srcsrv").expect("srcsrv");
    let server = stream(&pdb, server.0).expect("srcsrv bytes");
    assert!(contains(&server, b"\r\n/_/synthetic.c*synthetic.c\r\n"));
}

#[test]
fn remap_writes_new_paths_and_builds_names_again() {
    let source = with_source_server();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    let mut seen = Vec::new();
    let changed = editor
        .remap_source_paths(|path| {
            seen.push(path.to_vec());
            remap(path)
        })
        .expect("remap");
    // The source file and the object file.
    assert_eq!(changed, 2);
    let mut sorted = seen.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(seen, sorted, "each distinct path once, in byte order");
    assert_remapped(&output(&editor));

    // The new layout gives the same paths.
    let mut rebuilt = pdb.edit(IdentityPolicy::Preserve);
    rebuilt.decode_all().expect("decode_all");
    rebuilt.remap_source_paths(remap).expect("remap");
    assert_remapped(&output(&rebuilt));
}

#[test]
fn remap_that_keeps_every_path_gives_the_source_bytes() {
    let source = with_source_server();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    assert_eq!(editor.remap_source_paths(|_| None).expect("remap"), 0);
    let plan = editor.plan(EditOptions::default()).expect("plan");
    assert_eq!(plan.report().changed().count(), 0);
    let bytes = plan.to_vec().expect("output");
    let copy = Pdb::parse(bytes.as_slice()).expect("output parses");
    // Stream 0 (the previous stream directory) is empty in each plan output.
    for id in 1..pdb.container().stream_count() {
        assert_eq!(stream(&copy, id), stream(&pdb, id), "stream {id}");
    }
}

/// Apply a remap with the new path `new` to an editor with a Source Link change. Check that
/// it fails with `kind` and that the output stays the same.
fn rejected(pdb: &Pdb<&[u8]>, kind: ErrorKind, new: &[u8]) {
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor
        .set_source_link(br#"{"documents":{}}"#.to_vec())
        .expect("Source Link");
    let before = output(&editor);
    let error = editor
        .remap_source_paths(|path| remap(path).map(|_| new.to_vec()))
        .expect_err("rejected remap");
    assert_eq!(error.kind(), kind, "{error}");
    assert_eq!(output(&editor), before, "the model changed: {error}");
}

#[test]
fn an_invalid_new_path_leaves_the_model_unchanged() {
    let source = with_source_server();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    rejected(&pdb, ErrorKind::InvalidFormat, b"");
    rejected(&pdb, ErrorKind::InvalidFormat, b"/_/\0synthetic.c");
    // `S_OBJNAME` with this path is longer than its `u16` length field allows.
    rejected(&pdb, ErrorKind::LimitExceeded, &vec![b'p'; 0x1_0000]);
    // A `*` makes another first field of the `srcsrv` line.
    rejected(&pdb, ErrorKind::InvalidFormat, b"/_/a*b.c");
}

/// Check that `error` rejects opaque data in the stream `id`.
fn assert_blocked(error: &Error, id: StreamId) {
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit, "{error}");
    assert_eq!(error.stream_location().map(|(stream, _)| stream), Some(id));
}

/// Remap the paths of `pdb` and return the plan error.
fn plan_error<S: ReadAt>(editor: &mut PdbEditor<'_, S>) -> Error {
    editor.remap_source_paths(remap).expect("remap");
    editor
        .plan(EditOptions::default())
        .expect_err("the plan rejects opaque data")
}

#[test]
fn opaque_data_blocks_the_plan_and_names_the_stream() {
    // A module whose symbols do not form a scope tree can hold paths.
    let (source, module) = opaque_module();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    assert_blocked(&plan_error(&mut pdb.edit(IdentityPolicy::Preserve)), module);

    // A stream that no other stream names can hold paths and /names offsets.
    let opaque = b"opaque bytes";
    let source = build(|editor| {
        editor.unnamed_streams_mut()?.push(opaque.to_vec())?;
        Ok(())
    });
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let added = (0..pdb.container().stream_count())
        .find(|&id| stream(&pdb, id).as_deref() == Some(opaque.as_slice()))
        .map(StreamId)
        .expect("the opaque stream");
    assert_blocked(&plan_error(&mut pdb.edit(IdentityPolicy::Preserve)), added);

    // Without a remap, the PDB plans.
    pdb.edit(IdentityPolicy::Preserve)
        .plan(EditOptions::default())
        .expect("plan without a remap");
}
