//! The DBI stream, its substreams, the section headers and the module streams.

use pdbrs::native::address::{AddressMap, Rva, SectionOffset};
use pdbrs::native::codeview::symbol::{ModuleSymbols, SymbolData};
use pdbrs::native::dbi::{Dbi, ModuleStream};
use pdbrs::native::fpo::{FpoStream, FrameDataStream};
use pdbrs::native::lines::{LineSubsection, ModuleLines};
use pdbrs::native::msf::StreamId;
use pdbrs::native::section::SectionHeaders;

use crate::input::Input;
use crate::ok;

/// The most items that one walk visits, to bound the time of one run.
const MAX_ITEMS: usize = 1 << 16;

/// The deepest scope nesting that the module symbol walk accepts.
const MAX_SCOPE_DEPTH: usize = 64;

/// Decode a DBI stream. The input is a section header part and then the DBI stream.
pub fn dbi(data: &[u8]) {
    let mut input = Input::new(data);
    let headers = input.part();
    if let Some(dbi) = ok(Dbi::parse(input.rest().to_vec())) {
        walk_dbi(&dbi, Some(headers));
    }
}

/// Decode the FPO and frame data streams and the section headers.
///
/// # Panics
///
/// Panics if an iterator gives another number of records than the stream reports.
pub fn frames(data: &[u8]) {
    if let Some(fpo) = ok(FpoStream::parse(data.to_vec(), StreamId(9))) {
        assert_eq!(fpo.iter().count(), fpo.len());
        assert_eq!(fpo.iter().rev().count(), fpo.len());
    }
    if let Some(frames) = ok(FrameDataStream::parse(data.to_vec(), StreamId(9))) {
        for frame in &frames {
            let _ = (
                frame.has_seh(),
                frame.has_cpp_eh(),
                frame.is_function_start(),
            );
        }
        assert_eq!(frames.iter().rev().count(), frames.len());
    }
    if let Some(headers) = ok(SectionHeaders::parse(data)) {
        for header in &headers {
            let _ = header.name_bytes();
        }
        let _ = headers.section(0);
        let _ = headers.section(u16::try_from(headers.len()).unwrap_or(u16::MAX));
    }
}

/// Decode each substream of a DBI stream and translate its addresses.
pub(crate) fn walk_dbi(dbi: &Dbi, headers: Option<&[u8]>) {
    let headers = headers.and_then(|bytes| ok(SectionHeaders::parse(bytes)));
    let addresses = headers.map(|headers| AddressMap::new(headers, dbi));
    let _ = dbi.debug_streams().count();
    let mut module_count = 0_usize;
    for module in dbi.modules().take(MAX_ITEMS) {
        if module.is_err() {
            break;
        }
        module_count += 1;
    }
    if let Some(Some(contributions)) = ok(dbi.section_contributions()) {
        assert_eq!(contributions.iter().count(), contributions.len());
        for contribution in contributions.iter().take(MAX_ITEMS) {
            let offset = u32::from_ne_bytes(contribution.offset.to_ne_bytes());
            translate(addresses.as_ref(), contribution.section, offset);
        }
    }
    if let Some(Some(map)) = ok(dbi.section_map()) {
        assert_eq!(map.iter().count(), map.len());
        let _ = map.logical_count();
    }
    if let Some(Some(sources)) = ok(dbi.source_info()) {
        for module in sources.modules().take(MAX_ITEMS) {
            for name in module.file_names().take(MAX_ITEMS) {
                let _ = name;
            }
        }
        let _ = sources.file_name(0);
    }
    if let Some(Some(names)) = ok(dbi.ec_names()) {
        super::info::walk_names(&names);
    }
    let _ = dbi.verify_type_server_map();
    let _ = module_count;
}

/// Convert a section offset to an RVA and back.
fn translate(addresses: Option<&AddressMap<'_>>, section: u16, offset: u32) {
    let Some(map) = addresses else {
        return;
    };
    let address = SectionOffset { section, offset };
    if let Some(rva) = ok(map.rva(address)) {
        let _ = map.section_offset(rva);
    }
    let _ = map.section_offset(Rva(offset));
}

/// Build a DBI stream with one module record for the region sizes of a module stream.
fn module_dbi(sizes: [u32; 3]) -> Option<Dbi> {
    let mut record = vec![0; 64];
    record[34..36].copy_from_slice(&5_u16.to_le_bytes());
    for (offset, size) in [36, 40, 44].into_iter().zip(sizes) {
        record[offset..offset + 4].copy_from_slice(&size.to_le_bytes());
    }
    record.extend_from_slice(b"m.o\0m.o\0");
    let mut bytes = vec![0; 64];
    bytes[0..4].copy_from_slice(&(-1_i32).to_le_bytes());
    bytes[4..8].copy_from_slice(&19_990_903_u32.to_le_bytes());
    bytes[16..18].copy_from_slice(&0xffff_u16.to_le_bytes());
    let length = u32::try_from(record.len()).ok()?;
    bytes[24..28].copy_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&record);
    ok(Dbi::parse(bytes))
}

/// Decode a module stream. The input is the three region sizes and then the module stream.
pub fn module(data: &[u8]) {
    let mut input = Input::new(data);
    let sizes = [input.u32(), input.u32(), input.u32()];
    let Some(dbi) = module_dbi(sizes) else {
        return;
    };
    let Some(Ok(module)) = dbi.modules().next() else {
        return;
    };
    if let Some(stream) = ok(ModuleStream::parse(input.rest().to_vec(), &module)) {
        walk_module(&stream);
    }
}

/// Decode the symbols and the C13 line data of a module stream.
pub(crate) fn walk_module(stream: &ModuleStream) {
    let _ = (stream.c11_data(), stream.global_refs());
    for subsection in stream.c13_subsections().take(MAX_ITEMS) {
        let Some(subsection) = ok(subsection) else {
            break;
        };
        line_subsection(subsection.kind, subsection.data);
    }
    if let Some(lines) = ok(ModuleLines::parse(stream)) {
        let _ = lines.validate();
        let checksums = lines.checksums();
        for subsection in lines.subsections() {
            if let LineSubsection::Lines(data) = &subsection.data {
                for block in data.blocks() {
                    let _ = checksums.map(|table| table.get(block.checksum_offset));
                }
            }
        }
    }
    let base = u32::try_from(stream.symbol_offset()).unwrap_or(u32::MAX);
    let symbols = ModuleSymbols::new(stream.symbol_data(), stream.stream(), base, MAX_SCOPE_DEPTH);
    for scoped in symbols.take(MAX_ITEMS) {
        let Some(scoped) = ok(scoped) else {
            break;
        };
        super::symbols::walk_symbol(&scoped.symbol);
        if let SymbolData::InlineSite(site) = scoped.symbol.data() {
            let _ = site.annotations.iter().count();
        }
    }
    for record in stream.symbols().take(MAX_ITEMS) {
        if record.is_err() {
            break;
        }
    }
}

/// Decode one C13 subsection and visit its entries.
pub(crate) fn line_subsection(kind: u32, data: &[u8]) {
    let Some(decoded) = ok(LineSubsection::decode(kind, data)) else {
        return;
    };
    match &decoded {
        LineSubsection::Lines(lines) => {
            let _ = lines.header().has_columns();
            assert_eq!(lines.blocks().count(), lines.block_count());
            for block in lines.blocks() {
                let count = block.lines.iter().count();
                assert_eq!(count, block.lines.len());
                if let Some(columns) = block.columns {
                    assert_eq!(columns.iter().count(), count);
                }
            }
        }
        LineSubsection::FileChecksums(table) => {
            assert_eq!(table.iter().count(), table.len());
            for entry in table {
                assert_eq!(table.get(entry.offset), Some(entry));
                assert_eq!(entry.checksum.len(), entry.kind.size());
            }
        }
        LineSubsection::InlineeLines(lines) => {
            let _ = lines.signature();
            assert_eq!(lines.iter().count(), lines.len());
            for line in lines {
                let _ = line.extra_files.iter().count();
            }
        }
        LineSubsection::CoffSymbolRvas(rvas) => {
            assert_eq!(rvas.iter().count(), rvas.len());
        }
        _ => {}
    }
}

/// Decode one C13 subsection. The first four bytes are the kind.
pub fn lines(data: &[u8]) {
    let mut input = Input::new(data);
    let kind = input.u32();
    line_subsection(kind, input.rest());
}
