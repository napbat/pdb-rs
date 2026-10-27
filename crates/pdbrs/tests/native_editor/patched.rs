//! The reads of a patched stream (#48).
//!
//! A rename of a global symbol moves the later records of the symbol record
//! stream. A module that the editor did not decode then gets only new
//! global reference offsets. The plan keeps such a module stream as the
//! source stream and patches, and reads the source stream when it writes.
//!
//! The opaque check of #42 reads each module stream while the plan checks a
//! move of symbol records. The plan does not keep these bytes.

use std::collections::{BTreeMap, BTreeSet};

use pdbrs::Bytes;
use pdbrs::native::codeview::index::ModuleIndex;
use pdbrs::native::{EditOptions, IdentityPolicy, IndexSpace, Pdb, StreamChange};

use crate::layout::Layout;
use crate::read_count::Counting;
use crate::{build, stream};

/// The global data record that the test renames. The `S_UDT` record
/// `Point` follows it in the symbol record stream.
const RENAMED: &[u8] = b"g_value";
/// The new name: longer than the old one, so that the later records move.
const NEW_NAME: &[u8] = b"g_value_with_a_much_longer_name";

/// Return the stream ID and the global reference region start of module 0.
fn module_zero<S: pdbrs::io::ReadAt>(pdb: &Pdb<S>) -> (u32, usize) {
    let dbi = pdb.debug_info().expect("DBI").expect("DBI present");
    let (_, module) = dbi
        .modules()
        .next()
        .expect("module 0")
        .expect("module record");
    let id = module.stream.expect("module stream").0;
    let start = [module.symbol_bytes, module.c11_bytes, module.c13_bytes]
        .into_iter()
        .map(|size| usize::try_from(size).expect("size"))
        .sum();
    (id, start)
}

/// Return the offset of the record of the global symbol `name`.
fn global_offset<S: pdbrs::io::ReadAt>(pdb: &Pdb<S>, name: &[u8]) -> u32 {
    let globals = pdb.globals().expect("globals").expect("global index");
    globals
        .index()
        .hash_table()
        .lookup(name)
        .map(|record| record.symbol)
        .find(|&offset| {
            pdbrs::native::symbols::indexed_name(globals.records(), offset).ok() == Some(name)
        })
        .expect("global record")
        .0
}

#[test]
fn patched_module_stream_is_read_once_when_the_plan_writes() {
    // Module 0 refers to the `S_UDT` record `Point`.
    let source = build(|editor| {
        let point = editor
            .globals_mut()?
            .find(b"Point")?
            .expect("the program has the UDT Point");
        editor.module_mut(ModuleIndex(0))?.push_global_ref(point)
    });
    let layout = Layout::parse(&source);
    let parsed = Pdb::parse(source.as_slice()).expect("source");
    let (module, region_start) = module_zero(&parsed);
    let point_before = global_offset(&parsed, b"Point");
    let module_bytes = stream(&parsed, module).expect("module stream bytes");

    // Step 1: open the PDB, then reset the counts.
    let counting = Counting::new(&source);
    let pdb = Pdb::read(&counting).expect("source through the counting input");
    counting.take();

    // Step 2: rename `g_value` and plan.
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    let mut globals = editor.globals_mut().expect("globals");
    let handle = globals.find(RENAMED).expect("find").expect("g_value");
    *globals
        .get_mut(handle)
        .expect("record")
        .data_mut()
        .name_mut()
        .expect("named record") = Bytes::from(NEW_NAME.to_vec());
    let plan = editor.plan(EditOptions::default()).expect("plan");
    counting.take();
    let report = plan
        .report()
        .output(pdbrs::native::msf::StreamId(module))
        .expect("module stream report");
    assert_eq!(
        report.change,
        StreamChange::Remapped(IndexSpace::SymbolOffset)
    );

    // Step 3: the output reads each page of the module stream once.
    let mut output = Vec::new();
    plan.write_to(&mut output).expect("output");
    let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
    for (offset, length) in counting.take() {
        let mut pages = BTreeSet::new();
        layout.split(offset, length, |owner, _| {
            if let Some((id, position)) = owner
                && id == module
            {
                pages.insert(position);
            }
        });
        for page in pages {
            *counts.entry(page).or_default() += 1;
        }
    }
    let pages = layout.stream(module).expect("module pages").pages.len();
    let expected: BTreeMap<usize, usize> = (0..pages).map(|page| (page, 1)).collect();
    assert_eq!(counts, expected);

    // The output module stream is the source stream with the new offset of
    // `Point` in its global references.
    let edited = Pdb::parse(output.as_slice()).expect("output parses");
    let point_after = global_offset(&edited, b"Point");
    assert_ne!(point_before, point_after);
    let mut expected = module_bytes.clone();
    let position = (region_start + 4..module_bytes.len())
        .step_by(4)
        .find(|&at| module_bytes[at..at + 4] == point_before.to_le_bytes())
        .expect("the reference to Point");
    expected[position..position + 4].copy_from_slice(&point_after.to_le_bytes());
    assert_eq!(stream(&edited, module), Some(expected));
}
