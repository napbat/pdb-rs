//! The subsection kinds of object files and managed code.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::index::{NameOffset, TypeIndex};
use pdbrs::native::fpo::FrameData;
use pdbrs::native::lines::{
    AssemblyInput, CrossScopeExport, CrossScopeExports, CrossScopeImport, CrossScopeImports,
    DEBUG_S_CROSSSCOPEEXPORTS, DEBUG_S_CROSSSCOPEIMPORTS, DEBUG_S_FRAMEDATA,
    DEBUG_S_FUNC_MDTOKEN_MAP, DEBUG_S_IGNORE, DEBUG_S_MERGED_ASSEMBLYINPUT, DEBUG_S_STRINGTABLE,
    DEBUG_S_SYMBOLS, DEBUG_S_TYPE_MDTOKEN_MAP, DEBUG_S_XFGHASH_TYPE, DEBUG_S_XFGHASH_VIRTUAL,
    FrameDataSubsection, LineSubsection, MergedAssemblyInput, StringTableSubsection,
    SubsectionKind, SymbolsSubsection, TokenMap, TokenMapEntry, TokenMapValue, XfgTypeHashes,
    XfgVirtualHashes,
};

use crate::{error_kind, round_trip, u32s};

/// Return a cross-scope export with the given IDs.
fn export(local_id: u32, global_id: u32) -> CrossScopeExport {
    let mut export = CrossScopeExport::default();
    export.local_id = local_id;
    export.global_id = global_id;
    export
}

/// Return a token map entry with the given fields.
fn token_entry(key: u32, value: TokenMapValue) -> TokenMapEntry {
    let mut entry = TokenMapEntry::default();
    entry.key = key;
    entry.value = value;
    entry
}

/// Return a merged assembly input entry with the given fields.
fn assembly_input<'a>(
    timestamp: u32,
    index: u32,
    has_pdb: bool,
    version: &'a [u8],
    name: &'a [u8],
) -> AssemblyInput<'a> {
    let mut input = AssemblyInput::default();
    input.timestamp = timestamp;
    input.index = index;
    input.has_pdb = has_pdb;
    input.version = version;
    input.name = name;
    input
}

#[test]
fn symbols_round_trip_and_reject_truncated_records() {
    // An `S_COMPILE3`-sized record and an `S_END` record.
    let mut data = vec![0x06, 0x00, 0x3c, 0x11, 1, 2, 3, 4];
    data.extend_from_slice(&[0x02, 0x00, 0x06, 0x00]);
    let LineSubsection::Symbols(symbols) = round_trip(DEBUG_S_SYMBOLS, &data) else {
        panic!("expected DEBUG_S_SYMBOLS");
    };
    assert_eq!(symbols.len(), 2);
    let kinds: Vec<_> = symbols
        .records()
        .map(|record| record.unwrap().kind)
        .collect();
    assert_eq!(kinds, [0x113c, 0x0006]);
    round_trip(DEBUG_S_SYMBOLS, &[]);

    assert_eq!(
        error_kind(SymbolsSubsection::parse(&data[..10])),
        ErrorKind::UnexpectedEof
    );
    assert_eq!(
        error_kind(SymbolsSubsection::parse(&[0x01, 0x00, 0x06, 0x00])),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn string_tables_round_trip_and_find_strings() {
    let mut data = Vec::new();
    for string in [&b""[..], b"a.cpp", b"dir\\b.h"] {
        StringTableSubsection::encode_string(string, &mut data).unwrap();
    }
    // Zero padding at the end gives empty strings.
    data.extend_from_slice(&[0, 0]);
    let LineSubsection::StringTable(table) = round_trip(DEBUG_S_STRINGTABLE, &data) else {
        panic!("expected DEBUG_S_STRINGTABLE");
    };
    assert_eq!(table.get(1), Some(&b"a.cpp"[..]));
    assert_eq!(table.get(3), Some(&b"cpp"[..]));
    assert_eq!(table.get(u32::try_from(data.len()).unwrap()), None);
    let strings: Vec<_> = table.iter().collect();
    assert_eq!(strings[2], (7, &b"dir\\b.h"[..]));
    assert_eq!(strings.len(), 5);
    round_trip(DEBUG_S_STRINGTABLE, &[]);

    assert_eq!(
        error_kind(StringTableSubsection::parse(b"\0abc")),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        error_kind(StringTableSubsection::encode_string(
            b"a\0b",
            &mut Vec::new()
        )),
        ErrorKind::InvalidFormat
    );
}

/// Serialize one `FRAMEDATA` record with distinct field values.
fn frame_record(start: u32) -> Vec<u8> {
    let mut bytes = u32s(&[start, 0x20, 4, 8, 0x40, 0x99]);
    bytes.extend_from_slice(&3_u16.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&5_u32.to_le_bytes());
    bytes
}

#[test]
fn frame_data_round_trips_with_and_without_relocation() {
    let records = [frame_record(0x1000), frame_record(0x1020)].concat();
    let with = [u32s(&[0x1000]), records.clone()].concat();
    let LineSubsection::FrameData(frames) = round_trip(DEBUG_S_FRAMEDATA, &with) else {
        panic!("expected DEBUG_S_FRAMEDATA");
    };
    assert_eq!(frames.relocation(), Some(0x1000));
    let first: FrameData = frames.records().get(0).unwrap();
    assert_eq!(
        (first.start, first.prolog_size, first.flags),
        (0x1000, 3, 5)
    );
    assert!(first.has_seh() && first.is_function_start());

    let LineSubsection::FrameData(frames) = round_trip(DEBUG_S_FRAMEDATA, &records) else {
        panic!("expected DEBUG_S_FRAMEDATA");
    };
    assert_eq!(frames.relocation(), None);
    assert_eq!(frames.records().len(), 2);

    let decoded: Vec<_> = frames.records().iter().collect();
    let mut out = Vec::new();
    FrameDataSubsection::encode_parts(Some(0x1000), &decoded, &mut out).unwrap();
    assert_eq!(out, with);

    for size in [1, 8, 33, 37] {
        assert_eq!(
            error_kind(FrameDataSubsection::parse(&vec![0; size])),
            ErrorKind::InvalidFormat,
            "{size} bytes"
        );
    }
}

#[test]
fn cross_scope_subsections_round_trip() {
    let mut imports = Vec::new();
    CrossScopeImport::encode_parts(NameOffset(0x10), &[0x1001, 0x8000_1002], &mut imports).unwrap();
    CrossScopeImport::encode_parts(NameOffset(0x20), &[], &mut imports).unwrap();
    let LineSubsection::CrossScopeImports(decoded) =
        round_trip(DEBUG_S_CROSSSCOPEIMPORTS, &imports)
    else {
        panic!("expected DEBUG_S_CROSSSCOPEIMPORTS");
    };
    let entries: Vec<_> = decoded.iter().collect();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].module_name_offset, NameOffset(0x10));
    assert_eq!(
        entries[0].ids.iter().collect::<Vec<_>>(),
        [0x1001, 0x8000_1002]
    );
    assert!(entries[1].ids.is_empty());
    for bad in [&imports[..4], &imports[..12]] {
        assert_eq!(
            error_kind(CrossScopeImports::parse(bad)),
            ErrorKind::UnexpectedEof
        );
    }

    let mut exports = Vec::new();
    export(1, 0x1003).encode(&mut exports).unwrap();
    export(2, 0x1004).encode(&mut exports).unwrap();
    let LineSubsection::CrossScopeExports(decoded) =
        round_trip(DEBUG_S_CROSSSCOPEEXPORTS, &exports)
    else {
        panic!("expected DEBUG_S_CROSSSCOPEEXPORTS");
    };
    assert_eq!(decoded.get(1), Some(export(2, 0x1004)));
    assert_eq!(
        error_kind(CrossScopeExports::parse(&exports[..12])),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn token_maps_round_trip_and_check_counts() {
    let entries = [
        token_entry(0x1004, TokenMapValue::Token(0x0600_0001)),
        token_entry(0x1005, TokenMapValue::Offset(0x40)),
    ];
    let mut data = Vec::new();
    TokenMap::encode_parts(&entries, &mut data).unwrap();
    assert_eq!(data, u32s(&[2, 0x1004, 0x8600_0001, 0x1005, 0x40]));
    for kind in [DEBUG_S_FUNC_MDTOKEN_MAP, DEBUG_S_TYPE_MDTOKEN_MAP] {
        let (LineSubsection::FunctionTokenMap(map) | LineSubsection::TypeTokenMap(map)) =
            round_trip(kind, &data)
        else {
            panic!("expected a token map");
        };
        assert_eq!(map.entries().iter().collect::<Vec<_>>(), entries);
        assert_eq!(
            map.entries().get(0).unwrap().type_index(),
            TypeIndex(0x1004)
        );
    }
    assert_eq!(
        error_kind(TokenMap::parse(&[1, 0])),
        ErrorKind::UnexpectedEof
    );
    assert_eq!(
        error_kind(TokenMap::parse(&data[..16])),
        ErrorKind::UnexpectedEof
    );
    let long = [data.clone(), vec![0; 4]].concat();
    assert_eq!(error_kind(TokenMap::parse(&long)), ErrorKind::InvalidFormat);
    let wide = [token_entry(1, TokenMapValue::Token(0x8000_0000))];
    assert_eq!(
        error_kind(TokenMap::encode_parts(&wide, &mut Vec::new())),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn merged_assembly_input_round_trips_and_checks_padding() {
    let mut data = Vec::new();
    assembly_input(0x5f00_0000, 3, true, &[1, 0, 0, 0], b"lib.dll")
        .encode_padded(&mut data)
        .unwrap();
    assembly_input(0, 4, false, &[], b"b")
        .encode(&mut data)
        .unwrap();
    // 8 + 6 + 8 = 22 bytes, padded to 24, then 8 + 2 + 2 bytes without padding.
    assert_eq!(data.len(), 24 + 12);
    let LineSubsection::MergedAssemblyInput(inputs) =
        round_trip(DEBUG_S_MERGED_ASSEMBLYINPUT, &data)
    else {
        panic!("expected DEBUG_S_MERGED_ASSEMBLYINPUT");
    };
    let entries: Vec<_> = inputs.iter().collect();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, b"lib.dll");
    assert_eq!(entries[0].version, [1, 0, 0, 0]);
    assert!(entries[0].has_pdb && !entries[1].has_pdb);
    assert_eq!(entries[1].index, 4);

    let mut bad_padding = data.clone();
    bad_padding[23] = 1;
    assert_eq!(
        error_kind(MergedAssemblyInput::parse(&bad_padding)),
        ErrorKind::InvalidFormat
    );
    let mut small_version = data.clone();
    small_version[8] = 1;
    assert_eq!(
        error_kind(MergedAssemblyInput::parse(&small_version)),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        error_kind(MergedAssemblyInput::parse(&data[..21])),
        ErrorKind::UnexpectedEof
    );
    assert_eq!(
        error_kind(MergedAssemblyInput::parse(&data[..23])),
        ErrorKind::UnexpectedEof
    );
    let wide = assembly_input(0, 0x8000_0000, false, &[], b"x");
    assert_eq!(
        error_kind(wide.encode(&mut Vec::new())),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn xfg_hashes_round_trip_producer_bytes() {
    // The first entries of `types.obj` of the `msvc-x64-xfg` fixture (#27).
    let types = [
        0x70, 0x62, 0xda, 0x74, 0x56, 0x3e, 0xb9, 0xcb, 0x08, 0x10, 0x00, 0x00, //
        0x70, 0x3a, 0xde, 0x50, 0x39, 0x37, 0x98, 0x8c, 0x28, 0x12, 0x00, 0x00,
    ];
    let LineSubsection::XfgTypeHashes(hashes) = round_trip(DEBUG_S_XFGHASH_TYPE, &types) else {
        panic!("expected DEBUG_S_XFGHASH_TYPE");
    };
    let first = hashes.get(0).unwrap();
    assert_eq!(first.hash, 0xcbb9_3e56_74da_6270);
    assert_eq!(first.function_type, TypeIndex(0x1008));
    let virtuals = [
        0x70, 0x93, 0x52, 0x3c, 0xc1, 0xa6, 0x5d, 0xc1, 0x00, 0x10, 0x00, 0x00, 0x10, 0x00, 0x00,
        0x00,
    ];
    let LineSubsection::XfgVirtualHashes(hashes) = round_trip(DEBUG_S_XFGHASH_VIRTUAL, &virtuals)
    else {
        panic!("expected DEBUG_S_XFGHASH_VIRTUAL");
    };
    let entry = hashes.get(0).unwrap();
    assert_eq!((entry.class, entry.offset), (TypeIndex(0x1000), 0x10));
    assert_eq!(
        error_kind(XfgTypeHashes::parse(&types[..8])),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        error_kind(XfgVirtualHashes::parse(&virtuals[..12])),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn ignored_subsections_keep_their_bytes() {
    let data = [9, 8, 7];
    let kind = SubsectionKind(DEBUG_S_IGNORE.0 | DEBUG_S_STRINGTABLE.0);
    let decoded = round_trip(kind, &data);
    assert_eq!(decoded, LineSubsection::Ignored { kind, data: &data });
    round_trip(DEBUG_S_IGNORE, &[]);
}
