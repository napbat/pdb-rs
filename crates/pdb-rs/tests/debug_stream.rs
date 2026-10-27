//! Tests for the record streams and blobs of the optional debug streams.

use pdb_rs::ErrorKind;
use pdb_rs::native::address::Rva;
use pdb_rs::native::debug_stream::{
    Arm64RuntimeFunction, BLOB_HEADER_SIZE, BLOB_VERSION, BlobHeader, DebugRecord, Fixup,
    FixupStream, FunctionEntry, FunctionEntryStream, Rid, RuntimeFunctions, RvaVaBlob, TokenRidMap,
    X64RuntimeFunction,
};
use pdb_rs::native::msf::StreamId;
use pdb_rs::native::section::Machine;

/// The stream ID of the synthetic streams.
const STREAM: StreamId = StreamId(9);

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn assert_located(error: &pdb_rs::Error, kind: ErrorKind, offset: u64) {
    assert_eq!(error.kind(), kind, "{error}");
    assert_eq!(error.offset(), None);
    assert_eq!(error.stream_location(), Some((STREAM, offset)));
}

/// Build a blob stream with the given header words, extra bytes and data.
fn blob(header: [u32; 3], extra: &[u8], data: &[u8]) -> Vec<u8> {
    let mut bytes = words(&header);
    bytes.extend(words(&[0x1_0000, 0, 0x1, 0, 0]));
    bytes.extend_from_slice(extra);
    bytes.extend_from_slice(data);
    bytes
}

fn function_entry(start: u32, end: u32, end_of_prologue: u32) -> FunctionEntry {
    let mut entry = FunctionEntry::default();
    entry.start = start;
    entry.end = end;
    entry.end_of_prologue = end_of_prologue;
    entry
}

fn fixup(kind: u16, extra: u16, rva: u32, target: u32) -> Fixup {
    let mut fixup = Fixup::default();
    fixup.kind = kind;
    fixup.extra = extra;
    fixup.rva = rva;
    fixup.target = target;
    fixup
}

fn blob_header(data_rva: u32, image_base: u64) -> BlobHeader {
    let mut header = BlobHeader::default();
    header.version = BLOB_VERSION;
    header.data_rva = data_rva;
    header.image_base = image_base;
    header.reserved = [0; 2];
    header
}

fn x64_entry(begin: u32, end: u32, unwind_info: u32) -> X64RuntimeFunction {
    let mut entry = X64RuntimeFunction::default();
    entry.begin = begin;
    entry.end = end;
    entry.unwind_info = unwind_info;
    entry
}

fn arm64_entry(begin: u32, unwind_data: u32) -> Arm64RuntimeFunction {
    let mut entry = Arm64RuntimeFunction::default();
    entry.begin = begin;
    entry.unwind_data = unwind_data;
    entry
}

#[test]
fn function_entries_decode_and_encode() {
    let bytes = words(&[0x1000, 0x1040, 0x1008, u32::MAX, 0, 1]);
    let stream = FunctionEntryStream::parse(bytes.clone(), STREAM).unwrap();
    assert_eq!(stream.len(), 2);
    let entries: Vec<_> = stream.iter().collect();
    assert_eq!(entries[0], function_entry(0x1000, 0x1040, 0x1008));
    assert_eq!(
        (entries[1].start, entries[1].end, entries[1].end_of_prologue),
        (u32::MAX, 0, 1)
    );
    assert_eq!(stream.to_bytes().unwrap(), bytes);
    assert_eq!(FunctionEntryStream::from_records(&entries).unwrap(), stream);
    assert!(
        FunctionEntryStream::parse(Vec::new(), STREAM)
            .unwrap()
            .is_empty()
    );
    for (length, offset) in [(1, 0), (11, 0), (13, 12), (23, 12)] {
        let error = FunctionEntryStream::parse(vec![0; length], STREAM).unwrap_err();
        assert_located(&error, ErrorKind::UnexpectedEof, offset);
    }
}

#[test]
fn fixups_decode_and_encode() {
    // `link.exe /DEBUGTYPE:FIXUP` writes records like these for x64.
    let mut bytes = Vec::new();
    for (kind, extra, rva, target) in [(1_u16, 0_u16, 0xf080, 0xf088), (4, 0x8000, 0x297d, 0x2a90)]
    {
        bytes.extend(kind.to_le_bytes());
        bytes.extend(extra.to_le_bytes());
        bytes.extend(words(&[rva, target]));
    }
    let stream = FixupStream::parse(bytes.clone(), STREAM).unwrap();
    let fixups: Vec<_> = stream.iter().collect();
    assert_eq!(fixups[0], fixup(1, 0, 0xf080, 0xf088));
    assert_eq!(
        (
            fixups[1].kind,
            fixups[1].extra,
            fixups[1].rva,
            fixups[1].target
        ),
        (4, 0x8000, 0x297d, 0x2a90)
    );
    assert_eq!(stream.get(1), Some(fixups[1]));
    assert_eq!(stream.get(2), None);
    assert_eq!(stream.get(usize::MAX), None);
    assert_eq!(stream.iter().nth(1), Some(fixups[1]));
    assert_eq!(stream.iter().len(), 2);
    let mut out = vec![0xee];
    stream.encode(&mut out).unwrap();
    assert_eq!(&out[1..], bytes.as_slice());
    let error = FixupStream::parse(vec![0; 25], STREAM).unwrap_err();
    assert_located(&error, ErrorKind::UnexpectedEof, 24);
}

#[test]
fn token_rid_maps_decode_and_encode() {
    let bytes = words(&[0, 7, u32::MAX]);
    let map = TokenRidMap::parse(bytes.clone(), STREAM).unwrap();
    assert_eq!(
        map.iter().collect::<Vec<_>>(),
        [Rid(0), Rid(7), Rid(u32::MAX)]
    );
    assert_eq!(map.to_bytes().unwrap(), bytes);
    assert_eq!(map.as_bytes(), bytes.as_slice());
    for length in [1, 2, 3] {
        let error = TokenRidMap::parse(vec![0; 8 + length], STREAM).unwrap_err();
        assert_located(&error, ErrorKind::UnexpectedEof, 8);
    }
    let mut one = Vec::new();
    Rid(0x0102_0304).encode(&mut one).unwrap();
    assert_eq!(one, [4, 3, 2, 1]);
    assert_eq!(Rid::SIZE, 4);
}

#[test]
fn blobs_decode_the_header_and_encode_the_same_bytes() {
    let data = words(&[0x1010, 0x116c, 0xc910, 0x1180, 0x11c4, 0xc900]);
    let bytes = blob([1, 32, 24], &[], &data);
    let pdata = RvaVaBlob::parse(bytes.clone(), STREAM).unwrap();
    let header = pdata.header();
    assert_eq!(header, blob_header(0x1_0000, 0x1_0000_0000));
    assert_eq!((header.version, header.reserved), (BLOB_VERSION, [0, 0]));
    assert_eq!((pdata.header_size(), pdata.data_size()), (32, 24));
    assert_eq!(pdata.extra_header(), [0_u8; 0]);
    assert_eq!(pdata.data(), data.as_slice());
    assert_eq!(pdata.to_bytes().unwrap(), bytes);
    assert_eq!(pdata.data_at(Rva(0x1_0000)), Some(data.as_slice()));
    assert_eq!(pdata.data_at(Rva(0x1_0004)).unwrap().len(), 20);
    assert_eq!(pdata.data_at(Rva(0x1_0018)), Some(&[][..]));
    assert_eq!(pdata.data_at(Rva(0x1_0019)), None);
    assert_eq!(pdata.data_at(Rva(0xffff)), None);

    let rebuilt = RvaVaBlob::new(header, &[], &data).unwrap();
    assert_eq!(rebuilt.to_bytes().unwrap(), bytes);

    // Extra header bytes stay raw.
    let extended = blob([1, 36, 4], &[9, 8, 7, 6], &[1, 2, 3, 4]);
    let parsed = RvaVaBlob::parse(extended.clone(), STREAM).unwrap();
    assert_eq!(parsed.extra_header(), [9, 8, 7, 6]);
    assert_eq!(parsed.data(), [1, 2, 3, 4]);
    assert_eq!(parsed.to_bytes().unwrap(), extended);
    let rebuilt = RvaVaBlob::new(parsed.header(), &[9, 8, 7, 6], &[1, 2, 3, 4]).unwrap();
    assert_eq!(
        rebuilt,
        RvaVaBlob::new(parsed.header(), &[9, 8, 7, 6], &[1, 2, 3, 4]).unwrap()
    );
    assert_eq!(rebuilt.to_bytes().unwrap(), extended);

    // An empty data region is valid.
    let empty = RvaVaBlob::parse(blob([1, 32, 0], &[], &[]), STREAM).unwrap();
    assert_eq!(empty.data_size(), 0);
}

#[test]
fn blob_header_errors_have_locations() {
    for length in [0, 12, BLOB_HEADER_SIZE - 1] {
        let mut bytes = blob([1, 32, 0], &[], &[]);
        bytes.truncate(length);
        let error = RvaVaBlob::parse(bytes, STREAM).unwrap_err();
        assert_located(&error, ErrorKind::UnexpectedEof, 0);
    }
    for version in [0, 2, u32::MAX] {
        let error = RvaVaBlob::parse(blob([version, 32, 0], &[], &[]), STREAM).unwrap_err();
        assert_located(&error, ErrorKind::UnsupportedVersion, 0);
    }
    for header_size in [0, 12, 31, 37, u32::MAX] {
        let error = RvaVaBlob::parse(blob([1, header_size, 0], &[0; 4], &[]), STREAM).unwrap_err();
        assert_located(&error, ErrorKind::InvalidFormat, 4);
    }
    for data_size in [0, 3, 5, u32::MAX] {
        let error = RvaVaBlob::parse(blob([1, 32, data_size], &[], &[0; 4]), STREAM).unwrap_err();
        assert_located(&error, ErrorKind::InvalidFormat, 8);
    }
    let mut header = blob_header(0, 0);
    header.version = 2;
    let error = RvaVaBlob::new(header, &[], &[]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedVersion);
}

#[test]
fn pdata_entries_depend_on_the_machine() {
    let data = words(&[0x1010, 0x116c, 0xc910, 0x1180, 0x11c4, 0xc900]);
    let pdata = RvaVaBlob::parse(blob([1, 32, 24], &[], &data), STREAM).unwrap();
    let Some(RuntimeFunctions::X64(entries)) = pdata.runtime_functions(Machine::AMD64).unwrap()
    else {
        panic!("x64 entries expected");
    };
    let entries: Vec<_> = entries.collect();
    assert_eq!(
        entries,
        [
            x64_entry(0x1010, 0x116c, 0xc910),
            x64_entry(0x1180, 0x11c4, 0xc900)
        ]
    );
    let Some(RuntimeFunctions::Arm64(entries)) = pdata.runtime_functions(Machine::ARM64).unwrap()
    else {
        panic!("ARM64 entries expected");
    };
    let entries: Vec<_> = entries.collect();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0], arm64_entry(0x1010, 0x116c));
    assert_eq!(entries[0].flag(), 0);
    assert_eq!(arm64_entry(0, 0x0040_0011).flag(), 1);
    assert_eq!(arm64_entry(0, 0x0040_0012).flag(), 2);
    // x86 has no `.pdata` layout.
    assert!(pdata.runtime_functions(Machine::I386).unwrap().is_none());

    let mut encoded = Vec::new();
    for entry in &entries {
        entry.encode(&mut encoded).unwrap();
    }
    assert_eq!(encoded, data);

    // A partial entry fails with the stream offset of the entry.
    let odd = RvaVaBlob::parse(blob([1, 32, 20], &[], &data[..20]), STREAM).unwrap();
    let error = odd.runtime_functions(Machine::AMD64).unwrap_err();
    assert_located(&error, ErrorKind::UnexpectedEof, 32 + 12);
    let error = odd.runtime_functions(Machine::ARM64).unwrap_err();
    assert_located(&error, ErrorKind::UnexpectedEof, 32 + 16);
    let unlocated = RvaVaBlob::new(odd.header(), &[], odd.data()).unwrap();
    let error = unlocated.runtime_functions(Machine::AMD64).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(error.stream_location(), None);
}
