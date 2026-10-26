//! Tests for the encoders of DBI records: the header, module info records,
//! section contributions, the section map and the source file info.

use pdbrs::ErrorKind;
use pdbrs::native::dbi::{
    Dbi, DbiBuilder, DbiHeader, Module, OwnedModule, OwnedModuleSources, OwnedSectionMap,
    OwnedSourceInfo, SECTION_CONTRIBUTION_V2, SECTION_CONTRIBUTION_V60, SectionContribution,
    SectionContributionVersion, SectionContributions, SectionMap, SectionMapEntry, SegmentFlags,
    SourceInfo, Substream,
};
use pdbrs::native::msf::StreamId;

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

fn assert_error<T: core::fmt::Debug>(result: pdbrs::Result<T>, kind: ErrorKind) {
    assert_eq!(result.unwrap_err().kind(), kind);
}

fn header() -> DbiHeader {
    DbiHeader {
        age: 7,
        global_stream: Some(StreamId(0x10)),
        build_number: 0x8e33,
        public_stream: None,
        pdb_dll_version: 0x8da0,
        symbol_stream: Some(StreamId(0x12)),
        pdb_dll_rebuild: 3,
        mfc_type_server_index: 0x0102_0304,
        flags: 0xfff5,
        machine: 0x8664,
        reserved: 0xa5a5_a5a5,
    }
}

#[test]
fn header_encodes_all_fields() {
    let mut bytes = Vec::new();
    header().encode(&[1, 2, 3, 4, 5, 6, 8], &mut bytes).unwrap();
    let mut expected = words(&[0xffff_ffff, 19_990_903, 7]);
    expected.extend_from_slice(&words(&[
        pair(0x10, 0x8e33),
        pair(0xffff, 0x8da0),
        pair(0x12, 3),
        1,
        2,
        3,
        4,
        5,
        0x0102_0304,
        8,
        6,
        pair(0xfff5, 0x8664),
        0xa5a5_a5a5,
    ]));
    assert_eq!(bytes, expected);
}

#[test]
fn header_flag_and_build_accessors() {
    let mut value = header();
    assert!(value.incrementally_linked());
    assert!(!value.stripped());
    assert!(value.has_c_types());
    assert!(value.new_version_format());
    assert_eq!((value.build_major(), value.build_minor()), (14, 0x33));
    value.flags = 2;
    value.build_number = 0x0e00;
    assert!(!value.incrementally_linked());
    assert!(value.stripped());
    assert!(!value.has_c_types());
    assert!(!value.new_version_format());
    assert_eq!((value.build_major(), value.build_minor()), (14, 0));
}

#[test]
fn header_rejects_values_that_do_not_fit() {
    let mut bytes = vec![9];
    let too_large = usize::try_from(i32::MAX).unwrap() + 1;
    let mut sizes = [0; 7];
    sizes[Substream::Ec as usize] = too_large;
    assert_error(
        header().encode(&sizes, &mut bytes),
        ErrorKind::LimitExceeded,
    );
    for stream in [0xffff, 0x1_0000] {
        let mut value = header();
        value.public_stream = Some(StreamId(stream));
        assert_error(value.encode(&[0; 7], &mut bytes), ErrorKind::LimitExceeded);
    }
    // A failed encode does not change the output.
    assert_eq!(bytes, [9]);
}

/// Build one module info record with non-zero values in all fields.
fn module_record(name: &[u8], object_file: &[u8], padding: u8) -> Vec<u8> {
    let mut bytes = words(&[
        0x1111_1111,
        pair(2, 0xaaaa),
        0x10,
        0x20,
        0x6000_0020,
        pair(7, 0xbbbb),
        1,
        2,
        pair(0x0303, 9),
        4,
        0,
        8,
        pair(3, 0xcccc),
        0xdddd_dddd,
        5,
        6,
    ]);
    bytes.extend_from_slice(name);
    bytes.push(0);
    bytes.extend_from_slice(object_file);
    bytes.push(0);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(padding);
    }
    bytes
}

/// Build a DBI stream with only a module info substream.
fn module_dbi(bytes: &[u8]) -> Dbi {
    let mut dbi = header_bytes(bytes.len());
    dbi.extend_from_slice(bytes);
    Dbi::parse(dbi).unwrap()
}

fn modules(dbi: &Dbi) -> Vec<Module<'_>> {
    dbi.modules().collect::<pdbrs::Result<_>>().unwrap()
}

/// Build a DBI header for a stream with only a module info substream.
fn header_bytes(module_info: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    header()
        .encode(&[module_info, 0, 0, 0, 0, 0, 0], &mut bytes)
        .unwrap();
    bytes
}

#[test]
fn module_records_decode_and_encode_all_fields() {
    let record = module_record(b"a.obj", b"lib", 0xee);
    let dbi = module_dbi(&record);
    let decoded = modules(&dbi);
    let module = decoded[0];
    assert_eq!(module.unused1, 0x1111_1111);
    let contribution = module.section_contribution;
    assert_eq!(
        (
            contribution.section,
            contribution.padding1,
            contribution.module
        ),
        (2, 0xaaaa, 7)
    );
    assert_eq!(
        (contribution.data_crc, contribution.reloc_crc),
        (Some(1), Some(2))
    );
    assert_eq!(contribution.coff_section, None);
    assert_eq!(module.flags, 0x0303);
    assert!(module.written());
    assert!(module.ec_enabled());
    assert_eq!(module.type_server_index(), 3);
    assert_eq!(module.stream, Some(StreamId(9)));
    assert_eq!(module.source_file_count, 3);
    assert_eq!(module.padding, 0xcccc);
    assert_eq!(module.file_name_offset, 0xdddd_dddd);
    assert_eq!(module.source_file_name_index, 5);
    assert_eq!(module.pdb_file_path_index, 6);
    // 64 + 6 + 4 = 74 bytes, so two padding bytes follow the names.
    assert_eq!(module.name_padding, [0xee, 0xee, 0]);

    let mut bytes = Vec::new();
    module.encode(&mut bytes).unwrap();
    assert_eq!(bytes, record);
    let owned = OwnedModule::from_module(&module).unwrap();
    let mut bytes = Vec::new();
    owned.encode(&mut bytes).unwrap();
    assert_eq!(bytes, record);
    assert_eq!(owned.as_module().name, b"a.obj");
}

#[test]
fn module_records_keep_each_padding_length() {
    for name_length in 0..4 {
        let name = vec![b'n'; name_length];
        let record = module_record(&name, b"", 0x5a);
        let mut all = record.clone();
        all.extend_from_slice(&record);
        let dbi = module_dbi(&all);
        let decoded = modules(&dbi);
        assert_eq!(decoded.len(), 2);
        let mut bytes = Vec::new();
        for module in decoded {
            module.encode(&mut bytes).unwrap();
        }
        assert_eq!(bytes, all);
    }
}

#[test]
fn module_encode_rejects_bad_values() {
    let record = module_record(b"m", b"o", 0);
    let dbi = module_dbi(&record);
    let mut owned = OwnedModule::from_module(&modules(&dbi)[0]).unwrap();
    let mut bytes = Vec::new();
    owned.name = b"a\0b".to_vec();
    assert_error(owned.encode(&mut bytes), ErrorKind::InvalidFormat);
    owned.name = b"m".to_vec();
    owned.stream = Some(StreamId(0xffff));
    assert_error(owned.encode(&mut bytes), ErrorKind::LimitExceeded);
    owned.stream = None;
    owned.section_contribution.data_crc = None;
    assert_error(owned.encode(&mut bytes), ErrorKind::InvalidFormat);
    owned.section_contribution.data_crc = Some(0);
    owned.section_contribution.coff_section = Some(1);
    assert_error(owned.encode(&mut bytes), ErrorKind::InvalidFormat);
}

fn contribution(version: SectionContributionVersion) -> SectionContribution {
    let crc = version != SectionContributionVersion::V40;
    SectionContribution {
        section: 1,
        padding1: 0xcccc,
        offset: -4,
        size: 16,
        characteristics: 0x6000_0020,
        module: 2,
        padding2: 0xdddd,
        data_crc: crc.then_some(7),
        reloc_crc: crc.then_some(8),
        coff_section: (version == SectionContributionVersion::V2).then_some(9),
    }
}

#[test]
fn section_contributions_round_trip_in_each_version() {
    let cases = [
        (SectionContributionVersion::V40, Vec::new(), 20),
        (
            SectionContributionVersion::V60,
            words(&[SECTION_CONTRIBUTION_V60]),
            28,
        ),
        (
            SectionContributionVersion::V2,
            words(&[SECTION_CONTRIBUTION_V2]),
            32,
        ),
    ];
    for (version, mut expected, size) in cases {
        assert_eq!(version.record_size(), size);
        let record = contribution(version);
        let mut bytes = Vec::new();
        record.encode(version, &mut bytes).unwrap();
        assert_eq!(bytes.len(), size);
        expected.extend_from_slice(&bytes);
        expected.extend_from_slice(&bytes);
        let parsed = SectionContributions::parse(&expected).unwrap();
        assert_eq!(parsed.version(), version);
        assert_eq!(parsed.iter().next(), Some(record));
        let mut again = Vec::new();
        parsed.encode(&mut again).unwrap();
        assert_eq!(again, expected);
    }
}

#[test]
fn section_contribution_encode_errors() {
    let mut bytes = Vec::new();
    let v60 = contribution(SectionContributionVersion::V60);
    for version in [
        SectionContributionVersion::V40,
        SectionContributionVersion::V2,
    ] {
        assert_error(v60.encode(version, &mut bytes), ErrorKind::InvalidFormat);
    }
    let mut v2 = contribution(SectionContributionVersion::V2);
    v2.reloc_crc = None;
    assert_error(
        v2.encode(SectionContributionVersion::V2, &mut bytes),
        ErrorKind::InvalidFormat,
    );
    assert_eq!(bytes, []);
}

#[test]
fn v40_substream_cannot_start_with_a_version_value() {
    // The first word of this V40 record equals the V60 version value. A
    // reader would then select the V60 layout, so the encoder rejects it.
    let mut record = contribution(SectionContributionVersion::V40);
    record.section = 0xba2d;
    record.padding1 = 0xf12e;
    assert_eq!(pair(0xba2d, 0xf12e), SECTION_CONTRIBUTION_V60);
    let mut builder = DbiBuilder::new(header());
    builder.section_contribution_version = SectionContributionVersion::V40;
    builder.section_contributions = vec![contribution(SectionContributionVersion::V40), record];
    // A version value in the second record is not a problem.
    builder.to_bytes().unwrap();
    builder.section_contributions.reverse();
    assert_error(builder.to_bytes(), ErrorKind::InvalidFormat);
}

fn map_entry(frame: u16) -> SectionMapEntry {
    SectionMapEntry {
        flags: SegmentFlags(0x010d),
        overlay: 1,
        group: 2,
        frame,
        section_name: 0xffff,
        class_name: 0xfffe,
        offset: 0x10,
        length: 0x2000,
    }
}

#[test]
fn section_map_round_trip() {
    let mut bytes = words(&[pair(2, 1)]);
    for frame in [1, 2] {
        map_entry(frame).encode(&mut bytes).unwrap();
    }
    assert_eq!(bytes.len(), 44);
    let map = SectionMap::parse(&bytes).unwrap();
    assert_eq!(map.iter().nth(1), Some(map_entry(2)));
    let mut again = Vec::new();
    map.encode(&mut again).unwrap();
    assert_eq!(again, bytes);
    let owned = OwnedSectionMap::from_map(&map).unwrap();
    assert_eq!(owned.logical_count, 1);
    let mut again = Vec::new();
    owned.encode(&mut again).unwrap();
    assert_eq!(again, bytes);
}

#[test]
fn section_map_count_overflow() {
    let map = OwnedSectionMap {
        logical_count: 0,
        entries: vec![SectionMapEntry::default(); 65_536],
    };
    let mut bytes = Vec::new();
    assert_error(map.encode(&mut bytes), ErrorKind::LimitExceeded);
    let map = OwnedSectionMap {
        logical_count: 0,
        entries: vec![SectionMapEntry::default(); 65_535],
    };
    map.encode(&mut bytes).unwrap();
    assert_eq!(bytes.len(), 4 + 65_535 * 20);
}

#[test]
fn source_info_round_trip_keeps_raw_values() {
    // The raw file count and the start indexes do not match the real values.
    let mut bytes = words(&[pair(2, 0x7777), pair(0x1234, 0x5678), pair(1, 2), 0, 2, 0]);
    bytes.extend_from_slice(b"a\0b\0\xcc\xcc");
    let info = SourceInfo::parse(&bytes).unwrap();
    let mut again = Vec::new();
    info.encode(&mut again).unwrap();
    assert_eq!(again, bytes);
    let owned = OwnedSourceInfo::from_info(&info).unwrap();
    assert_eq!(owned.raw_file_count, 0x7777);
    assert_eq!(owned.modules[1].raw_start, 0x5678);
    assert_eq!(owned.modules[1].name_offsets, [2, 0]);
    let mut again = Vec::new();
    owned.encode(&mut again).unwrap();
    assert_eq!(again, bytes);
}

#[test]
fn source_info_from_file_names() {
    let modules: [&[&[u8]]; 3] = [&[b"a.c", b"x.h"], &[], &[b"x.h", b"b.c"]];
    let info = OwnedSourceInfo::from_file_names(modules.iter().map(|files| files.iter().copied()))
        .unwrap();
    assert_eq!(info.raw_file_count, 4);
    assert_eq!(
        info.modules.iter().map(|m| m.raw_start).collect::<Vec<_>>(),
        [0, 2, 2]
    );
    assert_eq!(info.names, b"a.c\0x.h\0b.c\0");
    assert_eq!(info.modules[2].name_offsets, [4, 8]);
    let mut bytes = Vec::new();
    info.encode(&mut bytes).unwrap();
    assert!(bytes.len().is_multiple_of(4));
    let parsed = SourceInfo::parse(&bytes).unwrap();
    let names: Vec<Vec<&[u8]>> = parsed
        .modules()
        .map(|module| module.file_names().map(Result::unwrap).collect())
        .collect();
    assert_eq!(
        names,
        [vec![&b"a.c"[..], b"x.h"], vec![], vec![b"x.h", b"b.c"]]
    );

    // Names that need padding get zero bytes after the last name.
    let info = OwnedSourceInfo::from_file_names([[&b"ab"[..]]]).unwrap();
    assert_eq!(info.names, b"ab\0\0");
    assert_error(
        OwnedSourceInfo::from_file_names([[&b"a\0b"[..]]]),
        ErrorKind::InvalidFormat,
    );
}

#[test]
fn source_info_16_bit_overflow() {
    // 70000 file entries: the raw values keep the low 16 bits.
    let files: Vec<&[u8]> = vec![b"f"; 35_000];
    let info = OwnedSourceInfo::from_file_names([files.clone(), files]).unwrap();
    assert_eq!(u32::from(info.raw_file_count), 70_000 - 65_536);
    assert_eq!(info.modules[1].raw_start, 35_000);
    let mut bytes = Vec::new();
    info.encode(&mut bytes).unwrap();
    assert_eq!(SourceInfo::parse(&bytes).unwrap().file_count(), 70_000);

    let wide = OwnedSourceInfo {
        raw_file_count: 0,
        modules: vec![OwnedModuleSources {
            raw_start: 0,
            name_offsets: vec![0; 65_536],
        }],
        names: vec![0; 4],
    };
    assert_error(wide.encode(&mut Vec::new()), ErrorKind::LimitExceeded);
    let many = OwnedSourceInfo {
        raw_file_count: 0,
        modules: vec![OwnedModuleSources::default(); 65_536],
        names: Vec::new(),
    };
    assert_error(many.encode(&mut Vec::new()), ErrorKind::LimitExceeded);
}
