//! The module stream writer and the global references.

use pdbrs::ErrorKind;
use pdbrs::native::dbi::{Dbi, ModuleRegionSizes, ModuleStream, ModuleStreamParts};
use pdbrs::native::lines::{DEBUG_S_COFF_SYMBOL_RVA, LineSubsection, ModuleLines};
use pdbrs::native::msf::StreamId;
use pdbrs::native::symbols::{RecordOffsets, SymbolOffset};

use crate::{error_kind, u32s};

/// The module stream index in the synthetic module record.
const MODULE_STREAM: u16 = 5;

/// Build a DBI stream with one module record with the region `sizes`.
fn module_dbi(sizes: ModuleRegionSizes) -> Dbi {
    let mut record = vec![0; 64];
    record[34..36].copy_from_slice(&MODULE_STREAM.to_le_bytes());
    record[36..40].copy_from_slice(&sizes.symbol_bytes.to_le_bytes());
    record[40..44].copy_from_slice(&sizes.c11_bytes.to_le_bytes());
    record[44..48].copy_from_slice(&sizes.c13_bytes.to_le_bytes());
    record.extend_from_slice(b"m.o\0m.o\0");
    let mut bytes = vec![0; 64];
    bytes[0..4].copy_from_slice(&(-1_i32).to_le_bytes());
    bytes[4..8].copy_from_slice(&19_990_903_u32.to_le_bytes());
    bytes[16..18].copy_from_slice(&0xffff_u16.to_le_bytes());
    bytes[24..28].copy_from_slice(&u32::try_from(record.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(&record);
    Dbi::parse(bytes).unwrap()
}

/// Parse `bytes` as the module stream of a record with `sizes`.
fn parse(bytes: Vec<u8>, sizes: ModuleRegionSizes) -> ModuleStream {
    let dbi = module_dbi(sizes);
    let module = dbi.modules().next().unwrap().unwrap();
    ModuleStream::parse(bytes, &module).unwrap()
}

/// One `S_END` record and one 8-byte `S_PUB32`-kind record.
const SYMBOLS: [u8; 12] = [2, 0, 6, 0, 6, 0, 0x0e, 0x11, 1, 2, 3, 4];

/// A C13 region with one COFF symbol RVA subsection.
fn c13() -> Vec<u8> {
    let data = u32s(&[0x1000, 0]);
    let subsection = LineSubsection::decode(DEBUG_S_COFF_SYMBOL_RVA, &data).unwrap();
    let mut out = Vec::new();
    subsection.encode_framed(&mut out).unwrap();
    out
}

#[test]
fn writer_joins_regions_and_reports_sizes() {
    let c13 = c13();
    let refs = [SymbolOffset(0), SymbolOffset(8)];
    let parts = ModuleStreamParts {
        symbols: Some(&SYMBOLS),
        c11: &[0xaa; 4],
        c13: &c13,
        global_refs: &refs,
    };
    let encoded = parts.encode().unwrap();
    assert_eq!(
        encoded.sizes,
        ModuleRegionSizes {
            symbol_bytes: 16,
            c11_bytes: 4,
            c13_bytes: 16,
        }
    );
    let mut expected = u32s(&[4]);
    expected.extend_from_slice(&SYMBOLS);
    expected.extend_from_slice(&[0xaa; 4]);
    expected.extend_from_slice(&c13);
    expected.extend_from_slice(&u32s(&[8, 0, 8]));
    assert_eq!(encoded.bytes, expected);

    let stream = parse(encoded.bytes.clone(), encoded.sizes);
    assert_eq!(stream.region_sizes(), encoded.sizes);
    assert_eq!(stream.symbol_data(), SYMBOLS);
    assert_eq!(
        stream.global_refs().unwrap().iter().collect::<Vec<_>>(),
        refs
    );
    assert_eq!(stream.encode().unwrap(), encoded);
    let lines = ModuleLines::parse(&stream).unwrap();
    let mut region = Vec::new();
    lines.encode(&mut region).unwrap();
    assert_eq!(region, c13);
}

#[test]
fn writer_distinguishes_absent_and_empty_symbol_regions() {
    for (symbols, symbol_bytes) in [(None, 0), (Some(&[][..]), 4)] {
        let parts = ModuleStreamParts {
            symbols,
            ..ModuleStreamParts::default()
        };
        let encoded = parts.encode().unwrap();
        assert_eq!(encoded.sizes.symbol_bytes, symbol_bytes);
        assert_eq!(
            encoded.bytes.len(),
            usize::try_from(symbol_bytes).unwrap() + 4
        );
        let stream = parse(encoded.bytes.clone(), encoded.sizes);
        assert_eq!(stream.encode().unwrap(), encoded);
    }
}

#[test]
fn writer_rejects_malformed_regions() {
    let c13 = c13();
    let cases: [(&[u8], &[u8], ErrorKind); 5] = [
        // Symbol records that do not end at a 4-byte boundary.
        (&SYMBOLS[..10], &[], ErrorKind::InvalidFormat),
        // A record length that does not cover the kind.
        (&[1, 0, 6, 0], &[], ErrorKind::InvalidFormat),
        // A record past the end of the region.
        (&[6, 0, 6, 0], &[], ErrorKind::UnexpectedEof),
        // A C13 region that does not end at a 4-byte boundary.
        (&[], &c13[..14], ErrorKind::InvalidFormat),
        // A subsection length past the end of the region.
        (&[], &c13[..12], ErrorKind::UnexpectedEof),
    ];
    for (symbols, c13, kind) in cases {
        let parts = ModuleStreamParts {
            symbols: Some(symbols),
            c13,
            ..ModuleStreamParts::default()
        };
        let error = parts.encode().unwrap_err();
        assert_eq!(error.kind(), kind, "{symbols:?} {c13:?}");
        assert_eq!(error.stream_location(), None);
    }
}

#[test]
fn global_refs_validate_against_the_symbol_record_stream() {
    // Records at offsets 0 and 4 of a 12-byte symbol record stream.
    let records = RecordOffsets::frame(&SYMBOLS, StreamId(7)).unwrap();
    for (offset, valid) in [(0, true), (4, true), (2, false), (12, false)] {
        let parts = ModuleStreamParts {
            global_refs: &[SymbolOffset(4), SymbolOffset(offset)],
            ..ModuleStreamParts::default()
        };
        let encoded = parts.encode().unwrap();
        let stream = parse(encoded.bytes, encoded.sizes);
        let refs = stream.global_refs().unwrap();
        assert_eq!(refs.len(), 2);
        let result = refs.validate(&records);
        if valid {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidReference);
            // The size field is at offset 0, the second reference at 8.
            assert_eq!(error.stream_location(), Some((StreamId(5), 8)));
        }
    }
}

#[test]
fn global_refs_reject_a_partial_reference() {
    let bytes = u32s(&[2, 0]);
    let stream = parse(bytes[..6].to_vec(), ModuleRegionSizes::default());
    assert_eq!(stream.global_ref_data(), [0, 0]);
    let error = stream.global_refs().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(error.stream_location(), Some((StreamId(5), 4)));
    assert_eq!(error_kind(stream.encode()), ErrorKind::InvalidFormat);
}
