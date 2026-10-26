//! Read every stream and record of a native PDB and report undecoded data.
//!
//! The example decodes each stream role, checks the references between
//! streams and prints counts. It stops at the first malformed structure.
//! At the end, it fails if a record kind, a subsection kind or a stream has
//! no typed decoder. It reports producer defects that do not stop reading,
//! for example an index record outside the bucket of its name, without
//! failing. The library keeps names as bytes.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::error::Error;

use pdbrs::io::FileReader;
use pdbrs::native::Pdb;
use pdbrs::native::address::{AddressMap, SectionOffset};
use pdbrs::native::auxiliary::{LinkInfo, NamedStreamKind, TmCache, UdtSourceLineUndone};
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::id_record::{IdRecord, IdTable};
use pdbrs::native::codeview::index::{IdIndex, TypeIndex};
use pdbrs::native::codeview::symbol::{Annotation, ModuleSymbols, Symbol, SymbolData};
use pdbrs::native::codeview::type_record::TypeLeaf;
use pdbrs::native::dbi::{Dbi, DebugStream};
use pdbrs::native::injected_source::{SourceHeaderBlock, content_stream_name};
use pdbrs::native::lines::{LineSubsection, ModuleLines};
use pdbrs::native::msf::{OldDirectory, StreamId};
use pdbrs::native::names::StringTable;
use pdbrs::native::section::SectionHeaders;
use pdbrs::native::symbols::{HashTable, RecordOffsets};
use pdbrs::native::types::{ForwardRefs, HEADER_SIZE, TypeHashes, TypeStream, TypeTable};

/// The deepest scope nesting that the example accepts in a module.
const MAX_SCOPE_DEPTH: usize = 256;

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

/// Counts and coverage gathered while reading.
#[derive(Default)]
struct Report {
    /// Every stream that a decoder read.
    claimed: BTreeSet<u32>,
    /// Counters in name order.
    counts: BTreeMap<&'static str, usize>,
    /// Descriptions of data without a typed decoder.
    undecoded: Vec<String>,
    /// Decoded data that breaks a producer rule but does not stop reading.
    inconsistencies: Vec<String>,
}

impl Report {
    fn claim(&mut self, id: StreamId) {
        self.claimed.insert(id.0);
    }

    fn add(&mut self, name: &'static str, count: usize) {
        *self.counts.entry(name).or_default() += count;
    }

    fn undecoded(&mut self, what: String) {
        self.undecoded.push(what);
    }

    fn inconsistency(&mut self, what: String) {
        self.inconsistencies.push(what);
    }
}

/// The decoded streams that the module and symbol checks need.
struct Context<'a> {
    names: StringTable<'a>,
    tpi_end: TypeIndex,
    ids: IdTable<'a>,
    addresses: Option<AddressMap<'a>>,
    module_count: usize,
}

fn main() -> Outcome {
    let mut arguments = std::env::args_os().skip(1);
    let path = arguments
        .next()
        .ok_or("usage: cargo run --example native -- <path>")?;
    if arguments.next().is_some() {
        return Err("expected exactly one input path".into());
    }
    let pdb = Pdb::open(path)?;
    let mut report = Report::default();
    container(&pdb, &mut report)?;

    let names_bytes = pdb.named_stream_bytes(b"/names")?.ok_or("missing /names")?;
    let names = StringTable::parse(&names_bytes)?;
    let placement = names.verify_buckets()?;
    report.add(
        "/names strings",
        usize::try_from(placement.home)? + usize::try_from(placement.probed)?,
    );
    let tpi = pdb.types()?.ok_or("missing TPI stream")?;
    let ipi = pdb.ids()?.ok_or("missing IPI stream")?;
    report.claim(StreamId(2));
    report.claim(StreamId(4));
    let tpi_hash_bytes = hash_stream_bytes(&pdb, &tpi, &mut report)?;
    let ipi_hash_bytes = hash_stream_bytes(&pdb, &ipi, &mut report)?;
    named_streams(&pdb, &names, &tpi, &mut report)?;
    types(&tpi, tpi_hash_bytes.as_deref(), &mut report)?;
    let ids = identifiers(&ipi, ipi_hash_bytes.as_deref(), &tpi, &names, &mut report)?;

    let dbi = pdb.debug_info()?.ok_or("missing DBI stream")?;
    report.claim(StreamId(3));
    let header_bytes = section_header_bytes(&pdb, &dbi, &mut report)?;
    let addresses = header_bytes
        .as_deref()
        .map(SectionHeaders::parse)
        .transpose()?
        .map(|headers| AddressMap::new(headers, &dbi));
    let module_count = debug_info_tables(&dbi, addresses.as_ref(), &mut report)?;
    let context = Context {
        names,
        tpi_end: TypeIndex(tpi.header().index_end),
        ids,
        addresses,
        module_count,
    };
    let procedures = modules(&pdb, &dbi, &context, &mut report)?;
    shared_symbols(&pdb, &dbi, &context, &procedures, &mut report)?;
    unclaimed_streams(&pdb, &mut report)?;
    print(&report);
    if report.undecoded.is_empty() {
        Ok(())
    } else {
        Err(format!("{} items have no typed decoder", report.undecoded.len()).into())
    }
}

/// Check the container: free-page map, stream 0 and the Info stream.
fn container(pdb: &Pdb<FileReader>, report: &mut Report) -> Outcome {
    let msf = pdb.msf();
    let map = msf.check_free_page_map()?;
    report.add("pages", usize::try_from(msf.page_count())?);
    report.add(
        "unreferenced allocated pages",
        usize::try_from(map.unreferenced_allocated_count())?,
    );
    if msf.stream(StreamId(0))?.is_some() {
        let bytes = pdb.stream_bytes(StreamId(0))?;
        if let Some(old) = OldDirectory::parse(&bytes, msf.block_size())? {
            report.add(
                "previous directory streams",
                usize::try_from(old.stream_count())?,
            );
        }
    }
    report.claim(StreamId(0));
    report.claim(StreamId(1));
    report.add("named streams", pdb.info().named_streams().count());
    Ok(())
}

/// Load the hash stream of a type stream, if the header names one.
fn hash_stream_bytes(
    pdb: &Pdb<FileReader>,
    types: &TypeStream,
    report: &mut Report,
) -> Outcome<Option<Vec<u8>>> {
    let Some(id) = types.header().hash_stream else {
        return Ok(None);
    };
    report.claim(id);
    if let Some(aux) = types.header().hash_aux_stream {
        report.claim(aux);
        report.undecoded(format!("auxiliary type hash stream {}", aux.0));
    }
    Ok(Some(pdb.stream_bytes(id)?))
}

/// Decode every named stream and verify the injected sources.
fn named_streams(
    pdb: &Pdb<FileReader>,
    names: &StringTable<'_>,
    tpi: &TypeStream,
    report: &mut Report,
) -> Outcome {
    let mut verified = HashSet::new();
    let info = pdb.info();
    for named in info.named_streams() {
        report.claim(named.stream);
        let bytes = pdb.stream_bytes(named.stream)?;
        match NamedStreamKind::classify(named.name) {
            NamedStreamKind::Names | NamedStreamKind::SourceFile { .. } => {}
            NamedStreamKind::LinkInfo => {
                LinkInfo::parse(&bytes, named.stream)?;
            }
            NamedStreamKind::TmCache => {
                TmCache::parse(&bytes, named.stream)?;
            }
            NamedStreamKind::UdtSourceLineUndone => {
                let list = UdtSourceLineUndone::parse(&bytes, named.stream)?;
                let table = TypeTable::new(tpi)?;
                for index in list.type_indices() {
                    table
                        .leaf(index)?
                        .tag()
                        .ok_or("/UDTSRCLINEUNDONE names a type that is not a UDT")?;
                }
                report.add("UDT source lines undone", list.len());
            }
            NamedStreamKind::SourceHeaderBlock => {
                let block = SourceHeaderBlock::parse(&bytes, named.stream)?;
                for entry in block.entries() {
                    let source = entry.value;
                    names.get(source.file_name_offset)?;
                    names.get(source.object_name_offset)?;
                    let virtual_name = names.get(source.virtual_file_name_offset)?;
                    let stream_name = content_stream_name(virtual_name)?;
                    let id = info
                        .named_stream(&stream_name)
                        .ok_or("injected source has no content stream")?;
                    source.verify_content(&pdb.stream_bytes(id)?, id)?;
                    verified.insert(id);
                }
                report.add("injected sources", block.entries().len());
            }
            _ => report.undecoded(format!(
                "named stream {}",
                String::from_utf8_lossy(named.name)
            )),
        }
    }
    for named in info.named_streams() {
        let content = matches!(
            NamedStreamKind::classify(named.name),
            NamedStreamKind::SourceFile { .. }
        );
        if content && !verified.contains(&named.stream) {
            report.undecoded(format!(
                "injected source stream {} without a header entry",
                named.stream.0
            ));
        }
    }
    Ok(())
}

/// Decode every TPI record, its field lists and its forward references.
fn types(tpi: &TypeStream, hash_bytes: Option<&[u8]>, report: &mut Report) -> Outcome {
    let hashes = hash_bytes
        .map(|bytes| TypeHashes::parse(tpi.header(), bytes))
        .transpose()?;
    if let Some(hashes) = &hashes {
        hashes.verify(tpi)?;
    }
    let table = TypeTable::new(tpi)?;
    let forward = ForwardRefs::new(&table, hashes.as_ref())?;
    let base = u64::try_from(HEADER_SIZE)?;
    let (mut resolved, mut unresolved) = (0, 0);
    for record in tpi.records() {
        let record = record?;
        let location =
            |error: pdbrs::Error| error.with_stream(2, base + u64::from(record.record.offset));
        let leaf = TypeLeaf::decode(record.record).map_err(location)?;
        match &leaf {
            TypeLeaf::Unsupported(raw) => {
                report.undecoded(format!("TPI leaf {:#06x}", raw.kind));
            }
            TypeLeaf::FieldList(_) => {
                for member in table.members(TypeIndex(record.index)).map_err(location)? {
                    member.map_err(location)?;
                    report.add("field-list members", 1);
                }
            }
            TypeLeaf::MethodList(list) => {
                for entry in list.entries() {
                    entry.map_err(location)?;
                }
            }
            _ => {}
        }
        if leaf
            .tag()
            .is_some_and(|tag| tag.options.is_forward_reference())
        {
            match forward.resolve(TypeIndex(record.index))? {
                Some(_) => resolved += 1,
                None => unresolved += 1,
            }
        }
        report.add("TPI records", 1);
    }
    report.add("forward references resolved", resolved);
    report.add("forward references without definition", unresolved);
    Ok(())
}

/// Decode every IPI record and validate its references.
fn identifiers<'a>(
    ipi: &'a TypeStream,
    hash_bytes: Option<&[u8]>,
    tpi: &TypeStream,
    names: &StringTable<'_>,
    report: &mut Report,
) -> Outcome<IdTable<'a>> {
    if let Some(bytes) = hash_bytes {
        TypeHashes::parse(ipi.header(), bytes)?.verify(ipi)?;
    }
    let ids = IdTable::new(ipi)?;
    ids.validate(TypeIndex(tpi.header().index_end))?;
    let mut text = Vec::new();
    for record in ipi.records() {
        let id = IdIndex(record?.index);
        match ids.decode(id)? {
            IdRecord::Unsupported(raw) => {
                report.undecoded(format!("IPI leaf {:#06x}", raw.kind));
            }
            IdRecord::String(_) => {
                text.clear();
                ids.resolve_string(id, &mut text)?;
            }
            IdRecord::UdtModuleSourceLine(line) => {
                names.get(line.name_offset)?;
            }
            _ => {}
        }
        report.add("IPI records", 1);
    }
    Ok(ids)
}

/// Load the section-header stream and claim every optional debug stream.
fn section_header_bytes(
    pdb: &Pdb<FileReader>,
    dbi: &Dbi,
    report: &mut Report,
) -> Outcome<Option<Vec<u8>>> {
    let mut headers = None;
    for (slot, id) in dbi.debug_streams().enumerate() {
        let Some(id) = id else { continue };
        report.claim(id);
        if slot == DebugStream::SectionHeader.index() {
            headers = Some(pdb.stream_bytes(id)?);
        } else if slot == DebugStream::OriginalSectionHeader.index() {
            SectionHeaders::parse(&pdb.stream_bytes(id)?)?;
        } else {
            report.undecoded(format!("optional debug stream slot {slot}"));
        }
    }
    Ok(headers)
}

/// Decode the DBI substreams and return the module count.
fn debug_info_tables(
    dbi: &Dbi,
    addresses: Option<&AddressMap<'_>>,
    report: &mut Report,
) -> Outcome<usize> {
    let mut module_count = 0;
    for module in dbi.modules() {
        module?;
        module_count += 1;
    }
    report.add("modules", module_count);
    if let Some(contributions) = dbi.section_contributions()? {
        for contribution in &contributions {
            if usize::from(contribution.module) >= module_count {
                return Err("section contribution names a missing module".into());
            }
            let offset = u32::try_from(contribution.offset)?;
            translate(addresses, contribution.section, offset, report)?;
            report.add("section contributions", 1);
        }
    }
    if let Some(map) = dbi.section_map()? {
        report.add("section map entries", map.iter().count());
    }
    if let Some(sources) = dbi.source_info()? {
        for module in sources.modules() {
            for name in module.file_names() {
                name?;
                report.add("module source files", 1);
            }
        }
    }
    if let Some(ec) = dbi.ec_names()? {
        ec.verify_buckets()?;
        report.add("EC names", ec.entries().count());
    }
    dbi.verify_type_server_map()?;
    Ok(module_count)
}

/// Convert a section-relative address to an RVA and count the result.
fn translate(
    addresses: Option<&AddressMap<'_>>,
    section: u16,
    offset: u32,
    report: &mut Report,
) -> Outcome {
    let Some(map) = addresses else {
        report.add("addresses without section headers", 1);
        return Ok(());
    };
    match map.rva(SectionOffset { section, offset }) {
        Ok(_) => report.add("addresses translated to RVAs", 1),
        // Section 0 and the absolute segment after the last section have no RVA.
        Err(_) if section == 0 || usize::from(section) > map.sections().len() => {
            report.add("addresses without an RVA", 1);
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// Check the type, ID and address of a symbol.
fn check_symbol(symbol: &Symbol<'_>, context: &Context<'_>, report: &mut Report) -> Outcome {
    if let Some(index) = symbol.type_index()
        && !(index.primitive()?.is_some() || index < context.tpi_end)
    {
        return Err("symbol refers to a missing type".into());
    }
    if let Some(id) = symbol.id_index()
        && context.ids.get(id).is_none()
    {
        return Err("symbol refers to a missing ID".into());
    }
    if let Some(address) = symbol.address() {
        translate(
            context.addresses.as_ref(),
            address.section,
            address.offset,
            report,
        )?;
    }
    Ok(())
}

/// Decode every module stream and return the procedure offsets per module.
fn modules(
    pdb: &Pdb<FileReader>,
    dbi: &Dbi,
    context: &Context<'_>,
    report: &mut Report,
) -> Outcome<HashSet<(u16, u32)>> {
    let mut procedures = HashSet::new();
    for (index, module) in dbi.modules().enumerate() {
        let module = module?;
        let index = u16::try_from(index)?;
        let Some(stream) = pdb.module_stream(&module)? else {
            continue;
        };
        report.claim(stream.stream());
        let lines = ModuleLines::parse(&stream)?;
        lines.validate()?;
        let checksums = lines.checksums();
        let base = u32::try_from(stream.symbol_offset())?;
        let symbols =
            ModuleSymbols::new(stream.symbol_data(), stream.stream(), base, MAX_SCOPE_DEPTH);
        for scoped in symbols {
            let scoped = scoped?;
            let symbol = scoped.symbol;
            check_symbol(&symbol, context, report)?;
            match symbol.data() {
                SymbolData::Unsupported(_) => {
                    report.undecoded(format!("module symbol {:#06x}", symbol.kind()));
                }
                SymbolData::Procedure(_) => {
                    procedures.insert((index, scoped.offset));
                }
                SymbolData::InlineSite(site) => {
                    for annotation in &site.annotations {
                        if let Annotation::ChangeFile(offset) = annotation
                            && !checksums.is_some_and(|table| table.contains(offset))
                        {
                            return Err("inline site names a missing file checksum".into());
                        }
                    }
                }
                _ => {}
            }
            report.add("module symbols", 1);
        }
        module_lines(&lines, context, report)?;
    }
    Ok(procedures)
}

/// Resolve the file names and inlinees of one module's line data.
fn module_lines(lines: &ModuleLines<'_>, context: &Context<'_>, report: &mut Report) -> Outcome {
    let checksums = lines.checksums();
    let file_name = |offset: u32| -> Outcome {
        let entry = checksums
            .and_then(|table| table.get(offset))
            .ok_or("line data names a missing file checksum")?;
        context.names.get(entry.name_offset)?;
        Ok(())
    };
    for subsection in lines.subsections() {
        match &subsection.data {
            LineSubsection::Lines(data) => {
                let header = data.header();
                translate(
                    context.addresses.as_ref(),
                    header.segment,
                    header.code_offset,
                    report,
                )?;
                for block in data.blocks() {
                    file_name(block.checksum_offset)?;
                    report.add("line entries", block.lines.len());
                }
            }
            LineSubsection::FileChecksums(table) => {
                for entry in table {
                    context.names.get(entry.name_offset)?;
                    report.add("file checksums", 1);
                }
            }
            LineSubsection::InlineeLines(data) => {
                for entry in data {
                    match context.ids.decode(entry.inlinee)? {
                        IdRecord::Function(_) | IdRecord::MemberFunction(_) => {}
                        _ => return Err("inlinee line names an ID that is not a function".into()),
                    }
                    file_name(entry.checksum_offset)?;
                    for extra in entry.extra_files {
                        file_name(extra)?;
                    }
                    report.add("inlinee lines", 1);
                }
            }
            _ => report.undecoded(format!("C13 subsection {:#x}", subsection.kind)),
        }
        report.add("C13 subsections", 1);
    }
    Ok(())
}

/// Decode the shared symbol records and the global and public indexes.
fn shared_symbols(
    pdb: &Pdb<FileReader>,
    dbi: &Dbi,
    context: &Context<'_>,
    procedures: &HashSet<(u16, u32)>,
    report: &mut Report,
) -> Outcome {
    let Some(id) = dbi.header().symbol_stream else {
        return Ok(());
    };
    report.claim(id);
    let bytes = pdb.stream_bytes(id)?;
    let offsets = RecordOffsets::frame(&bytes, id)?;
    for record in Records::new(&bytes) {
        let record = record?;
        let symbol = record
            .symbol()
            .map_err(|error| error.with_stream(id.0, u64::from(record.offset)))?;
        check_symbol(&symbol, context, report)?;
        if let Some(reference) = symbol.procedure_reference() {
            let module = reference
                .module_index()
                .filter(|&module| usize::from(module) < context.module_count)
                .ok_or("procedure reference names a missing module")?;
            if !procedures.contains(&(module, reference.offset)) {
                return Err("procedure reference does not name a module procedure".into());
            }
        }
        if matches!(symbol.data(), SymbolData::Unsupported(_)) {
            report.undecoded(format!("shared symbol {:#06x}", symbol.kind()));
        }
        report.add("shared symbols", 1);
    }
    let mut indexed = HashSet::new();
    if let Some(globals) = pdb.global_symbols(dbi)? {
        report.claim(dbi.header().global_stream.ok_or("missing global stream")?);
        globals.hash_table().check_records(&offsets)?;
        check_lookup("global", globals.hash_table(), &bytes, &mut indexed, report)?;
        report.add("global index records", globals.hash_table().len());
    }
    if let Some(publics) = pdb.public_symbols(dbi)? {
        report.claim(dbi.header().public_stream.ok_or("missing public stream")?);
        publics.check_records(&offsets)?;
        publics.check_address_order(|offset| {
            let symbol = offset.record(&bytes)?.symbol()?;
            let address = symbol.address().ok_or_else(|| {
                pdbrs::Error::new(pdbrs::ErrorKind::InvalidReference, None)
                    .with_context("public address map names a symbol without an address")
            })?;
            Ok((address.section, address.offset))
        })?;
        check_lookup("public", publics.hash_table(), &bytes, &mut indexed, report)?;
        report.add("public index records", publics.hash_table().len());
    }
    report.add(
        "shared symbols outside both indexes",
        offsets.len() - indexed.len(),
    );
    Ok(())
}

/// Make sure that name lookup finds every record of a symbol hash table.
///
/// A record outside the bucket of its name is a producer defect: lookup by
/// name cannot find it. The example reports it and continues.
fn check_lookup(
    label: &str,
    table: HashTable<'_>,
    bytes: &[u8],
    indexed: &mut HashSet<u32>,
    report: &mut Report,
) -> Outcome {
    for entry in table.records() {
        let symbol = entry.symbol.record(bytes)?.symbol()?;
        let name = symbol.name().ok_or("indexed symbol has no name")?;
        if !table
            .lookup(name)
            .any(|candidate| candidate.symbol == entry.symbol)
        {
            report.inconsistency(format!(
                "{label} index record at symbol offset {} is outside the bucket of {}",
                entry.symbol.0,
                String::from_utf8_lossy(name)
            ));
        }
        indexed.insert(entry.symbol.0);
    }
    Ok(())
}

/// Report present streams that no decoder read.
fn unclaimed_streams(pdb: &Pdb<FileReader>, report: &mut Report) -> Outcome {
    for index in 0..pdb.msf().stream_count() {
        if pdb.msf().stream(StreamId(index))?.is_some() && !report.claimed.contains(&index) {
            report.undecoded(format!("stream {index}"));
        }
    }
    report.add("streams read", report.claimed.len());
    Ok(())
}

fn print(report: &Report) {
    for (name, count) in &report.counts {
        println!("{name}: {count}");
    }
    println!("undecoded items: {}", report.undecoded.len());
    for item in &report.undecoded {
        println!("  {item}");
    }
    println!("inconsistencies: {}", report.inconsistencies.len());
    for item in &report.inconsistencies {
        println!("  {item}");
    }
}
