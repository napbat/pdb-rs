//! Tests for image section headers and address translation.

use pdbrs::ErrorKind;
use pdbrs::native::address::{AddressMap, Rva, SectionOffset};
use pdbrs::native::dbi::{Dbi, DebugStream};
use pdbrs::native::section::{SECTION_HEADER_SIZE, SectionHeaders};

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

fn assert_error<T: core::fmt::Debug>(result: pdbrs::Result<T>, kind: ErrorKind) {
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
fn address_translation_round_trips_inside_sections() {
    let bytes = image();
    let dbi = dbi_with_slots(&[0xffff; 11]);
    let map = AddressMap::new(SectionHeaders::parse(&bytes).unwrap(), &dbi);
    assert_eq!(map.sections().len(), 3);
    for (section, offset, rva) in [(1, 0, 0x1000), (1, 0x14ff, 0x24ff), (2, 0x80, 0x3080)] {
        let address = SectionOffset { section, offset };
        assert_eq!(map.rva(address).unwrap(), Rva(rva));
        assert_eq!(map.section_offset(Rva(rva)).unwrap(), address);
    }
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
    assert_eq!(map.rva(at(1, 0x1500)).unwrap(), Rva(0x2500));
    assert_error(map.rva(at(1, 0x1501)), ErrorKind::InvalidReference);
    assert_error(map.rva(at(3, 0x8)), ErrorKind::OffsetOverflow);
    assert_eq!(map.rva(at(3, 0x7)).unwrap(), Rva(u32::MAX));
    assert_error(map.section_offset(Rva(0xfff)), ErrorKind::InvalidReference);
    assert_error(map.section_offset(Rva(0x2500)), ErrorKind::InvalidReference);
    assert_error(map.section_offset(Rva(0x3100)), ErrorKind::InvalidReference);
}

#[test]
fn omap_slots_make_translation_unsupported() {
    let bytes = image();
    for slot in [DebugStream::OmapToSource, DebugStream::OmapFromSource] {
        let mut slots = [0xffff; 11];
        slots[slot.index()] = 9;
        let dbi = dbi_with_slots(&slots);
        let map = AddressMap::new(SectionHeaders::parse(&bytes).unwrap(), &dbi);
        assert_error(
            map.rva(SectionOffset {
                section: 1,
                offset: 0,
            }),
            ErrorKind::UnsupportedVersion,
        );
        assert_error(
            map.section_offset(Rva(0x1000)),
            ErrorKind::UnsupportedVersion,
        );
    }
}
