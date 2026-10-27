//! Tests for the DBI section contribution, section map, source info, type
//! server map and EC substreams.

use pdb_rs::ErrorKind;
use pdb_rs::native::codeview::index::{ModuleIndex, NameOffset};
use pdb_rs::native::dbi::{
    Dbi, SECTION_CONTRIBUTION_V2, SECTION_CONTRIBUTION_V60, SectionContributionVersion,
    SectionContributions, SectionMap, SegmentFlags, SourceInfo,
};
use pdb_rs::native::msf::StreamId;
use pdb_rs::native::names::SIGNATURE;

/// The header offsets of the substream sizes, in disk order.
const SIZE_FIELDS: [usize; 7] = [24, 28, 32, 36, 40, 52, 48];

fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
    bytes[offset..offset + value.len()].copy_from_slice(value);
}

/// Build a modern DBI stream from substreams in disk order.
fn dbi(parts: [&[u8]; 7]) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    put(&mut bytes, 0, &(-1_i32).to_le_bytes());
    put(&mut bytes, 4, &19_990_903_u32.to_le_bytes());
    for (offset, part) in SIZE_FIELDS.into_iter().zip(parts) {
        let size = i32::try_from(part.len()).unwrap();
        put(&mut bytes, offset, &size.to_le_bytes());
        bytes.extend_from_slice(part);
    }
    bytes
}

/// Build one padded module info record.
fn module(name: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    put(&mut bytes, 34, &0xffff_u16.to_le_bytes());
    bytes.extend_from_slice(name);
    bytes.push(0);
    bytes.extend_from_slice(name);
    bytes.push(0);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
}

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// Pack two `u16` values into one word, the first at the lower address.
fn pair(low: u16, high: u16) -> u32 {
    u32::from(low) | (u32::from(high) << 16)
}

fn assert_error<T: core::fmt::Debug>(result: pdb_rs::Result<T>, kind: ErrorKind) {
    let error = result.unwrap_err();
    assert_eq!(error.kind(), kind);
    assert_eq!(error.offset(), None);
    assert_eq!(error.stream_location(), None);
}

fn assert_located<T: core::fmt::Debug>(result: pdb_rs::Result<T>, kind: ErrorKind, offset: u64) {
    let error = result.unwrap_err();
    assert_eq!(error.kind(), kind);
    assert_eq!(error.offset(), None);
    assert_eq!(error.stream_location(), Some((StreamId(3), offset)));
}

#[test]
fn v60_contributions_keep_raw_fields() {
    let bytes = words(&[
        SECTION_CONTRIBUTION_V60,
        pair(1, 0xcccc),
        0x10,
        0xffff_fff0,
        0x6000_0020,
        pair(2, 0xdddd),
        7,
        8,
        pair(3, 0),
        0,
        4,
        0x4000_0040,
        pair(0, 0),
        0,
        0,
    ]);
    let contributions = SectionContributions::parse(&bytes).unwrap();
    assert_eq!(contributions.version(), SectionContributionVersion::V60);
    assert_eq!(contributions.len(), 2);
    assert!(!contributions.is_empty());
    let first = contributions.iter().next().unwrap();
    assert_eq!(first.section, 1);
    assert_eq!(first.padding1, 0xcccc);
    assert_eq!(first.offset, 0x10);
    assert_eq!(first.size, -16);
    assert_eq!(first.characteristics, 0x6000_0020);
    assert_eq!(first.module, ModuleIndex(2));
    assert_eq!(first.padding2, 0xdddd);
    assert_eq!(first.data_crc, Some(7));
    assert_eq!(first.reloc_crc, Some(8));
    assert_eq!(first.coff_section, None);
    let last = contributions.iter().next_back().unwrap();
    assert_eq!((last.section, last.size), (3, 4));
    assert_eq!((&contributions).into_iter().len(), 2);
}

#[test]
fn v2_and_v40_contribution_layouts() {
    let v2 = words(&[
        SECTION_CONTRIBUTION_V2,
        pair(1, 0),
        0,
        1,
        2,
        pair(4, 0),
        5,
        6,
        9,
    ]);
    let contributions = SectionContributions::parse(&v2).unwrap();
    assert_eq!(contributions.version(), SectionContributionVersion::V2);
    let record = contributions.iter().next().unwrap();
    assert_eq!(record.coff_section, Some(9));
    assert_eq!((record.data_crc, record.reloc_crc), (Some(5), Some(6)));

    // Headerless VC++ 4.0 records have five words each and no CRC.
    let v40 = words(&[
        pair(2, 0),
        0x20,
        0x30,
        0x40,
        pair(1, 0),
        pair(3, 0),
        0,
        1,
        2,
        pair(0, 0),
    ]);
    let contributions = SectionContributions::parse(&v40).unwrap();
    assert_eq!(contributions.version(), SectionContributionVersion::V40);
    assert_eq!(contributions.len(), 2);
    let record = contributions.iter().next().unwrap();
    assert_eq!(
        (record.section, record.offset, record.size),
        (2, 0x20, 0x30)
    );
    assert_eq!(record.characteristics, 0x40);
    assert_eq!(record.module, ModuleIndex(1));
    assert_eq!(
        (record.data_crc, record.reloc_crc, record.coff_section),
        (None, None, None)
    );

    let empty = words(&[SECTION_CONTRIBUTION_V60]);
    let contributions = SectionContributions::parse(&empty).unwrap();
    assert!(contributions.is_empty());
    assert_eq!(contributions.iter().count(), 0);
}

#[test]
fn contribution_errors() {
    assert_error(
        SectionContributions::parse(&[0; 3]),
        ErrorKind::UnexpectedEof,
    );
    let mut partial = words(&[SECTION_CONTRIBUTION_V60, 0, 0, 0, 0, 0, 0]);
    assert_error(
        SectionContributions::parse(&partial),
        ErrorKind::InvalidFormat,
    );
    partial.extend_from_slice(&[0, 0, 0, 0, 0]);
    assert_error(
        SectionContributions::parse(&partial),
        ErrorKind::InvalidFormat,
    );
    assert_error(
        SectionContributions::parse(&[0; 24]),
        ErrorKind::InvalidFormat,
    );
}

#[test]
fn section_map_decodes_descriptors() {
    let bytes = words(&[
        pair(2, 2),
        pair(0x010d, 0),
        pair(0, 1),
        pair(0xffff, 0xffff),
        0,
        0x1234,
        pair(0x0208, 0),
        pair(0, 0),
        pair(0xffff, 0xffff),
        0,
        u32::MAX,
    ]);
    let map = SectionMap::parse(&bytes).unwrap();
    assert_eq!((map.len(), map.logical_count), (2, 2));
    let entries: Vec<_> = (&map).into_iter().collect();
    let text = entries[0];
    assert!(text.flags.contains(SegmentFlags::READ));
    assert!(text.flags.contains(SegmentFlags::EXECUTE));
    assert!(text.flags.contains(SegmentFlags::ADDRESS_IS_32_BIT));
    assert!(text.flags.contains(SegmentFlags::IS_SELECTOR));
    assert!(!text.flags.contains(SegmentFlags::WRITE));
    assert_eq!((text.overlay, text.group, text.frame), (0, 0, 1));
    assert_eq!((text.section_name, text.class_name), (0xffff, 0xffff));
    assert_eq!((text.offset, text.length), (0, 0x1234));
    let absolute = entries[1];
    assert!(absolute.flags.contains(SegmentFlags::IS_ABSOLUTE_ADDRESS));
    assert!(!absolute.flags.contains(SegmentFlags::IS_GROUP));
    assert_eq!((absolute.frame, absolute.length), (0, u32::MAX));
    assert_eq!(map.iter().next_back(), Some(absolute));
}

#[test]
fn section_map_errors() {
    assert_error(SectionMap::parse(&[1, 0]), ErrorKind::UnexpectedEof);
    let short = words(&[pair(1, 1), 0, 0, 0, 0]);
    assert_error(SectionMap::parse(&short), ErrorKind::UnexpectedEof);
    let mut long = words(&[pair(0, 0)]);
    long.push(0);
    assert_error(SectionMap::parse(&long), ErrorKind::InvalidFormat);
    let extra = words(&[pair(0, 0), 0, 0, 0, 0, 0]);
    assert_error(SectionMap::parse(&extra), ErrorKind::InvalidFormat);
}

/// Build a source info substream; each module lists `(name offset)` values.
fn source_info(raw_count: u16, starts: &[u16], modules: &[&[u32]], names: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&u16::try_from(modules.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(&raw_count.to_le_bytes());
    for start in starts {
        bytes.extend_from_slice(&start.to_le_bytes());
    }
    for files in modules {
        bytes.extend_from_slice(&u16::try_from(files.len()).unwrap().to_le_bytes());
    }
    for offset in modules.iter().flat_map(|files| files.iter()) {
        bytes.extend_from_slice(&offset.to_le_bytes());
    }
    bytes.extend_from_slice(names);
    bytes
}

#[test]
fn source_info_lists_files_per_module() {
    let names = b"a.rs\0lib\\b.h\0\0\0";
    let bytes = source_info(3, &[0, 2], &[&[0, 5], &[5]], names);
    let info = SourceInfo::parse(&bytes).unwrap();
    assert_eq!(info.modules.len(), 2);
    assert_eq!(info.raw_file_count, 3);
    assert_eq!(info.file_count(), 3);
    assert_eq!(info.names, names);
    assert_eq!(info.file_name(9).unwrap(), b"b.h");
    assert_eq!(info.file_name(14).unwrap(), b"");
    assert_error(info.file_name(15), ErrorKind::InvalidReference);
    assert_error(info.file_name(u32::MAX), ErrorKind::InvalidReference);

    let modules = &info.modules;
    assert_eq!(modules.len(), 2);
    assert_eq!(modules[0].raw_start, 0);
    assert_eq!(modules[0].name_offsets, [0, 5]);
    let files: Vec<_> = info
        .file_names(&modules[0])
        .collect::<pdb_rs::Result<_>>()
        .unwrap();
    assert_eq!(files, [b"a.rs".as_slice(), b"lib\\b.h"]);
    assert_eq!(modules[1].raw_start, 2);
    assert_eq!(modules[1].name_offsets, [5]);
    assert_eq!(
        info.file_names(&modules[1]).next().unwrap().unwrap(),
        b"lib\\b.h"
    );
}

#[test]
fn source_info_ignores_overflowed_16_bit_counts() {
    let first: Vec<u32> = vec![0; 40_000];
    let second: Vec<u32> = vec![2; 30_000];
    // The sum is 70000. The header count and the second start wrap at 65536.
    let raw_count = u16::try_from(70_000_u32 % 65_536).unwrap();
    let bytes = source_info(raw_count, &[0, 40_000], &[&first, &second], b"x\0y\0");
    let info = SourceInfo::parse(&bytes).unwrap();
    assert_eq!(info.raw_file_count, 4464);
    assert_eq!(info.file_count(), 70_000);
    assert_eq!(info.modules[0].name_offsets.len(), 40_000);
    assert_eq!(info.modules[1].name_offsets.len(), 30_000);
    assert!(
        info.file_names(&info.modules[1])
            .all(|name| name.unwrap() == b"y")
    );
    assert_eq!(info.names, b"x\0y\0");
}

#[test]
fn source_info_bad_name_offset_is_reported_per_file() {
    let bytes = source_info(1, &[0], &[&[7, 2]], b"a\0b");
    let info = SourceInfo::parse(&bytes).unwrap();
    let mut names = info.file_names(&info.modules[0]);
    // The first offset is past the buffer; the second name has no NUL.
    assert_error(names.next().unwrap(), ErrorKind::InvalidReference);
    assert_error(names.next().unwrap(), ErrorKind::InvalidReference);
    assert!(names.next().is_none());
}

#[test]
fn source_info_truncation() {
    let bytes = source_info(2, &[0, 1], &[&[0], &[0]], b"");
    // The header, the arrays and the offsets are required; names are optional.
    for end in 0..bytes.len() {
        assert_error(SourceInfo::parse(&bytes[..end]), ErrorKind::UnexpectedEof);
    }
    assert_eq!(SourceInfo::parse(&bytes).unwrap().names, b"");
}

fn string_table() -> Vec<u8> {
    let mut bytes = words(&[SIGNATURE, 1, 3]);
    bytes.extend_from_slice(b"\0a\0");
    // One bucket that holds offset 1, then the name count.
    bytes.extend_from_slice(&words(&[1, 1, 1]));
    bytes
}

#[test]
fn dbi_accessors_decode_substreams() {
    let info = [module(b"m0"), module(b"m1")].concat();
    let contributions = words(&[
        SECTION_CONTRIBUTION_V60,
        pair(1, 0),
        0,
        4,
        0,
        pair(1, 0),
        0,
        0,
    ]);
    let map = words(&[pair(0, 0)]);
    let sources = source_info(1, &[0, 0], &[&[], &[0]], b"f.rs\0\0\0\0");
    let ec = string_table();
    let bytes = dbi([&info, &contributions, &map, &sources, &[], &ec, &[]]);
    let dbi = Dbi::parse(bytes).unwrap();

    let contributions = dbi.section_contributions().unwrap().unwrap();
    assert_eq!(contributions.iter().next().unwrap().module, ModuleIndex(1));
    assert!(dbi.section_map().unwrap().unwrap().is_empty());
    let sources = dbi.source_info().unwrap().unwrap();
    assert_eq!(sources.modules[1].name_offsets.len(), 1);
    assert_eq!(dbi.type_server_map().unwrap(), []);
    let names = dbi.ec_names().unwrap().unwrap();
    assert_eq!(names.find(b"a").unwrap(), Some(NameOffset(1)));
}

#[test]
fn dbi_accessors_return_none_for_empty_substreams() {
    let dbi = Dbi::parse(dbi([&[], &[], &[], &[], &[], &[], &[]])).unwrap();
    assert!(dbi.section_contributions().unwrap().is_none());
    assert!(dbi.section_map().unwrap().is_none());
    assert!(dbi.source_info().unwrap().is_none());
    assert!(dbi.ec_names().unwrap().is_none());
    assert_eq!(dbi.type_server_map().unwrap(), []);
}

#[test]
fn dbi_accessor_errors_have_substream_locations() {
    let info = module(b"m0");
    let info_len = u64::try_from(info.len()).unwrap();
    let contributions = words(&[SECTION_CONTRIBUTION_V60, 0]);
    let map = words(&[pair(1, 1)]);
    // The source info names two modules, but the module info has one.
    let sources = source_info(0, &[0, 0], &[&[], &[]], b"");
    let type_servers = [0_u8; 4];
    let ec = words(&[0]);
    let bytes = dbi([
        &info,
        &contributions,
        &map,
        &sources,
        &type_servers,
        &ec,
        &[],
    ]);
    let dbi = Dbi::parse(bytes).unwrap();
    let base = 64 + info_len;
    assert_located(dbi.section_contributions(), ErrorKind::InvalidFormat, base);
    let base = base + 8;
    assert_located(dbi.section_map(), ErrorKind::UnexpectedEof, base);
    let base = base + 4;
    assert_located(dbi.source_info(), ErrorKind::InvalidFormat, base);
    let base = base + 12;
    assert_located(dbi.type_server_map(), ErrorKind::UnexpectedEof, base);
    let base = base + 4;
    assert_located(dbi.ec_names(), ErrorKind::InvalidSignature, base);
}
