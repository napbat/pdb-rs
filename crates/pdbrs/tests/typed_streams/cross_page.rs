//! Records and fields that cross a page boundary of a stream with noncontiguous pages.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::codeview::symbol::{Symbol, SymbolData};
use pdbrs::native::codeview::type_record::TypeLeaf;
use pdbrs::native::dbi::{Dbi, DebugStream, Module, ModuleStream};
use pdbrs::native::debug_stream::OmapStream;
use pdbrs::native::lines::{ChecksumKind, ModuleLines};
use pdbrs::native::names::StringTable;
use pdbrs::native::section::SectionHeaderStream;
use pdbrs::native::symbols::{RecordOffsets, SymbolOffset};
use pdbrs::native::types::{HEADER_SIZE, TypeStream};

use super::{
    DBI, MODULE, MODULE_SYMBOL_BYTES, NAMES, OMAP, OMAP_RECORDS, Order, PAGE, SECTIONS,
    STRING_COUNT, SYMBOLS, Streams, TARGET_CHECKSUM_OFFSET, TARGET_MODULE, TARGET_MODULE_OFFSET,
    TARGET_SECTION, TARGET_SYMBOL, TARGET_SYMBOL_OFFSET, TARGET_TYPE_ARGUMENTS, TARGET_TYPE_OFFSET,
    crosses_page, dbi_stream, module_stream, names_stream, omap, omap_record, open, section,
    sections, sha256, string_name, to_u32, to_u64, type_stream,
};

/// Both page orders. The decoded values must not depend on the order.
const ORDERS: [Order; 2] = [Order::Contiguous, Order::Scattered];

/// Return the target module record of `dbi`.
fn target_module(dbi: &Dbi) -> Module<'_> {
    dbi.modules().nth(4).unwrap().unwrap()
}

/// Return the name of an `S_UDT` symbol.
fn udt_name(symbol: Symbol<'_>) -> &[u8] {
    let SymbolData::UserDefinedType(udt) = *symbol.data() else {
        panic!("not a user-defined type: {symbol:?}");
    };
    udt.name
}

#[test]
fn names_string_across_pages_resolves() {
    let (expected, offsets) = names_stream();
    let (index, offset) = (0..STRING_COUNT)
        .zip(&offsets)
        .find(|&(index, &offset)| {
            let start = 12 + usize::try_from(offset).unwrap();
            crosses_page(start, string_name(index).len() + 1)
        })
        .unwrap();
    for order in ORDERS {
        let bytes = Streams::new().container(order);
        let pdb = open(&bytes);
        assert_eq!(pdb.info().named_stream(b"/names"), Some(NAMES));
        let names = pdb.named_stream_bytes(b"/names").unwrap().unwrap();
        assert_eq!(names, expected, "{order:?}");
        let table = StringTable::parse(&names).unwrap();
        let name = string_name(index);
        assert_eq!(table.get(*offset).unwrap(), name.as_slice());
        assert_eq!(table.find(&name).unwrap(), Some(*offset));
        table.verify_buckets().unwrap();
    }
}

#[test]
fn truncated_names_stream_across_pages_is_eof() {
    let mut streams = Streams::new();
    streams.names.truncate(PAGE + 8);
    let bytes = streams.container(Order::Scattered);
    let names = open(&bytes).named_stream_bytes(b"/names").unwrap().unwrap();
    assert_eq!(names.len(), PAGE + 8);
    let error = StringTable::parse(&names).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
}

#[test]
fn type_record_across_pages_decodes_like_one_buffer() {
    let expected = TypeStream::parse(type_stream(None)).unwrap();
    for order in ORDERS {
        let bytes = Streams::new().container(order);
        let types = open(&bytes).types().unwrap().unwrap();
        assert_eq!(types, expected, "{order:?}");
        let target = types.records().nth(1).unwrap().unwrap();
        assert_eq!(target.index, 0x1001);
        assert_eq!(target.record.offset, to_u32(TARGET_TYPE_OFFSET));
        let TypeLeaf::ArgumentList(list) = TypeLeaf::decode(target.record).unwrap() else {
            panic!("not an argument list");
        };
        let arguments: Vec<TypeIndex> = list.iter().collect();
        let expected: Vec<TypeIndex> = (0..TARGET_TYPE_ARGUMENTS)
            .map(|index| TypeIndex(0x74 + index))
            .collect();
        assert_eq!(arguments, expected);
    }
}

#[test]
fn truncated_type_record_across_pages_is_eof() {
    // The record bytes end 14 bytes after the page boundary, in the target record.
    let record_bytes = PAGE + 14 - HEADER_SIZE;
    let mut streams = Streams::new();
    streams.types = type_stream(Some(record_bytes));
    let bytes = streams.container(Order::Scattered);
    let types = open(&bytes).types().unwrap().unwrap();
    let mut records = types.records();
    assert_eq!(records.next().unwrap().unwrap().index, 0x1000);
    let error = records.next().unwrap().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(records.position(), HEADER_SIZE + TARGET_TYPE_OFFSET);
    assert!(records.next().is_none());
}

#[test]
fn module_record_across_pages_decodes_like_one_buffer() {
    let (_, sizes) = module_stream();
    let expected = Dbi::parse(dbi_stream(sizes, None)).unwrap();
    let expected: Vec<Module<'_>> = expected.modules().map(Result::unwrap).collect();
    for order in ORDERS {
        let bytes = Streams::new().container(order);
        let dbi = open(&bytes).debug_info().unwrap().unwrap();
        let modules: Vec<Module<'_>> = dbi.modules().map(Result::unwrap).collect();
        assert_eq!(modules, expected, "{order:?}");
        let target = target_module(&dbi);
        assert_eq!(target.offset, to_u32(TARGET_MODULE_OFFSET));
        assert_eq!(target.name, TARGET_MODULE);
        assert_eq!(target.object_file, TARGET_MODULE);
        assert_eq!(target.stream, Some(MODULE));
        assert_eq!(
            [target.symbol_bytes, target.c11_bytes, target.c13_bytes],
            sizes
        );
    }
}

#[test]
fn truncated_module_record_across_pages_is_eof() {
    // The module info ends 12 bytes after the page boundary, in the target record.
    let module_info = PAGE + 12 - 64;
    let (_, sizes) = module_stream();
    let mut streams = Streams::new();
    streams.dbi = dbi_stream(sizes, Some(module_info));
    let bytes = streams.container(Order::Scattered);
    let error = open(&bytes).debug_info().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(error.stream_location(), Some((DBI.0, to_u64(PAGE + 12))));
}

#[test]
fn module_symbol_and_checksum_across_pages_decode_like_one_buffer() {
    let (module_bytes, sizes) = module_stream();
    let reference_dbi = Dbi::parse(dbi_stream(sizes, None)).unwrap();
    let reference = ModuleStream::parse(module_bytes, &target_module(&reference_dbi)).unwrap();
    let reference_lines = ModuleLines::parse(&reference).unwrap();
    for order in ORDERS {
        let bytes = Streams::new().container(order);
        let pdb = open(&bytes);
        let dbi = pdb.debug_info().unwrap().unwrap();
        let stream = pdb.module_stream(&target_module(&dbi)).unwrap().unwrap();

        let records: Vec<_> = stream.symbols().map(Result::unwrap).collect();
        assert_eq!(
            records,
            reference.symbols().map(Result::unwrap).collect::<Vec<_>>()
        );
        let target = records[15];
        assert_eq!(target.offset, to_u32(TARGET_SYMBOL_OFFSET));
        assert_eq!(udt_name(Symbol::decode(target).unwrap()), TARGET_SYMBOL);

        let lines = ModuleLines::parse(&stream).unwrap();
        assert_eq!(lines, reference_lines, "{order:?}");
        let entry = lines
            .checksums()
            .unwrap()
            .get(TARGET_CHECKSUM_OFFSET)
            .unwrap();
        assert_eq!(entry.kind, ChecksumKind::Sha256);
        assert_eq!(entry.name_offset, 12);
        assert_eq!(entry.checksum, sha256(11));
        lines.validate().unwrap();
    }
}

#[test]
fn truncated_checksum_subsection_across_pages_is_eof() {
    // The C13 region ends 6 bytes after the second page boundary, in the target entry.
    let c13_end = 2 * PAGE + 6;
    let (mut module, mut sizes) = module_stream();
    module.truncate(c13_end);
    module.extend_from_slice(&0_u32.to_le_bytes());
    sizes[2] = to_u32(c13_end - MODULE_SYMBOL_BYTES);
    let mut streams = Streams::new();
    streams.module = module;
    streams.dbi = dbi_stream(sizes, None);
    let bytes = streams.container(Order::Scattered);
    let pdb = open(&bytes);
    let dbi = pdb.debug_info().unwrap().unwrap();
    let stream = pdb.module_stream(&target_module(&dbi)).unwrap().unwrap();
    let error = ModuleLines::parse(&stream).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(error.stream_location(), Some((MODULE.0, to_u64(c13_end))));
}

#[test]
fn global_symbol_across_pages_resolves_like_one_buffer() {
    for order in ORDERS {
        let bytes = Streams::new().container(order);
        let pdb = open(&bytes);
        let dbi = pdb.debug_info().unwrap().unwrap();
        let symbols = pdb.stream_bytes(SYMBOLS).unwrap();
        assert_eq!(symbols, Streams::new().symbols, "{order:?}");
        let offsets = RecordOffsets::frame(&symbols, SYMBOLS).unwrap();
        let index = pdb.global_symbols(&dbi).unwrap().unwrap();
        index.hash_table().check_records(&offsets).unwrap();
        let found: Vec<_> = index.hash_table().lookup(TARGET_SYMBOL).collect();
        let target = SymbolOffset(to_u32(TARGET_SYMBOL_OFFSET));
        assert!(found.iter().any(|record| record.symbol == target));
        let symbol = Symbol::decode(target.record(&symbols).unwrap()).unwrap();
        assert_eq!(udt_name(symbol), TARGET_SYMBOL);
    }
}

#[test]
fn truncated_symbol_record_across_pages_is_eof() {
    let mut streams = Streams::new();
    streams.symbols.truncate(PAGE + 8);
    let bytes = streams.container(Order::Scattered);
    let symbols = open(&bytes).stream_bytes(SYMBOLS).unwrap();
    let error = RecordOffsets::frame(&symbols, SYMBOLS).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(
        error.stream_location(),
        Some((SYMBOLS.0, to_u64(TARGET_SYMBOL_OFFSET)))
    );
    let error = SymbolOffset(to_u32(TARGET_SYMBOL_OFFSET))
        .record(&symbols)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
}

#[test]
fn section_header_across_pages_decodes_like_one_buffer() {
    let expected = SectionHeaderStream::parse(sections(), SECTIONS).unwrap();
    let number = u16::try_from(TARGET_SECTION + 1).unwrap();
    for order in ORDERS {
        let bytes = Streams::new().container(order);
        let pdb = open(&bytes);
        let dbi = pdb.debug_info().unwrap().unwrap();
        let headers = pdb
            .section_headers(&dbi, DebugStream::SectionHeader)
            .unwrap()
            .unwrap();
        assert_eq!(headers, expected, "{order:?}");
        assert_eq!(headers.get(TARGET_SECTION), Some(section(TARGET_SECTION)));
        assert_eq!(
            headers.headers().section(number),
            Some(section(TARGET_SECTION))
        );
    }
}

#[test]
fn truncated_section_header_across_pages_is_eof() {
    let mut streams = Streams::new();
    streams.sections.truncate(PAGE + 4);
    let bytes = streams.container(Order::Scattered);
    let pdb = open(&bytes);
    let dbi = pdb.debug_info().unwrap().unwrap();
    let error = pdb
        .section_headers(&dbi, DebugStream::SectionHeader)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(
        error.stream_location(),
        Some((SECTIONS.0, to_u64(TARGET_SECTION * 40)))
    );
}

#[test]
fn omap_table_across_pages_maps_like_one_buffer() {
    let expected = OmapStream::parse(omap(), OMAP).unwrap();
    // Record 64 is the first record of the second page.
    let address = omap_record(64).rva + 4;
    for order in ORDERS {
        let bytes = Streams::new().container(order);
        let pdb = open(&bytes);
        let dbi = pdb.debug_info().unwrap().unwrap();
        let table = pdb.omap(&dbi, DebugStream::OmapToSource).unwrap().unwrap();
        assert_eq!(table, expected, "{order:?}");
        assert_eq!(table.iter().count(), usize::try_from(OMAP_RECORDS).unwrap());
        assert_eq!(table.record_for(address), Some(omap_record(64)));
        assert_eq!(table.record_for(address - 8), Some(omap_record(63)));
        let tables = pdb.address_tables(&dbi).unwrap().unwrap();
        assert_eq!(tables.to_source, Some(expected.clone()));
        assert_eq!(tables.from_source, None);
    }
}

#[test]
fn truncated_omap_across_pages_is_eof() {
    let mut streams = Streams::new();
    streams.omap.truncate(PAGE + 4);
    let bytes = streams.container(Order::Scattered);
    let pdb = open(&bytes);
    let dbi = pdb.debug_info().unwrap().unwrap();
    let error = pdb.omap(&dbi, DebugStream::OmapToSource).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(error.stream_location(), Some((OMAP.0, to_u64(PAGE))));
}
