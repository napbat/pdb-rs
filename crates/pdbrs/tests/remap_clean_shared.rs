//! Tests of the clean output of the symbol record stream and its indexes: the addresses of
//! the publics and the global data, the address map of the public
//! index, the thunk table, the section starts, and the records with removed code.
//!
//! The source PDB is the synthetic program of the `native_synthetic` example: `.text` at RVA
//! `0x1000` (`sum` at `0x1000`, 0x20 bytes; `main` at `0x1020`, 0x30 bytes) and `.data` at
//! RVA `0x2000` (`g_value`).

#[path = "../examples/native_synthetic/program.rs"]
mod program;

use pdbrs::ErrorKind;
use pdbrs::io::ReadAt;
use pdbrs::native::address::{PdbRva, Rva, SectionOffset};
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::index::ModuleIndex;
use pdbrs::native::codeview::symbol::{S_GDATA32, S_PROCREF, S_PUB32, Symbol, SymbolData};
use pdbrs::native::locate::SymbolLocator;
use pdbrs::native::section::{SectionHeader, SectionHeaderStream};
use pdbrs::native::symbols::SectionStart;
use pdbrs::native::{
    EditOptions, IdentityPolicy, ImageRemap, Pdb, PdbEditor, RemapAddress, RemapItem, RemapMode,
    RemovedCode,
};

/// The characteristics of `.text` and `.data` in the synthetic program.
const TEXT: u32 = 0x6000_0020;
const DATA: u32 = 0xc000_0040;

/// One range: old RVA, length, new RVA.
type Range = (u32, u32, u32);

/// The ranges of a new image: `main` moves before `sum` in `.text` at `0x3000`, the rest of
/// `.text` follows, and `.data` stays. `.data` is section 1 and `.text` is section 2.
const MOVED: [Range; 4] = [
    (0x1020, 0x30, 0x3000),
    (0x1000, 0x20, 0x3030),
    (0x1050, 0xb0, 0x3050),
    (0x2000, 0x100, 0x2000),
];

/// The ranges of the source image without the first four bytes of `.data` (`g_value`).
const NO_VALUE: [Range; 2] = [(0x1000, 0x100, 0x1000), (0x2004, 0xfc, 0x2004)];

/// The ranges of the source image without `main`.
const NO_MAIN: [Range; 3] = [
    (0x1000, 0x20, 0x1000),
    (0x1050, 0xb0, 0x1050),
    (0x2000, 0x100, 0x2000),
];

/// Plan an editor and return the output bytes.
fn plan_bytes<S: ReadAt>(editor: &PdbEditor<'_, S>) -> Vec<u8> {
    editor
        .plan(EditOptions::default())
        .and_then(|plan| plan.to_vec())
        .expect("output")
}

/// The PDB of the synthetic program, after `edit` changes the model of the program.
fn synthetic_with(edit: impl FnOnce(&mut PdbEditor<'static>)) -> Vec<u8> {
    let mut editor = program::build().expect("the synthetic program builds");
    edit(&mut editor);
    plan_bytes(&editor)
}

/// The PDB of the synthetic program.
fn synthetic() -> Vec<u8> {
    synthetic_with(|_| {})
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

/// Make a map with `sections` and `ranges`.
fn remap(sections: SectionHeaderStream, ranges: &[Range]) -> ImageRemap {
    ranges
        .iter()
        .fold(ImageRemap::new(sections), |remap, &(old, length, new)| {
            remap
                .with_range(Rva(old), length, Rva(new))
                .expect("valid range")
        })
}

/// Return an editor of `source` with one clean remap.
fn clean<'p>(
    pdb: &'p Pdb<&'p [u8]>,
    remap: &ImageRemap,
    policy: RemovedCode,
) -> PdbEditor<'p, &'p [u8]> {
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .remap_addresses(remap, RemapMode::Clean(policy))
        .expect("clean output");
    editor
}

/// Return the address of a section.
const fn at(section: u16, offset: u32) -> SectionOffset {
    SectionOffset { section, offset }
}

/// Return the name and the address of each public in the order of the address map.
fn address_map(bytes: &[u8]) -> Vec<(Vec<u8>, SectionOffset)> {
    let pdb = Pdb::parse(bytes).expect("PDB");
    let publics = pdb.publics().expect("publics").expect("public index");
    publics
        .index()
        .address_map()
        .map(|offset| {
            let record = offset.record(publics.records()).expect("record");
            match Symbol::decode(record).expect("symbol").data() {
                SymbolData::Public(public) => (public.name.as_bytes().to_vec(), public.address),
                other => panic!("the address map names a record that is not public: {other:?}"),
            }
        })
        .collect()
}

/// Return the kind, the name and the address of each global data record.
fn global_data(bytes: &[u8]) -> Vec<(Vec<u8>, SectionOffset)> {
    let pdb = Pdb::parse(bytes).expect("PDB");
    let globals = pdb.globals().expect("globals").expect("global index");
    globals
        .symbols()
        .filter_map(|symbol| match symbol.expect("global symbol").data() {
            SymbolData::Data(data) => Some((data.name.as_bytes().to_vec(), data.address)),
            _ => None,
        })
        .collect()
}

/// Return the name of each reference record of the global index.
fn references(bytes: &[u8]) -> Vec<Vec<u8>> {
    let pdb = Pdb::parse(bytes).expect("PDB");
    let globals = pdb.globals().expect("globals").expect("global index");
    globals
        .symbols()
        .filter_map(|symbol| {
            let symbol = symbol.expect("global symbol");
            symbol
                .reference()
                .map(|reference| reference.name.as_bytes().to_vec())
        })
        .collect()
}

/// Check that a PDB reads again and that the symbol locator accepts it.
fn check_reads(bytes: &[u8]) {
    let pdb = Pdb::parse(bytes).expect("the output reads again");
    SymbolLocator::new(&pdb).expect("the locator accepts the output");
}

#[test]
fn publics_and_global_data_get_new_addresses_in_a_sorted_address_map() {
    let source = synthetic();
    assert_eq!(
        address_map(&source),
        [
            (b"sum".to_vec(), at(1, 0)),
            (b"main".to_vec(), at(1, 0x20)),
            (b"g_value".to_vec(), at(2, 0)),
        ]
    );
    let pdb = Pdb::parse(&source[..]).expect("source");
    let moved = remap(moved_sections(), &MOVED);
    let expected = [
        (b"g_value".to_vec(), at(1, 0)),
        (b"main".to_vec(), at(2, 0)),
        (b"sum".to_vec(), at(2, 0x30)),
    ];
    // The copy-on-write layout and the new layout.
    for decode_all in [false, true] {
        let mut editor = clean(&pdb, &moved, RemovedCode::Fail);
        if decode_all {
            editor.decode_all().expect("decode_all");
        }
        let bytes = plan_bytes(&editor);
        assert_eq!(address_map(&bytes), expected, "decode_all {decode_all}");
        assert_eq!(
            global_data(&bytes),
            [(b"g_value".to_vec(), at(1, 0))],
            "decode_all {decode_all}"
        );
        check_reads(&bytes);
    }
}

#[test]
fn a_new_public_address_sorts_the_address_map_again() {
    let source = synthetic();
    let pdb = Pdb::parse(&source[..]).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let mut publics = editor.publics_mut().expect("publics");
    let sum = publics.find(b"sum").expect("find").expect("sum");
    if let SymbolData::Public(public) = publics.get_mut(sum).expect("record").data_mut() {
        public.address = at(1, 0x40);
    }
    let bytes = plan_bytes(&editor);
    assert_eq!(
        address_map(&bytes),
        [
            (b"main".to_vec(), at(1, 0x20)),
            (b"sum".to_vec(), at(1, 0x40)),
            (b"g_value".to_vec(), at(2, 0)),
        ]
    );
}

#[test]
fn a_new_thunk_table_reaches_the_public_index() {
    let source = synthetic();
    let pdb = Pdb::parse(&source[..]).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .publics_mut()
        .expect("publics")
        .set_thunk_table(5, 1, 0x40, vec![0x1000]);
    let bytes = plan_bytes(&editor);
    let output = Pdb::parse(&bytes[..]).expect("output");
    let publics = output.publics().expect("publics").expect("public index");
    let header = publics.index().header();
    assert_eq!(
        (
            header.thunk_size,
            header.thunk_table_section,
            header.thunk_table_offset
        ),
        (5, 1, 0x40)
    );
    assert_eq!(publics.index().thunk_map().collect::<Vec<_>>(), [0x1000]);
}

/// Return a section start entry.
fn section_start(section: u16, offset: u32) -> SectionStart {
    let mut start = SectionStart::default();
    start.section = section;
    start.offset = offset;
    start
}

#[test]
fn the_thunk_table_and_the_section_starts_get_new_addresses() {
    let source = synthetic_with(|editor| {
        let mut publics = editor.publics_mut().expect("publics");
        // The targets are RVAs: `sum`, `main` and no target.
        publics.set_thunk_table(5, 1, 0x40, vec![0x1000, 0x1020, 0]);
        publics.set_section_starts(vec![section_start(1, 0x1000), section_start(2, 0x2000)]);
    });
    let pdb = Pdb::parse(&source[..]).expect("source");
    let editor = clean(&pdb, &remap(moved_sections(), &MOVED), RemovedCode::Fail);
    let bytes = plan_bytes(&editor);
    let output = Pdb::parse(&bytes[..]).expect("output");
    let publics = output.publics().expect("publics").expect("public index");
    let header = publics.index().header();
    // `.text+0x40` is in the range of `main`: RVA `0x3020`, `.text` is section 2.
    assert_eq!(
        (
            header.thunk_size,
            header.thunk_table_section,
            header.thunk_table_offset
        ),
        (5, 2, 0x20)
    );
    assert_eq!(
        publics.index().thunk_map().collect::<Vec<_>>(),
        [0x3030, 0x3000, 0]
    );
    // The source values are the section RVAs, so the values of the new headers replace them.
    assert_eq!(
        publics.index().section_starts().collect::<Vec<_>>(),
        [section_start(1, 0x2000), section_start(2, 0x3000)]
    );
}

#[test]
fn section_starts_that_are_not_section_rvas_map_as_points() {
    let source = synthetic_with(|editor| {
        editor
            .publics_mut()
            .expect("publics")
            .set_section_starts(vec![section_start(1, 0x1020)]);
    });
    let pdb = Pdb::parse(&source[..]).expect("source");
    let editor = clean(&pdb, &remap(moved_sections(), &MOVED), RemovedCode::Fail);
    let bytes = plan_bytes(&editor);
    let output = Pdb::parse(&bytes[..]).expect("output");
    let publics = output.publics().expect("publics").expect("public index");
    assert_eq!(
        publics.index().section_starts().collect::<Vec<_>>(),
        [section_start(2, 0x3000)]
    );
}

/// Return the offset of the global data record `name` in the symbol record stream of `pdb`.
fn data_offset(pdb: &Pdb<&[u8]>, name: &[u8]) -> u32 {
    let records = pdb
        .symbol_records()
        .expect("records")
        .expect("symbol records");
    Records::new(records)
        .find_map(|record| {
            let record = record.expect("record");
            match Symbol::decode(record).expect("symbol").data() {
                SymbolData::Data(data) if data.name.as_bytes() == name => Some(record.offset),
                _ => None,
            }
        })
        .expect("the data record")
}

#[test]
fn fail_names_the_first_shared_record_with_removed_code() {
    let source = synthetic();
    let pdb = Pdb::parse(&source[..]).expect("source");
    let editor = clean(
        &pdb,
        &remap(source_sections(), &NO_VALUE),
        RemovedCode::Fail,
    );
    let error = editor
        .plan(EditOptions::default())
        .expect_err("g_value has no range");
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
    assert_eq!(
        error.remap_address(),
        Some(RemapAddress::new(
            RemapItem::Symbol(S_GDATA32),
            PdbRva(0x2000)
        ))
    );
    let stream = pdb
        .debug_info()
        .expect("DBI")
        .expect("DBI present")
        .header()
        .symbol_stream
        .expect("symbol record stream");
    assert_eq!(
        error.stream_location(),
        Some((stream, u64::from(data_offset(&pdb, b"g_value"))))
    );
}

#[test]
fn drop_removes_the_records_at_a_removed_address_and_their_global_references() {
    let source = synthetic_with(|editor| {
        let value = editor
            .globals_mut()
            .expect("globals")
            .find(b"g_value")
            .expect("find")
            .expect("g_value");
        editor
            .module_mut(ModuleIndex(0))
            .expect("module")
            .push_global_ref(value)
            .expect("global reference");
    });
    let pdb = Pdb::parse(&source[..]).expect("source");
    let module = pdb
        .module(ModuleIndex(0))
        .expect("module")
        .expect("module stream");
    assert_eq!(
        module
            .global_refs()
            .expect("global references")
            .iter()
            .count(),
        1
    );
    let editor = clean(
        &pdb,
        &remap(source_sections(), &NO_VALUE),
        RemovedCode::Drop,
    );
    let plan = editor.plan(EditOptions::default()).expect("plan");
    let report = plan.report().remap().expect("clean output report");
    assert_eq!(report.removed(RemapItem::Symbol(S_GDATA32)), 1);
    assert_eq!(report.removed(RemapItem::Symbol(S_PUB32)), 1);
    let bytes = plan.to_vec().expect("output");
    check_reads(&bytes);
    assert_eq!(global_data(&bytes), []);
    assert_eq!(
        address_map(&bytes),
        [(b"sum".to_vec(), at(1, 0)), (b"main".to_vec(), at(1, 0x20))]
    );
    let output = Pdb::parse(&bytes[..]).expect("output");
    let module = output
        .module(ModuleIndex(0))
        .expect("module")
        .expect("module stream");
    assert_eq!(
        module
            .global_refs()
            .expect("global references")
            .iter()
            .count(),
        0
    );
}

#[test]
fn drop_removes_a_reference_to_a_removed_procedure() {
    let source = synthetic();
    assert_eq!(references(&source), [b"sum".to_vec(), b"main".to_vec()]);
    let pdb = Pdb::parse(&source[..]).expect("source");
    let editor = clean(&pdb, &remap(source_sections(), &NO_MAIN), RemovedCode::Drop);
    let plan = editor.plan(EditOptions::default()).expect("plan");
    let report = plan.report().remap().expect("clean output report");
    assert_eq!(report.removed(RemapItem::Symbol(S_PROCREF)), 1);
    assert_eq!(report.removed(RemapItem::Symbol(S_PUB32)), 1);
    let bytes = plan.to_vec().expect("output");
    check_reads(&bytes);
    assert_eq!(references(&bytes), [b"sum".to_vec()]);
    assert_eq!(
        address_map(&bytes),
        [(b"sum".to_vec(), at(1, 0)), (b"g_value".to_vec(), at(2, 0))]
    );
}
