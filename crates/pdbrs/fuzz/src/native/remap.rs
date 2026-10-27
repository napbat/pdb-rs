//! The address remap of a rewritten image (#43): OMAP output and clean output of
//! [`PdbEditor::remap_addresses`](pdbrs::native::PdbEditor::remap_addresses).
//!
//! The source PDB is the synthetic program of the `native_synthetic` example ([`program`]),
//! with three more records: a def-range in `sum`, and a block and a label in `main`. The
//! input gives the new image:
//!
//! 1. A shape byte. `shape % 3` selects the new RVA of `.text` (`0x1000`, `0x3000` or
//!    `0x4000`). Bit 2 puts `.data` before `.text` in the new section headers. Bits 3 to 7
//!    give the offset of the first range in the new `.text`.
//! 2. Pieces of the old `.text`, 3 bytes each: the length (1 to 64 bytes), an action byte and
//!    a gap byte. An action byte with bits 0 and 1 set removes the piece. Else the piece moves,
//!    and bits 2 to 7 give its order in the new `.text`. A gap byte with bit 7 set puts a gap
//!    of `gap & 0x1f` bytes before the piece. At the end of the input, or after
//!    [`MAX_PIECES`] pieces, one piece keeps the rest of `.text`.
//!
//! `.data` keeps an identity range. An error of
//! [`ImageRemap::with_range`](pdbrs::native::ImageRemap::with_range) ends the run.

#[path = "../../../examples/native_synthetic/program.rs"]
pub mod program;

use std::sync::LazyLock;

use pdbrs::ErrorKind;
use pdbrs::io::ReadAt;
use pdbrs::native::address::{AddressTables, PdbRva, Rva, SectionOffset, Translation};
use pdbrs::native::codeview::index::{ModuleIndex, TypeIndex};
use pdbrs::native::codeview::symbol::{
    AddressGaps, AddressRange, Block, DefRangeRegister, Label, Local, LocalFlags, ProcedureFlags,
    S_BLOCK32, S_DEFRANGE_REGISTER, S_END, S_LABEL32, S_LOCAL, SymbolData,
};
use pdbrs::native::editor::{ModuleSymbol, Scope, SymbolRecord};
use pdbrs::native::locate::{Location, SymbolLocator};
use pdbrs::native::section::{SectionHeader, SectionHeaderStream};
use pdbrs::native::{
    EditOptions, IdentityPolicy, ImageRemap, Pdb, RemapMode, RemapRange, RemovedCode,
};

use crate::input::Input;

/// The most pieces of `.text` in one run.
pub const MAX_PIECES: usize = 16;

/// The RVA of `.text` in the source image.
const TEXT: u32 = 0x1000;
/// The size of `.text` in the source image.
const TEXT_SIZE: u32 = 0x100;
/// The end of the code that records of the source hold: the section contribution of the
/// module covers `sum` and `main`.
const HELD_END: u32 = 0x1050;
/// The size of `.text` in the new image. All pieces with the largest gaps fit.
const NEW_TEXT_SIZE: u32 = 0x300;
/// The RVA of `.data` in both images.
const DATA: u32 = 0x2000;
/// The size of `.data` in both images.
const DATA_SIZE: u32 = 0x100;
/// The `.text` section characteristics: code, execute, read.
const TEXT_CHARACTERISTICS: u32 = 0x6000_0020;
/// The `.data` section characteristics: initialized data, read, write.
const DATA_CHARACTERISTICS: u32 = 0xc000_0040;
/// The x64 register number of `RCX` (`CV_AMD64_RCX`).
const CV_AMD64_RCX: u16 = 330;
/// The `T_INT4` primitive type.
const T_INT4: TypeIndex = TypeIndex(0x74);

/// The procedure name and the source line at an address, as owned values.
type Answer = Option<(Option<Vec<u8>>, Option<(Vec<u8>, u32)>)>;

/// The new image of one run.
struct Layout {
    /// The map of the run.
    remap: ImageRemap,
    /// The section headers of the new image.
    sections: SectionHeaderStream,
    /// The ranges of the map, by old RVA.
    ranges: Vec<RemapRange>,
    /// The removed pieces of `.text`: old RVA and length.
    removed: Vec<(u32, u32)>,
    /// The new RVA after the last byte of `.text`.
    text_end: u32,
}

impl Layout {
    /// Return `true` if a removed piece holds code that a record of the source holds.
    fn holds_removed_code(&self) -> bool {
        self.removed
            .iter()
            .any(|&(old, length)| old < HELD_END && old + length > TEXT)
    }
}

/// One piece of the old `.text`.
struct Piece {
    /// The old RVA.
    old: u32,
    /// The length in bytes.
    length: u32,
    /// `false` if the image has no copy of the piece.
    keep: bool,
    /// The order of the piece in the new `.text`.
    key: u8,
    /// The number of bytes before the piece in the new `.text`.
    gap: u32,
}

/// Remap the synthetic PDB with OMAP output and with clean output, and compare the results.
///
/// # Panics
///
/// Panics if one of these checks fails:
///
/// - OMAP output plans and reads again. Its tables equal
///   [`ImageRemap::address_tables`](pdbrs::native::ImageRemap::address_tables). The address
///   map and the two OMAP tables translate the start, the middle and the last byte of each
///   range as the range says, in both directions. A removed piece and a gap of the new
///   `.text` have no mapping.
/// - Clean output with [`RemovedCode::Drop`] and with [`RemovedCode::Fail`] either fails with
///   [`ErrorKind::UnsupportedEdit`] and names a record and its address, or reads again
///   without OMAP and with the new section headers. At the sampled new RVAs of each range,
///   [`SymbolLocator::locate`] gives the same procedure name and the same source line as on
///   the OMAP output.
/// - [`RemovedCode::Fail`] fails if a record holds removed code. Else both policies give
///   the same result.
pub fn remap(data: &[u8]) {
    let source = SOURCE.as_slice();
    let Some(layout) = layout(&mut Input::new(data)) else {
        return;
    };
    let source_tables = Pdb::parse(source)
        .and_then(|pdb| pdb.address_tables())
        .expect("the source has address tables")
        .expect("the source has section headers");
    let omap = output(source, &layout.remap, RemapMode::Omap).expect("OMAP output plans");
    let omap_pdb = Pdb::parse(&omap[..]).expect("the OMAP output reads again");
    check_omap(&omap_pdb, &layout, &source_tables);
    let expected = omap_answers(&omap_pdb, &layout);
    let dropped = clean(source, &layout.remap, RemovedCode::Drop);
    let failed = clean(source, &layout.remap, RemovedCode::Fail);
    for bytes in [&dropped, &failed].into_iter().flatten() {
        check_clean(bytes, &layout, &expected);
    }
    if layout.holds_removed_code() {
        assert!(failed.is_none(), "RemovedCode::Fail accepts removed code");
    } else {
        assert!(
            failed == dropped,
            "the policies give different results without removed code"
        );
    }
}

/// The bytes of the source PDB.
static SOURCE: LazyLock<Vec<u8>> = LazyLock::new(|| build().expect("the synthetic program builds"));

/// Return an address in a section.
const fn at(section: u16, offset: u32) -> SectionOffset {
    SectionOffset { section, offset }
}

/// Build the synthetic program with a def-range, a block and a label more.
fn build() -> pdbrs::Result<Vec<u8>> {
    let mut editor = program::build()?;
    let module = editor.module_mut(ModuleIndex(0))?;
    // A local `q` of `sum` in RCX from `sum+0x4` for 0x18 bytes, with a gap at 0x8.
    let sum = module.procedure_mut(b"sum")?;
    let mut local = Local::default();
    local.type_index = T_INT4;
    local.flags = LocalFlags(0);
    local.name = b"q".into();
    let mut range = AddressRange::default();
    range.start = at(1, 0x4);
    range.length = 0x18;
    let mut location = DefRangeRegister::default();
    location.register = CV_AMD64_RCX;
    location.attributes = 0;
    location.range = range;
    location.gaps = AddressGaps::parse(&[8_u8, 0, 4, 0][..])?.into_owned();
    sum.children.extend([
        ModuleSymbol::Record(SymbolRecord::new(S_LOCAL, SymbolData::Local(local))),
        ModuleSymbol::Record(SymbolRecord::new(
            S_DEFRANGE_REGISTER,
            SymbolData::DefRangeRegister(location),
        )),
    ]);
    // A block `inner` at `main+0x8` for 0x10 bytes and a label `done` at `main+0x2c`.
    let main = module.procedure_mut(b"main")?;
    let mut block = Block::default();
    block.code_length = 0x10;
    block.address = at(1, 0x28);
    block.name = b"inner".into();
    let mut label = Label::default();
    label.address = at(1, 0x4c);
    label.flags = ProcedureFlags(0);
    label.name = b"done".into();
    main.children.extend([
        ModuleSymbol::Scope(Box::new(Scope::new(
            SymbolRecord::new(S_BLOCK32, SymbolData::Block(block)),
            SymbolRecord::new(S_END, SymbolData::ScopeEnd),
        ))),
        ModuleSymbol::Record(SymbolRecord::new(S_LABEL32, SymbolData::Label(label))),
    ]);
    editor.plan(EditOptions::default())?.to_vec()
}

/// Return a section header.
fn header(name: [u8; 8], virtual_address: u32, size: u32, characteristics: u32) -> SectionHeader {
    let mut header = SectionHeader::new(name, size, virtual_address);
    header.characteristics = characteristics;
    header
}

/// Read the new image from the input. Return `None` if the map rejects a range.
fn layout(input: &mut Input<'_>) -> Option<Layout> {
    let shape = input.u8();
    let text = [0x1000, 0x3000, 0x4000][usize::from(shape % 3)];
    let text_header = header(*b".text\0\0\0", text, NEW_TEXT_SIZE, TEXT_CHARACTERISTICS);
    let data_header = header(*b".data\0\0\0", DATA, DATA_SIZE, DATA_CHARACTERISTICS);
    let headers = if shape & 4 == 0 {
        [text_header, data_header]
    } else {
        [data_header, text_header]
    };
    let sections = SectionHeaderStream::from_records(&headers).expect("two section headers");
    let mut pieces = Vec::new();
    let mut old = TEXT;
    while old < TEXT + TEXT_SIZE {
        let rest = TEXT + TEXT_SIZE - old;
        let last = input.is_empty() || pieces.len() + 1 == MAX_PIECES;
        let length = if last {
            rest
        } else {
            (u32::from(input.u8() % 0x40) + 1).min(rest)
        };
        let action = input.u8();
        let gap = input.u8();
        pieces.push(Piece {
            old,
            length,
            keep: last || action & 3 != 3,
            key: action >> 2,
            gap: if gap & 0x80 == 0 {
                0
            } else {
                u32::from(gap & 0x1f)
            },
        });
        old += length;
    }
    let removed = pieces
        .iter()
        .filter(|piece| !piece.keep)
        .map(|piece| (piece.old, piece.length))
        .collect();
    let mut kept: Vec<Piece> = pieces.into_iter().filter(|piece| piece.keep).collect();
    kept.sort_by_key(|piece| piece.key);
    let mut remap = ImageRemap::new(sections.clone())
        .with_range(Rva(DATA), DATA_SIZE, Rva(DATA))
        .ok()?;
    let mut cursor = text + u32::from(shape >> 3);
    for piece in &kept {
        let new = cursor + piece.gap;
        remap = remap
            .with_range(Rva(piece.old), piece.length, Rva(new))
            .ok()?;
        cursor = new + piece.length;
    }
    let ranges = remap.ranges().collect();
    Some(Layout {
        remap,
        sections,
        ranges,
        removed,
        text_end: text + NEW_TEXT_SIZE,
    })
}

/// Return the offsets of the first byte, the middle byte and the last byte of `length` bytes.
fn samples(length: u32) -> [u32; 3] {
    [0, length / 2, length - 1]
}

/// Remap `source` in `mode` and return the output, or the error of the call or the plan.
fn output(source: &[u8], remap: &ImageRemap, mode: RemapMode) -> pdbrs::Result<Vec<u8>> {
    let pdb = Pdb::parse(source)?;
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor.remap_addresses(remap, mode)?;
    let plan = editor.plan(EditOptions::default())?;
    if let RemapMode::Clean(policy) = mode {
        let report = plan
            .report()
            .remap()
            .expect("clean output has a remap report");
        if policy == RemovedCode::Fail {
            assert_eq!(
                report.removed_items().count(),
                0,
                "RemovedCode::Fail removed records"
            );
        }
    }
    plan.to_vec()
}

/// Return the clean output with `policy`, or `None` if the plan rejects a record.
fn clean(source: &[u8], remap: &ImageRemap, policy: RemovedCode) -> Option<Vec<u8>> {
    match output(source, remap, RemapMode::Clean(policy)) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            assert_eq!(
                error.kind(),
                ErrorKind::UnsupportedEdit,
                "clean output with {policy:?}: {error}"
            );
            assert!(
                error.remap_address().is_some(),
                "the error does not name a record: {error}"
            );
            None
        }
    }
}

/// Check the address tables of the OMAP output `pdb`.
fn check_omap<S: ReadAt>(pdb: &Pdb<S>, layout: &Layout, source: &AddressTables) {
    let tables = pdb
        .address_tables()
        .expect("OMAP output tables")
        .expect("OMAP output section headers");
    assert_eq!(
        tables,
        layout
            .remap
            .address_tables(source)
            .expect("the tables of the map"),
        "the OMAP output tables"
    );
    let map = tables.map();
    let from_source = tables.from_source.as_ref().expect("OMAP from source");
    let to_source = tables.to_source.as_ref().expect("OMAP to source");
    for range in &layout.ranges {
        for offset in samples(range.length) {
            let (old, new) = (range.old.0 + offset, range.new.0 + offset);
            assert_eq!(
                map.pdb_rva_to_rva(PdbRva(old)).expect("lookup"),
                Translation::Mapped(Rva(new)),
                "PDB RVA {old:#x}"
            );
            assert_eq!(
                map.rva_to_pdb_rva(Rva(new)).expect("lookup"),
                Translation::Mapped(PdbRva(old)),
                "RVA {new:#x}"
            );
            assert_eq!(
                from_source.lookup(old).expect("lookup"),
                Translation::Mapped(new)
            );
            assert_eq!(
                to_source.lookup(new).expect("lookup"),
                Translation::Mapped(old)
            );
        }
    }
    for &(old, length) in &layout.removed {
        for offset in samples(length) {
            let rva = PdbRva(old + offset);
            assert!(
                matches!(map.pdb_rva_to_rva(rva), Ok(Translation::Unmapped(_))),
                "removed PDB RVA {:#x}",
                rva.0
            );
        }
    }
    for range in &layout.ranges {
        let end = range.new.0 + range.length;
        let gap = end < layout.text_end && layout.ranges.iter().all(|other| other.new.0 != end);
        if gap {
            assert!(
                matches!(map.rva_to_pdb_rva(Rva(end)), Ok(Translation::Unmapped(_))),
                "gap RVA {end:#x}"
            );
        }
    }
}

/// Return the answer of [`SymbolLocator::locate`] on the OMAP output at each sampled new RVA
/// of each range.
fn omap_answers<S: ReadAt>(pdb: &Pdb<S>, layout: &Layout) -> Vec<(u32, Answer)> {
    let locator = SymbolLocator::new(pdb).expect("a locator of the OMAP output");
    let mut answers = Vec::new();
    for range in &layout.ranges {
        for offset in samples(range.length) {
            let new = range.new.0 + offset;
            answers.push((new, answer(locator.locate(Rva(new)).expect("OMAP locate"))));
        }
    }
    answers
}

/// Return the procedure name and the line of `location`.
fn answer(location: Option<Location<'_>>) -> Answer {
    location.map(|location| {
        (
            location.procedure.map(|procedure| procedure.name.to_vec()),
            location.line.map(|line| (line.file.to_vec(), line.line)),
        )
    })
}

/// Check the clean output `bytes` against the answers of the OMAP output.
fn check_clean(bytes: &[u8], layout: &Layout, expected: &[(u32, Answer)]) {
    let pdb = Pdb::parse(bytes).expect("the clean output reads again");
    let tables = pdb
        .address_tables()
        .expect("clean output tables")
        .expect("clean output section headers");
    assert!(
        tables.original.is_none() && tables.to_source.is_none() && tables.from_source.is_none(),
        "the clean output has OMAP"
    );
    assert_eq!(
        tables.sections, layout.sections,
        "the clean section headers"
    );
    let locator = SymbolLocator::new(&pdb).expect("a locator of the clean output");
    for (new, omap) in expected {
        assert_eq!(
            &answer(locator.locate(Rva(*new)).expect("clean locate")),
            omap,
            "the location at the new RVA {new:#x}"
        );
    }
}
