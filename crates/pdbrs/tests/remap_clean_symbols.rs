//! Tests of the clean rewrite of the module symbols (#43, decisions 4 and 5).
//!
//! The source PDB is the synthetic program of the `native_synthetic` example (`sum` at
//! `.text+0x0`, `main` at `.text+0x20`) with two more procedures in module 0:
//!
//! - `big` at `.text+0x50`, 0x40 bytes. It holds an `S_BLOCK32` at `0x58..0x68`, an
//!   `S_LOCAL` with an `S_DEFRANGE_FRAMEPOINTER_REL` at `0x54..0x8c` (gaps at offsets
//!   `4..8` and `0x18..0x20` of the range) and an inline site of `sum` at offsets
//!   `0x28..0x30` of `big`.
//! - `extra` at `.text+0xa0`, 0x10 bytes.
//!
//! `.text` is at RVA `0x1000` and `.data` at RVA `0x2000`. The maps keep the section headers,
//! so that the tests do not depend on the other parts of the clean output.

#[path = "../examples/native_synthetic/program.rs"]
mod program;

use pdbrs::ErrorKind;
use pdbrs::native::address::{PdbRva, Rva, SectionOffset};
use pdbrs::native::codeview::SymbolKind;
use pdbrs::native::codeview::index::{IdIndex, ModuleIndex, ModuleSymbolOffset, TypeIndex};
use pdbrs::native::codeview::symbol::{
    AddressGap, AddressGaps, AddressRange, Annotation, BinaryAnnotations, Block,
    DefRangeFramePointerRelative, InlineSite, Local, LocalFlags, Procedure, ProcedureFlags,
    ProcedureType, S_BLOCK32, S_DEFRANGE_FRAMEPOINTER_REL, S_END, S_GPROC32, S_INLINESITE,
    S_INLINESITE_END, S_LOCAL, S_SEPCODE, Symbol, SymbolData,
};
use pdbrs::native::lines::{DEBUG_S_INLINEELINES, LineSubsection};
use pdbrs::native::locate::SymbolLocator;
use pdbrs::native::msf::StreamId;
use pdbrs::native::section::{SectionHeader, SectionHeaderStream};
use pdbrs::native::{
    EditOptions, IdentityPolicy, ImageRemap, Pdb, RemapItem, RemapMode, RemovedCode,
};

/// The ID of `sum` in the IPI stream of the synthetic program.
const SUM_ID: u32 = 0x1000;

/// One range: old RVA, length, new RVA.
type Range = (u32, u32, u32);

/// The identity range of `.data`.
const DATA: Range = (0x2000, 0x100, 0x2000);

/// Return an address in `.text`.
const fn text(offset: u32) -> SectionOffset {
    SectionOffset { section: 1, offset }
}

/// Encode `annotations`.
fn annotations(annotations: &[Annotation]) -> BinaryAnnotations<'static> {
    let mut bytes = Vec::new();
    for annotation in annotations {
        annotation.encode(&mut bytes).expect("annotation");
    }
    BinaryAnnotations::parse(bytes).expect("annotations")
}

/// Return a procedure record at `.text+offset`.
fn procedure(name: &'static [u8], offset: u32, length: u32) -> SymbolData<'static> {
    let mut procedure = Procedure::new(ProcedureType::Type(TypeIndex(0)));
    procedure.next = ModuleSymbolOffset(0);
    procedure.code_length = length;
    procedure.debug_start = 4;
    procedure.debug_end = length - 8;
    procedure.address = text(offset);
    procedure.flags = ProcedureFlags(0);
    procedure.extended_flags = None;
    procedure.name = name.into();
    SymbolData::Procedure(procedure)
}

/// Return the gap bytes of `(start, length)` pairs.
fn gaps(pairs: &[(u16, u16)]) -> AddressGaps<'static> {
    let mut bytes = Vec::new();
    for (start, length) in pairs {
        bytes.extend_from_slice(&start.to_le_bytes());
        bytes.extend_from_slice(&length.to_le_bytes());
    }
    AddressGaps::parse(bytes).expect("gaps")
}

/// Build the source PDB.
fn source() -> Vec<u8> {
    let mut editor = program::build().expect("synthetic program");
    let module = editor.module_mut(ModuleIndex(0)).expect("module 0");
    module
        .push(S_GPROC32, procedure(b"big", 0x50, 0x40))
        .expect("big");
    let mut block = Block::default();
    block.code_length = 0x10;
    block.address = text(0x58);
    module
        .push(S_BLOCK32, SymbolData::Block(block))
        .expect("block");
    module.push(S_END, SymbolData::ScopeEnd).expect("block end");
    let mut local = Local::default();
    local.type_index = TypeIndex(0x74);
    local.flags = LocalFlags(0);
    local.name = b"value".into();
    module
        .push(S_LOCAL, SymbolData::Local(local))
        .expect("local");
    let mut range = AddressRange::default();
    range.start = text(0x54);
    range.length = 0x38;
    let mut def_range = DefRangeFramePointerRelative::default();
    def_range.offset = 8;
    def_range.range = range;
    def_range.gaps = gaps(&[(4, 4), (0x18, 8)]);
    module
        .push(
            S_DEFRANGE_FRAMEPOINTER_REL,
            SymbolData::DefRangeFramePointerRelative(def_range),
        )
        .expect("def-range");
    let mut site = InlineSite::default();
    site.inlinee = IdIndex(SUM_ID);
    site.invocations = None;
    site.annotations = annotations(&[
        Annotation::ChangeLineOffset(1),
        Annotation::ChangeCodeOffset(0x28),
        Annotation::ChangeCodeLength(8),
    ]);
    module
        .push(S_INLINESITE, SymbolData::InlineSite(site))
        .expect("inline site");
    module
        .push(S_INLINESITE_END, SymbolData::InlineSiteEnd)
        .expect("inline site end");
    module.push(S_END, SymbolData::ScopeEnd).expect("big end");
    module
        .push(S_GPROC32, procedure(b"extra", 0xa0, 0x10))
        .expect("extra");
    module.push(S_END, SymbolData::ScopeEnd).expect("extra end");
    // The inlinee line of `sum`: the file checksum at offset 0, line 3.
    let mut data = Vec::new();
    for value in [0_u32, SUM_ID, 0, 3] {
        data.extend_from_slice(&value.to_le_bytes());
    }
    let inlinees = LineSubsection::decode(DEBUG_S_INLINEELINES, &data)
        .expect("inlinee lines")
        .into_owned();
    module.lines_mut().push(inlinees);
    editor
        .plan(EditOptions::default())
        .and_then(|plan| plan.to_vec())
        .expect("source PDB")
}

/// Return the section headers of the source image.
fn sections() -> SectionHeaderStream {
    let header = |name: [u8; 8], virtual_address: u32, characteristics: u32| {
        let mut header = SectionHeader::new(name, 0x100, virtual_address);
        header.characteristics = characteristics;
        header
    };
    SectionHeaderStream::from_records(&[
        header(*b".text\0\0\0", 0x1000, 0x6000_0020),
        header(*b".data\0\0\0", 0x2000, 0xc000_0040),
    ])
    .expect("section headers")
}

/// Make a map with the source section headers and `ranges`.
fn remap(ranges: &[Range]) -> ImageRemap {
    ranges
        .iter()
        .fold(ImageRemap::new(sections()), |remap, &(old, length, new)| {
            remap
                .with_range(Rva(old), length, Rva(new))
                .expect("valid range")
        })
}

/// The output bytes of a clean remap and the report counts `(removed, added)` of some kinds.
type Output = (Vec<u8>, Vec<(usize, usize)>);

/// Remap `source` with `ranges` and `policy`, and return the output bytes and the report
/// counts of `kinds`.
fn clean(
    source: &[u8],
    ranges: &[Range],
    policy: RemovedCode,
    kinds: &[SymbolKind],
) -> pdbrs::Result<Output> {
    let pdb = Pdb::parse(source).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor.remap_addresses(&remap(ranges), RemapMode::Clean(policy))?;
    let plan = editor.plan(EditOptions::default())?;
    let report = plan.report().remap().expect("clean report");
    let counts = kinds
        .iter()
        .map(|&kind| {
            let item = RemapItem::Symbol(kind);
            (report.removed(item), report.added(item))
        })
        .collect();
    Ok((plan.to_vec()?, counts))
}

/// The symbol records of module 0: the module stream, the module stream offset and the
/// decoded record.
fn module_symbols(bytes: &[u8]) -> (StreamId, Vec<(u32, Symbol<'static>)>) {
    let pdb = Pdb::parse(bytes).expect("PDB");
    let stream = pdb
        .module(ModuleIndex(0))
        .expect("module stream")
        .expect("module 0 has a stream");
    let base = u32::try_from(stream.symbol_offset()).expect("offset");
    let records = stream
        .symbols()
        .map(|record| {
            let record = record.expect("record");
            let symbol = Symbol::decode(record).expect("symbol").into_owned();
            (base + record.offset, symbol)
        })
        .collect();
    (stream.stream(), records)
}

/// Return the procedure record `name` of module 0.
fn find_procedure<'s>(
    symbols: &'s [(u32, Symbol<'static>)],
    name: &[u8],
) -> Option<&'s Procedure<'static>> {
    symbols.iter().find_map(|(_, symbol)| match symbol.data() {
        SymbolData::Procedure(procedure) if procedure.name.as_bytes() == name => Some(procedure),
        _ => None,
    })
}

/// Return the module stream offset of the first record of `kind` in module 0.
fn offset_of(symbols: &[(u32, Symbol<'static>)], kind: SymbolKind) -> u64 {
    symbols
        .iter()
        .find(|(_, symbol)| symbol.kind() == kind)
        .map(|(offset, _)| u64::from(*offset))
        .expect("record of the kind")
}

/// Return the name of the procedure and the inlinees at `rva` of a PDB.
fn locate(bytes: &[u8], rva: u32) -> (Option<Vec<u8>>, Vec<IdIndex>) {
    let pdb = Pdb::parse(bytes).expect("PDB");
    let locator = SymbolLocator::new(&pdb).expect("locator");
    let location = locator.locate(Rva(rva)).expect("locate").expect("location");
    (
        location.procedure.map(|procedure| procedure.name.to_vec()),
        location
            .inline_frames
            .iter()
            .map(|frame| frame.inlinee)
            .collect(),
    )
}

/// The map that moves `big` to `.text+0xc0` and the code after it to `.text+0x50`.
const MOVED: [Range; 4] = [
    (0x1000, 0x50, 0x1000),
    (0x1050, 0x40, 0x10c0),
    (0x1090, 0x70, 0x1050),
    DATA,
];

/// The map that moves the second half of `big` to `.text+0xe0`, and the code after `big` to
/// `.text+0x70`.
const SPLIT: [Range; 4] = [
    (0x1000, 0x70, 0x1000),
    (0x1070, 0x20, 0x10e0),
    (0x1090, 0x50, 0x1070),
    DATA,
];

#[test]
fn a_moved_procedure_gets_its_new_address_and_keeps_its_inline_site() {
    let source = source();
    let (output, counts) =
        clean(&source, &MOVED, RemovedCode::Fail, &[S_SEPCODE]).expect("clean output");
    assert_eq!(counts, [(0, 0)]);
    let (_, symbols) = module_symbols(&output);
    let big = find_procedure(&symbols, b"big").expect("big");
    assert_eq!(big.address, text(0xc0));
    assert_eq!(big.code_length, 0x40);
    assert_eq!(
        find_procedure(&symbols, b"extra").expect("extra").address,
        text(0x60)
    );
    let block = symbols
        .iter()
        .find_map(|(_, symbol)| match symbol.data() {
            SymbolData::Block(block) => Some(block.address),
            _ => None,
        })
        .expect("block");
    assert_eq!(block, text(0xc8));
    // A procedure that does not split keeps the annotations of its inline sites.
    let (_, source_symbols) = module_symbols(&source);
    let site = |symbols: &[(u32, Symbol<'static>)]| {
        symbols
            .iter()
            .find(|(_, symbol)| symbol.kind() == S_INLINESITE)
            .map(|(_, symbol)| symbol.record().data.to_vec())
    };
    assert_eq!(site(&symbols), site(&source_symbols));
    assert!(!symbols.iter().any(|(_, symbol)| symbol.kind() == S_SEPCODE));
    assert_eq!(
        locate(&output, 0x10c0 + 0x2a),
        (Some(b"big".to_vec()), vec![IdIndex(SUM_ID)])
    );
}

#[test]
fn a_split_procedure_gets_a_separated_code_scope() {
    let source = source();
    let (output, counts) = clean(
        &source,
        &SPLIT,
        RemovedCode::Fail,
        &[S_SEPCODE, S_DEFRANGE_FRAMEPOINTER_REL],
    )
    .expect("clean output");
    assert_eq!(counts, [(0, 1), (0, 1)]);
    let (_, symbols) = module_symbols(&output);
    let big = find_procedure(&symbols, b"big").expect("big");
    assert_eq!(big.address, text(0x50));
    assert_eq!(big.code_length, 0x20);
    assert_eq!(big.debug_start, 4);
    assert_eq!(big.debug_end, 0x20);
    let separated: Vec<_> = symbols
        .iter()
        .filter_map(|(_, symbol)| match symbol.data() {
            SymbolData::SeparatedCode(value) => Some(*value),
            _ => None,
        })
        .collect();
    assert_eq!(separated.len(), 1);
    assert_eq!(separated[0].address, text(0xe0));
    assert_eq!(separated[0].code_length, 0x20);
    assert_eq!(separated[0].parent_address, text(0x50));
    assert_eq!(separated[0].flags.0, 0);
    // The scope follows the end of `big` and has no children.
    let position = symbols
        .iter()
        .position(|(_, symbol)| symbol.kind() == S_SEPCODE)
        .expect("S_SEPCODE");
    assert_eq!(symbols[position - 1].1.kind(), S_END);
    assert_eq!(symbols[position + 1].1.kind(), S_END);
    assert_eq!(
        find_procedure(&symbols, b"extra").expect("extra").address,
        text(0x80)
    );
    assert_eq!(locate(&output, 0x10f0).0, Some(b"big".to_vec()));
    assert_eq!(locate(&output, 0x1058).0, Some(b"big".to_vec()));
}

#[test]
fn a_split_def_range_gives_one_record_for_each_part() {
    let source = source();
    let (output, _) = clean(&source, &SPLIT, RemovedCode::Fail, &[]).expect("clean output");
    let (_, symbols) = module_symbols(&output);
    let ranges: Vec<_> = symbols
        .iter()
        .filter_map(|(_, symbol)| match symbol.data() {
            SymbolData::DefRangeFramePointerRelative(value) => Some((
                value.offset,
                value.range.start,
                value.range.length,
                value
                    .gaps
                    .iter()
                    .map(|AddressGap { start, length, .. }| (start, length))
                    .collect::<Vec<_>>(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        ranges,
        [
            (8, text(0x54), 0x1c, vec![(4, 4), (0x18, 4)]),
            (8, text(0xe0), 0x1c, vec![(0, 4)]),
        ]
    );
}

#[test]
fn an_inline_site_in_the_second_part_selects_the_new_chunk() {
    let source = source();
    let (output, _) = clean(&source, &SPLIT, RemovedCode::Fail, &[]).expect("clean output");
    let (_, symbols) = module_symbols(&output);
    let site = symbols
        .iter()
        .find_map(|(_, symbol)| match symbol.data() {
            SymbolData::InlineSite(site) => Some(site.annotations.iter().collect::<Vec<_>>()),
            _ => None,
        })
        .expect("inline site");
    assert_eq!(
        site,
        [
            Annotation::ChangeCodeOffsetBase(1),
            Annotation::ChangeLineOffset(1),
            Annotation::ChangeCodeOffset(8),
            Annotation::ChangeCodeLength(8),
        ]
    );
    // Offset 0x28 of `big` is offset 8 of the part at `.text+0xe0`.
    assert_eq!(
        locate(&output, 0x10e8 + 2),
        (Some(b"big".to_vec()), vec![IdIndex(SUM_ID)])
    );
    assert_eq!(locate(&output, 0x10e0).1, Vec::<IdIndex>::new());
}

#[test]
fn a_split_block_fails_and_names_the_block() {
    let source = source();
    let (stream, symbols) = module_symbols(&source);
    // `.text+0x60..0x70` moves to the end of `.text`: the range boundary at `0x60` cuts the
    // block at `0x58..0x68`.
    let ranges = [
        (0x1000, 0x60, 0x1000),
        (0x1060, 0x10, 0x10f0),
        (0x1070, 0x80, 0x1060),
        DATA,
    ];
    let error = clean(&source, &ranges, RemovedCode::Drop, &[]).expect_err("a split block fails");
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
    let address = error.remap_address().expect("remap address");
    assert_eq!(address.item, RemapItem::Symbol(S_BLOCK32));
    assert_eq!(address.address, PdbRva(0x1058));
    assert_eq!(
        error.stream_location(),
        Some((stream, offset_of(&symbols, S_BLOCK32)))
    );
}

/// The map without a range for `extra`.
const WITHOUT_EXTRA: [Range; 3] = [(0x1000, 0xa0, 0x1000), (0x10b0, 0x50, 0x10b0), DATA];

#[test]
fn a_removed_procedure_fails_with_the_fail_policy() {
    let source = source();
    let (stream, symbols) = module_symbols(&source);
    let extra = symbols
        .iter()
        .find(|(_, symbol)| {
            matches!(symbol.data(), SymbolData::Procedure(value) if value.name.as_bytes() == b"extra")
        })
        .map(|(offset, _)| u64::from(*offset))
        .expect("extra");
    let error =
        clean(&source, &WITHOUT_EXTRA, RemovedCode::Fail, &[]).expect_err("removed code fails");
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
    let address = error.remap_address().expect("remap address");
    assert_eq!(address.item, RemapItem::Symbol(S_GPROC32));
    assert_eq!(address.address, PdbRva(0x10a0));
    assert_eq!(error.stream_location(), Some((stream, extra)));
}

#[test]
fn a_removed_procedure_goes_away_with_the_drop_policy() {
    let source = source();
    let (output, counts) = clean(
        &source,
        &WITHOUT_EXTRA,
        RemovedCode::Drop,
        &[S_GPROC32, S_END],
    )
    .expect("clean output");
    assert_eq!(counts, [(1, 0), (0, 0)]);
    let (_, symbols) = module_symbols(&output);
    assert!(find_procedure(&symbols, b"extra").is_none());
    assert_eq!(
        find_procedure(&symbols, b"big").expect("big").address,
        text(0x50)
    );
    assert!(find_procedure(&symbols, b"main").is_some());
}
