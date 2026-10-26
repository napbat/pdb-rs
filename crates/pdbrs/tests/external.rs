//! External dependencies: type servers, precompiled types and FASTLINK objects.

use pdbrs::native::Pdb;
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::dbi::{DbiBuilder, DbiHeader, OwnedModule, SectionContribution, TypeServer};
use pdbrs::native::hash_table::HashTableProducer;
use pdbrs::native::info::{Feature, InfoBuilder};
use pdbrs::native::msf::{MsfBuilder, StreamId, WriteOptions};

/// Append one symbol record.
fn record(out: &mut Vec<u8>, kind: u16, payload: &[u8]) {
    out.extend_from_slice(&u16::try_from(payload.len() + 2).unwrap().to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(payload);
}

/// A module stream with an `S_OBJNAME` signature and `fastlink` `S_FASTLINK` records.
fn module_stream(signature: u32, fastlink: usize) -> Vec<u8> {
    let mut symbols = 4_u32.to_le_bytes().to_vec();
    let mut payload = signature.to_le_bytes().to_vec();
    payload.extend_from_slice(b"a.obj\0");
    record(&mut symbols, 0x1101, &payload);
    for _ in 0..fastlink {
        record(&mut symbols, 0x1167, &[1, 0, 0, 0, 0, 0, b'f', 0]);
    }
    symbols
}

fn module(name: &[u8], stream: u32, symbol_bytes: usize, type_server: u8) -> OwnedModule {
    OwnedModule {
        unused1: 0,
        section_contribution: SectionContribution {
            section: 0,
            padding1: 0,
            offset: 0,
            size: 0,
            characteristics: 0,
            module: 0,
            padding2: 0,
            data_crc: Some(0),
            reloc_crc: Some(0),
            coff_section: None,
        },
        flags: u16::from(type_server) << 8,
        stream: Some(StreamId(stream)),
        symbol_bytes: u32::try_from(symbol_bytes).unwrap(),
        c11_bytes: 0,
        c13_bytes: 0,
        source_file_count: 0,
        padding: 0,
        file_name_offset: 0,
        source_file_name_index: 0,
        pdb_file_path_index: 0,
        name: name.to_vec(),
        object_file: b"lib.lib".to_vec(),
        name_padding: [0; 3],
    }
}

/// Build a PDB with three modules: two share a precompiled header, one has FASTLINK records.
fn pdb() -> Vec<u8> {
    let streams: Vec<Vec<u8>> = [
        module_stream(77, 0),
        module_stream(77, 0),
        module_stream(0, 3),
    ]
    .into_iter()
    .map(|mut symbols| {
        // No C11 or C13 lines, and no global references.
        symbols.extend_from_slice(&0_u32.to_le_bytes());
        symbols
    })
    .collect();
    let mut info = InfoBuilder::new(HashTableProducer::Microsoft, 1, [3; 16], 1).unwrap();
    info.set_features(&[Feature::Vc140, Feature::MinimalDebugInfo])
        .unwrap();
    let mut info_bytes = Vec::new();
    info.encode(&mut info_bytes).unwrap();
    let header = DbiHeader {
        age: 1,
        ..DbiHeader::default()
    };
    let mut dbi = DbiBuilder::new(header);
    for (index, (name, server)) in [(&b"pch.obj"[..], 0), (b"use.obj", 1), (b"fast.obj", 0)]
        .into_iter()
        .enumerate()
    {
        let stream = u32::try_from(index).unwrap() + 5;
        dbi.modules
            .push(module(name, stream, streams[index].len() - 4, server));
    }
    dbi.type_servers.push(TypeServer {
        reserved: 0,
        type_index_base: TypeIndex(0x0100_1000),
        signature: 9,
        age: 2,
        name: b"vc140.pdb".to_vec(),
        path: b"C:\\obj".to_vec(),
        padding: [0; 3],
    });
    let dbi_bytes = dbi.to_bytes().unwrap();
    let mut builder = MsfBuilder::new();
    builder.push(&[][..]).unwrap();
    builder.push(info_bytes.as_slice()).unwrap();
    builder.push_nil().unwrap();
    builder.push(dbi_bytes.as_slice()).unwrap();
    builder.push_nil().unwrap();
    for stream in &streams {
        builder.push(stream.as_slice()).unwrap();
    }
    builder
        .plan(WriteOptions::default())
        .unwrap()
        .to_vec()
        .unwrap()
}

#[test]
fn lists_type_servers_precompiled_types_and_fastlink_objects() {
    let bytes = pdb();
    let pdb = Pdb::parse(&bytes).unwrap();
    let dbi = pdb.debug_info().unwrap().unwrap();
    let dependencies = pdb.external_dependencies(&dbi).unwrap();
    assert!(dependencies.mini_pdb);
    assert!(!dependencies.is_empty());
    assert_eq!(dependencies.type_servers.len(), 1);
    assert_eq!(dependencies.type_servers[0].name, b"vc140.pdb");
    let modules: Vec<(u32, u8)> = dependencies
        .type_server_modules
        .iter()
        .map(|entry| (entry.module, entry.type_server))
        .collect();
    assert_eq!(modules, [(1, 1)]);
    assert_eq!(dependencies.precompiled.len(), 1);
    assert_eq!(dependencies.precompiled[0].signature, 77);
    assert_eq!(dependencies.precompiled[0].modules, [0, 1]);
    assert_eq!(dependencies.fastlink_objects.len(), 1);
    let object = &dependencies.fastlink_objects[0];
    assert_eq!(
        (
            object.module,
            object.references,
            object.module_name.as_slice()
        ),
        (2, 3, &b"fast.obj"[..])
    );
    assert_eq!(object.object_file, b"lib.lib");
}
