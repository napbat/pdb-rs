//! Tests of the clean output of `PdbEditor::remap_addresses`: the identity map, determinism,
//! other edits in the same session and a sequence of
//! remaps.
//!
//! The source PDB is the synthetic program of the `native_synthetic` example: `.text` at RVA
//! `0x1000` (`sum` at `0x1000`, 0x20 bytes; `main` at `0x1020`, 0x30 bytes) and `.data` at
//! RVA `0x2000` (`g_value`).

#[path = "../examples/native_synthetic/program.rs"]
mod program;

use pdbrs::ErrorKind;
use pdbrs::io::ReadAt;
use pdbrs::native::address::{Rva, SectionOffset};
use pdbrs::native::codeview::symbol::{Public, SymbolData};
use pdbrs::native::dbi::DebugStream;
use pdbrs::native::locate::SymbolLocator;
use pdbrs::native::msf::StreamId;
use pdbrs::native::section::{SectionHeader, SectionHeaderStream};
use pdbrs::native::{
    EditOptions, IdentityPolicy, ImageRemap, Pdb, PdbEditor, RemapMode, RemovedCode,
};

/// The characteristics of `.text` and `.data` in the synthetic program.
const TEXT: u32 = 0x6000_0020;
const DATA: u32 = 0xc000_0040;

/// One range: old RVA, length, new RVA.
type Range = (u32, u32, u32);

/// The PDB of the synthetic program.
fn synthetic() -> Vec<u8> {
    program::build()
        .and_then(|editor| editor.plan(EditOptions::default())?.to_vec())
        .expect("the synthetic program builds")
}

fn section(name: [u8; 8], virtual_address: u32, characteristics: u32) -> SectionHeader {
    let mut header = SectionHeader::new(name, 0x100, virtual_address);
    header.characteristics = characteristics;
    header
}

/// The section headers of the source image.
fn source_sections() -> SectionHeaderStream {
    SectionHeaderStream::from_records(&[
        section(*b".text\0\0\0", 0x1000, TEXT),
        section(*b".data\0\0\0", 0x2000, DATA),
    ])
    .expect("section headers")
}

/// The section headers of a new image with `.data` at `0x2000` and `.text` at `0x3000`.
fn moved_sections() -> SectionHeaderStream {
    SectionHeaderStream::from_records(&[
        section(*b".data\0\0\0", 0x2000, DATA),
        section(*b".text\0\0\0", 0x3000, TEXT),
    ])
    .expect("section headers")
}

/// Make a map with `sections` and `ranges`, in the order of the slice.
fn remap(sections: SectionHeaderStream, ranges: &[Range]) -> ImageRemap {
    ranges
        .iter()
        .fold(ImageRemap::new(sections), |remap, &(old, length, new)| {
            remap
                .with_range(Rva(old), length, Rva(new))
                .expect("valid range")
        })
}

/// The identity map of the source image.
fn identity() -> ImageRemap {
    remap(
        source_sections(),
        &[(0x1000, 0x100, 0x1000), (0x2000, 0x100, 0x2000)],
    )
}

/// The ranges of a new image: `main` moves before `sum` in `.text` at `0x3000`, the rest of
/// `.text` follows, and `.data` stays.
const MOVED: [Range; 4] = [
    (0x1020, 0x30, 0x3000),
    (0x1000, 0x20, 0x3030),
    (0x1050, 0xb0, 0x3050),
    (0x2000, 0x100, 0x2000),
];

/// Remap `source` with `steps` in call order, and return the output bytes.
fn output(source: &[u8], steps: &[(&ImageRemap, RemapMode)]) -> Vec<u8> {
    let pdb = Pdb::parse(source).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    for (remap, mode) in steps {
        editor.remap_addresses(remap, *mode).expect("remap");
    }
    plan_bytes(&editor)
}

/// Plan an editor and return the output bytes.
fn plan_bytes<S: ReadAt>(editor: &PdbEditor<'_, S>) -> Vec<u8> {
    editor
        .plan(EditOptions::default())
        .and_then(|plan| plan.to_vec())
        .expect("output")
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

/// Return the name of the procedure at `rva` of a PDB.
fn procedure_at(bytes: &[u8], rva: u32) -> Option<Vec<u8>> {
    let pdb = Pdb::parse(bytes).expect("PDB");
    let locator = SymbolLocator::new(&pdb).expect("locator");
    let location = locator.locate(Rva(rva)).expect("locate")?;
    location.procedure.map(|procedure| procedure.name.to_vec())
}

/// Return the section and offset of the public symbol `name` of a PDB.
fn public_address(bytes: &[u8], name: &[u8]) -> Option<SectionOffset> {
    let pdb = Pdb::parse(bytes).expect("PDB");
    let publics = pdb.publics().expect("publics")?;
    publics.symbols().find_map(|symbol| {
        let symbol = symbol.expect("public symbol");
        match symbol.data() {
            SymbolData::Public(public) if public.name.as_bytes() == name => Some(public.address),
            _ => None,
        }
    })
}

#[test]
fn the_clean_identity_keeps_each_stream_but_the_info_and_dbi_streams() {
    let source = synthetic();
    let remap = identity();
    let pdb = Pdb::parse(&source[..]).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .remap_addresses(&remap, RemapMode::Clean(RemovedCode::Fail))
        .expect("clean output");
    let plan = editor.plan(EditOptions::default()).expect("plan");
    let changed: Vec<u32> = plan.report().changed().map(|id| id.0).collect();
    assert!(
        changed.iter().all(|id| [1, 3].contains(id)),
        "changed streams {changed:?}"
    );
    assert_eq!(plan.report().dropped().count(), 0);
    let report = plan.report().remap().expect("clean output report");
    assert_eq!(report.removed_items().count(), 0);
    assert_eq!(report.added_items().count(), 0);
    assert_eq!(report.cleared_crcs(), 0);
    let bytes = plan.to_vec().expect("output");
    let output = Pdb::parse(&bytes[..]).expect("output reads again");
    for id in (2..pdb.container().stream_count()).filter(|&id| id != 3) {
        assert_eq!(
            stream(&output, StreamId(id)),
            stream(&pdb, StreamId(id)),
            "stream {id}"
        );
    }
}

#[test]
fn clean_output_writes_the_new_addresses_and_no_omap() {
    let source = synthetic();
    let moved = remap(moved_sections(), &MOVED);
    let bytes = output(&source, &[(&moved, RemapMode::Clean(RemovedCode::Fail))]);
    let pdb = Pdb::parse(&bytes[..]).expect("output reads again");
    let dbi = pdb.debug_info().expect("DBI").expect("DBI present");
    for kind in [
        DebugStream::OmapToSource,
        DebugStream::OmapFromSource,
        DebugStream::OriginalSectionHeader,
    ] {
        assert_eq!(dbi.debug_stream(kind), None, "{kind:?}");
    }
    assert_eq!(
        pdb.section_headers(DebugStream::SectionHeader)
            .expect("section headers"),
        Some(moved_sections())
    );
    // `.text` is section 2 of the new image.
    assert_eq!(
        public_address(&bytes, b"sum"),
        Some(SectionOffset {
            section: 2,
            offset: 0x30
        })
    );
    assert_eq!(
        public_address(&bytes, b"g_value"),
        Some(SectionOffset {
            section: 1,
            offset: 0
        })
    );
    assert_eq!(procedure_at(&bytes, 0x3000), Some(b"main".to_vec()));
    assert_eq!(procedure_at(&bytes, 0x3031), Some(b"sum".to_vec()));
}

#[test]
fn the_same_input_gives_the_same_bytes() {
    let source = synthetic();
    let mode = RemapMode::Clean(RemovedCode::Fail);
    let first = output(&source, &[(&remap(moved_sections(), &MOVED), mode)]);
    let mut reversed = MOVED;
    reversed.reverse();
    let second = output(&source, &[(&remap(moved_sections(), &reversed), mode)]);
    assert_eq!(first, second);
    let new_layout = || {
        let pdb = Pdb::parse(&source[..]).expect("source");
        let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
        editor.decode_all().expect("decode_all");
        editor
            .remap_addresses(&remap(moved_sections(), &MOVED), mode)
            .expect("clean output");
        plan_bytes(&editor)
    };
    let rebuilt = new_layout();
    assert_eq!(rebuilt, new_layout());
    assert_eq!(procedure_at(&rebuilt, 0x3000), Some(b"main".to_vec()));
}

#[test]
fn records_of_other_edits_use_the_source_addresses() {
    let source = synthetic();
    let pdb = Pdb::parse(&source[..]).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .remap_addresses(
            &remap(moved_sections(), &MOVED),
            RemapMode::Clean(RemovedCode::Fail),
        )
        .expect("clean output");
    let before = plan_bytes(&editor);
    assert_eq!(public_address(&before, b"later"), None);
    // A record that an edit adds after the call uses the address of the source PDB.
    let mut public = Public::default();
    public.address = SectionOffset {
        section: 1,
        offset: 0x24,
    };
    public.name = b"later".into();
    editor
        .publics_mut()
        .expect("publics")
        .push(public)
        .expect("new public");
    let after = plan_bytes(&editor);
    assert_eq!(
        public_address(&after, b"later"),
        Some(SectionOffset {
            section: 2,
            offset: 4
        })
    );
}

#[test]
fn a_second_clean_remap_rewrites_the_first_new_image() {
    let source = synthetic();
    let first = remap(moved_sections(), &MOVED);
    // The second rewrite moves `.text` of the first new image by 0x10 bytes.
    let second = remap(
        moved_sections(),
        &[(0x3000, 0xf0, 0x3010), (0x2000, 0x100, 0x2000)],
    );
    let mode = RemapMode::Clean(RemovedCode::Drop);
    let bytes = output(&source, &[(&first, mode), (&second, mode)]);
    assert_eq!(procedure_at(&bytes, 0x3010), Some(b"main".to_vec()));
    assert_eq!(procedure_at(&bytes, 0x3041), Some(b"sum".to_vec()));
}

#[test]
fn clean_output_composes_with_omap_output() {
    let source = synthetic();
    let first = remap(moved_sections(), &MOVED);
    let second = remap(
        moved_sections(),
        &[(0x3000, 0xf0, 0x3010), (0x2000, 0x100, 0x2000)],
    );
    let bytes = output(
        &source,
        &[
            (&first, RemapMode::Omap),
            (&second, RemapMode::Clean(RemovedCode::Fail)),
        ],
    );
    let pdb = Pdb::parse(&bytes[..]).expect("output reads again");
    let tables = pdb
        .address_tables()
        .expect("tables")
        .expect("section headers");
    assert!(tables.from_source.is_none() && tables.to_source.is_none());
    assert!(tables.original.is_none());
    assert_eq!(procedure_at(&bytes, 0x3010), Some(b"main".to_vec()));
}

#[test]
fn omap_output_cannot_follow_clean_output() {
    let source = synthetic();
    let pdb = Pdb::parse(&source[..]).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let moved = remap(moved_sections(), &MOVED);
    editor
        .remap_addresses(&moved, RemapMode::Clean(RemovedCode::Fail))
        .expect("clean output");
    let error = editor
        .remap_addresses(&moved, RemapMode::Omap)
        .expect_err("OMAP output after clean output");
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
    // The model keeps the clean remap, and no OMAP stream.
    let bytes = plan_bytes(&editor);
    let tables = Pdb::parse(&bytes[..])
        .expect("output")
        .address_tables()
        .expect("tables")
        .expect("section headers");
    assert!(tables.from_source.is_none());
}
