//! Tests for the OMAP tables of the optional debug streams.

use pdbrs::ErrorKind;
use pdbrs::native::address::{Translation, Unmapped};
use pdbrs::native::debug_stream::{DebugRecord, OMAP_RECORD_SIZE, OmapRecord, OmapStream};
use pdbrs::native::msf::StreamId;

/// The stream ID of the synthetic streams.
const STREAM: StreamId = StreamId(14);

fn bytes(records: &[(u32, u32)]) -> Vec<u8> {
    records
        .iter()
        .flat_map(|&(rva, to)| [rva.to_le_bytes(), to.to_le_bytes()].concat())
        .collect()
}

fn table(records: &[(u32, u32)]) -> OmapStream {
    OmapStream::parse(bytes(records), STREAM).unwrap()
}

/// Return an OMAP record that maps `rva` to `rva_to`.
fn record(rva: u32, rva_to: u32) -> OmapRecord {
    let mut record = OmapRecord::default();
    record.rva = rva;
    record.rva_to = rva_to;
    record
}

fn mapped(value: u32) -> Translation<u32> {
    Translation::Mapped(value)
}

#[test]
fn empty_table_maps_no_address() {
    let empty = table(&[]);
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.record_for(0), None);
    for address in [0, 1, u32::MAX] {
        assert_eq!(
            empty.lookup(address).unwrap(),
            Translation::Unmapped(Unmapped::BeforeFirstRecord)
        );
    }
    assert_eq!(empty.to_bytes().unwrap(), [0_u8; 0]);
}

#[test]
fn truncated_tables_fail_with_the_record_offset() {
    for (length, offset) in [(1, 0), (7, 0), (9, 8), (15, 8), (20, 16)] {
        let error = OmapStream::parse(vec![0; length], STREAM).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof, "{error}");
        assert_eq!(error.stream_location(), Some((STREAM, offset)));
    }
}

#[test]
fn unsorted_tables_fail_with_the_record_offset() {
    let error =
        OmapStream::parse(bytes(&[(0x1000, 1), (0x2000, 2), (0x1fff, 3)]), STREAM).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(error.stream_location(), Some((STREAM, 16)));
    let error = OmapStream::from_records(&[record(2, 0), record(1, 0)]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(error.stream_location(), None);
    // Equal source addresses keep the order: the last record maps the address.
    let equal = table(&[(0x1000, 0x5000), (0x1000, 0x6000), (0x2000, 0)]);
    assert_eq!(equal.lookup(0x1004).unwrap(), mapped(0x6004));
}

#[test]
fn lookup_uses_the_last_record_at_or_before_the_address() {
    let omap = table(&[
        (0x1000, 0x3000),
        (0x1800, 0),
        (0x2000, 0x1000),
        (0x3000, 0x2000),
    ]);
    assert_eq!(omap.len(), 4);
    assert_eq!(
        omap.lookup(0xfff).unwrap(),
        Translation::Unmapped(Unmapped::BeforeFirstRecord)
    );
    assert_eq!(
        omap.lookup(0).unwrap(),
        Translation::Unmapped(Unmapped::BeforeFirstRecord)
    );
    // Record boundaries.
    assert_eq!(omap.lookup(0x1000).unwrap(), mapped(0x3000));
    assert_eq!(omap.lookup(0x17ff).unwrap(), mapped(0x37ff));
    let gap = record(0x1800, 0);
    for address in [0x1800, 0x1fff] {
        assert_eq!(
            omap.lookup(address).unwrap(),
            Translation::Unmapped(Unmapped::NoTarget(gap))
        );
    }
    assert_eq!(omap.lookup(0x2000).unwrap(), mapped(0x1000));
    assert_eq!(omap.lookup(0x2fff).unwrap(), mapped(0x1fff));
    // DIA maps only the `rva` of the last record.
    let last = record(0x3000, 0x2000);
    assert_eq!(omap.lookup(0x3000).unwrap(), mapped(0x2000));
    for address in [0x3001, 0x10_0000, u32::MAX] {
        assert_eq!(
            omap.lookup(address).unwrap(),
            Translation::Unmapped(Unmapped::AfterLastRecord(last))
        );
    }
    assert_eq!(omap.record_for(0x3001), Some(last));
    // The last record with `rvaTo == 0` ends the table without a mapping.
    let ended = table(&[(0x1000, 0x2000), (0x2000, 0)]);
    assert_eq!(
        ended.lookup(0x2000).unwrap(),
        Translation::Unmapped(Unmapped::NoTarget(record(0x2000, 0)))
    );
    assert_eq!(ended.lookup(0x1fff).unwrap(), mapped(0x2fff));
    assert_eq!(omap.record_for(0x2abc), Some(record(0x2000, 0x1000)));
}

#[test]
fn lookup_fails_if_the_target_overflows() {
    let omap = table(&[(0, 0xffff_fff0), (0x100, 0)]);
    assert_eq!(omap.lookup(0xf).unwrap(), mapped(0xffff_ffff));
    let last = table(&[(0xffff_fff0, 0xffff_ffff)]);
    assert_eq!(last.lookup(0xffff_fff0).unwrap(), mapped(0xffff_ffff));
    let error = omap.lookup(0x10).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::OffsetOverflow);
}

#[test]
fn tables_encode_the_same_bytes() {
    let records = [(0x1000, 0x3000), (0x1800, 0), (0xffff_ffff, 1)];
    let serialized = bytes(&records);
    let omap = OmapStream::parse(serialized.clone(), STREAM).unwrap();
    assert_eq!(omap.to_bytes().unwrap(), serialized);
    let mut out = vec![7];
    omap.encode(&mut out).unwrap();
    assert_eq!(&out[1..], serialized.as_slice());

    let decoded: Vec<_> = omap.iter().collect();
    assert_eq!(decoded[1], record(0x1800, 0));
    assert_eq!((&omap).into_iter().next_back(), Some(decoded[2]));
    let rebuilt = OmapStream::from_records(&decoded).unwrap();
    assert_eq!(rebuilt, omap);
    assert_eq!(rebuilt.records().as_bytes(), serialized.as_slice());

    let mut one = Vec::new();
    record(1, 2).encode(&mut one).unwrap();
    assert_eq!(one, [1, 0, 0, 0, 2, 0, 0, 0]);
    assert_eq!(one.len(), OMAP_RECORD_SIZE);
    assert_eq!(OmapRecord::SIZE, OMAP_RECORD_SIZE);
}
