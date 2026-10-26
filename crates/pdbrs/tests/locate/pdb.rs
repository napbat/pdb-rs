//! Address queries on a synthetic PDB with one module.
//!
//! The module has these procedures in section 1 (RVA 0x1000, 0x100 bytes).
//! The offsets are section offsets:
//!
//! - `outer` (`S_GPROC32_ID`) at offset 0x10, 0x40 bytes. It inlines
//!   `inner` at 0x10..0x18 and 0x20..0x24. `inner` inlines `leaf` at
//!   0x12..0x14. The annotations give offsets from the start of `outer`.
//! - `second` (`S_GPROC32`) at offset 0x60, 0x10 bytes.
//!
//! Offsets 0x50 to 0x5f are a gap between the procedures.
//!
//! The variant with separated code adds an `S_SEPCODE` record of `outer`
//! after its end, at offset 0x80 with 0x40 bytes (chunk 1). In `outer` it
//! adds two inline sites with `ChangeCodeOffsetBase`:
//!
//! - `leaf` at offset 0x20..0x24 of chunk 1 (0xa0..0xa4).
//! - `inner` from offset 0x38 of chunk 0 to the end of the chunk
//!   (0x48..0x50), and at offset 0x30..0x32 of chunk 1 (0xb0..0xb2).

use pdbrs::ErrorKind;
use pdbrs::native::Pdb;
use pdbrs::native::address::{Rva, SectionOffset};
use pdbrs::native::codeview::id_record::LF_FUNC_ID;
use pdbrs::native::codeview::index::IdIndex;
use pdbrs::native::codeview::symbol::{
    S_END, S_GPROC32, S_GPROC32_ID, S_INLINESITE, S_INLINESITE_END, S_PROC_ID_END, S_SEPCODE,
};
use pdbrs::native::lines::{DEBUG_S_FILECHKSMS, DEBUG_S_INLINEELINES, DEBUG_S_LINES};
use pdbrs::native::locate::{Location, SourceLine, SymbolLocator};
use pdbrs::native::msf::{MsfBuilder, WriteOptions};

use crate::{annotation, lines_subsection, signed, u32s};

/// The PDB age in the Info and DBI streams.
const AGE: u32 = 7;
/// The IDs of `outer`, `inner` and `leaf` in the IPI stream.
const OUTER: u32 = 0x1000;
const INNER: u32 = 0x1001;
const LEAF: u32 = 0x1002;
/// The file checksum offsets of `a.cpp` and `b.h`.
const FILE_A: u32 = 0;
const FILE_B: u32 = 8;

/// Frame a symbol record: pad the payload with zeros to 4 bytes.
fn symbol(kind: u16, mut payload: Vec<u8>) -> Vec<u8> {
    while !(payload.len() + 4).is_multiple_of(4) {
        payload.push(0);
    }
    let mut bytes = u16::try_from(payload.len() + 2)
        .unwrap()
        .to_le_bytes()
        .to_vec();
    bytes.extend_from_slice(&kind.to_le_bytes());
    bytes.extend_from_slice(&payload);
    bytes
}

/// The payload of a procedure record in section 1.
fn procedure(links: [u32; 2], length: u32, index: u32, offset: u32, name: &[u8]) -> Vec<u8> {
    let [parent, end] = links;
    let mut payload = u32s(&[parent, end, 0, length, 0, 0, index, offset]);
    payload.extend_from_slice(&1_u16.to_le_bytes());
    payload.push(0);
    payload.extend_from_slice(name);
    payload.push(0);
    payload
}

/// The payload of an inline site record.
fn site(parent: u32, end: u32, inlinee: u32, annotations: &[u8]) -> Vec<u8> {
    let mut payload = u32s(&[parent, end, inlinee]);
    payload.extend_from_slice(annotations);
    payload
}

/// The payload of the `S_SEPCODE` record of `outer` at offset 0x80.
fn separated_code(end: u32) -> Vec<u8> {
    let mut payload = u32s(&[0, end, 0x40, 0, 0x80, 0x10]);
    payload.extend_from_slice(&[1, 0, 1, 0]);
    payload
}

/// Build the symbol records with the signature, and fill the scope links.
/// `separated` adds the separated code of `outer`.
fn symbols(separated: bool) -> Vec<u8> {
    let inner = [
        annotation(6, &[signed(1)]),
        annotation(3, &[0]),
        annotation(4, &[8]),
        annotation(5, &[FILE_B]),
        annotation(6, &[signed(2)]),
        annotation(3, &[8]),
        annotation(4, &[4]),
    ]
    .concat();
    let leaf = [annotation(3, &[2]), annotation(4, &[2])].concat();
    // `leaf` in chunk 1 only, at line 205.
    let cold_leaf = [
        annotation(2, &[1]),
        annotation(6, &[signed(5)]),
        annotation(12, &[4, 0x20]),
    ]
    .concat();
    // `inner` at line 140 in chunk 0 without a length, then in chunk 1.
    let cold_inner = [
        annotation(6, &[signed(40)]),
        annotation(3, &[0x38]),
        annotation(2, &[1]),
        annotation(12, &[2, 0x30]),
    ]
    .concat();
    let build = |offsets: &[u32]| {
        let end = if separated { 9 } else { 5 };
        let mut records = vec![
            symbol(
                S_GPROC32_ID,
                procedure([0, offsets[end]], 0x40, OUTER, 0x10, b"outer"),
            ),
            symbol(S_INLINESITE, site(offsets[0], offsets[4], INNER, &inner)),
            symbol(S_INLINESITE, site(offsets[1], offsets[3], LEAF, &leaf)),
            symbol(S_INLINESITE_END, Vec::new()),
            symbol(S_INLINESITE_END, Vec::new()),
        ];
        if separated {
            records.extend([
                symbol(S_INLINESITE, site(offsets[0], offsets[6], LEAF, &cold_leaf)),
                symbol(S_INLINESITE_END, Vec::new()),
                symbol(
                    S_INLINESITE,
                    site(offsets[0], offsets[8], INNER, &cold_inner),
                ),
                symbol(S_INLINESITE_END, Vec::new()),
            ]);
        }
        records.push(symbol(S_PROC_ID_END, Vec::new()));
        if separated {
            // MSVC writes the separated code after the end of its procedure.
            records.push(symbol(S_SEPCODE, separated_code(offsets[end + 2])));
            records.push(symbol(S_END, Vec::new()));
        }
        let second = records.len();
        records.push(symbol(
            S_GPROC32,
            procedure([0, offsets[second + 1]], 0x10, 0x74, 0x60, b"second"),
        ));
        records.push(symbol(S_END, Vec::new()));
        let mut next = 4;
        let mut starts = Vec::new();
        for record in &records {
            starts.push(next);
            next += u32::try_from(record.len()).unwrap();
        }
        (records.concat(), starts)
    };
    let (_, offsets) = build(&[0; 14]);
    let (records, _) = build(&offsets);
    [4_u32.to_le_bytes().to_vec(), records].concat()
}

/// Frame a C13 subsection and pad it to 4 bytes.
fn subsection(kind: u32, data: &[u8]) -> Vec<u8> {
    let mut bytes = u32s(&[kind, u32::try_from(data.len()).unwrap()]);
    bytes.extend_from_slice(data);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
}

/// Build the C13 line region. `separated` adds the lines of the separated
/// code of `outer`.
fn c13(inlinee_lines: bool, separated: bool) -> Vec<u8> {
    // Two entries without checksums: name offset, size 0, kind 0, padding.
    let checksums = [u32s(&[1]), vec![0, 0, 0, 0], u32s(&[7]), vec![0, 0, 0, 0]].concat();
    let outer = lines_subsection(
        0x10,
        0x40,
        &[(FILE_A, &[(0, 10), (0, 11), (0x8, 12), (0x30, 15)])],
    );
    let second = lines_subsection(0x60, 0x10, &[(FILE_B, &[(0, 30)])]);
    let inlinees = u32s(&[0, INNER, FILE_A, 100, LEAF, FILE_B, 200]);
    let mut bytes = [
        subsection(DEBUG_S_FILECHKSMS, &checksums),
        subsection(DEBUG_S_LINES, &outer),
        subsection(DEBUG_S_LINES, &second),
    ]
    .concat();
    if separated {
        let cold = lines_subsection(0x80, 0x40, &[(FILE_A, &[(0, 40)])]);
        bytes.extend_from_slice(&subsection(DEBUG_S_LINES, &cold));
    }
    if inlinee_lines {
        bytes.extend_from_slice(&subsection(DEBUG_S_INLINEELINES, &inlinees));
    }
    bytes
}

/// Build the module stream and its module info record.
fn module(inlinee_lines: bool, separated: bool) -> (Vec<u8>, Vec<u8>) {
    let symbols = symbols(separated);
    let c13 = c13(inlinee_lines, separated);
    let mut record = vec![0; 64];
    record[34..36].copy_from_slice(&5_u16.to_le_bytes());
    record[36..40].copy_from_slice(&u32::try_from(symbols.len()).unwrap().to_le_bytes());
    record[44..48].copy_from_slice(&u32::try_from(c13.len()).unwrap().to_le_bytes());
    record.extend_from_slice(b"m.obj\0m.obj\0");
    let stream = [symbols, c13, vec![0; 4]].concat();
    (stream, record)
}

/// Build a modern DBI stream with one module and a section header slot.
fn dbi(module: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[0..4].copy_from_slice(&(-1_i32).to_le_bytes());
    bytes[4..8].copy_from_slice(&19_990_903_u32.to_le_bytes());
    bytes[8..12].copy_from_slice(&AGE.to_le_bytes());
    for offset in [12, 16, 20] {
        bytes[offset..offset + 2].copy_from_slice(&0xffff_u16.to_le_bytes());
    }
    bytes[58..60].copy_from_slice(&0x8664_u16.to_le_bytes());
    // The optional debug header: slot 5 names the section header stream 6.
    let mut debug = Vec::new();
    for slot in 0..11 {
        let stream: u16 = if slot == 5 { 6 } else { 0xffff };
        debug.extend_from_slice(&stream.to_le_bytes());
    }
    bytes[24..28].copy_from_slice(&u32::try_from(module.len()).unwrap().to_le_bytes());
    bytes[48..52].copy_from_slice(&u32::try_from(debug.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(module);
    bytes.extend_from_slice(&debug);
    bytes
}

/// Build the PDB Info stream with a `/names` entry for stream 7.
fn info() -> Vec<u8> {
    let mut bytes = u32s(&[20_000_404, 0, AGE]);
    bytes.extend_from_slice(&[0; 16]);
    let name = b"/names\0";
    bytes.extend_from_slice(&u32::try_from(name.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(name);
    // One entry in bucket 1 of 2: the bucket of `/names` (`hash_string_v1 % 2`).
    bytes.extend_from_slice(&u32s(&[1, 2, 1, 2, 0, 0, 7, 0]));
    bytes
}

/// Build an IPI stream with the `LF_FUNC_ID` records of the three functions.
fn ipi() -> Vec<u8> {
    let mut body = Vec::new();
    for name in [&b"outer"[..], b"inner", b"leaf"] {
        let mut payload = u32s(&[0, 0x74]);
        payload.extend_from_slice(name);
        payload.push(0);
        while !(payload.len() + 2).is_multiple_of(4) {
            let left = 4 - (payload.len() + 2) % 4;
            payload.push(0xf0 + u8::try_from(left).unwrap());
        }
        body.extend_from_slice(&u16::try_from(payload.len() + 2).unwrap().to_le_bytes());
        body.extend_from_slice(&LF_FUNC_ID.to_le_bytes());
        body.extend_from_slice(&payload);
    }
    let mut bytes = u32s(&[
        20_040_203,
        56,
        0x1000,
        0x1003,
        u32::try_from(body.len()).unwrap(),
    ]);
    bytes.extend_from_slice(&0xffff_u16.to_le_bytes());
    bytes.extend_from_slice(&0xffff_u16.to_le_bytes());
    bytes.extend_from_slice(&u32s(&[4, 0x3_ffff, 0, u32::MAX, 0, u32::MAX, 0, u32::MAX]));
    bytes.extend_from_slice(&body);
    bytes
}

/// Build the `/names` stream with `a.cpp` at offset 1 and `b.h` at offset 7.
fn names() -> Vec<u8> {
    let strings = b"\0a.cpp\0b.h\0";
    let mut bytes = u32s(&[0xeffe_effe, 1, u32::try_from(strings.len()).unwrap()]);
    bytes.extend_from_slice(strings);
    bytes.extend_from_slice(&u32s(&[1, 0, 0]));
    bytes
}

/// Build the section header stream with one section at RVA 0x1000.
fn section_headers() -> Vec<u8> {
    let mut bytes = b".text\0\0\0".to_vec();
    bytes.extend_from_slice(&u32s(&[0x100, 0x1000, 0x100, 0x400, 0, 0, 0, 0x6000_0020]));
    bytes
}

/// Build the PDB file. `ids` adds the IPI stream. `separated` adds the
/// separated code of `outer`.
fn pdb_bytes(inlinee_lines: bool, ids: bool, separated: bool) -> Vec<u8> {
    let (module_stream, record) = module(inlinee_lines, separated);
    let streams = [
        Some(Vec::new()),
        Some(info()),
        None,
        Some(dbi(&record)),
        ids.then(ipi),
        Some(module_stream),
        Some(section_headers()),
        Some(names()),
    ];
    let mut builder = MsfBuilder::new();
    for stream in &streams {
        match stream {
            Some(bytes) => builder.push(bytes.as_slice()).unwrap(),
            None => builder.push_nil().unwrap(),
        };
    }
    builder
        .plan(WriteOptions::default())
        .unwrap()
        .to_vec()
        .unwrap()
}

fn locate(locator: &SymbolLocator, rva: u32) -> Location<'_> {
    locator.locate(Rva(rva)).unwrap().unwrap()
}

/// Return the file and line of a source line.
fn place(line: Option<SourceLine<'_>>) -> Option<(&[u8], u32)> {
    line.map(|line| (line.file, line.line))
}

/// Return the inline frames as name, line and call site line.
fn frames<'a>(location: &Location<'a>) -> Vec<(&'a [u8], Option<u32>, Option<u32>)> {
    location
        .inline_frames
        .iter()
        .map(|frame| {
            (
                frame.name,
                frame.line.map(|line| line.line),
                frame.call_site.map(|line| line.line),
            )
        })
        .collect()
}

#[test]
fn procedures_come_in_rva_order() {
    let bytes = pdb_bytes(true, true, false);
    let pdb = Pdb::parse(&bytes).unwrap();
    let locator = SymbolLocator::new(&pdb).unwrap();
    let procedures: Vec<_> = locator.procedures().map(Result::unwrap).collect();
    assert_eq!(
        procedures
            .iter()
            .map(|procedure| (procedure.name, procedure.start, procedure.length))
            .collect::<Vec<_>>(),
        [
            (&b"outer"[..], Some(Rva(0x1010)), 0x40),
            (&b"second"[..], Some(Rva(0x1060)), 0x10)
        ]
    );
    assert_eq!(locator.module_count(), 1);
    assert_eq!(locator.incomplete_modules(), []);
    let sites = locator.inline_sites(&procedures[0]).unwrap();
    assert_eq!(
        sites
            .iter()
            .map(|site| (site.inlinee, site.ranges.len()))
            .collect::<Vec<_>>(),
        [(IdIndex(INNER), 2), (IdIndex(LEAF), 1)]
    );
    assert_eq!(locator.inline_sites(&procedures[1]).unwrap(), []);
}

#[test]
fn locates_procedure_boundaries_and_gaps() {
    let bytes = pdb_bytes(true, true, false);
    let pdb = Pdb::parse(&bytes).unwrap();
    let locator = SymbolLocator::new(&pdb).unwrap();
    // The first byte: the first of two line entries at offset 0.
    let first = locate(&locator, 0x1010);
    assert_eq!(first.procedure.unwrap().name, b"outer");
    assert_eq!(first.module.unwrap().name, b"m.obj");
    assert_eq!(place(first.line), Some((&b"a.cpp"[..], 10)));
    assert_eq!(first.line.unwrap().length, Some(0));
    assert_eq!(
        place(locate(&locator, 0x1011).line),
        Some((&b"a.cpp"[..], 11))
    );
    // The last byte of `outer`.
    let last = locate(&locator, 0x104f);
    assert_eq!(last.procedure.unwrap().name, b"outer");
    assert_eq!(place(last.line), Some((&b"a.cpp"[..], 15)));
    assert_eq!(last.inline_frames, []);
    // One byte past the end of `outer` is in the gap.
    for rva in [0x1050, 0x105f, 0x1000, 0x100f] {
        let gap = locate(&locator, rva);
        assert_eq!(gap.procedure, None);
        assert_eq!(gap.line, None);
        assert_eq!(gap.module, None);
        assert_eq!(gap.inline_frames, []);
    }
    let second = locate(&locator, 0x1060);
    assert_eq!(second.procedure.unwrap().name, b"second");
    assert_eq!(place(second.line), Some((&b"b.h"[..], 30)));
    // Outside the section.
    assert_eq!(locator.locate(Rva(0x1100)).unwrap(), None);
    assert_eq!(locator.locate(Rva(0x0fff)).unwrap(), None);
    let address = SectionOffset {
        section: 1,
        offset: 0x60,
    };
    let by_section = locator.locate_section_offset(address).unwrap().unwrap();
    assert_eq!(by_section.rva, Rva(0x1060));
    let missing = SectionOffset {
        section: 2,
        offset: 0,
    };
    assert_eq!(locator.locate_section_offset(missing).unwrap(), None);
}

#[test]
fn builds_inline_call_chains_from_the_innermost_frame() {
    let bytes = pdb_bytes(true, true, false);
    let pdb = Pdb::parse(&bytes).unwrap();
    let locator = SymbolLocator::new(&pdb).unwrap();
    // `inner` holds 0x10 but `leaf` does not.
    let start = locate(&locator, 0x1010);
    assert_eq!(frames(&start), [(&b"inner"[..], Some(101), Some(10))]);
    // `leaf` inside `inner` inside `outer`.
    let nested = locate(&locator, 0x1012);
    assert_eq!(
        frames(&nested),
        [
            (&b"leaf"[..], Some(200), Some(101)),
            (&b"inner"[..], Some(101), Some(11))
        ]
    );
    let leaf = nested.inline_frames[0];
    assert_eq!(leaf.qualified_name, b"leaf");
    assert_eq!(leaf.inlinee, IdIndex(LEAF));
    assert_eq!(place(leaf.line), Some((&b"b.h"[..], 200)));
    assert_eq!(leaf.line.unwrap().start, Rva(0x1012));
    assert_eq!(leaf.line.unwrap().length, Some(2));
    assert_eq!(frames(&locate(&locator, 0x1013)).len(), 2);
    assert_eq!(frames(&locate(&locator, 0x1014)).len(), 1);
    // The gap between the two ranges of `inner`.
    assert_eq!(locate(&locator, 0x1018).inline_frames, []);
    // The second range of `inner` changes the file.
    let second = locate(&locator, 0x1023);
    assert_eq!(frames(&second), [(&b"inner"[..], Some(103), Some(12))]);
    assert_eq!(
        place(second.inline_frames[0].line),
        Some((&b"b.h"[..], 103))
    );
    assert_eq!(locate(&locator, 0x1024).inline_frames, []);
}

#[test]
fn code_chunks_come_from_the_separated_code_after_the_procedure() {
    let bytes = pdb_bytes(true, true, true);
    let pdb = Pdb::parse(&bytes).unwrap();
    let locator = SymbolLocator::new(&pdb).unwrap();
    let procedures: Vec<_> = locator.procedures().map(Result::unwrap).collect();
    assert_eq!(procedures.len(), 2);
    let chunks = locator.code_chunks(&procedures[0]).unwrap();
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| (
                chunk.offset.is_some(),
                chunk.address.offset,
                chunk.start,
                chunk.length
            ))
            .collect::<Vec<_>>(),
        [
            (false, 0x10, Some(Rva(0x1010)), 0x40),
            (true, 0x80, Some(Rva(0x1080)), 0x40)
        ]
    );
    assert_eq!(locator.code_chunks(&procedures[1]).unwrap().len(), 1);
    let sites = locator.inline_sites(&procedures[0]).unwrap();
    let ranges = |index: usize| {
        sites[index]
            .ranges
            .iter()
            .map(|range| {
                let place = (range.chunk, range.code_offset, range.code_length);
                (place, range.rva_range(&chunks))
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(ranges(2), [((1, 0x20, Some(4)), Some((Rva(0x10a0), 4)))]);
    // `ChangeCodeOffsetBase` ends a range without a length at the end of
    // its chunk.
    assert_eq!(
        ranges(3),
        [
            ((0, 0x38, None), Some((Rva(0x1048), 8))),
            ((1, 0x30, Some(2)), Some((Rva(0x10b0), 2)))
        ]
    );
    // A chunk that the procedure does not have gives no RVA range.
    assert_eq!(sites[2].ranges[0].rva_range(&chunks[..1]), None);
}

#[test]
fn locates_inline_frames_in_separated_code() {
    let bytes = pdb_bytes(true, true, true);
    let pdb = Pdb::parse(&bytes).unwrap();
    let locator = SymbolLocator::new(&pdb).unwrap();
    let cold = locate(&locator, 0x10a0);
    assert_eq!(cold.procedure.unwrap().name, b"outer");
    assert_eq!(cold.module.unwrap().name, b"m.obj");
    assert_eq!(place(cold.line), Some((&b"a.cpp"[..], 40)));
    assert_eq!(frames(&cold), [(&b"leaf"[..], Some(205), Some(40))]);
    let leaf = cold.inline_frames[0];
    assert_eq!(leaf.range.unwrap().chunk, 1);
    assert_eq!(leaf.range.unwrap().code_offset, 0x20);
    assert_eq!(place(leaf.line), Some((&b"b.h"[..], 205)));
    assert_eq!(leaf.line.unwrap().start, Rva(0x10a0));
    assert_eq!(leaf.line.unwrap().length, Some(4));
    assert_eq!(frames(&locate(&locator, 0x10a3)).len(), 1);
    // The chunk holds the address after the range, but no inline site does.
    let after = locate(&locator, 0x10a4);
    assert_eq!(after.procedure.unwrap().name, b"outer");
    assert_eq!(after.inline_frames, []);
    assert_eq!(
        frames(&locate(&locator, 0x10b1)),
        [(&b"inner"[..], Some(140), Some(40))]
    );
    // The procedure start plus the chunk number and the code offset is not
    // an address of the separated range.
    assert_eq!(locate(&locator, 0x1031).inline_frames, []);
    // The range of chunk 0 without a length ends with the procedure.
    let open = locate(&locator, 0x104f);
    assert_eq!(frames(&open), [(&b"inner"[..], Some(140), Some(15))]);
    assert_eq!(open.inline_frames[0].line.unwrap().start, Rva(0x1048));
    assert_eq!(open.inline_frames[0].line.unwrap().length, Some(8));
    // After the separated code.
    let past = locate(&locator, 0x10c0);
    assert_eq!(past.procedure, None);
    assert_eq!(past.inline_frames, []);
}

#[test]
fn a_missing_inlinee_line_entry_is_an_error() {
    let bytes = pdb_bytes(false, true, false);
    let pdb = Pdb::parse(&bytes).unwrap();
    let locator = SymbolLocator::new(&pdb).unwrap();
    let error = locator.locate(Rva(0x1012)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
    // `second` has no inline sites, so it still works.
    assert!(locator.locate(Rva(0x1060)).unwrap().is_some());
}

#[test]
fn inline_sites_need_the_ipi_stream() {
    let bytes = pdb_bytes(true, false, false);
    let pdb = Pdb::parse(&bytes).unwrap();
    let error = SymbolLocator::new(&pdb).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::MissingStream);
}

#[test]
fn name_and_public_queries_without_symbol_indexes_are_empty() {
    let bytes = pdb_bytes(true, true, false);
    let pdb = Pdb::parse(&bytes).unwrap();
    let locator = SymbolLocator::new(&pdb).unwrap();
    assert_eq!(locator.find_name(b"outer").unwrap(), []);
    assert_eq!(locator.public_at(Rva(0x1010)).unwrap(), None);
}
