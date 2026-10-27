//! Tests for the DBI stream, module records and module streams.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::index::ModuleIndex;
use pdbrs::native::dbi::{Dbi, DebugStream, Module, ModuleStream, Substream};
use pdbrs::native::msf::StreamId;
use pdbrs::native::section::Machine;
use pdbrs::native::symbols::SymbolOffset;

/// The header offsets of the substream sizes, in disk order.
const SIZE_FIELDS: [usize; 7] = [24, 28, 32, 36, 40, 52, 48];
/// The module stream index in the synthetic module records.
const MODULE_STREAM: u16 = 5;

fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
    bytes[offset..offset + value.len()].copy_from_slice(value);
}

/// Build a modern DBI stream from substreams in disk order.
fn dbi(parts: [&[u8]; 7]) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    put(&mut bytes, 0, &(-1_i32).to_le_bytes());
    put(&mut bytes, 4, &19_990_903_u32.to_le_bytes());
    put(&mut bytes, 8, &7_u32.to_le_bytes());
    put(&mut bytes, 12, &0x10_u16.to_le_bytes());
    put(&mut bytes, 16, &0xffff_u16.to_le_bytes());
    put(&mut bytes, 20, &0x12_u16.to_le_bytes());
    put(&mut bytes, 56, &1_u16.to_le_bytes());
    put(&mut bytes, 58, &0x8664_u16.to_le_bytes());
    for (offset, part) in SIZE_FIELDS.into_iter().zip(parts) {
        let size = i32::try_from(part.len()).unwrap();
        put(&mut bytes, offset, &size.to_le_bytes());
        bytes.extend_from_slice(part);
    }
    bytes
}

fn modules_only(info: &[u8]) -> Vec<u8> {
    dbi([info, &[], &[], &[], &[], &[], &[]])
}

/// Build one padded module info record.
fn module(stream: u16, sizes: [u32; 3], name: &[u8], object_file: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    put(&mut bytes, 34, &stream.to_le_bytes());
    for (offset, size) in [36, 40, 44].into_iter().zip(sizes) {
        put(&mut bytes, offset, &size.to_le_bytes());
    }
    put(&mut bytes, 48, &3_u16.to_le_bytes());
    bytes.extend_from_slice(name);
    bytes.push(0);
    bytes.extend_from_slice(object_file);
    bytes.push(0);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0xcc);
    }
    bytes
}

fn module_dbi(stream: u16, sizes: [u32; 3]) -> Dbi {
    Dbi::parse(modules_only(&module(stream, sizes, b"m.o", b"m.o"))).unwrap()
}

fn first_module(dbi: &Dbi) -> Module<'_> {
    dbi.modules().next().unwrap().unwrap().1
}

fn assert_error<T: core::fmt::Debug>(
    result: pdbrs::Result<T>,
    kind: ErrorKind,
    location: (u32, u64),
) {
    let error = result.unwrap_err();
    assert_eq!(error.kind(), kind);
    assert_eq!(
        error.stream_location(),
        Some((StreamId(location.0), location.1))
    );
    assert_eq!(error.offset(), None);
}

fn len(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap()
}

/// Build a module stream from its regions; `symbols` includes the signature.
fn module_stream(symbols: &[u8], c11: &[u8], c13: &[u8], refs: &[u8]) -> Vec<u8> {
    let mut bytes = [symbols, c11, c13].concat();
    bytes.extend_from_slice(&u32::try_from(refs.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(refs);
    bytes
}

fn sizes(symbols: &[u8], c11: &[u8], c13: &[u8]) -> [u32; 3] {
    [symbols, c11, c13].map(|region| u32::try_from(region.len()).unwrap())
}

#[test]
fn header_and_modules_keep_raw_values() {
    let first = module(
        MODULE_STREAM,
        [12, 0, 24],
        b"C:\\src\\\xff\xfe.o",
        b"lib.rlib",
    );
    let second = module(0xffff, [0, 0, 0], b"* Linker *", b"");
    let info = [first.as_slice(), second.as_slice()].concat();
    let dbi = Dbi::parse(modules_only(&info)).unwrap();

    let header = dbi.header();
    assert_eq!(header.age, 7);
    assert_eq!(header.global_stream, Some(StreamId(0x10)));
    assert_eq!(header.public_stream, None);
    assert_eq!(header.symbol_stream, Some(StreamId(0x12)));
    assert_eq!(header.flags, 1);
    assert_eq!(header.machine, Machine::AMD64);

    // The module info substream starts after the 64-byte header.
    let mut iter = dbi.modules();
    let mut indices = Vec::new();
    let mut offsets = Vec::new();
    let mut modules = Vec::new();
    loop {
        let offset = 64 + iter.position();
        let Some(result) = iter.next() else { break };
        let (index, module) = result.unwrap();
        offsets.push(offset);
        indices.push(index);
        modules.push(module);
    }
    assert_eq!(modules.len(), 2);
    assert_eq!(modules[0].name, b"C:\\src\\\xff\xfe.o");
    assert_eq!(modules[0].object_file, b"lib.rlib");
    assert_eq!(modules[0].stream, Some(StreamId(5)));
    assert_eq!(
        (
            modules[0].symbol_bytes,
            modules[0].c11_bytes,
            modules[0].c13_bytes
        ),
        (12, 0, 24)
    );
    assert_eq!(modules[0].source_file_count, 3);
    assert_eq!((indices[0], offsets[0]), (ModuleIndex(0), 64));
    assert_eq!(modules[1].name, b"* Linker *");
    assert_eq!(modules[1].object_file, b"");
    assert_eq!(modules[1].stream, None);
    assert_eq!((indices[1], offsets[1]), (ModuleIndex(1), 64 + first.len()));
}

#[test]
fn substreams_follow_disk_order() {
    let info = module(MODULE_STREAM, [0, 0, 0], b"a", b"a");
    let ec = b"ec names";
    let debug = [1, 0, 0xff, 0xff, 3, 0];
    let parts: [&[u8]; 7] = [&info, b"SC..", b"SM......", b"SI..", b"", ec, &debug];
    let dbi = Dbi::parse(dbi(parts)).unwrap();

    let kinds = [
        Substream::ModuleInfo,
        Substream::SectionContributions,
        Substream::SectionMap,
        Substream::SourceInfo,
        Substream::TypeServerMap,
        Substream::Ec,
        Substream::OptionalDebugHeader,
    ];
    let mut offset = 64;
    for (kind, part) in kinds.into_iter().zip(parts) {
        assert_eq!(dbi.substream(kind), part);
        assert_eq!(dbi.substream_offset(kind), offset);
        offset += part.len();
    }

    let streams: Vec<_> = dbi.debug_streams().collect();
    assert_eq!(streams, [Some(StreamId(1)), None, Some(StreamId(3))]);
    assert_eq!(dbi.debug_stream(DebugStream::Fpo), Some(StreamId(1)));
    assert_eq!(dbi.debug_stream(DebugStream::Exception), None);
    assert_eq!(dbi.debug_stream(DebugStream::Fixup), Some(StreamId(3)));
    assert_eq!(dbi.debug_stream(DebugStream::SectionHeader), None);
}

#[test]
fn rejects_negative_substream_sizes() {
    for offset in SIZE_FIELDS {
        let mut bytes = modules_only(&[]);
        put(&mut bytes, offset, &(-4_i32).to_le_bytes());
        let field = u64::try_from(offset).unwrap();
        assert_error(Dbi::parse(bytes), ErrorKind::InvalidFormat, (3, field));
    }
}

#[test]
fn rejects_substreams_that_do_not_fill_the_stream() {
    let bytes = dbi([&[], &[], &[], &[], &[], b"ec..", &[]]);
    let end = len(&bytes);

    let mut short = bytes.clone();
    short.pop();
    assert_error(Dbi::parse(short), ErrorKind::UnexpectedEof, (3, end - 1));

    let mut long = bytes;
    long.push(0);
    assert_error(Dbi::parse(long), ErrorKind::InvalidFormat, (3, end));
}

#[test]
fn rejects_truncated_or_legacy_headers() {
    let bytes = modules_only(&[]);
    assert_error(
        Dbi::parse(bytes[..63].to_vec()),
        ErrorKind::UnexpectedEof,
        (3, 63),
    );

    let mut legacy = bytes.clone();
    put(&mut legacy, 0, &0_i32.to_le_bytes());
    assert_error(Dbi::parse(legacy), ErrorKind::UnsupportedVersion, (3, 0));

    let mut v110 = bytes;
    put(&mut v110, 4, &20_091_201_u32.to_le_bytes());
    assert_error(Dbi::parse(v110), ErrorKind::UnsupportedVersion, (3, 4));
}

#[test]
fn rejects_odd_debug_header_size() {
    let bytes = dbi([&[], &[], &[], &[], &[], &[], &[1, 0, 2]]);
    assert_error(Dbi::parse(bytes), ErrorKind::InvalidFormat, (3, 48));
}

#[test]
fn module_records_reject_truncation() {
    let valid = module(MODULE_STREAM, [0, 0, 0], b"ok", b"ok");
    let fixed = [0; 64];
    let cases: [&[&[u8]]; 4] = [
        &[&fixed[..60]],
        &[&fixed[..], b"name"],
        &[&fixed[..], b"name\0obj"],
        &[&fixed[..], b"ab\0c\0\0"],
    ];
    for parts in cases {
        let tail = parts.concat();
        let info = [valid.as_slice(), tail.as_slice()].concat();
        let dbi = Dbi::parse(modules_only(&info)).unwrap();
        let mut modules = dbi.modules();
        assert_eq!(modules.next().unwrap().unwrap().1.name, b"ok");
        assert_error(
            modules.next().unwrap(),
            ErrorKind::UnexpectedEof,
            (3, 64 + len(&info)),
        );
        assert!(modules.next().is_none());
    }
}

#[test]
fn module_stream_splits_regions() {
    let symbols = [4, 0, 0, 0, 6, 0, 0x0e, 0x11, 1, 2, 3, 4];
    let c11 = [0xaa; 4];
    let c13 = [0xf4, 0, 0, 0, 4, 0, 0, 0, 9, 8, 7, 6];
    let refs = [0x10, 0, 0, 0];
    let dbi = module_dbi(MODULE_STREAM, sizes(&symbols, &c11, &c13));
    let bytes = module_stream(&symbols, &c11, &c13, &refs);
    let stream = ModuleStream::parse(bytes, &first_module(&dbi)).unwrap();

    assert_eq!(stream.stream(), StreamId(5));
    assert_eq!(stream.symbol_offset(), 4);
    assert_eq!(stream.symbol_data(), &symbols[4..]);
    let records: Vec<_> = stream.symbols().collect::<pdbrs::Result<_>>().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!((records[0].offset, records[0].kind), (0, 0x110e));
    assert_eq!(records[0].data, [1, 2, 3, 4]);
    assert_eq!(stream.c11_data(), c11);
    assert_eq!(stream.c13_data(), c13);
    assert_eq!(stream.global_ref_data(), refs);
    let global_refs = stream.global_refs().unwrap();
    assert_eq!(global_refs.len(), 1);
    assert_eq!(global_refs.get(0), Some(SymbolOffset(0x10)));
}

#[test]
fn module_stream_accepts_absent_symbol_region() {
    let c13 = [0xf4, 0, 0, 0, 0, 0, 0, 0];
    let dbi = module_dbi(MODULE_STREAM, sizes(&[], &[], &c13));
    let stream =
        ModuleStream::parse(module_stream(&[], &[], &c13, &[]), &first_module(&dbi)).unwrap();

    assert_eq!(stream.symbol_offset(), 0);
    assert!(stream.symbols().next().is_none());
    assert_eq!(stream.c13_data(), c13);
    assert_eq!(stream.global_ref_data(), []);
    assert!(stream.global_refs().unwrap().is_empty());
}

#[test]
fn module_stream_rejects_bad_symbol_region() {
    let dbi = module_dbi(MODULE_STREAM, [2, 0, 0]);
    let bytes = module_stream(&[4, 0], &[], &[], &[]);
    assert_error(
        ModuleStream::parse(bytes, &first_module(&dbi)),
        ErrorKind::InvalidFormat,
        (5, 0),
    );

    let c11_signature = [2, 0, 0, 0];
    let dbi = module_dbi(MODULE_STREAM, sizes(&c11_signature, &[], &[]));
    let bytes = module_stream(&c11_signature, &[], &[], &[]);
    assert_error(
        ModuleStream::parse(bytes, &first_module(&dbi)),
        ErrorKind::UnsupportedVersion,
        (5, 0),
    );
}

#[test]
fn module_stream_rejects_inconsistent_regions() {
    let symbols = [4, 0, 0, 0];
    let dbi = module_dbi(MODULE_STREAM, [4, 0, 16]);
    let short = module_stream(&symbols, &[], &[0; 4], &[]);
    let end = len(&short);
    assert_error(
        ModuleStream::parse(short, &first_module(&dbi)),
        ErrorKind::UnexpectedEof,
        (5, end),
    );

    let dbi = module_dbi(MODULE_STREAM, [4, 0, 0]);
    let module = first_module(&dbi);
    assert_error(
        ModuleStream::parse(symbols.to_vec(), &module),
        ErrorKind::UnexpectedEof,
        (5, 4),
    );

    let mut overlong_refs = module_stream(&symbols, &[], &[], &[1, 2, 3, 4]);
    overlong_refs.pop();
    let end = len(&overlong_refs);
    assert_error(
        ModuleStream::parse(overlong_refs, &module),
        ErrorKind::UnexpectedEof,
        (5, end),
    );

    let mut trailing = module_stream(&symbols, &[], &[], &[]);
    trailing.push(0);
    let end = len(&trailing);
    assert_error(
        ModuleStream::parse(trailing, &module),
        ErrorKind::InvalidFormat,
        (5, end - 1),
    );
}

#[test]
fn module_stream_requires_a_stream() {
    let dbi = module_dbi(0xffff, [0, 0, 0]);
    let error = ModuleStream::parse(vec![0; 4], &first_module(&dbi)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::MissingStream);
}

#[test]
fn c13_subsections_preserve_unknown_kinds() {
    let mut c13 = Vec::new();
    c13.extend_from_slice(&0x8000_00f4_u32.to_le_bytes());
    c13.extend_from_slice(&3_u32.to_le_bytes());
    c13.extend_from_slice(&[1, 2, 3, 0]);
    c13.extend_from_slice(&0x1234_5678_u32.to_le_bytes());
    c13.extend_from_slice(&0_u32.to_le_bytes());
    let dbi = module_dbi(MODULE_STREAM, sizes(&[], &[], &c13));
    let stream =
        ModuleStream::parse(module_stream(&[], &[], &c13, &[]), &first_module(&dbi)).unwrap();

    let subsections: Vec<_> = stream
        .c13_subsections()
        .collect::<pdbrs::Result<_>>()
        .unwrap();
    assert_eq!(subsections.len(), 2);
    assert_eq!(
        (subsections[0].offset, subsections[0].kind.0),
        (0, 0x8000_00f4)
    );
    assert_eq!(subsections[0].data, [1, 2, 3]);
    assert_eq!(
        (subsections[1].offset, subsections[1].kind.0),
        (12, 0x1234_5678)
    );
    assert_eq!(subsections[1].data, []);
}

#[test]
fn c13_subsections_reject_truncation() {
    let symbols = [4, 0, 0, 0];
    let cases: [&[u8]; 3] = [
        &[0xf4, 0, 0, 0],
        &[0xf4, 0, 0, 0, 8, 0, 0, 0, 1, 2, 3, 4],
        &[0xf4, 0, 0, 0, 3, 0, 0, 0, 1, 2, 3],
    ];
    for c13 in cases {
        let dbi = module_dbi(MODULE_STREAM, sizes(&symbols, &[], c13));
        let bytes = module_stream(&symbols, &[], c13, &[]);
        let stream = ModuleStream::parse(bytes, &first_module(&dbi)).unwrap();
        let mut subsections = stream.c13_subsections();
        assert_error(
            subsections.next().unwrap(),
            ErrorKind::UnexpectedEof,
            (5, 4 + len(c13)),
        );
        assert!(subsections.next().is_none());
    }
}
