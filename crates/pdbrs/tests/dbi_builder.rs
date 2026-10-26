//! Tests for the type server map, the optional debug header, the encoder of
//! a whole DBI stream and the owned DBI builder.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::dbi::{
    Dbi, DbiBuilder, DbiHeader, DebugStream, OwnedModule, OwnedSourceInfo,
    SECTION_CONTRIBUTION_V60, SectionContribution, Substream, TypeServer, encode_debug_streams,
};
use pdbrs::native::msf::StreamId;
use pdbrs::native::names::{HashVersion, SIGNATURE, StringTableBuilder};

/// The header offsets of the substream sizes, in disk order.
const SIZE_FIELDS: [usize; 7] = [24, 28, 32, 36, 40, 52, 48];

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn pair(low: u16, high: u16) -> u32 {
    u32::from(low) | (u32::from(high) << 16)
}

fn assert_error<T: core::fmt::Debug>(result: pdbrs::Result<T>, kind: ErrorKind) {
    assert_eq!(result.unwrap_err().kind(), kind);
}

/// Build a modern DBI stream from substreams in disk order.
fn dbi(parts: [&[u8]; 7]) -> Vec<u8> {
    let mut bytes = words(&[0xffff_ffff, 19_990_903, 2, pair(1, 0x8e00), pair(0xffff, 1)]);
    bytes.extend_from_slice(&words(&[
        pair(4, 5),
        0,
        0,
        0,
        0,
        0,
        0x1234,
        0,
        0,
        pair(5, 0x14c),
        9,
    ]));
    for (offset, part) in SIZE_FIELDS.into_iter().zip(parts) {
        let size = i32::try_from(part.len()).unwrap().to_le_bytes();
        bytes[offset..offset + 4].copy_from_slice(&size);
        bytes.extend_from_slice(part);
    }
    bytes
}

/// Build one type server record with `padding` bytes after the path.
fn type_server(name: &[u8], path: &[u8], padding: u8) -> Vec<u8> {
    let mut bytes = words(&[0xdead_beef, 0x0100_1000, 0x1234_5678, 3]);
    bytes.extend_from_slice(name);
    bytes.push(0);
    bytes.extend_from_slice(path);
    bytes.push(0);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(padding);
    }
    bytes
}

#[test]
fn type_server_map_round_trip_with_padding() {
    // 16 + 4 + 1 = 21 bytes: three padding bytes. 16 + 2 + 1 = 19 bytes: one.
    let map = [type_server(b"t.p", b"", 0xcc), type_server(b"a", b"", 0)].concat();
    let servers = TypeServer::parse_map(&map).unwrap();
    assert_eq!(servers.len(), 2);
    let first = &servers[0];
    assert_eq!(first.reserved, 0xdead_beef);
    assert_eq!(first.type_index_base, TypeIndex(0x0100_1000));
    assert_eq!((first.signature, first.age), (0x1234_5678, 3));
    assert_eq!(
        (first.name.as_slice(), first.path.as_slice()),
        (&b"t.p"[..], &b""[..])
    );
    assert_eq!(first.padding, [0xcc, 0xcc, 0xcc]);
    assert_eq!(servers[1].padding, [0; 3]);
    let mut bytes = Vec::new();
    for server in &servers {
        server.encode(&mut bytes).unwrap();
    }
    assert_eq!(bytes, map);

    let stream = dbi([&[], &[], &[], &[], &map, &[], &[]]);
    let decoded = Dbi::parse(stream.clone()).unwrap();
    assert_eq!(decoded.type_server_map().unwrap(), servers);
    let mut again = Vec::new();
    decoded.encode(&mut again).unwrap();
    assert_eq!(again, stream);
}

#[test]
fn type_server_map_truncation() {
    let map = type_server(b"t.pdb", b"p", 0);
    for end in 1..map.len() {
        assert_error(TypeServer::parse_map(&map[..end]), ErrorKind::UnexpectedEof);
    }
    let stream = dbi([&[], &[], &[], &[], &map[..20], &[], &[]]);
    let error = Dbi::parse(stream).unwrap().type_server_map().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(error.stream_location(), Some((StreamId(3), 64)));
}

#[test]
fn type_server_encode_rejects_nul_in_names() {
    let mut server = TypeServer::parse_map(&type_server(b"t", b"p", 0))
        .unwrap()
        .remove(0);
    server.path = b"a\0".to_vec();
    assert_error(server.encode(&mut Vec::new()), ErrorKind::InvalidFormat);
}

#[test]
fn debug_header_keeps_unknown_slots() {
    let mut slots = Vec::new();
    for value in [
        1_u16, 0xffff, 2, 3, 4, 5, 6, 7, 8, 9, 10, 0x20, 0xffff, 0x21,
    ] {
        slots.extend_from_slice(&value.to_le_bytes());
    }
    let stream = dbi([&[], &[], &[], &[], &[], &[], &slots]);
    let decoded = Dbi::parse(stream.clone()).unwrap();
    assert_eq!(decoded.debug_streams().len(), 14);
    assert_eq!(
        decoded.debug_stream(DebugStream::OriginalSectionHeader),
        Some(StreamId(10))
    );
    assert_eq!(decoded.debug_streams().nth(11), Some(Some(StreamId(0x20))));
    let mut bytes = Vec::new();
    encode_debug_streams(decoded.debug_streams(), &mut bytes).unwrap();
    assert_eq!(bytes, slots);
    let mut again = Vec::new();
    decoded.encode(&mut again).unwrap();
    assert_eq!(again, stream);
    assert_error(
        encode_debug_streams([Some(StreamId(0xffff))], &mut Vec::new()),
        ErrorKind::LimitExceeded,
    );
}

fn module_record(name: &[u8]) -> Vec<u8> {
    let mut bytes = words(&[0, pair(1, 0), 0, 4, 0x20, pair(0, 0), 7, 8, pair(1, 0xffff)]);
    bytes.extend_from_slice(&words(&[0, 0, 0, pair(1, 0), 0, 1, 0]));
    bytes.extend_from_slice(name);
    bytes.push(0);
    bytes.extend_from_slice(name);
    bytes.push(0);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
}

/// Build a DBI stream with all seven substreams.
fn full_stream() -> Vec<u8> {
    let modules = [module_record(b"m0"), module_record(b"m1.obj")].concat();
    let contributions = words(&[
        SECTION_CONTRIBUTION_V60,
        pair(1, 0),
        0,
        4,
        0x20,
        pair(1, 0),
        0,
        0,
    ]);
    let map = words(&[
        pair(1, 1),
        pair(0x10d, 0),
        pair(0, 1),
        pair(0xffff, 0xffff),
        0,
        4,
    ]);
    let mut sources = words(&[pair(2, 1), pair(0, 0), pair(0, 1), 0]);
    sources.extend_from_slice(b"f.c\0");
    let mut ec = words(&[SIGNATURE, 1, 3]);
    ec.extend_from_slice(b"\0a\0");
    ec.extend_from_slice(&words(&[1, 1, 1]));
    let tsm = [0_u8; 20];
    let debug = [0xff, 0xff, 3, 0];
    dbi([&modules, &contributions, &map, &sources, &tsm, &ec, &debug])
}

#[test]
fn whole_stream_and_each_substream_round_trip() {
    let stream = full_stream();
    let decoded = Dbi::parse(stream.clone()).unwrap();
    let mut bytes = Vec::new();
    decoded.encode(&mut bytes).unwrap();
    assert_eq!(bytes, stream);
    for kind in Substream::ALL {
        let mut part = Vec::new();
        decoded.encode_substream(kind, &mut part).unwrap();
        assert_eq!(part, decoded.substream(kind), "{kind:?}");
    }
    let builder = DbiBuilder::from_dbi(&decoded).unwrap();
    assert_eq!(builder.to_bytes().unwrap(), stream);
    assert_eq!(builder.header.reserved, 9);
    assert_eq!(builder.header.mfc_type_server_index, 0x1234);
    assert_eq!(builder.type_servers.len(), 1);
}

#[test]
fn empty_substreams_round_trip() {
    let stream = dbi([&[]; 7]);
    let decoded = Dbi::parse(stream.clone()).unwrap();
    let mut bytes = Vec::new();
    decoded.encode(&mut bytes).unwrap();
    assert_eq!(bytes, stream);
    assert_eq!(
        DbiBuilder::from_dbi(&decoded).unwrap().to_bytes().unwrap(),
        stream
    );
}

#[test]
fn encode_errors_remove_appended_bytes() {
    // The section contribution substream is not a whole number of records.
    let stream = dbi([&[], &[1, 2, 3, 4, 5, 6, 7, 8], &[], &[], &[], &[], &[]]);
    let decoded = Dbi::parse(stream).unwrap();
    let mut bytes = vec![7];
    let error = decoded.encode(&mut bytes).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(bytes, [7]);
}

fn owned_module(name: &[u8]) -> OwnedModule {
    OwnedModule {
        unused1: 0,
        section_contribution: SectionContribution {
            section: 1,
            padding1: 0,
            offset: 0,
            size: 4,
            characteristics: 0x6000_0020,
            module: 0,
            padding2: 0,
            data_crc: Some(0),
            reloc_crc: Some(0),
            coff_section: None,
        },
        flags: 0,
        stream: Some(StreamId(12)),
        symbol_bytes: 4,
        c11_bytes: 0,
        c13_bytes: 0,
        source_file_count: 1,
        padding: 0,
        file_name_offset: 0,
        source_file_name_index: 0,
        pdb_file_path_index: 0,
        name: name.to_vec(),
        object_file: name.to_vec(),
        name_padding: [0; 3],
    }
}

#[test]
fn builder_fills_a_new_stream() {
    let header = DbiHeader {
        age: 1,
        global_stream: Some(StreamId(7)),
        machine: 0x8664,
        ..DbiHeader::default()
    };
    let mut builder = DbiBuilder::new(header);
    let mut names = StringTableBuilder::new(HashVersion::V1).unwrap();
    let mut module = owned_module(b"a.obj");
    module.source_file_name_index = names.insert(b"a.c").unwrap();
    builder.modules.push(module);
    builder.ec_names = Some(names);
    builder.source_info = Some(OwnedSourceInfo::from_file_names([[&b"a.c"[..]]]).unwrap());
    builder.debug_streams = vec![None; 11];
    let bytes = builder.to_bytes().unwrap();

    let decoded = Dbi::parse(bytes.clone()).unwrap();
    assert_eq!(*decoded.header(), header);
    // V60 without records keeps its version value.
    assert_eq!(decoded.substream(Substream::SectionContributions).len(), 4);
    let module = decoded.modules().next().unwrap().unwrap();
    let ec = decoded.ec_names().unwrap().unwrap();
    assert_eq!(ec.get(module.source_file_name_index).unwrap(), b"a.c");
    assert_eq!(decoded.debug_streams().len(), 11);
    assert_eq!(DbiBuilder::from_dbi(&decoded).unwrap(), builder);
}

#[test]
fn builder_checks_widths_and_counts() {
    let mut builder = DbiBuilder::new(DbiHeader::default());
    builder.modules = vec![owned_module(b""); 65_536];
    assert_error(builder.to_bytes(), ErrorKind::LimitExceeded);
    builder.modules.truncate(2);
    builder.to_bytes().unwrap();

    builder.source_info = Some(OwnedSourceInfo::from_file_names([[&b"x"[..]]]).unwrap());
    assert_error(builder.to_bytes(), ErrorKind::InvalidFormat);
    builder.source_info = None;

    builder.header.symbol_stream = Some(StreamId(0x1_0000));
    assert_error(builder.to_bytes(), ErrorKind::LimitExceeded);
    builder.header.symbol_stream = None;
    builder.debug_streams = vec![Some(StreamId(0xffff))];
    let mut bytes = vec![1];
    assert_error(builder.encode(&mut bytes), ErrorKind::LimitExceeded);
    assert_eq!(bytes, [1]);
}
