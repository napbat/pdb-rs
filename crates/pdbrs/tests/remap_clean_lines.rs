//! Tests of the clean output of `PdbEditor::remap_addresses` for the C13 subsections of a
//! module: line blocks, IL lines, frame data, COFF symbol RVAs and opaque subsections.
//!
//! The source PDB is the synthetic program of the `native_synthetic` example: `.text` at RVA
//! `0x1000` (`sum` at `0x1000`, 0x20 bytes, lines 3, 4 and 5 at the offsets 0, 8 and 0x1a;
//! `main` at `0x1020`, 0x30 bytes) and `.data` at RVA `0x2000`. Some tests add subsections
//! to the module of the program and write a new source PDB.

#[path = "../examples/native_synthetic/program.rs"]
mod program;

use pdbrs::native::address::{PdbRva, Rva};
use pdbrs::native::codeview::index::{ChecksumOffset, ModuleIndex};
use pdbrs::native::fpo::FrameData;
use pdbrs::native::lines::{
    CoffSymbolRvas, DEBUG_S_COFF_SYMBOL_RVA, DEBUG_S_FRAMEDATA, DEBUG_S_IL_LINES, DEBUG_S_LINES,
    FrameDataSubsection, LineBlock, LineEntry, LineSubsection, LinesHeader, ModuleLines,
    SubsectionKind,
};
use pdbrs::native::locate::SymbolLocator;
use pdbrs::native::msf::StreamId;
use pdbrs::native::section::{SectionHeader, SectionHeaderStream};
use pdbrs::native::{
    EditOptions, IdentityPolicy, ImageRemap, Pdb, RemapItem, RemapMode, RemovedCode,
};
use pdbrs::{Error, ErrorKind};

/// The characteristics of `.text` and `.data` in the synthetic program.
const TEXT: u32 = 0x6000_0020;
const DATA: u32 = 0xc000_0040;

/// One range: old RVA, length, new RVA.
type Range = (u32, u32, u32);

/// The ranges of a new image with `.text` at `0x3000`: `main` moves before `sum`, the rest
/// of `.text` follows, and `.data` stays.
const MOVED: [Range; 4] = [
    (0x1020, 0x30, 0x3000),
    (0x1000, 0x20, 0x3030),
    (0x1050, 0xb0, 0x3050),
    (0x2000, 0x100, 0x2000),
];

/// The ranges of a new image with the source sections: `sum` splits at offset 0xc, inside
/// the entry of line 4 (offsets 8 to 0x1a). The second part moves to `0x1000`, the first
/// part to `0x1080`.
const SPLIT: [Range; 5] = [
    (0x1000, 0xc, 0x1080),
    (0x100c, 0x14, 0x1000),
    (0x1020, 0x30, 0x1020),
    (0x1050, 0x30, 0x1050),
    (0x2000, 0x100, 0x2000),
];

/// The new address of the old RVA `old` for [`SPLIT`].
fn split_rva(old: u32) -> u32 {
    SPLIT
        .iter()
        .find(|&&(start, length, _)| (start..start + length).contains(&old))
        .map(|&(start, _, new)| new + (old - start))
        .expect("the RVA has a range")
}

/// The ranges of a new image with the source sections that remove `.text+0x68` to
/// `.text+0x70`.
const HOLE: [Range; 3] = [
    (0x1000, 0x68, 0x1000),
    (0x1070, 0x90, 0x1070),
    (0x2000, 0x100, 0x2000),
];

/// The PDB of the synthetic program.
fn synthetic() -> Vec<u8> {
    program::build()
        .and_then(|editor| editor.plan(EditOptions::default())?.to_vec())
        .expect("the synthetic program builds")
}

/// Return the synthetic PDB with the subsections `extra` after the subsections of its
/// module.
fn with_subsections(extra: &[(SubsectionKind, Vec<u8>)]) -> Vec<u8> {
    let source = synthetic();
    let pdb = Pdb::parse(&source[..]).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let module = editor.module_mut(ModuleIndex(0)).expect("module");
    for (kind, data) in extra {
        let subsection = LineSubsection::decode(*kind, data)
            .expect("subsection")
            .into_owned();
        module.lines_mut().push(subsection);
    }
    editor
        .plan(EditOptions::default())
        .and_then(|plan| plan.to_vec())
        .expect("source with subsections")
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

/// Remap `source` with `remap` in `mode` and plan it. Return the output bytes and the
/// report counts `(removed, added)` of `item`.
fn plan(
    source: &[u8],
    remap: &ImageRemap,
    mode: RemapMode,
    item: RemapItem,
) -> Result<(Vec<u8>, usize, usize), Error> {
    let pdb = Pdb::parse(source).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor.remap_addresses(remap, mode).expect("remap");
    let plan = editor.plan(EditOptions::default())?;
    let (removed, added) = plan
        .report()
        .remap()
        .map_or((0, 0), |report| (report.removed(item), report.added(item)));
    Ok((plan.to_vec().expect("output"), removed, added))
}

/// Return the error of a clean plan of `source` with `remap`.
fn plan_error(source: &[u8], remap: &ImageRemap, policy: RemovedCode) -> Error {
    match plan(
        source,
        remap,
        RemapMode::Clean(policy),
        RemapItem::LineBlock,
    ) {
        Ok(_) => panic!("the plan must fail"),
        Err(error) => error,
    }
}

/// Return the subsections of module 0 of a PDB and the module stream offset of each header.
fn subsections(bytes: &[u8]) -> (StreamId, Vec<(u64, LineSubsection<'static>)>) {
    let pdb = Pdb::parse(bytes).expect("PDB");
    let stream = pdb.module(ModuleIndex(0)).expect("module").expect("stream");
    let base = stream.symbol_offset() + stream.symbol_data().len() + stream.c11_data().len();
    let lines = ModuleLines::parse(&stream).expect("C13 subsections");
    lines.check().expect("file references resolve");
    let list = lines
        .subsections()
        .iter()
        .map(|subsection| {
            let offset = u64::try_from(base).unwrap() + u64::from(subsection.offset);
            (offset, subsection.data.clone().into_owned())
        })
        .collect();
    (stream.stream(), list)
}

/// Return the header and the `(offset, line)` entries of each `DEBUG_S_LINES` subsection.
fn line_blocks(bytes: &[u8]) -> Vec<(LinesHeader, Vec<(u32, u32)>)> {
    subsections(bytes)
        .1
        .into_iter()
        .filter_map(|(_, subsection)| match subsection {
            LineSubsection::Lines(lines) => Some((
                *lines.header(),
                lines
                    .blocks()
                    .flat_map(|block| {
                        block
                            .lines
                            .iter()
                            .map(|entry| (entry.offset, entry.line_start))
                            .collect::<Vec<_>>()
                    })
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

/// Return the section, the offset and the code size of a header.
fn extent(header: &LinesHeader) -> (u16, u32, u32) {
    (header.segment, header.code_offset, header.code_size)
}

/// Return the file and the line of `SymbolLocator::locate` at `rva`.
fn line_at(locator: &SymbolLocator, rva: u32) -> Option<(Vec<u8>, u32)> {
    let location = locator.locate(Rva(rva)).expect("locate")?;
    location.line.map(|line| (line.file.to_vec(), line.line))
}

/// Return the data of a `DEBUG_S_LINES` or `DEBUG_S_IL_LINES` subsection with one block.
fn lines_data(segment: u16, code_offset: u32, code_size: u32, lines: &[(u32, u32)]) -> Vec<u8> {
    let mut header = LinesHeader::default();
    header.segment = segment;
    header.code_offset = code_offset;
    header.code_size = code_size;
    let mut data = Vec::new();
    header.encode(&mut data).expect("header");
    let entries: Vec<LineEntry> = lines
        .iter()
        .map(|&(offset, line)| {
            let mut entry = LineEntry::default();
            entry.offset = offset;
            entry.line_start = line;
            entry.is_statement = true;
            entry
        })
        .collect();
    // The file checksum entry of the source is at offset 0.
    LineBlock::encode_parts(ChecksumOffset(0), &entries, None, &mut data).expect("block");
    data
}

/// Return the data of a `DEBUG_S_FRAMEDATA` subsection with one record at `start`.
fn frame_data(start: u32, size: u32) -> Vec<u8> {
    let mut record = FrameData::default();
    record.start = start;
    record.size = size;
    record.prolog_size = 4;
    let mut data = Vec::new();
    FrameDataSubsection::encode_parts(Some(0), &[record], &mut data).expect("frame data");
    data
}

/// Return the frame data records of module 0 of a PDB as `(start, size)`.
fn frame_records(bytes: &[u8]) -> Vec<(u32, u32)> {
    subsections(bytes)
        .1
        .iter()
        .filter_map(|(_, subsection)| match subsection {
            LineSubsection::FrameData(frames) => Some(
                frames
                    .records()
                    .iter()
                    .map(|record| (record.start, record.size))
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect()
}

#[test]
fn a_moved_line_block_gets_a_new_header() {
    let source = synthetic();
    let moved = remap(moved_sections(), &MOVED);
    let mode = RemapMode::Clean(RemovedCode::Fail);
    let (bytes, removed, added) = plan(&source, &moved, mode, RemapItem::LineBlock).expect("plan");
    assert_eq!((removed, added), (0, 0));
    let blocks = line_blocks(&bytes);
    let extents: Vec<_> = blocks.iter().map(|(header, _)| extent(header)).collect();
    // `.text` is section 2 of the new image: `sum` at 0x30, `main` at 0.
    assert_eq!(extents, [(2, 0x30, 0x20), (2, 0, 0x30)]);
    assert_eq!(blocks[0].1, [(0, 3), (8, 4), (0x1a, 5)]);
    assert_eq!(blocks, {
        let mut source_blocks = line_blocks(&source);
        for ((header, _), new) in source_blocks.iter_mut().zip(&extents) {
            (header.segment, header.code_offset) = (new.0, new.1);
        }
        source_blocks
    });
}

#[test]
fn a_split_line_block_gets_one_subsection_for_each_part() {
    let source = synthetic();
    let split = remap(source_sections(), &SPLIT);
    let mode = RemapMode::Clean(RemovedCode::Fail);
    let (bytes, removed, added) = plan(&source, &split, mode, RemapItem::LineBlock).expect("plan");
    assert_eq!((removed, added), (0, 1));
    let blocks = line_blocks(&bytes);
    let summary: Vec<_> = blocks
        .iter()
        .map(|(header, entries)| (extent(header), entries.clone()))
        .collect();
    assert_eq!(
        summary,
        [
            // The first part of `sum` at `.text+0x80`.
            ((1, 0x80, 0xc), vec![(0, 3), (8, 4)]),
            // The second part starts inside the entry of line 4, so it gets an entry at 0.
            ((1, 0, 0x14), vec![(0, 4), (0xe, 5)]),
            (
                (1, 0x20, 0x30),
                vec![(0, 7), (8, 8), (0x14, 9), (0x24, 10), (0x2c, 11)]
            ),
        ]
    );
    let old_pdb = Pdb::parse(&source[..]).expect("source");
    let old = SymbolLocator::new(&old_pdb).expect("source locator");
    let new_pdb = Pdb::parse(&bytes[..]).expect("output");
    let new = SymbolLocator::new(&new_pdb).expect("output locator");
    for rva in 0x1000..0x1050 {
        let expected = line_at(&old, rva);
        assert!(expected.is_some(), "source line at {rva:#x}");
        assert_eq!(line_at(&new, split_rva(rva)), expected, "old RVA {rva:#x}");
    }
    assert_eq!(line_at(&new, 0x1000), Some((program::SOURCE.to_vec(), 4)));
}

#[test]
fn a_removed_part_is_dropped_and_counted() {
    // A block without symbols at `.text+0x60`: line 20 at 0, line 21 at 8.
    let source = with_subsections(&[(
        DEBUG_S_LINES,
        lines_data(1, 0x60, 0x10, &[(0, 20), (8, 21)]),
    )]);
    let hole = remap(source_sections(), &HOLE);
    let mode = RemapMode::Clean(RemovedCode::Drop);
    let (bytes, removed, added) = plan(&source, &hole, mode, RemapItem::LineBlock).expect("plan");
    assert_eq!((removed, added), (1, 0));
    let blocks = line_blocks(&bytes);
    let last = blocks.last().expect("the block of the hole");
    assert_eq!(
        (extent(&last.0), last.1.clone()),
        ((1, 0x60, 8), vec![(0, 20)])
    );
    // The other blocks do not change.
    assert_eq!(blocks[..2], line_blocks(&source)[..2]);
}

#[test]
fn a_removed_part_fails_with_the_fail_policy() {
    let source = with_subsections(&[(
        DEBUG_S_LINES,
        lines_data(1, 0x60, 0x10, &[(0, 20), (8, 21)]),
    )]);
    let (stream, list) = subsections(&source);
    let (offset, _) = list.last().expect("the added subsection");
    let error = plan_error(&source, &remap(source_sections(), &HOLE), RemovedCode::Fail);
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
    let address = error.remap_address().expect("record address");
    assert_eq!(address.item, RemapItem::LineBlock);
    assert_eq!(address.address, PdbRva(0x1068));
    assert_eq!(error.stream_location(), Some((stream, *offset)));
}

#[test]
fn frame_data_moves_with_its_range() {
    let source = with_subsections(&[(DEBUG_S_FRAMEDATA, frame_data(0x1020, 0x30))]);
    let moved = remap(moved_sections(), &MOVED);
    let mode = RemapMode::Clean(RemovedCode::Fail);
    let (bytes, _, _) = plan(&source, &moved, mode, RemapItem::ModuleFrameData).expect("plan");
    assert_eq!(frame_records(&bytes), [(0x3000, 0x30)]);
}

#[test]
fn split_frame_data_fails() {
    let source = with_subsections(&[(DEBUG_S_FRAMEDATA, frame_data(0x1000, 0x20))]);
    let (stream, list) = subsections(&source);
    let (offset, _) = list.last().expect("the added subsection");
    let error = plan_error(
        &source,
        &remap(source_sections(), &SPLIT),
        RemovedCode::Drop,
    );
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
    let address = error.remap_address().expect("record address");
    assert_eq!(address.item, RemapItem::ModuleFrameData);
    assert_eq!(address.address, PdbRva(0x1000));
    assert_eq!(error.stream_location(), Some((stream, *offset)));
}

#[test]
fn coff_symbol_rvas_map_as_points() {
    let rvas = [Rva(0x1020), Rva(0), Rva(u32::MAX), Rva(0x1004)];
    let mut data = Vec::new();
    CoffSymbolRvas::encode_parts(&rvas, &mut data).expect("RVAs");
    let source = with_subsections(&[(DEBUG_S_COFF_SYMBOL_RVA, data)]);
    let moved = remap(moved_sections(), &MOVED);
    let mode = RemapMode::Clean(RemovedCode::Fail);
    let (bytes, _, _) = plan(&source, &moved, mode, RemapItem::CoffSymbolRva).expect("plan");
    let values: Vec<u32> = subsections(&bytes)
        .1
        .iter()
        .filter_map(|(_, subsection)| match subsection {
            LineSubsection::CoffSymbolRvas(rvas) => {
                Some(rvas.iter().map(|rva| rva.0).collect::<Vec<_>>())
            }
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(values, [0x3000, 0, u32::MAX, 0x3034]);
}

#[test]
fn the_header_start_of_il_lines_maps_as_a_point() {
    let source = with_subsections(&[(
        DEBUG_S_IL_LINES,
        lines_data(1, 0x20, 0x100, &[(0, 7), (0x40, 8)]),
    )]);
    let moved = remap(moved_sections(), &MOVED);
    let mode = RemapMode::Clean(RemovedCode::Fail);
    let (bytes, _, _) = plan(&source, &moved, mode, RemapItem::LineBlock).expect("plan");
    let headers: Vec<_> = subsections(&bytes)
        .1
        .iter()
        .filter_map(|(_, subsection)| match subsection {
            LineSubsection::IlLines(lines) => Some(extent(lines.header())),
            _ => None,
        })
        .collect();
    // The IL code size is not a code extent, so it stays.
    assert_eq!(headers, [(2, 0, 0x100)]);
}

#[test]
fn the_clean_identity_keeps_the_module_stream() {
    let mut rvas = Vec::new();
    CoffSymbolRvas::encode_parts(&[Rva(0x1020), Rva(0)], &mut rvas).expect("RVAs");
    let source = with_subsections(&[
        (DEBUG_S_FRAMEDATA, frame_data(0x1000, 0x20)),
        (DEBUG_S_COFF_SYMBOL_RVA, rvas),
    ]);
    let identity = remap(
        source_sections(),
        &[(0x1000, 0x100, 0x1000), (0x2000, 0x100, 0x2000)],
    );
    let mode = RemapMode::Clean(RemovedCode::Fail);
    let (bytes, _, _) = plan(&source, &identity, mode, RemapItem::LineBlock).expect("plan");
    let module = |bytes: &[u8]| {
        let pdb = Pdb::parse(bytes).expect("PDB");
        let stream = pdb.module(ModuleIndex(0)).expect("module").expect("stream");
        stream.c13_data().to_vec()
    };
    assert_eq!(module(&bytes), module(&source));
}

#[test]
fn an_unknown_subsection_fails_only_in_clean_output() {
    let source = with_subsections(&[(SubsectionKind(0x1234), vec![1, 2, 3, 4])]);
    let (stream, list) = subsections(&source);
    let (offset, subsection) = list.last().expect("the added subsection");
    assert!(matches!(subsection, LineSubsection::Unsupported { .. }));
    let moved = remap(moved_sections(), &MOVED);
    for policy in [RemovedCode::Fail, RemovedCode::Drop] {
        let error = plan_error(&source, &moved, policy);
        assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
        assert_eq!(error.stream_location(), Some((stream, *offset)));
    }
    plan(&source, &moved, RemapMode::Omap, RemapItem::LineBlock).expect("OMAP output");
}
