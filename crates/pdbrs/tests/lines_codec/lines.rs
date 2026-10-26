//! Line blocks, IL lines, file checksums, inlinee lines and COFF symbol RVAs.

use pdbrs::ErrorKind;
use pdbrs::native::address::Rva;
use pdbrs::native::codeview::index::{ChecksumOffset, IdIndex, NameOffset};
use pdbrs::native::lines::{
    ChecksumKind, CoffSymbolRvas, ColumnEntry, DEBUG_S_COFF_SYMBOL_RVA, DEBUG_S_FILECHKSMS,
    DEBUG_S_IL_LINES, DEBUG_S_INLINEELINES, DEBUG_S_LINES, FileChecksum, FileChecksums,
    InlineeLine, InlineeSignature, LineBlock, LineEntry, LineSubsection, LinesHeader,
};

use crate::{error_kind, round_trip, u32s};

/// Return a subsection header with the given fields.
fn lines_header(code_offset: u32, segment: u16, flags: u16, code_size: u32) -> LinesHeader {
    let mut header = LinesHeader::default();
    header.code_offset = code_offset;
    header.segment = segment;
    header.flags = flags;
    header.code_size = code_size;
    header
}

/// Return a line entry with the given fields.
fn line_entry(offset: u32, line_start: u32, end_delta: u8, is_statement: bool) -> LineEntry {
    let mut entry = LineEntry::default();
    entry.offset = offset;
    entry.line_start = line_start;
    entry.end_delta = end_delta;
    entry.is_statement = is_statement;
    entry
}

/// Return a column entry with the given fields.
fn column_entry(start: u16, end: u16) -> ColumnEntry {
    let mut entry = ColumnEntry::default();
    entry.start = start;
    entry.end = end;
    entry
}

/// Build line subsection data with the typed encoders.
fn lines_data(columns: bool) -> Vec<u8> {
    let header = lines_header(0x10, 1, u16::from(columns), 0x40);
    let lines = [
        line_entry(0, 0x00ff_ffff, 0x7f, true),
        line_entry(8, 7, 0, false),
    ];
    let column_entries = [column_entry(1, 5), column_entry(2, 0)];
    let mut out = Vec::new();
    header.encode(&mut out).unwrap();
    LineBlock::encode_parts(
        ChecksumOffset(0),
        &lines,
        columns.then_some(&column_entries[..]),
        &mut out,
    )
    .unwrap();
    LineBlock::encode_parts(
        ChecksumOffset(8),
        &lines[1..],
        columns.then_some(&column_entries[1..]),
        &mut out,
    )
    .unwrap();
    out
}

#[test]
fn line_blocks_round_trip_with_and_without_columns() {
    for columns in [false, true] {
        let data = lines_data(columns);
        let LineSubsection::Lines(lines) = round_trip(DEBUG_S_LINES, &data) else {
            panic!("expected DEBUG_S_LINES");
        };
        assert_eq!(lines.block_count(), 2);
        let first = lines.blocks().next().unwrap();
        let entry = first.lines.get(0).unwrap();
        assert_eq!((entry.line_start, entry.end_delta), (0x00ff_ffff, 0x7f));
        assert!(entry.is_statement);
        assert_eq!(first.columns.is_some(), columns);
        // IL lines use the same layout.
        assert!(matches!(
            round_trip(DEBUG_S_IL_LINES, &data),
            LineSubsection::IlLines(_)
        ));
    }
}

#[test]
fn line_encoders_reject_values_that_do_not_fit() {
    let mut out = Vec::new();
    for (entry, what) in [
        (line_entry(0, 0x0100_0000, 0, true), "24-bit line"),
        (line_entry(0, 1, 0x80, true), "7-bit delta"),
    ] {
        assert_eq!(
            error_kind(entry.encode(&mut out)),
            ErrorKind::InvalidFormat,
            "{what}"
        );
    }
    assert_eq!(
        error_kind(lines_header(0, 1, 2, 0).encode(&mut out)),
        ErrorKind::InvalidFormat
    );
    let lines = [line_entry(0, 1, 0, true)];
    assert_eq!(
        error_kind(LineBlock::encode_parts(
            ChecksumOffset(0),
            &lines,
            Some(&[]),
            &mut out
        )),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn file_checksums_round_trip_with_and_without_last_padding() {
    let mut data = Vec::new();
    FileChecksum::encode_parts(NameOffset(1), ChecksumKind::None, &[], &mut data).unwrap();
    FileChecksum::encode_parts(NameOffset(9), ChecksumKind::Md5, &[7; 16], &mut data).unwrap();
    FileChecksum::encode_parts(NameOffset(20), ChecksumKind::Sha1, &[8; 20], &mut data).unwrap();
    assert_eq!(data.len(), 8 + 24 + 28);
    let LineSubsection::FileChecksums(table) = round_trip(DEBUG_S_FILECHKSMS, &data) else {
        panic!("expected DEBUG_S_FILECHKSMS");
    };
    assert_eq!(table.len(), 3);
    assert_eq!(
        table.get(ChecksumOffset(32)).unwrap().name_offset,
        NameOffset(20)
    );
    // The framing can exclude the padding of the last entry.
    let short = &data[..data.len() - 2];
    let table = FileChecksums::parse(short).unwrap();
    let mut out = Vec::new();
    table.encode(&mut out).unwrap();
    assert_eq!(out, short);

    let mut out = Vec::new();
    assert_eq!(
        error_kind(FileChecksum::encode_parts(
            NameOffset(0),
            ChecksumKind::Sha256,
            &[0; 31],
            &mut out
        )),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn inlinee_lines_round_trip_both_signatures() {
    for signature in [InlineeSignature::Normal, InlineeSignature::ExtraFiles] {
        let extra: &[ChecksumOffset] = if signature == InlineeSignature::ExtraFiles {
            &[ChecksumOffset(8), ChecksumOffset(0x18)]
        } else {
            &[]
        };
        let mut data = Vec::new();
        signature.encode(&mut data).unwrap();
        InlineeLine::encode_parts(
            signature,
            IdIndex(0x1001),
            ChecksumOffset(0),
            10,
            extra,
            &mut data,
        )
        .unwrap();
        InlineeLine::encode_parts(
            signature,
            IdIndex(0x1002),
            ChecksumOffset(8),
            20,
            &[],
            &mut data,
        )
        .unwrap();
        let LineSubsection::InlineeLines(lines) = round_trip(DEBUG_S_INLINEELINES, &data) else {
            panic!("expected DEBUG_S_INLINEELINES");
        };
        assert_eq!(lines.len(), 2);
        assert_eq!(lines.iter().next().unwrap().extra_files.len(), extra.len());
    }
    let mut out = Vec::new();
    assert_eq!(
        error_kind(InlineeLine::encode_parts(
            InlineeSignature::Normal,
            IdIndex(0x1001),
            ChecksumOffset(0),
            1,
            &[ChecksumOffset(4)],
            &mut out
        )),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn coff_symbol_rvas_round_trip() {
    let mut data = Vec::new();
    CoffSymbolRvas::encode_parts(&[Rva(0), Rva(0x1_0078), Rva(u32::MAX)], &mut data).unwrap();
    assert_eq!(data, u32s(&[0, 0x1_0078, u32::MAX]));
    let LineSubsection::CoffSymbolRvas(rvas) = round_trip(DEBUG_S_COFF_SYMBOL_RVA, &data) else {
        panic!("expected DEBUG_S_COFF_SYMBOL_RVA");
    };
    assert_eq!(rvas.len(), 3);
    assert!(matches!(
        round_trip(DEBUG_S_COFF_SYMBOL_RVA, &[]),
        LineSubsection::CoffSymbolRvas(_)
    ));
}
