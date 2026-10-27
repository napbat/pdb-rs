//! Tests of the clean output of `PdbEditor::remap_addresses` for the DBI values and the
//! optional debug streams (#43, decisions 4 to 7 and 11).
//!
//! The source PDB is the synthetic program of the `native_synthetic` example: `.text` at RVA
//! `0x1000` (`sum` at `0x1000`, `main` at `0x1020`) and `.data` at RVA `0x2000` (`g_value`).
//! The map of most tests moves `main` before `sum` in a new `.text` section at `0x3000`, and
//! `.data` to section 1. It removes the rest of `.text` (from `0x1050`).

#[path = "../examples/native_synthetic/program.rs"]
mod program;

use pdbrs::ErrorKind;
use pdbrs::native::address::{PdbRva, Rva};
use pdbrs::native::codeview::index::ModuleIndex;
use pdbrs::native::dbi::{
    DebugStream, SectionContribution, SectionContributionVersion, SectionMap, SegmentFlags,
};
use pdbrs::native::debug_stream::{
    BlobHeader, Fixup, FixupStream, FunctionEntry, FunctionEntryStream, RvaVaBlob,
};
use pdbrs::native::fpo::{FpoData, FpoStream, FrameData, FrameDataStream};
use pdbrs::native::section::{SectionHeader, SectionHeaderStream};
use pdbrs::native::{
    EditOptions, IdentityPolicy, ImageRemap, Pdb, PdbEditor, RemapItem, RemapMode, RemovedCode,
};

/// The `.text` section characteristics: code, execute, read.
const TEXT: u32 = 0x6000_0020;
/// The `.data` section characteristics: initialized data, read, write.
const DATA: u32 = 0xc000_0040;

/// One range: old RVA, length, new RVA.
type Range = (u32, u32, u32);

/// `main` moves before `sum` and `.data` moves to section 1. The rest of `.text` (from
/// `0x1050`) is removed.
const RANGES: [Range; 3] = [
    (0x1020, 0x30, 0x3000),
    (0x1000, 0x20, 0x3030),
    (0x2000, 0x100, 0x2000),
];

/// The identity map of the synthetic program.
const IDENTITY: [Range; 2] = [(0x1000, 0x100, 0x1000), (0x2000, 0x100, 0x2000)];

fn section(name: [u8; 8], virtual_address: u32, characteristics: u32) -> SectionHeader {
    let mut header = SectionHeader::new(name, 0x100, virtual_address);
    header.characteristics = characteristics;
    header
}

/// The section headers of the new image: `.data` at `0x2000`, `.text` at `0x3000`.
fn new_sections() -> Vec<SectionHeader> {
    vec![
        section(*b".data\0\0\0", 0x2000, DATA),
        section(*b".text\0\0\0", 0x3000, TEXT),
    ]
}

/// The section headers of the synthetic program.
fn source_sections() -> Vec<SectionHeader> {
    vec![
        section(*b".text\0\0\0", 0x1000, TEXT),
        section(*b".data\0\0\0", 0x2000, DATA),
    ]
}

/// Make a map to the image with `sections` with `ranges`.
fn remap(sections: &[SectionHeader], ranges: &[Range]) -> ImageRemap {
    ranges.iter().fold(
        ImageRemap::new(SectionHeaderStream::from_records(sections).expect("headers")),
        |remap, &(old, length, new)| {
            remap
                .with_range(Rva(old), length, Rva(new))
                .expect("valid range")
        },
    )
}

/// Return a V60 contribution of module 0 with the CRCs `crc`.
fn contribution(
    section: u16,
    offset: i32,
    size: i32,
    characteristics: u32,
    crc: u32,
) -> SectionContribution {
    let mut contribution = SectionContribution::default();
    contribution.section = section;
    contribution.offset = offset;
    contribution.size = size;
    contribution.characteristics = characteristics;
    contribution.module = ModuleIndex(0);
    contribution.data_crc = Some(crc);
    contribution.reloc_crc = Some(crc.wrapping_add(1));
    contribution
}

/// Build the synthetic program, change it with `change` and return the PDB bytes.
fn synthetic(change: impl FnOnce(&mut PdbEditor<'static>)) -> Vec<u8> {
    let mut editor = program::build().expect("the synthetic program builds");
    change(&mut editor);
    editor
        .plan(EditOptions::default())
        .and_then(|plan| plan.to_vec())
        .expect("the source writes")
}

/// Set the section contributions of the synthetic program with CRCs that are not 0.
fn with_crcs(editor: &mut PdbEditor<'static>) {
    editor
        .debug_info_mut()
        .expect("DBI")
        .set_section_contributions(
            SectionContributionVersion::V60,
            vec![
                contribution(1, 0, 0x50, TEXT, 0x11),
                contribution(2, 0, 4, DATA, 0x21),
            ],
        );
}

/// Set the bytes of the debug stream `kind`.
fn set_stream(editor: &mut PdbEditor<'static>, kind: DebugStream, bytes: Vec<u8>) {
    editor
        .debug_streams_mut()
        .expect("debug streams")
        .set(kind, bytes)
        .expect("set");
}

fn fpo(start: u32, size: u32) -> FpoData {
    let mut record = FpoData::default();
    record.start = start;
    record.size = size;
    record.locals = start;
    record
}

fn frame(start: u32, size: u32) -> FrameData {
    let mut record = FrameData::default();
    record.start = start;
    record.size = size;
    record.program = start;
    record
}

fn function(start: u32, end: u32, end_of_prologue: u32) -> FunctionEntry {
    let mut record = FunctionEntry::default();
    record.start = start;
    record.end = end;
    record.end_of_prologue = end_of_prologue;
    record
}

fn fixup(kind: u16, rva: u32, target: u32) -> Fixup {
    let mut record = Fixup::default();
    record.kind = kind;
    record.rva = rva;
    record.target = target;
    record
}

/// Add the FPO, new FPO, exception data and fixup streams with `fpos` (start and size) for
/// the first three, and `fixups`.
fn with_records(editor: &mut PdbEditor<'static>, extents: &[(u32, u32)], fixups: &[Fixup]) {
    let fpos: Vec<FpoData> = extents
        .iter()
        .map(|&(start, size)| fpo(start, size))
        .collect();
    let frames: Vec<FrameData> = extents
        .iter()
        .map(|&(start, size)| frame(start, size))
        .collect();
    let functions: Vec<FunctionEntry> = extents
        .iter()
        .map(|&(start, size)| function(start, start + size, start + 4))
        .collect();
    let streams = [
        (
            DebugStream::Fpo,
            FpoStream::from_records(&fpos).and_then(|s| s.to_bytes()),
        ),
        (
            DebugStream::NewFpo,
            FrameDataStream::from_records(&frames).and_then(|s| s.to_bytes()),
        ),
        (
            DebugStream::Exception,
            FunctionEntryStream::from_records(&functions).and_then(|s| s.to_bytes()),
        ),
        (
            DebugStream::Fixup,
            FixupStream::from_records(fixups).and_then(|s| s.to_bytes()),
        ),
    ];
    for (kind, bytes) in streams {
        set_stream(editor, kind, bytes.expect("records"));
    }
}

/// Write the clean output of `source` with `remap` and `policy`. Return the output and the
/// report counts.
fn clean(
    source: &[u8],
    remap: &ImageRemap,
    policy: RemovedCode,
) -> pdbrs::Result<(Vec<u8>, Counts)> {
    let pdb = Pdb::parse(source).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor.remap_addresses(remap, RemapMode::Clean(policy))?;
    let plan = editor.plan(EditOptions::default())?;
    let report = plan
        .report()
        .remap()
        .expect("clean output has a remap report");
    let counts = Counts {
        cleared_crcs: report.cleared_crcs(),
        added: report.added(RemapItem::SectionContribution),
        removed: [
            RemapItem::SectionContribution,
            RemapItem::Fpo,
            RemapItem::NewFpo,
            RemapItem::ExceptionData,
            RemapItem::Fixup,
        ]
        .map(|item| report.removed(item)),
    };
    Ok((plan.to_vec()?, counts))
}

/// The counts of the report that the tests check.
#[derive(Debug, Eq, PartialEq)]
struct Counts {
    /// The contributions with new CRC values.
    cleared_crcs: usize,
    /// The added contribution parts.
    added: usize,
    /// The removed contributions, FPO, new FPO, exception data and fixup records.
    removed: [usize; 5],
}

/// Return the section contributions and the module record contribution of a PDB.
fn contributions(bytes: &[u8]) -> (Vec<SectionContribution>, SectionContribution) {
    let pdb = Pdb::parse(bytes).expect("PDB");
    let dbi = pdb.debug_info().expect("DBI").expect("DBI present");
    let list = dbi
        .section_contributions()
        .expect("contributions")
        .expect("contribution substream")
        .iter()
        .collect();
    let (_, module) = dbi
        .modules()
        .next()
        .expect("one module")
        .expect("module record");
    (list, module.section_contribution)
}

/// Return the section map of a PDB.
fn section_map(bytes: &[u8]) -> Option<SectionMap> {
    let pdb = Pdb::parse(bytes).expect("PDB");
    let dbi = pdb.debug_info().expect("DBI").expect("DBI present");
    dbi.section_map().expect("section map")
}

/// Return the slots of the optional debug header of a PDB.
fn slots(bytes: &[u8]) -> Vec<DebugStream> {
    let pdb = Pdb::parse(bytes).expect("PDB");
    let dbi = pdb.debug_info().expect("DBI").expect("DBI present");
    [
        DebugStream::Fpo,
        DebugStream::Exception,
        DebugStream::Fixup,
        DebugStream::OmapToSource,
        DebugStream::OmapFromSource,
        DebugStream::SectionHeader,
        DebugStream::TokenRidMap,
        DebugStream::Xdata,
        DebugStream::Pdata,
        DebugStream::NewFpo,
        DebugStream::OriginalSectionHeader,
    ]
    .into_iter()
    .filter(|&kind| dbi.debug_stream(kind).is_some())
    .collect()
}

/// Return the section headers of slot 5 of a PDB.
fn headers(bytes: &[u8]) -> Vec<SectionHeader> {
    Pdb::parse(bytes)
        .expect("PDB")
        .section_headers(DebugStream::SectionHeader)
        .expect("section headers")
        .expect("slot 5")
        .headers()
        .iter()
        .collect()
}

#[test]
fn contributions_split_sort_and_clear_crcs() {
    let source = synthetic(with_crcs);
    let (output, counts) =
        clean(&source, &remap(&new_sections(), &RANGES), RemovedCode::Fail).expect("clean output");
    let (list, record) = contributions(&output);
    // `.data` is section 1 now. The `.text` contribution splits into `main` and `sum`.
    assert_eq!(
        list,
        vec![
            contribution(1, 0, 4, DATA, 0).with_crcs(0),
            contribution(2, 0, 0x30, TEXT, 0).with_crcs(0),
            contribution(2, 0x30, 0x20, TEXT, 0).with_crcs(0),
        ]
    );
    // The module record gets the first contribution of its module.
    assert_eq!(record, contribution(1, 0, 4, DATA, 0).with_crcs(0));
    assert_eq!(
        counts,
        Counts {
            cleared_crcs: 4,
            added: 1,
            removed: [0; 5],
        }
    );
}

/// Set both CRCs of a contribution.
trait WithCrcs {
    fn with_crcs(self, crc: u32) -> Self;
}

impl WithCrcs for SectionContribution {
    fn with_crcs(mut self, crc: u32) -> Self {
        self.data_crc = Some(crc);
        self.reloc_crc = Some(crc);
        self
    }
}

#[test]
fn the_identity_keeps_the_dbi_values() {
    let source = synthetic(with_crcs);
    let (output, counts) = clean(
        &source,
        &remap(&source_sections(), &IDENTITY),
        RemovedCode::Fail,
    )
    .expect("clean output");
    assert_eq!(contributions(&output), contributions(&source));
    assert_eq!(
        contributions(&output).0[0],
        contribution(1, 0, 0x50, TEXT, 0x11)
    );
    assert_eq!(section_map(&output), section_map(&source));
    assert_eq!(headers(&output), headers(&source));
    assert_eq!(
        counts,
        Counts {
            cleared_crcs: 0,
            added: 0,
            removed: [0; 5],
        }
    );
}

#[test]
fn the_section_map_follows_the_new_section_order() {
    let source = synthetic(|_| {});
    let (output, _) =
        clean(&source, &remap(&new_sections(), &RANGES), RemovedCode::Fail).expect("clean output");
    let map = section_map(&output).expect("section map");
    let selector = SegmentFlags::IS_SELECTOR.0 | SegmentFlags::ADDRESS_IS_32_BIT.0;
    let entries: Vec<(u16, u16, u32)> = map
        .entries
        .iter()
        .map(|entry| (entry.flags.0, entry.frame, entry.length))
        .collect();
    assert_eq!(
        entries,
        vec![
            (
                selector | SegmentFlags::READ.0 | SegmentFlags::WRITE.0,
                1,
                0x100
            ),
            (
                selector | SegmentFlags::READ.0 | SegmentFlags::EXECUTE.0,
                2,
                0x100
            ),
            (
                SegmentFlags::IS_ABSOLUTE_ADDRESS.0 | SegmentFlags::ADDRESS_IS_32_BIT.0,
                0,
                u32::MAX,
            ),
        ]
    );
    assert_eq!(map.logical_count, 3);
    for entry in &map.entries {
        assert_eq!((entry.section_name, entry.class_name), (0xffff, 0xffff));
    }
    // The map of the source section headers is the map of the synthetic program.
    let (same, _) = clean(
        &source,
        &remap(&source_sections(), &IDENTITY),
        RemovedCode::Fail,
    )
    .expect("clean output");
    assert_eq!(section_map(&same), section_map(&source));
}

#[test]
fn the_absolute_entry_has_frame_0_also_after_an_lld_section_map() {
    // `lld-link` writes the number of the entry as the frame of the absolute entry. DIA then
    // finds no address in the last section, which is `.text` in the new image.
    let mut lld = section_map(&synthetic(|_| {})).expect("section map");
    let count = u16::try_from(lld.entries.len()).expect("count");
    lld.entries.last_mut().expect("absolute entry").frame = count;
    let source = synthetic(|editor| {
        editor
            .debug_info_mut()
            .expect("DBI")
            .set_section_map(Some(lld.clone()));
    });
    assert_eq!(section_map(&source), Some(lld));
    let (output, _) =
        clean(&source, &remap(&new_sections(), &RANGES), RemovedCode::Fail).expect("clean output");
    let map = section_map(&output).expect("section map");
    let frames: Vec<u16> = map.entries.iter().map(|entry| entry.frame).collect();
    assert_eq!(frames, [1, 2, 0]);
    assert_eq!(map.logical_count, 3);
}

#[test]
fn records_move_and_sort_again() {
    let source = synthetic(|editor| {
        with_records(
            editor,
            &[(0x1000, 0x20), (0x1020, 0x30)],
            &[
                fixup(1, 0x1010, 0x2000),
                fixup(2, 0x1030, 0x1000),
                fixup(3, 0, 0x1020),
            ],
        );
    });
    let (output, _) =
        clean(&source, &remap(&new_sections(), &RANGES), RemovedCode::Fail).expect("clean output");
    let pdb = Pdb::parse(&output[..]).expect("output");
    // `main` (old 0x1020) is first now. The other fields stay.
    let fpos: Vec<FpoData> = pdb.fpo().expect("FPO").expect("slot 0").iter().collect();
    let mut expected = [fpo(0x1020, 0x30), fpo(0x1000, 0x20)];
    expected[0].start = 0x3000;
    expected[1].start = 0x3030;
    assert_eq!(fpos, expected);
    let frames: Vec<FrameData> = pdb
        .frame_data()
        .expect("new FPO")
        .expect("slot 9")
        .iter()
        .collect();
    let mut expected = [frame(0x1020, 0x30), frame(0x1000, 0x20)];
    expected[0].start = 0x3000;
    expected[1].start = 0x3030;
    assert_eq!(frames, expected);
    let functions: Vec<FunctionEntry> = pdb
        .exception_data()
        .expect("exception data")
        .expect("slot 1")
        .iter()
        .collect();
    assert_eq!(
        functions,
        [
            function(0x3000, 0x3030, 0x3004),
            function(0x3030, 0x3050, 0x3034)
        ]
    );
    // The fixups keep their order. A value 0 is not an address.
    let fixups: Vec<Fixup> = pdb
        .fixups()
        .expect("fixups")
        .expect("slot 2")
        .iter()
        .collect();
    assert_eq!(
        fixups,
        [
            fixup(1, 0x3040, 0x2000),
            fixup(2, 0x3010, 0x3030),
            fixup(3, 0, 0x3000)
        ]
    );
}

#[test]
fn a_split_record_has_no_split_form() {
    for (kind, item) in [
        (DebugStream::Fpo, RemapItem::Fpo),
        (DebugStream::NewFpo, RemapItem::NewFpo),
        (DebugStream::Exception, RemapItem::ExceptionData),
    ] {
        // The record runs from `sum` into `main`, so the map splits it.
        let source = synthetic(|editor| {
            with_records(editor, &[(0x1010, 0x20)], &[]);
            let list = editor.debug_streams_mut().expect("debug streams");
            for other in [
                DebugStream::Fpo,
                DebugStream::NewFpo,
                DebugStream::Exception,
            ] {
                if other != kind {
                    list.remove(other);
                }
            }
        });
        for policy in [RemovedCode::Fail, RemovedCode::Drop] {
            let error = clean(&source, &remap(&new_sections(), &RANGES), policy)
                .expect_err("a split record fails");
            assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
            let address = error.remap_address().expect("the error names the record");
            assert_eq!((address.item, address.address), (item, PdbRva(0x1010)));
        }
    }
}

#[test]
fn removed_records_fail_or_drop() {
    // The records at 0x1060 lie in the removed rest of `.text`.
    let source = synthetic(|editor| {
        with_records(
            editor,
            &[(0x1000, 0x20), (0x1060, 0x10)],
            &[
                fixup(1, 0x1010, 0x2000),
                fixup(2, 0x1060, 0x2000),
                fixup(3, 0x1010, 0x1064),
            ],
        );
        let info = editor.debug_info_mut().expect("DBI");
        let mut list = info.section_contributions().to_vec();
        list.push(contribution(1, 0x60, 0x10, TEXT, 0x31));
        info.set_section_contributions(SectionContributionVersion::V60, list);
    });
    let remap = remap(&new_sections(), &RANGES);
    let error = clean(&source, &remap, RemovedCode::Fail).expect_err("removed code fails");
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
    let address = error.remap_address().expect("the error names the record");
    assert_eq!(address.address, PdbRva(0x1060));
    let (output, counts) = clean(&source, &remap, RemovedCode::Drop).expect("clean output");
    assert_eq!(counts.removed, [1, 1, 1, 1, 2]);
    let pdb = Pdb::parse(&output[..]).expect("output");
    let starts: Vec<u32> = pdb
        .fpo()
        .expect("FPO")
        .expect("slot 0")
        .iter()
        .map(|record| record.start)
        .collect();
    assert_eq!(starts, [0x3030]);
    assert_eq!(pdb.frame_data().expect("new FPO").expect("slot 9").len(), 1);
    assert_eq!(
        pdb.exception_data()
            .expect("exception data")
            .expect("slot 1")
            .len(),
        1
    );
    let fixups: Vec<Fixup> = pdb
        .fixups()
        .expect("fixups")
        .expect("slot 2")
        .iter()
        .collect();
    assert_eq!(fixups, [fixup(1, 0x3040, 0x2000)]);
    assert_eq!(contributions(&output).0.len(), 3);
}

/// Return a blob with `data` at `data_rva`.
fn blob(data_rva: u32, data: &[u8]) -> RvaVaBlob {
    let mut header = BlobHeader::default();
    header.data_rva = data_rva;
    header.image_base = 0x1_4000_0000;
    RvaVaBlob::new(header, &[], data).expect("blob")
}

#[test]
fn unwind_streams_come_from_the_caller() {
    let source = synthetic(|editor| {
        set_stream(
            editor,
            DebugStream::Xdata,
            blob(0x1100, &[1, 2, 3, 4]).to_bytes().expect("xdata"),
        );
    });
    let pdb = Pdb::parse(&source[..]).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let plain = remap(&new_sections(), &RANGES);
    let error = editor
        .remap_addresses(&plain, RemapMode::Clean(RemovedCode::Fail))
        .expect_err("clean output needs the unwind streams");
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
    // OMAP output copies the streams.
    editor
        .remap_addresses(&plain, RemapMode::Omap)
        .expect("OMAP output");
    editor.plan(EditOptions::default()).expect("OMAP plan");
    let pdata = blob(0x3000, &[5; 12]);
    let xdata = blob(0x3100, &[6, 7, 8, 9]);
    let with_unwind = plain.with_unwind_streams(pdata.clone(), xdata.clone());
    let (output, _) = clean(&source, &with_unwind, RemovedCode::Fail).expect("clean output");
    let pdb = Pdb::parse(&output[..]).expect("output");
    for (kind, expected) in [(DebugStream::Pdata, &pdata), (DebugStream::Xdata, &xdata)] {
        let written = pdb.debug_blob(kind).expect("blob").expect("blob slot");
        assert_eq!(
            written.to_bytes().expect("bytes"),
            expected.to_bytes().expect("bytes")
        );
    }
}

#[test]
fn clean_output_has_no_omap_and_the_new_section_headers() {
    let source = synthetic(|_| {});
    let first = remap(&new_sections(), &RANGES);
    let (output, _) = clean(&source, &first, RemovedCode::Fail).expect("clean output");
    assert_eq!(slots(&output), [DebugStream::SectionHeader]);
    assert_eq!(headers(&output), new_sections());
    // A source with OMAP: the OMAP output of the first map.
    let pdb = Pdb::parse(&source[..]).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .remap_addresses(&first, RemapMode::Omap)
        .expect("OMAP output");
    let omap = editor
        .plan(EditOptions::default())
        .and_then(|plan| plan.to_vec())
        .expect("OMAP output");
    assert_eq!(
        slots(&omap),
        [
            DebugStream::OmapToSource,
            DebugStream::OmapFromSource,
            DebugStream::SectionHeader,
            DebugStream::OriginalSectionHeader,
        ]
    );
    // The second map moves `.text` of the OMAP image to 0x4000 and `.data` to 0x5000.
    let last = vec![
        section(*b".text\0\0\0", 0x4000, TEXT),
        section(*b".data\0\0\0", 0x5000, DATA),
    ];
    let second = remap(&last, &[(0x3000, 0x50, 0x4000), (0x2000, 0x100, 0x5000)]);
    let (output, _) = clean(&omap, &second, RemovedCode::Fail).expect("clean output");
    assert_eq!(slots(&output), [DebugStream::SectionHeader]);
    assert_eq!(headers(&output), last);
    // The contributions use the addresses of the last image.
    assert_eq!(
        contributions(&output).0,
        [
            contribution(1, 0, 0x30, TEXT, 0).with_crcs(0),
            contribution(1, 0x30, 0x20, TEXT, 0).with_crcs(0),
            contribution(2, 0, 4, DATA, 0).with_crcs(0),
        ]
    );
}
