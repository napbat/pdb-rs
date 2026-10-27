//! Tests of the OMAP output of `PdbEditor::remap_addresses` (#43, work order step 3).
//!
//! The source PDB is the synthetic program of the `native_synthetic` example: `.text` at RVA
//! `0x1000` (`sum` at `0x1000`, `main` at `0x1020`) and `.data` at RVA `0x2000` (`g_value`).

#[path = "../examples/native_synthetic/program.rs"]
mod program;

use std::collections::BTreeSet;

use pdbrs::ErrorKind;
use pdbrs::io::ReadAt;
use pdbrs::native::address::{AddressTables, PdbRva, Rva, Translation, Unmapped};
use pdbrs::native::dbi::DebugStream;
use pdbrs::native::msf::StreamId;
use pdbrs::native::section::{SectionHeader, SectionHeaderStream};
use pdbrs::native::{
    EditOptions, IdentityPolicy, ImageRemap, Pdb, PdbEditor, RebuildReason, RemapMode, RemovedCode,
    StreamChange,
};

/// The 4 debug streams of OMAP output.
const OMAP_STREAMS: [DebugStream; 4] = [
    DebugStream::OmapToSource,
    DebugStream::OmapFromSource,
    DebugStream::SectionHeader,
    DebugStream::OriginalSectionHeader,
];

/// One range: old RVA, length, new RVA.
type Range = (u32, u32, u32);

/// The ranges of the tests: `main` moves before `sum`, `.data` moves before `.text`, and the
/// rest of `.text` (from `0x1050`) is removed.
const RANGES: [Range; 3] = [
    (0x1020, 0x30, 0x3000),
    (0x1000, 0x20, 0x3030),
    (0x2000, 0x100, 0x2000),
];

/// The PDB of the synthetic program.
fn synthetic() -> Vec<u8> {
    program::build()
        .and_then(|editor| editor.plan(EditOptions::default())?.to_vec())
        .expect("the synthetic program builds")
}

fn section(name: [u8; 8], virtual_address: u32, virtual_size: u32) -> SectionHeader {
    SectionHeader::new(name, virtual_size, virtual_address)
}

/// The section headers of the new image: `.data` at `0x2000`, `.text` at `0x3000`.
fn new_sections() -> SectionHeaderStream {
    SectionHeaderStream::from_records(&[
        section(*b".data\0\0\0", 0x2000, 0x100),
        section(*b".text\0\0\0", 0x3000, 0x100),
    ])
    .expect("section headers")
}

/// Make a map with `ranges`, in the order of the slice.
fn remap(ranges: &[Range]) -> ImageRemap {
    ranges.iter().fold(
        ImageRemap::new(new_sections()),
        |remap, &(old, length, new)| {
            remap
                .with_range(Rva(old), length, Rva(new))
                .expect("valid range")
        },
    )
}

/// Return the bytes of stream `id`, or `None` for a nil or absent stream.
fn stream<S: ReadAt>(pdb: &Pdb<S>, id: StreamId) -> Option<Vec<u8>> {
    if id.0 >= pdb.container().stream_count() {
        return None;
    }
    pdb.container()
        .stream(id)
        .expect("stream directory")
        .map(|reader| reader.read_all(usize::MAX).expect("stream bytes"))
}

/// Remap `source` with `remap` in OMAP mode and return the output and the changed streams.
fn omap_output(source: &[u8], remap: &ImageRemap) -> (Vec<u8>, Vec<(StreamId, StreamChange)>) {
    let pdb = Pdb::parse(source).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .remap_addresses(remap, RemapMode::Omap)
        .expect("OMAP output");
    let plan = editor.plan(EditOptions::default()).expect("plan");
    let changed = plan
        .report()
        .streams()
        .iter()
        .filter(|entry| entry.change != StreamChange::Copied)
        .filter_map(|entry| Some((entry.output?, entry.change)))
        .filter(|(id, _)| id.0 != 0)
        .collect();
    (plan.to_vec().expect("output"), changed)
}

/// Return the address tables of a PDB.
fn tables(bytes: &[u8]) -> AddressTables {
    Pdb::parse(bytes)
        .expect("PDB")
        .address_tables()
        .expect("tables")
        .expect("section headers")
}

/// Return the new RVA of the PDB RVA `rva` in a PDB with `tables`.
fn new_rva(tables: &AddressTables, rva: u32) -> Translation<Rva> {
    tables.map().pdb_rva_to_rva(PdbRva(rva)).expect("lookup")
}

#[test]
fn omap_output_writes_only_the_identity_and_the_address_streams() {
    let source = synthetic();
    let remap = remap(&RANGES);
    let (output, changed) = omap_output(&source, &remap);
    let input = Pdb::parse(&source[..]).expect("source");
    let pdb = Pdb::parse(&output[..]).expect("output reads again");
    let dbi = pdb.debug_info().expect("DBI").expect("DBI present");
    let slots: BTreeSet<StreamId> = OMAP_STREAMS
        .iter()
        .map(|&kind| dbi.debug_stream(kind).expect("OMAP output slot"))
        .collect();
    let source_slot = input
        .debug_info()
        .expect("DBI")
        .expect("DBI present")
        .debug_stream(DebugStream::SectionHeader)
        .expect("source section headers");
    let mut expected = vec![
        (StreamId(1), StreamChange::Rebuilt(RebuildReason::Identity)),
        (
            StreamId(3),
            StreamChange::Rebuilt(RebuildReason::DebugHeader),
        ),
    ];
    for &id in &slots {
        let change = if id == source_slot {
            StreamChange::Edited
        } else {
            StreamChange::Added
        };
        expected.push((id, change));
    }
    let mut changed = changed;
    changed.sort();
    expected.sort();
    assert_eq!(changed, expected);
    // Each other stream keeps the source bytes.
    for id in 1..input.container().stream_count() {
        let id = StreamId(id);
        if [StreamId(1), StreamId(3)].contains(&id) || slots.contains(&id) {
            continue;
        }
        assert_eq!(stream(&pdb, id), stream(&input, id), "stream {}", id.0);
    }
}

#[test]
fn omap_output_translates_as_the_ranges_say() {
    let source = synthetic();
    let remap = remap(&RANGES);
    let (output, _) = omap_output(&source, &remap);
    let expected = remap
        .address_tables(&tables(&source))
        .expect("tables of the ranges");
    let tables = tables(&output);
    assert_eq!(tables, expected);
    // `sum`, `main` and `g_value` and their last bytes.
    for (old, new) in [
        (0x1000, 0x3030),
        (0x101f, 0x304f),
        (0x1020, 0x3000),
        (0x104f, 0x302f),
        (0x2000, 0x2000),
    ] {
        assert_eq!(new_rva(&tables, old), Translation::Mapped(Rva(new)));
    }
    // The removed rest of `.text`.
    assert!(matches!(
        new_rva(&tables, 0x1050),
        Translation::Unmapped(Unmapped::NoTarget(_))
    ));
    assert_eq!(
        tables.original,
        Pdb::parse(&source[..])
            .expect("source")
            .section_headers(DebugStream::SectionHeader)
            .expect("section headers")
    );
}

#[test]
fn the_same_input_gives_the_same_bytes() {
    let source = synthetic();
    let (first, _) = omap_output(&source, &remap(&RANGES));
    let mut reversed = RANGES;
    reversed.reverse();
    let (second, _) = omap_output(&source, &remap(&reversed));
    assert_eq!(first, second);
    // The new layout gives the same bytes for the same input too.
    let new_layout = || {
        let pdb = Pdb::parse(&source[..]).expect("source");
        let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
        editor.decode_all().expect("decode_all");
        editor
            .remap_addresses(&remap(&RANGES), RemapMode::Omap)
            .expect("OMAP output");
        editor
            .plan(EditOptions::default())
            .and_then(|plan| plan.to_vec())
            .expect("output")
    };
    let rebuilt = new_layout();
    assert_eq!(rebuilt, new_layout());
    assert_eq!(tables(&rebuilt), tables(&first));
}

#[test]
fn a_second_remap_composes_with_the_first() {
    let source = synthetic();
    let first = remap(&RANGES);
    // The second rewrite moves `.text` of the first new image by 0x10 bytes.
    let second = ImageRemap::new(new_sections())
        .with_range(Rva(0x3000), 0x50, Rva(0x3010))
        .and_then(|remap| remap.with_range(Rva(0x2000), 0x100, Rva(0x2000)))
        .expect("ranges");
    let pdb = Pdb::parse(&source[..]).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .remap_addresses(&first, RemapMode::Omap)
        .and_then(|()| editor.remap_addresses(&second, RemapMode::Omap))
        .expect("two remaps");
    let output = editor
        .plan(EditOptions::default())
        .and_then(|plan| plan.to_vec())
        .expect("output");
    let tables = tables(&output);
    assert_eq!(new_rva(&tables, 0x1000), Translation::Mapped(Rva(0x3040)));
    assert_eq!(new_rva(&tables, 0x1020), Translation::Mapped(Rva(0x3010)));
    assert_eq!(
        tables.map().rva_to_pdb_rva(Rva(0x3040)).expect("lookup"),
        Translation::Mapped(PdbRva(0x1000))
    );
}

#[test]
fn clean_output_is_not_supported_yet() {
    let source = synthetic();
    let pdb = Pdb::parse(&source[..]).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    for policy in [RemovedCode::Fail, RemovedCode::Drop] {
        let error = editor
            .remap_addresses(&remap(&RANGES), RemapMode::Clean(policy))
            .expect_err("clean output is not supported");
        assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
    }
    let plan = editor.plan(EditOptions::default()).expect("plan");
    assert_eq!(plan.report().changed().count(), 0);
}

#[test]
fn a_pdb_without_section_headers_cannot_be_remapped() {
    let mut editor = PdbEditor::new(1, [1; 16], 1).expect("editor");
    let error = editor
        .remap_addresses(&remap(&RANGES), RemapMode::Omap)
        .expect_err("no section headers");
    assert_eq!(error.kind(), ErrorKind::MissingStream);
    assert_eq!(editor.debug_streams_mut().expect("list").kinds().count(), 0);
}
