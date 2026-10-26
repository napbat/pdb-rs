//! Line blocks, IL lines, file checksums, inlinee lines and COFF symbol RVAs.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::index::IdIndex;
use pdbrs::native::lines::{
    ChecksumKind, ColumnEntry, DEBUG_S_COFF_SYMBOL_RVA, DEBUG_S_FILECHKSMS, DEBUG_S_IL_LINES,
    DEBUG_S_INLINEELINES, DEBUG_S_LINES, FileChecksums, InlineeSignature, LineEntry,
    LineSubsection, LinesHeader, encode_coff_symbol_rvas, encode_file_checksum,
    encode_inlinee_line, encode_line_block,
};

use crate::{error_kind, round_trip, u32s};

/// Build line subsection data with the typed encoders.
fn lines_data(columns: bool) -> Vec<u8> {
    let header = LinesHeader::new(0x10, 1, u16::from(columns), 0x40);
    let lines = [
        LineEntry::new(0, 0x00ff_ffff, 0x7f, true),
        LineEntry::new(8, 7, 0, false),
    ];
    let column_entries = [ColumnEntry::new(1, 5), ColumnEntry::new(2, 0)];
    let mut out = Vec::new();
    header.encode(&mut out).unwrap();
    encode_line_block(0, &lines, columns.then_some(&column_entries[..]), &mut out).unwrap();
    encode_line_block(
        8,
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
        (LineEntry::new(0, 0x0100_0000, 0, true), "24-bit line"),
        (LineEntry::new(0, 1, 0x80, true), "7-bit delta"),
    ] {
        assert_eq!(
            error_kind(entry.encode(&mut out)),
            ErrorKind::InvalidFormat,
            "{what}"
        );
    }
    assert_eq!(
        error_kind(LinesHeader::new(0, 1, 2, 0).encode(&mut out)),
        ErrorKind::InvalidFormat
    );
    let lines = [LineEntry::new(0, 1, 0, true)];
    assert_eq!(
        error_kind(encode_line_block(0, &lines, Some(&[]), &mut out)),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn file_checksums_round_trip_with_and_without_last_padding() {
    let mut data = Vec::new();
    encode_file_checksum(1, ChecksumKind::None, &[], &mut data).unwrap();
    encode_file_checksum(9, ChecksumKind::Md5, &[7; 16], &mut data).unwrap();
    encode_file_checksum(20, ChecksumKind::Sha1, &[8; 20], &mut data).unwrap();
    assert_eq!(data.len(), 8 + 24 + 28);
    let LineSubsection::FileChecksums(table) = round_trip(DEBUG_S_FILECHKSMS, &data) else {
        panic!("expected DEBUG_S_FILECHKSMS");
    };
    assert_eq!(table.len(), 3);
    assert_eq!(table.get(32).unwrap().name_offset, 20);
    // The framing can exclude the padding of the last entry.
    let short = &data[..data.len() - 2];
    let table = FileChecksums::decode(short).unwrap();
    let mut out = Vec::new();
    table.encode(&mut out).unwrap();
    assert_eq!(out, short);

    let mut out = Vec::new();
    assert_eq!(
        error_kind(encode_file_checksum(
            0,
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
        let extra: &[u32] = if signature == InlineeSignature::ExtraFiles {
            &[8, 0x18]
        } else {
            &[]
        };
        let mut data = Vec::new();
        signature.encode(&mut data).unwrap();
        encode_inlinee_line(signature, IdIndex(0x1001), 0, 10, extra, &mut data).unwrap();
        encode_inlinee_line(signature, IdIndex(0x1002), 8, 20, &[], &mut data).unwrap();
        let LineSubsection::InlineeLines(lines) = round_trip(DEBUG_S_INLINEELINES, &data) else {
            panic!("expected DEBUG_S_INLINEELINES");
        };
        assert_eq!(lines.len(), 2);
        assert_eq!(lines.iter().next().unwrap().extra_files.len(), extra.len());
    }
    let mut out = Vec::new();
    assert_eq!(
        error_kind(encode_inlinee_line(
            InlineeSignature::Normal,
            IdIndex(0x1001),
            0,
            1,
            &[4],
            &mut out
        )),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn coff_symbol_rvas_round_trip() {
    let mut data = Vec::new();
    encode_coff_symbol_rvas(&[0, 0x1_0078, u32::MAX], &mut data).unwrap();
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
