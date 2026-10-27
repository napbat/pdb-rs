//! Tests for image section headers and address translation.

use pdb_rs::ErrorKind;
use pdb_rs::native::address::{AddressMap, PdbRva, Rva, SectionOffset, Translation};
use pdb_rs::native::dbi::{Dbi, DebugStream};
use pdb_rs::native::debug_stream::DebugRecord;
use pdb_rs::native::msf::StreamId;
use pdb_rs::native::section::{
    SECTION_HEADER_SIZE, SectionHeader, SectionHeaderStream, SectionHeaders,
};

/// The header offsets of the substream sizes, in disk order.
const SIZE_FIELDS: [usize; 7] = [24, 28, 32, 36, 40, 52, 48];

fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
    bytes[offset..offset + value.len()].copy_from_slice(value);
}

/// Build a DBI stream with only an optional debug header.
fn dbi_with_slots(slots: &[u16]) -> Dbi {
    let debug: Vec<u8> = slots.iter().flat_map(|slot| slot.to_le_bytes()).collect();
    let mut bytes = vec![0; 64];
    put(&mut bytes, 0, &(-1_i32).to_le_bytes());
    put(&mut bytes, 4, &19_990_903_u32.to_le_bytes());
    for (offset, part) in SIZE_FIELDS
        .into_iter()
        .zip([&[][..], &[], &[], &[], &[], &[], &debug])
    {
        put(
            &mut bytes,
            offset,
            &i32::try_from(part.len()).unwrap().to_le_bytes(),
        );
        bytes.extend_from_slice(part);
    }
    Dbi::parse(bytes).unwrap()
}

/// Build one section header.
fn header(name: &[u8], virtual_size: u32, virtual_address: u32, characteristics: u32) -> Vec<u8> {
    let mut bytes = vec![0; SECTION_HEADER_SIZE];
    put(&mut bytes, 0, name);
    put(&mut bytes, 8, &virtual_size.to_le_bytes());
    put(&mut bytes, 12, &virtual_address.to_le_bytes());
    put(&mut bytes, 16, &0x200_u32.to_le_bytes());
    put(&mut bytes, 20, &0x400_u32.to_le_bytes());
    put(&mut bytes, 24, &0x11_u32.to_le_bytes());
    put(&mut bytes, 28, &0x22_u32.to_le_bytes());
    put(&mut bytes, 32, &3_u16.to_le_bytes());
    put(&mut bytes, 34, &4_u16.to_le_bytes());
    put(&mut bytes, 36, &characteristics.to_le_bytes());
    bytes
}

fn image() -> Vec<u8> {
    [
        header(b".text", 0x1500, 0x1000, 0x6000_0020),
        header(b".rdata\0\0", 0x100, 0x3000, 0x4000_0040),
        header(b"12345678", 0x10, 0xffff_fff8, 0),
    ]
    .concat()
}

fn assert_error<T: core::fmt::Debug>(result: pdb_rs::Result<T>, kind: ErrorKind) {
    let error = result.unwrap_err();
    assert_eq!(error.kind(), kind);
    assert_eq!(error.offset(), None);
    assert_eq!(error.stream_location(), None);
}

#[test]
fn section_headers_decode_all_fields() {
    let bytes = image();
    let headers = SectionHeaders::parse(&bytes).unwrap();
    assert_eq!(headers.len(), 3);
    assert!(!headers.is_empty());
    let text = headers.section(1).unwrap();
    assert_eq!(&text.name, b".text\0\0\0");
    assert_eq!(text.name_bytes(), b".text");
    assert_eq!((text.virtual_size, text.virtual_address), (0x1500, 0x1000));
    assert_eq!(
        (text.size_of_raw_data, text.pointer_to_raw_data),
        (0x200, 0x400)
    );
    assert_eq!(
        (text.pointer_to_relocations, text.pointer_to_line_numbers),
        (0x11, 0x22)
    );
    assert_eq!((text.relocation_count, text.line_number_count), (3, 4));
    assert_eq!(text.characteristics, 0x6000_0020);
    assert_eq!(headers.section(3).unwrap().name_bytes(), b"12345678");
    assert!(headers.section(0).is_none());
    assert!(headers.section(4).is_none());
    let names: Vec<_> = (&headers).into_iter().map(|h| h.virtual_address).collect();
    assert_eq!(names, [0x1000, 0x3000, 0xffff_fff8]);
    assert_eq!(headers.iter().next_back().unwrap().virtual_size, 0x10);
    assert_eq!(headers.iter().nth(1).unwrap().name_bytes(), b".rdata");
}

#[test]
fn section_header_stream_length_must_be_a_multiple_of_40() {
    assert!(SectionHeaders::parse(&[]).unwrap().is_empty());
    assert_error(SectionHeaders::parse(&[0; 41]), ErrorKind::InvalidFormat);
    assert_error(SectionHeaders::parse(&[0; 39]), ErrorKind::InvalidFormat);
}

#[test]
fn section_headers_encode_the_same_bytes() {
    let bytes = image();
    let headers = SectionHeaders::parse(&bytes).unwrap();
    let mut out = vec![0xaa];
    headers.encode(&mut out).unwrap();
    assert_eq!(out[0], 0xaa);
    assert_eq!(&out[1..], bytes.as_slice());

    let stream = SectionHeaderStream::parse(bytes.clone(), StreamId(12)).unwrap();
    assert_eq!(stream.len(), 3);
    assert_eq!(stream.to_bytes().unwrap(), bytes);
    assert_eq!(stream.headers().section(2), headers.section(2));
    assert_eq!(stream.get(2), headers.section(3));
    assert_eq!(stream.get(3), None);

    let records: Vec<_> = headers.iter().collect();
    let rebuilt = SectionHeaderStream::from_records(&records).unwrap();
    assert_eq!(rebuilt.as_bytes(), bytes.as_slice());

    let mut header = SectionHeader::new(*b".data\0\0\0", 0x20, 0x5000);
    header.characteristics = 0xc000_0040;
    let mut one = Vec::new();
    header.encode(&mut one).unwrap();
    assert_eq!(one.len(), SectionHeader::SIZE);
    let decoded = SectionHeaders::parse(&one).unwrap().section(1).unwrap();
    assert_eq!(decoded, header);
    assert_eq!(decoded.size_of_raw_data, 0);
}

#[test]
fn section_header_stream_rejects_partial_records_with_a_location() {
    assert!(
        SectionHeaderStream::parse(Vec::new(), StreamId(5))
            .unwrap()
            .is_empty()
    );
    for (length, offset) in [(39, 0), (41, 40), (79, 40)] {
        let error = SectionHeaderStream::parse(vec![0; length], StreamId(5)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
        assert_eq!(error.stream_location(), Some((StreamId(5), offset)));
    }
}

#[test]
fn address_translation_round_trips_inside_sections() {
    let bytes = image();
    let dbi = dbi_with_slots(&[0xffff; 11]);
    let map = AddressMap::new(SectionHeaders::parse(&bytes).unwrap(), &dbi);
    assert_eq!(map.sections().len(), 3);
    for (section, offset, rva) in [(1, 0, 0x1000), (1, 0x14ff, 0x24ff), (2, 0x80, 0x3080)] {
        let address = SectionOffset { section, offset };
        assert_eq!(map.rva(address).unwrap(), Translation::Mapped(Rva(rva)));
        assert_eq!(
            map.section_offset(Rva(rva)).unwrap(),
            Translation::Mapped(address)
        );
        assert_eq!(map.pdb_rva(address).unwrap(), PdbRva(rva));
        assert_eq!(map.pdb_section_offset(PdbRva(rva)).unwrap(), address);
        assert_eq!(map.image_section_rva(address).unwrap(), Rva(rva));
        assert_eq!(map.image_section_offset(Rva(rva)).unwrap(), address);
        assert_eq!(
            map.pdb_rva_to_rva(PdbRva(rva)).unwrap(),
            Translation::Mapped(Rva(rva))
        );
        assert_eq!(
            map.rva_to_pdb_rva(Rva(rva)).unwrap(),
            Translation::Mapped(PdbRva(rva))
        );
    }
    assert!(!map.is_remapped());
}

#[test]
fn address_translation_checks_bounds() {
    let bytes = image();
    let dbi = dbi_with_slots(&[]);
    let map = AddressMap::new(SectionHeaders::parse(&bytes).unwrap(), &dbi);
    let at = |section, offset| SectionOffset { section, offset };
    assert_error(map.rva(at(0, 0)), ErrorKind::InvalidReference);
    assert_error(map.rva(at(4, 0)), ErrorKind::InvalidReference);
    // The end of a section has an RVA; addresses after the end do not.
    assert_eq!(
        map.rva(at(1, 0x1500)).unwrap(),
        Translation::Mapped(Rva(0x2500))
    );
    assert_error(map.rva(at(1, 0x1501)), ErrorKind::InvalidReference);
    assert_error(map.rva(at(3, 0x8)), ErrorKind::OffsetOverflow);
    assert_eq!(
        map.rva(at(3, 0x7)).unwrap(),
        Translation::Mapped(Rva(u32::MAX))
    );
    assert_error(map.section_offset(Rva(0xfff)), ErrorKind::InvalidReference);
    assert_error(map.section_offset(Rva(0x2500)), ErrorKind::InvalidReference);
    assert_error(map.section_offset(Rva(0x3100)), ErrorKind::InvalidReference);
}

#[test]
fn omap_slots_without_tables_make_translation_fail() {
    let bytes = image();
    for slot in [DebugStream::OmapToSource, DebugStream::OmapFromSource] {
        let mut slots = [0xffff; 11];
        slots[slot.index()] = 9;
        let dbi = dbi_with_slots(&slots);
        let map = AddressMap::new(SectionHeaders::parse(&bytes).unwrap(), &dbi);
        assert!(map.is_remapped());
        assert_error(
            map.rva(SectionOffset {
                section: 1,
                offset: 0,
            }),
            ErrorKind::MissingStream,
        );
        assert_error(map.section_offset(Rva(0x1000)), ErrorKind::MissingStream);
        assert_error(map.pdb_rva_to_rva(PdbRva(0x1000)), ErrorKind::MissingStream);
        assert_error(map.rva_to_pdb_rva(Rva(0x1000)), ErrorKind::MissingStream);
        // The image section headers do not need OMAP.
        assert_eq!(
            map.image_section_rva(SectionOffset {
                section: 1,
                offset: 0
            })
            .unwrap(),
            Rva(0x1000)
        );
    }
}
