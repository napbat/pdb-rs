//! Tests for address translation through OMAP tables.

use pdb_rs::ErrorKind;
use pdb_rs::native::address::{
    AddressMap, AddressTables, PdbRva, Rva, SectionOffset, Translation, Unmapped,
};
use pdb_rs::native::debug_stream::{OmapRecord, OmapStream};
use pdb_rs::native::section::{SectionHeader, SectionHeaderStream};

/// Build a section header stream from `(virtual_address, virtual_size)` pairs.
fn sections(layout: &[(u32, u32)]) -> SectionHeaderStream {
    let headers: Vec<_> = layout
        .iter()
        .map(|&(address, size)| SectionHeader::new(*b".sect\0\0\0", size, address))
        .collect();
    SectionHeaderStream::from_records(&headers).unwrap()
}

/// Return an OMAP record that maps `rva` to `rva_to`.
fn record(rva: u32, rva_to: u32) -> OmapRecord {
    let mut record = OmapRecord::default();
    record.rva = rva;
    record.rva_to = rva_to;
    record
}

fn omap(records: &[(u32, u32)]) -> OmapStream {
    let records: Vec<_> = records.iter().map(|&(rva, to)| record(rva, to)).collect();
    OmapStream::from_records(&records).unwrap()
}

/// The original image has `.text` at 0x1000 (0x3000 bytes) and `.data` at 0x4000.
/// The rewrite moves three code ranges, removes one, adds new code at 0x3800
/// and moves `.data` to 0x5000. A record with `rvaTo == 0` ends each table.
fn tables() -> AddressTables {
    let mut tables = AddressTables::new(sections(&[(0x1000, 0x3800), (0x5000, 0x100)]));
    tables.original = Some(sections(&[(0x1000, 0x3000), (0x4000, 0x100)]));
    tables.to_source = Some(omap(&[
        (0x1000, 0x2000),
        (0x2000, 0x3000),
        (0x3000, 0x1000),
        (0x3800, 0),
        (0x5000, 0x4000),
        (0x5100, 0),
    ]));
    tables.from_source = Some(omap(&[
        (0x1000, 0x3000),
        (0x1800, 0),
        (0x2000, 0x1000),
        (0x3000, 0x2000),
        (0x4000, 0x5000),
        (0x4100, 0),
    ]));
    tables
}

const fn at(section: u16, offset: u32) -> SectionOffset {
    SectionOffset { section, offset }
}

#[test]
fn pdb_addresses_map_to_image_addresses() {
    let tables = tables();
    let map = tables.map();
    assert!(map.is_remapped());
    assert_eq!(map.sections().len(), 2);
    for (address, image) in [
        (at(1, 0), 0x3000),
        (at(1, 0x7ff), 0x37ff),
        (at(1, 0x1000), 0x1000),
        (at(1, 0x1fff), 0x1fff),
        (at(1, 0x2000), 0x2000),
        (at(1, 0x2fff), 0x2fff),
        (at(2, 0x10), 0x5010),
    ] {
        assert_eq!(
            map.rva(address).unwrap(),
            Translation::Mapped(Rva(image)),
            "{address:?}"
        );
    }
    // The original section headers give the PDB RVA.
    assert_eq!(map.pdb_rva(at(2, 0x10)).unwrap(), PdbRva(0x4010));
    assert_eq!(
        map.pdb_section_offset(PdbRva(0x1900)).unwrap(),
        at(1, 0x900)
    );
    // The rewrite removed the original range 0x1800..0x2000.
    let removed = Translation::Unmapped(Unmapped::NoTarget(record(0x1800, 0)));
    assert_eq!(map.rva(at(1, 0x800)).unwrap(), removed);
    assert_eq!(map.pdb_rva_to_rva(PdbRva(0x1fff)).unwrap(), removed);
    assert_eq!(
        map.pdb_rva_to_rva(PdbRva(0xfff)).unwrap(),
        Translation::Unmapped(Unmapped::BeforeFirstRecord)
    );
}

#[test]
fn image_addresses_map_to_pdb_addresses() {
    let tables = tables();
    let map = tables.map();
    for (image, address) in [
        (0x1000, at(1, 0x1000)),
        (0x1fff, at(1, 0x1fff)),
        (0x2000, at(1, 0x2000)),
        (0x3000, at(1, 0)),
        (0x37ff, at(1, 0x7ff)),
        (0x5000, at(2, 0)),
        (0x50ff, at(2, 0xff)),
    ] {
        assert_eq!(
            map.section_offset(Rva(image)).unwrap(),
            Translation::Mapped(address),
            "{image:#x}"
        );
    }
    // New code at 0x3800..0x5000 has no original address.
    let added = Unmapped::NoTarget(record(0x3800, 0));
    assert_eq!(
        map.section_offset(Rva(0x3800)).unwrap(),
        Translation::Unmapped(added)
    );
    assert_eq!(
        map.rva_to_pdb_rva(Rva(0x4fff)).unwrap(),
        Translation::Unmapped(added)
    );
    assert_eq!(
        map.section_offset(Rva(0x500)).unwrap(),
        Translation::Unmapped(Unmapped::BeforeFirstRecord)
    );
    // The end records give no mapping after `.data`.
    let end = Translation::Unmapped(Unmapped::NoTarget(record(0x5100, 0)));
    assert_eq!(map.section_offset(Rva(0x5100)).unwrap(), end);
    let after = Unmapped::AfterLastRecord(record(0x5100, 0));
    assert_eq!(
        map.rva_to_pdb_rva(Rva(0x6000)).unwrap(),
        Translation::Unmapped(after)
    );
    // No original section holds the end of `.data`.
    let error = map.pdb_section_offset(PdbRva(0x4100)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
    // The image section headers describe the image layout.
    assert_eq!(
        map.image_section_offset(Rva(0x3800)).unwrap(),
        at(1, 0x2800)
    );
    assert_eq!(map.image_section_rva(at(2, 0)).unwrap(), Rva(0x5000));
}

#[test]
fn mapped_addresses_round_trip_in_both_directions() {
    let tables = tables();
    let map = tables.map();
    for original in (0x1000..0x1800).chain(0x2000..0x4000).step_by(0x7f) {
        let Translation::Mapped(image) = map.pdb_rva_to_rva(PdbRva(original)).unwrap() else {
            panic!("{original:#x} has no mapping");
        };
        assert_eq!(
            map.rva_to_pdb_rva(image).unwrap(),
            Translation::Mapped(PdbRva(original))
        );
        let address = map.pdb_section_offset(PdbRva(original)).unwrap();
        assert_eq!(
            map.section_offset(image).unwrap(),
            Translation::Mapped(address)
        );
        assert_eq!(map.rva(address).unwrap(), Translation::Mapped(image));
    }
}

#[test]
fn missing_original_section_headers_fail_pdb_offsets() {
    let mut tables = tables();
    tables.original = None;
    let map = tables.map();
    for error in [
        map.rva(at(1, 0)).unwrap_err(),
        map.section_offset(Rva(0x1000)).unwrap_err(),
        map.pdb_rva(at(1, 0)).unwrap_err(),
        map.pdb_section_offset(PdbRva(0x1000)).unwrap_err(),
    ] {
        assert_eq!(error.kind(), ErrorKind::MissingStream);
    }
    // The RVA translation does not need section headers.
    assert_eq!(
        map.pdb_rva_to_rva(PdbRva(0x1000)).unwrap(),
        Translation::Mapped(Rva(0x3000))
    );
    assert_eq!(
        map.rva_to_pdb_rva(Rva(0x1000)).unwrap(),
        Translation::Mapped(PdbRva(0x2000))
    );
}

#[test]
fn a_missing_table_fails_only_its_direction() {
    let mut tables = tables();
    tables.to_source = None;
    let map = tables.map();
    assert!(map.is_remapped());
    assert_eq!(
        map.rva_to_pdb_rva(Rva(0x1000)).unwrap_err().kind(),
        ErrorKind::MissingStream
    );
    assert_eq!(map.rva(at(1, 0)).unwrap(), Translation::Mapped(Rva(0x3000)));

    let mut tables = self::tables();
    tables.from_source = None;
    let map = tables.map();
    assert_eq!(
        map.pdb_rva_to_rva(PdbRva(0x1000)).unwrap_err().kind(),
        ErrorKind::MissingStream
    );
    assert_eq!(
        map.section_offset(Rva(0x1000)).unwrap(),
        Translation::Mapped(at(1, 0x1000))
    );
}

#[test]
fn tables_without_omap_translate_directly() {
    let mut tables = AddressTables::new(sections(&[(0x1000, 0x100)]));
    // Without OMAP, the map ignores the original section headers.
    tables.original = Some(sections(&[(0x8000, 0x100)]));
    let map = tables.map();
    assert!(!map.is_remapped());
    assert_eq!(
        map.rva(at(1, 0x10)).unwrap(),
        Translation::Mapped(Rva(0x1010))
    );
    assert_eq!(
        map.section_offset(Rva(0x1010)).unwrap(),
        Translation::Mapped(at(1, 0x10))
    );
}

#[test]
fn with_omap_and_empty_tables_map_no_address() {
    let image = sections(&[(0x1000, 0x100)]);
    let empty = omap(&[]);
    let map = AddressMap::with_omap(
        image.headers(),
        Some(image.headers()),
        Some(&empty),
        Some(&empty),
    );
    let before = Unmapped::BeforeFirstRecord;
    assert_eq!(map.rva(at(1, 0)).unwrap(), Translation::Unmapped(before));
    assert_eq!(
        map.section_offset(Rva(0x1000)).unwrap(),
        Translation::Unmapped(before)
    );
    assert_eq!(
        Translation::<u32>::Unmapped(Unmapped::BeforeFirstRecord).mapped(),
        None
    );
    assert_eq!(
        Translation::Mapped(3).map(|value| value + 1).mapped(),
        Some(4)
    );
}
