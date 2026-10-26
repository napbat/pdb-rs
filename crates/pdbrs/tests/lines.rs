//! Tests for C13 line subsections and module line validation.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::index::IdIndex;
use pdbrs::native::dbi::{Dbi, ModuleStream};
use pdbrs::native::lines::{
    ALWAYS_STEP_INTO_LINE, ChecksumKind, CoffSymbolRvas, DEBUG_S_COFF_SYMBOL_RVA,
    DEBUG_S_FILECHKSMS, DEBUG_S_FRAMEDATA, DEBUG_S_IGNORE, DEBUG_S_INLINEELINES, DEBUG_S_LINES,
    FileChecksums, InlineeLines, InlineeSignature, LineEntry, LineSubsection, LinesSubsection,
    ModuleLines, SubsectionKind,
};

/// The module stream index in the synthetic module record.
const MODULE_STREAM: u16 = 5;

fn u32s(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// Serialize a `DEBUG_S_LINES` header.
fn lines_header(code_offset: u32, segment: u16, flags: u16, code_size: u32) -> Vec<u8> {
    let mut bytes = code_offset.to_le_bytes().to_vec();
    bytes.extend_from_slice(&segment.to_le_bytes());
    bytes.extend_from_slice(&flags.to_le_bytes());
    bytes.extend_from_slice(&code_size.to_le_bytes());
    bytes
}

/// Serialize a block with `(offset, flags)` lines and optional columns.
fn block(checksum_offset: u32, lines: &[(u32, u32)], columns: Option<&[(u16, u16)]>) -> Vec<u8> {
    let count = u32::try_from(lines.len()).unwrap();
    let entry = if columns.is_some() { 12 } else { 8 };
    let mut bytes = u32s(&[checksum_offset, count, 12 + count * entry]);
    for &(offset, flags) in lines {
        bytes.extend_from_slice(&u32s(&[offset, flags]));
    }
    for &(start, end) in columns.unwrap_or_default() {
        bytes.extend_from_slice(&start.to_le_bytes());
        bytes.extend_from_slice(&end.to_le_bytes());
    }
    bytes
}

/// Serialize one file checksum entry with zero padding.
fn checksum(name_offset: u32, kind: u8, bytes: &[u8]) -> Vec<u8> {
    let mut entry = name_offset.to_le_bytes().to_vec();
    entry.push(u8::try_from(bytes.len()).unwrap());
    entry.push(kind);
    entry.extend_from_slice(bytes);
    while !entry.len().is_multiple_of(4) {
        entry.push(0);
    }
    entry
}

fn error_kind<T: core::fmt::Debug>(result: pdbrs::Result<T>) -> ErrorKind {
    let error = result.unwrap_err();
    assert_eq!(error.offset(), None);
    error.kind()
}

#[test]
fn lines_decode_blocks_and_packed_flags() {
    let data = [
        lines_header(0x660, 1, 0, 0x3a7),
        block(0, &[(0, 0x8000_0067), (0x2e, 0x0300_0068)], None),
        block(0x28, &[(0x324, ALWAYS_STEP_INTO_LINE)], None),
    ]
    .concat();
    let lines = LinesSubsection::decode(&data).unwrap();
    let header = lines.header();
    assert_eq!(
        (
            header.code_offset,
            header.segment,
            header.flags,
            header.code_size
        ),
        (0x660, 1, 0, 0x3a7)
    );
    assert!(!header.has_columns());
    assert_eq!(lines.block_count(), 2);
    let blocks: Vec<_> = lines.blocks().collect();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].checksum_offset, 0);
    assert_eq!(blocks[0].columns, None);
    let entries: Vec<LineEntry> = blocks[0].lines.iter().collect();
    assert_eq!(entries[0].offset, 0);
    assert_eq!(entries[0].line_start, 0x67);
    assert_eq!(entries[0].end_delta, 0);
    assert!(entries[0].is_statement);
    assert_eq!(entries[1].line_start, 0x68);
    assert_eq!(entries[1].end_delta, 3);
    assert!(!entries[1].is_statement);
    assert_eq!(blocks[1].checksum_offset, 0x28);
    assert_eq!(
        blocks[1].lines.get(0).map(|line| line.line_start),
        Some(ALWAYS_STEP_INTO_LINE)
    );
    assert_eq!(blocks[1].lines.get(1), None);
}

#[test]
fn lines_decode_columns_when_the_flag_is_set() {
    let data = [
        lines_header(0, 1, 1, 0x10),
        block(0, &[(0, 1), (4, 2)], Some(&[(5, 9), (1, 0)])),
    ]
    .concat();
    let lines = LinesSubsection::decode(&data).unwrap();
    assert!(lines.header().has_columns());
    let block = lines.blocks().next().unwrap();
    let columns: Vec<_> = block
        .columns
        .unwrap()
        .into_iter()
        .map(|column| (column.start, column.end))
        .collect();
    assert_eq!(columns, [(5, 9), (1, 0)]);
    assert_eq!(block.lines.len(), 2);

    // Without the flag, the same bytes have an unexpected block size.
    let mut without = data.clone();
    without[6] = 0;
    assert_eq!(
        error_kind(LinesSubsection::decode(&without)),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn lines_reject_invalid_blocks() {
    let header = lines_header(0, 1, 0, 0x10);
    let good = block(0, &[(0, 1)], None);
    let cases: [(Vec<u8>, ErrorKind); 8] = [
        (header[..11].to_vec(), ErrorKind::UnexpectedEof),
        ([&header[..], &good[..8]].concat(), ErrorKind::UnexpectedEof),
        (
            [&header[..], &good[..15]].concat(),
            ErrorKind::UnexpectedEof,
        ),
        (lines_header(0, 1, 2, 0x10), ErrorKind::UnsupportedVersion),
        // The block size is larger than its entries.
        (
            [&header[..], &u32s(&[0, 1, 24, 0, 1, 0])[..]].concat(),
            ErrorKind::InvalidFormat,
        ),
        // The block size is correct, but the entries are cut off.
        (
            [&header[..], &u32s(&[0, 2, 28, 0, 1])[..]].concat(),
            ErrorKind::UnexpectedEof,
        ),
        // The line offset is at the end of the code contribution.
        (
            [header.clone(), block(0, &[(0x10, 1)], None)].concat(),
            ErrorKind::InvalidFormat,
        ),
        // The second block starts before the end of the first block.
        (
            [
                header.clone(),
                block(0, &[(0, 1), (8, 2)], None),
                block(0x28, &[(4, 3)], None),
            ]
            .concat(),
            ErrorKind::InvalidFormat,
        ),
    ];
    for (data, kind) in cases {
        assert_eq!(
            error_kind(LinesSubsection::decode(&data)),
            kind,
            "{data:x?}"
        );
    }
    // An empty subsection after the header has no blocks.
    assert_eq!(LinesSubsection::decode(&header).unwrap().block_count(), 0);
}

#[test]
fn checksums_decode_all_kinds_and_find_entries() {
    let data = [
        checksum(1, 0, &[]),
        checksum(20, 1, &[0x11; 16]),
        checksum(40, 2, &[0x22; 20]),
        checksum(60, 3, &[0x33; 32]),
    ]
    .concat();
    let table = FileChecksums::decode(&data).unwrap();
    assert_eq!(table.len(), 4);
    let entries: Vec<_> = table.iter().collect();
    let offsets: Vec<_> = entries.iter().map(|entry| entry.offset).collect();
    assert_eq!(offsets, [0, 8, 32, 60]);
    let kinds: Vec<_> = entries.iter().map(|entry| entry.kind).collect();
    assert_eq!(
        kinds,
        [
            ChecksumKind::None,
            ChecksumKind::Md5,
            ChecksumKind::Sha1,
            ChecksumKind::Sha256
        ]
    );
    let sha1 = table.get(32).unwrap();
    assert_eq!(sha1.name_offset, 40);
    assert_eq!(sha1.checksum, &[0x22; 20]);
    assert!(table.contains(60));
    // An offset inside an entry does not identify an entry.
    assert_eq!(table.get(12), None);
    assert_eq!(table.get(100), None);
    assert_eq!(ChecksumKind::from_raw(3), Some(ChecksumKind::Sha256));
    assert_eq!(ChecksumKind::Sha256.raw(), 3);
    assert_eq!(ChecksumKind::from_raw(4), None);
}

#[test]
fn checksums_accept_a_last_entry_without_padding() {
    let mut data = [checksum(0, 1, &[0; 16]), checksum(4, 3, &[1; 32])].concat();
    data.truncate(data.len() - 2);
    let table = FileChecksums::decode(&data).unwrap();
    assert_eq!(table.len(), 2);
    assert_eq!(table.get(24).unwrap().checksum, &[1; 32]);
}

#[test]
fn checksums_reject_invalid_entries() {
    let mut nonzero = checksum(0, 1, &[0; 16]);
    nonzero.extend_from_slice(&checksum(0, 0, &[]));
    nonzero[22] = 0xcc;
    let mut partial_padding = checksum(0, 1, &[0; 16]);
    partial_padding.extend_from_slice(&checksum(0, 0, &[]));
    partial_padding.truncate(23);
    let cases: [(Vec<u8>, ErrorKind); 6] = [
        (checksum(0, 1, &[0; 20]), ErrorKind::InvalidFormat),
        (checksum(0, 0, &[0; 4]), ErrorKind::InvalidFormat),
        (checksum(0, 4, &[]), ErrorKind::UnsupportedVersion),
        (
            checksum(0, 3, &[0; 32])[..30].to_vec(),
            ErrorKind::UnexpectedEof,
        ),
        (vec![0; 5], ErrorKind::UnexpectedEof),
        (nonzero, ErrorKind::InvalidFormat),
    ];
    for (data, kind) in cases {
        assert_eq!(error_kind(FileChecksums::decode(&data)), kind, "{data:x?}");
    }
    // The padding of the first entry is cut off, so the second entry
    // cannot start: the remaining bytes form a truncated entry.
    assert_eq!(
        error_kind(FileChecksums::decode(&partial_padding)),
        ErrorKind::UnexpectedEof
    );
}

#[test]
fn inlinee_lines_decode_both_signatures() {
    let normal = [u32s(&[0]), u32s(&[0x1234, 0x18, 42]), u32s(&[0x1235, 0, 7])].concat();
    let lines = InlineeLines::decode(&normal).unwrap();
    assert_eq!(lines.signature(), InlineeSignature::Normal);
    assert_eq!(lines.len(), 2);
    let entries: Vec<_> = lines.iter().collect();
    assert_eq!(entries[0].inlinee, IdIndex(0x1234));
    assert_eq!(entries[0].checksum_offset, 0x18);
    assert_eq!(entries[0].source_line, 42);
    assert!(entries[0].extra_files.is_empty());

    let extra = [
        u32s(&[1]),
        u32s(&[0x1234, 0, 10, 2, 0x18, 0x30]),
        u32s(&[0x1235, 0x18, 11, 0]),
    ]
    .concat();
    let lines = InlineeLines::decode(&extra).unwrap();
    assert_eq!(lines.signature(), InlineeSignature::ExtraFiles);
    assert_eq!(lines.signature().raw(), 1);
    let entries: Vec<_> = lines.into_iter().collect();
    assert_eq!(entries.len(), 2);
    let files: Vec<_> = entries[0].extra_files.iter().collect();
    assert_eq!(files, [0x18, 0x30]);
    assert_eq!(entries[1].source_line, 11);
    assert!(entries[1].extra_files.is_empty());

    assert!(InlineeLines::decode(&u32s(&[0])).unwrap().is_empty());
}

#[test]
fn inlinee_lines_reject_invalid_data() {
    let cases: [(Vec<u8>, ErrorKind); 5] = [
        (vec![0; 3], ErrorKind::UnexpectedEof),
        (u32s(&[2]), ErrorKind::UnsupportedVersion),
        (u32s(&[0, 0x1000, 0]), ErrorKind::UnexpectedEof),
        (u32s(&[1, 0x1000, 0, 1]), ErrorKind::UnexpectedEof),
        (u32s(&[1, 0x1000, 0, 1, 2, 0]), ErrorKind::UnexpectedEof),
    ];
    for (data, kind) in cases {
        assert_eq!(error_kind(InlineeLines::decode(&data)), kind, "{data:x?}");
    }
}

#[test]
fn classifier_names_kinds_without_decoding_them() {
    assert_eq!(
        SubsectionKind::from_raw(DEBUG_S_LINES),
        SubsectionKind::Lines
    );
    assert_eq!(
        SubsectionKind::from_raw(0xfd),
        SubsectionKind::CoffSymbolRva
    );
    assert_eq!(
        SubsectionKind::from_raw(0x100),
        SubsectionKind::XfgHashVirtual
    );
    assert_eq!(
        SubsectionKind::from_raw(DEBUG_S_IGNORE | DEBUG_S_LINES),
        SubsectionKind::Ignored(0x8000_00f2)
    );
    assert_eq!(
        SubsectionKind::from_raw(0x42),
        SubsectionKind::Unknown(0x42)
    );
    assert!(SubsectionKind::FileChecksums.is_decoded());
    assert!(!SubsectionKind::FrameData.is_decoded());

    let data = [1, 2, 3, 4];
    assert_eq!(
        LineSubsection::decode(DEBUG_S_FRAMEDATA, &data).unwrap(),
        LineSubsection::Unsupported {
            kind: SubsectionKind::FrameData,
            data: &data,
        }
    );
    // The ignore bit keeps even a known kind raw.
    assert!(matches!(
        LineSubsection::decode(DEBUG_S_IGNORE | DEBUG_S_LINES, &data).unwrap(),
        LineSubsection::Unsupported {
            kind: SubsectionKind::Ignored(_),
            ..
        }
    ));
    assert!(matches!(
        LineSubsection::decode(DEBUG_S_INLINEELINES, &u32s(&[0])).unwrap(),
        LineSubsection::InlineeLines(_)
    ));
}

#[test]
fn decodes_coff_symbol_rvas() {
    // The values come from module 0 of the FASTLINK fixture: absolute
    // symbols, a section and its auxiliary record, then two data symbols.
    let data = u32s(&[0, 0, 0x1_0078, 0, 0x1_0078, 0x1_0080, u32::MAX]);
    let LineSubsection::CoffSymbolRvas(rvas) =
        LineSubsection::decode(DEBUG_S_COFF_SYMBOL_RVA, &data).unwrap()
    else {
        panic!("expected DEBUG_S_COFF_SYMBOL_RVA");
    };
    assert!(SubsectionKind::CoffSymbolRva.is_decoded());
    assert_eq!(rvas.len(), 7);
    assert_eq!(rvas.get(2), Some(0x1_0078));
    assert_eq!(rvas.get(7), None);
    assert_eq!(rvas.iter().next_back(), Some(u32::MAX));
    assert_eq!(
        rvas.iter().collect::<Vec<_>>(),
        [0, 0, 0x1_0078, 0, 0x1_0078, 0x1_0080, u32::MAX]
    );
}

#[test]
fn coff_symbol_rvas_accept_empty_data_and_reject_partial_entries() {
    assert!(CoffSymbolRvas::decode(&[]).unwrap().is_empty());
    for length in [1, 2, 3, 5] {
        let data = vec![0; length];
        assert_eq!(
            error_kind(CoffSymbolRvas::decode(&data)),
            ErrorKind::InvalidFormat,
            "{length} bytes"
        );
    }
}

/// Serialize one C13 subsection with its header and zero padding.
fn subsection(kind: u32, data: &[u8]) -> Vec<u8> {
    let mut bytes = u32s(&[kind, u32::try_from(data.len()).unwrap()]);
    bytes.extend_from_slice(data);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
}

/// Build a DBI stream with one module record for a symbol region of 8 bytes
/// and a C13 region of `c13_size` bytes.
fn module_dbi(c13_size: u32) -> Dbi {
    let mut record = vec![0; 64];
    record[34..36].copy_from_slice(&MODULE_STREAM.to_le_bytes());
    record[36..40].copy_from_slice(&8_u32.to_le_bytes());
    record[44..48].copy_from_slice(&c13_size.to_le_bytes());
    record.extend_from_slice(b"m.o\0m.o\0");
    let mut bytes = vec![0; 64];
    bytes[0..4].copy_from_slice(&(-1_i32).to_le_bytes());
    bytes[4..8].copy_from_slice(&19_990_903_u32.to_le_bytes());
    bytes[16..18].copy_from_slice(&0xffff_u16.to_le_bytes());
    bytes[24..28].copy_from_slice(&u32::try_from(record.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(&record);
    Dbi::parse(bytes).unwrap()
}

/// Build a module stream with an 8-byte symbol region and `c13`.
fn module_stream(c13: &[u8]) -> ModuleStream {
    let dbi = module_dbi(u32::try_from(c13.len()).unwrap());
    let module = dbi.modules().next().unwrap().unwrap();
    let mut bytes = u32s(&[4, 0x0002_0002]);
    bytes.extend_from_slice(c13);
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    ModuleStream::parse(bytes, &module).unwrap()
}

fn module_c13(checksum_offset: u32, inlinee_offset: u32) -> Vec<u8> {
    let checksums = [checksum(1, 0, &[]), checksum(9, 1, &[0; 16])].concat();
    let lines = [
        lines_header(0, 1, 0, 8),
        block(checksum_offset, &[(0, 1)], None),
    ]
    .concat();
    let inlinees = u32s(&[1, 0x1000, 0, 3, 1, inlinee_offset]);
    [
        subsection(DEBUG_S_LINES, &lines),
        subsection(DEBUG_S_FILECHKSMS, &checksums),
        subsection(DEBUG_S_FRAMEDATA, &[1, 2]),
        subsection(DEBUG_S_INLINEELINES, &inlinees),
    ]
    .concat()
}

#[test]
fn module_lines_decode_and_validate_references() {
    let stream = module_stream(&module_c13(8, 8));
    let lines = ModuleLines::parse(&stream).unwrap();
    let kinds: Vec<_> = lines.subsections().iter().map(|sub| sub.kind).collect();
    assert_eq!(
        kinds,
        [
            DEBUG_S_LINES,
            DEBUG_S_FILECHKSMS,
            DEBUG_S_FRAMEDATA,
            DEBUG_S_INLINEELINES
        ]
    );
    assert_eq!(lines.subsections()[1].offset, 40);
    assert!(matches!(
        lines.subsections()[2].data,
        LineSubsection::Unsupported { .. }
    ));
    assert_eq!(lines.checksums().unwrap().len(), 2);
    lines.validate().unwrap();
}

#[test]
fn module_lines_reject_invalid_file_references() {
    // The C13 region starts at offset 8 in the module stream.
    for (c13, location) in [
        // The line block refers to the middle of the MD5 entry.
        (module_c13(12, 0), 8),
        // The extra file of the inlinee entry is past the table.
        (module_c13(0, 32), 8 + 92),
    ] {
        let stream = module_stream(&c13);
        let lines = ModuleLines::parse(&stream).unwrap();
        let error = lines.validate().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidReference);
        assert_eq!(error.stream_location(), Some((5, location)));
    }

    // Line blocks without a file checksum subsection.
    let lines = [lines_header(0, 1, 0, 8), block(0, &[(0, 1)], None)].concat();
    let stream = module_stream(&subsection(DEBUG_S_LINES, &lines));
    let error = ModuleLines::parse(&stream).unwrap().validate().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
}

#[test]
fn module_lines_locate_decode_errors() {
    let checksums = checksum(1, 0, &[]);
    let twice = [
        subsection(DEBUG_S_FILECHKSMS, &checksums),
        subsection(DEBUG_S_FILECHKSMS, &checksums),
    ]
    .concat();
    let stream = module_stream(&twice);
    let error = ModuleLines::parse(&stream).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(error.stream_location(), Some((5, 8 + 16)));

    let bad = [
        subsection(DEBUG_S_FILECHKSMS, &checksums),
        subsection(DEBUG_S_INLINEELINES, &u32s(&[7])),
    ]
    .concat();
    let stream = module_stream(&bad);
    let error = ModuleLines::parse(&stream).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedVersion);
    assert_eq!(error.stream_location(), Some((5, 8 + 16)));
}
