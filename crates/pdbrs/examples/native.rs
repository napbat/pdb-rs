//! Read every stream and record of a native PDB and report undecoded data.
//!
//! The example decodes each stream role, checks the references between
//! streams and prints counts. It stops at the first malformed structure.
//! A recognized format that this crate does not decode, for example a record
//! kind without a decoder, is counted as undecoded and reading continues.
//! At the end, the example fails if anything is undecoded. It reports
//! producer defects that do not stop reading, for example an index record
//! outside the bucket of its name, without failing. It also reports decoded
//! producer data that no stream names, with the reason, without failing.
//! Names stay bytes.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::error::Error;

use pdbrs::io::FileReader;
use pdbrs::native::Pdb;
use pdbrs::native::address::{AddressMap, Rva, SectionOffset};
use pdbrs::native::auxiliary::{
    LinkInfo, NamedStreamKind, TmCache, TypeMapKind, TypeMapStream, UdtSourceLineUndone,
};
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::id_record::{IdRecord, IdTable};
use pdbrs::native::codeview::index::{IdIndex, TypeIndex};
use pdbrs::native::codeview::symbol::{
    Annotation, ModuleSymbols, Symbol, SymbolData, Trampoline, TrampolineKind,
};
use pdbrs::native::codeview::type_record::TypeLeaf;
use pdbrs::native::dbi::{Dbi, DebugStream};
use pdbrs::native::injected_source::{SourceHeaderBlock, content_stream_name};
use pdbrs::native::lines::{LineSubsection, ModuleLines};
use pdbrs::native::msf::{OldDirectory, StreamId};
use pdbrs::native::names::StringTable;
use pdbrs::native::section::SectionHeaders;
use pdbrs::native::symbols::{HashTable, PublicHeader, PublicSymbolIndex, RecordOffsets};
use pdbrs::native::types::{ForwardRefs, HEADER_SIZE, TypeHashes, TypeStream, TypeTable};
use pdbrs::{ErrorKind, Result as PdbResult};

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
    /// Data without a typed decoder, with the number of occurrences.
    undecoded: BTreeMap<String, usize>,
    /// Decoded data that breaks a producer rule but does not stop reading.
    inconsistencies: Vec<String>,
    /// Decoded streams without an owner, with the reason and the number of occurrences.
    unowned: BTreeMap<String, usize>,
    /// The bytes of the type map streams that `/TMCache` names.
    type_maps: HashSet<Vec<u8>>,
}

impl Report {
    fn claim(&mut self, id: StreamId) {
        self.claimed.insert(id.0);
    }

    fn add(&mut self, name: &'static str, count: usize) {
        *self.counts.entry(name).or_default() += count;
    }

    fn undecoded(&mut self, what: String) {
        *self.undecoded.entry(what).or_default() += 1;
    }

    fn inconsistency(&mut self, what: String) {
        self.inconsistencies.push(what);
    }

    /// Return the value, or count an unsupported format and return `None`.
    ///
    /// Errors other than [`ErrorKind::UnsupportedVersion`] stop the read.
    fn supported<T>(&mut self, result: PdbResult<T>, what: &str) -> Outcome<Option<T>> {
        match result {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == ErrorKind::UnsupportedVersion => {
                let reason = error.context().unwrap_or("unsupported format");
                self.undecoded(format!("{what}: {reason}"));
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
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
    if let Some(placement) = report.supported(names.verify_buckets(), "/names hashing")? {
        report.add(
            "/names strings",
            usize::try_from(placement.home)? + usize::try_from(placement.probed)?,
        );
    }
    let tpi = pdb.types()?.ok_or("missing TPI stream")?;
    let ipi = pdb.ids()?.ok_or("missing IPI stream")?;
    report.claim(StreamId(2));
    report.claim(StreamId(4));
    let tpi_hash_bytes = hash_stream_bytes(&pdb, &tpi, &mut report)?;
    let ipi_hash_bytes = hash_stream_bytes(&pdb, &ipi, &mut report)?;
    named_streams(&pdb, &names, &tpi, &ipi, &mut report)?;
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
    frame_data(&pdb, &dbi, &context, &mut report)?;
    let procedures = modules(&pdb, &dbi, &context, &mut report)?;
    shared_symbols(&pdb, &dbi, &context, &procedures, &mut report)?;
    unclaimed_streams(&pdb, &mut report)?;
    print(&report);
    let undecoded: usize = report.undecoded.values().sum();
    if undecoded == 0 {
        Ok(())
    } else {
        Err(format!("{undecoded} items have no typed decoder").into())
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
        report.undecoded("auxiliary type hash stream".to_owned());
    }
    Ok(Some(pdb.stream_bytes(id)?))
}

/// Decode every named stream and verify the injected sources.
fn named_streams(
    pdb: &Pdb<FileReader>,
    names: &StringTable<'_>,
    tpi: &TypeStream,
    ipi: &TypeStream,
    report: &mut Report,
) -> Outcome {
    let mut handled = HashSet::new();
    let info = pdb.info();
    for named in info.named_streams() {
        report.claim(named.stream);
        let bytes = pdb.stream_bytes(named.stream)?;
        match NamedStreamKind::classify(named.name) {
            NamedStreamKind::Names | NamedStreamKind::SourceFile { .. } => {}
            NamedStreamKind::LinkInfo => {
                report.supported(LinkInfo::parse(&bytes, named.stream), "/LinkInfo")?;
            }
            NamedStreamKind::TmCache => {
                let parsed = TmCache::parse(&bytes, named.stream);
                if let Some(cache) = report.supported(parsed, "/TMCache")? {
                    let ends = [tpi.header().index_end, ipi.header().index_end];
                    type_maps(pdb, &cache, ends, report)?;
                }
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
                let parsed = SourceHeaderBlock::parse(&bytes, named.stream);
                let Some(Some(block)) = report.supported(parsed, "/src/headerblock")? else {
                    continue;
                };
                for entry in block.entries() {
                    let source = entry.value;
                    names.get(source.file_name_offset)?;
                    names.get(source.object_name_offset)?;
                    let virtual_name = names.get(source.virtual_file_name_offset)?;
                    let stream_name = content_stream_name(virtual_name)?;
                    let id = info
                        .named_stream(&stream_name)
                        .ok_or("injected source has no content stream")?;
                    let content = pdb.stream_bytes(id)?;
                    report.supported(source.verify_content(&content, id), "injected source")?;
                    handled.insert(id);
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
        if content && !handled.contains(&named.stream) {
            report.undecoded("injected source stream without a header entry".to_owned());
        }
    }
    Ok(())
}

/// Decode the type map streams that `/TMCache` names, with their precompiled-header maps.
///
/// `ends` holds the TPI and IPI index ends.
fn type_maps(
    pdb: &Pdb<FileReader>,
    cache: &TmCache<'_>,
    ends: [u32; 2],
    report: &mut Report,
) -> Outcome {
    let mut pending: Vec<StreamId> = cache.streams().flatten().collect();
    let mut seen = HashSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        report.claim(id);
        let bytes = pdb.stream_bytes(id)?;
        let map = TypeMapStream::parse(&bytes, id)?;
        if let TypeMapKind::Object {
            precompiled: Some(precompiled),
            ..
        } = map.kind()
        {
            pending.push(precompiled);
        }
        check_type_map(&map, ends)?;
        report.add("type map streams", 1);
        report.type_maps.insert(bytes);
    }
    report.add("/TMCache entries", cache.entries().len());
    Ok(())
}

/// Check that the map values and the function ID pairs of a type map are in range.
fn check_type_map(map: &TypeMapStream<'_>, [tpi_end, ipi_end]: [u32; 2]) -> Outcome {
    let within = |value: u32, end: u32| value == 0 || (0x1000..end).contains(&value);
    let count = |len: usize| u32::try_from(len).map_err(|_| "type map is too long");
    let (type_end, function_ids) = if map.kind() == TypeMapKind::TypeServer {
        if !map.ids().all(|value| within(value, ipi_end)) {
            return Err("type map names an ID past the IPI stream".into());
        }
        (tpi_end, 0x1000..0x1000 + count(map.ids().len())?)
    } else {
        let first = map.first_index();
        let end = first
            .checked_add(count(map.types().len())?)
            .ok_or("type map index range overflows")?;
        (tpi_end.max(ipi_end), first..end)
    };
    if !map.types().all(|value| within(value, type_end)) {
        return Err("type map names an index past the TPI and IPI streams".into());
    }
    for (function, ty) in map.function_types() {
        if !function_ids.contains(&function) || !(0x1000..tpi_end).contains(&ty) {
            return Err("type map function ID pair is out of range".into());
        }
    }
    Ok(())
}

/// Decode every TPI record, its field lists and its forward references.
fn types(tpi: &TypeStream, hash_bytes: Option<&[u8]>, report: &mut Report) -> Outcome {
    let parsed = hash_bytes
        .map(|bytes| TypeHashes::parse(tpi.header(), bytes))
        .transpose();
    let hashes = report.supported(parsed, "TPI hash stream")?.flatten();
    if let Some(hashes) = &hashes {
        report.supported(hashes.verify(tpi), "TPI hash values")?;
    }
    let table = TypeTable::new(tpi)?;
    let forward = report.supported(
        ForwardRefs::new(&table, hashes.as_ref()),
        "TPI forward-reference index",
    )?;
    let base = u64::try_from(HEADER_SIZE)?;
    let (mut resolved, mut unresolved) = (0, 0);
    for record in tpi.records() {
        let record = record?;
        report.add("TPI records", 1);
        let location =
            |error: pdbrs::Error| error.with_stream(2, base + u64::from(record.record.offset));
        let decoded = TypeLeaf::decode(record.record).map_err(location);
        let what = format!("TPI leaf {:#06x}", record.record.kind);
        let Some(leaf) = report.supported(decoded, &what)? else {
            continue;
        };
        match &leaf {
            TypeLeaf::Unsupported(raw) => {
                report.undecoded(format!("TPI leaf {:#06x}", raw.kind));
            }
            TypeLeaf::FieldList(_) => {
                for member in table.members(TypeIndex(record.index)).map_err(location)? {
                    if report
                        .supported(member.map_err(location), "field-list member")?
                        .is_none()
                    {
                        break;
                    }
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
        let forward_reference = leaf
            .tag()
            .is_some_and(|tag| tag.options.is_forward_reference());
        if let (true, Some(forward)) = (forward_reference, &forward) {
            match forward.resolve(TypeIndex(record.index))? {
                Some(_) => resolved += 1,
                None => unresolved += 1,
            }
        }
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
    if let Some(bytes) = hash_bytes
        && let Some(hashes) =
            report.supported(TypeHashes::parse(ipi.header(), bytes), "IPI hash stream")?
    {
        report.supported(hashes.verify(ipi), "IPI hash values")?;
    }
    let ids = IdTable::new(ipi)?;
    report.supported(
        ids.validate(TypeIndex(tpi.header().index_end)),
        "IPI references",
    )?;
    let mut text = Vec::new();
    for record in ipi.records() {
        let id = IdIndex(record?.index);
        report.add("IPI records", 1);
        let Some(decoded) = report.supported(ids.decode(id), "IPI record")? else {
            continue;
        };
        match decoded {
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
    }
    Ok(ids)
}

/// Load the section-header stream and claim every optional debug stream.
///
/// [`frame_data`] decodes the FPO and new-FPO streams.
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
        } else if slot != DebugStream::Fpo.index() && slot != DebugStream::NewFpo.index() {
            report.undecoded(format!("optional debug stream slot {slot}"));
        }
    }
    Ok(headers)
}

/// Decode the FPO and new-FPO streams, and resolve each frame program in `/names`.
fn frame_data(
    pdb: &Pdb<FileReader>,
    dbi: &Dbi,
    context: &Context<'_>,
    report: &mut Report,
) -> Outcome {
    let mut starts = Vec::new();
    if let Some(fpo) = pdb.fpo(dbi)? {
        starts.extend(fpo.iter().map(|record| record.start));
        report.add("FPO records", fpo.len());
    }
    if let Some(frames) = pdb.frame_data(dbi)? {
        for frame in &frames {
            context.names.get(frame.program)?;
            starts.push(frame.start);
        }
        report.add("frame data records", frames.len());
    }
    if let Some(addresses) = &context.addresses {
        for start in starts {
            addresses.section_offset(Rva(start))?;
        }
    }
    Ok(())
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
    if let Some(ec) = dbi.ec_names()?
        && report
            .supported(ec.verify_buckets(), "EC name hashing")?
            .is_some()
    {
        report.add("EC names", ec.entries().count());
    }
    report.supported(dbi.verify_type_server_map(), "DBI type server map")?;
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
        // OMAP translation is not implemented.
        Err(error) if error.kind() == ErrorKind::UnsupportedVersion => {
            report.add("addresses not translated (OMAP)", 1);
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

/// The module records that the shared symbol checks need.
#[derive(Default)]
struct Procedures {
    /// The module index and module-stream offset of each decoded procedure.
    offsets: HashSet<(u16, u32)>,
    /// Modules whose symbol walk stopped at a scope that is not decoded.
    incomplete: HashSet<u16>,
    /// The `S_TRAMPOLINE` records of all modules.
    trampolines: Vec<Trampoline>,
}

/// Decode every module stream and return the procedure offsets per module.
fn modules(
    pdb: &Pdb<FileReader>,
    dbi: &Dbi,
    context: &Context<'_>,
    report: &mut Report,
) -> Outcome<Procedures> {
    let mut procedures = Procedures::default();
    for (index, module) in dbi.modules().enumerate() {
        let module = module?;
        let index = u16::try_from(index)?;
        let Some(stream) = pdb.module_stream(&module)? else {
            continue;
        };
        report.claim(stream.stream());
        let lines = report.supported(ModuleLines::parse(&stream), "C13 line data")?;
        if let Some(lines) = &lines {
            lines.validate()?;
        }
        let checksums = lines.as_ref().and_then(ModuleLines::checksums);
        let base = u32::try_from(stream.symbol_offset())?;
        let symbols =
            ModuleSymbols::new(stream.symbol_data(), stream.stream(), base, MAX_SCOPE_DEPTH);
        for scoped in symbols {
            let Some(scoped) = report.supported(scoped, "module symbol scope")? else {
                procedures.incomplete.insert(index);
                break;
            };
            let symbol = scoped.symbol;
            check_symbol(&symbol, context, report)?;
            match symbol.data() {
                SymbolData::Unsupported(_) => {
                    report.undecoded(format!("module symbol {:#06x}", symbol.kind()));
                }
                SymbolData::Procedure(_) => {
                    procedures.offsets.insert((index, scoped.offset));
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
                SymbolData::FileStatic(value) => {
                    context.names.get(value.module_name_offset)?;
                    report.add("file statics joined to /names", 1);
                }
                SymbolData::Trampoline(value) => procedures.trampolines.push(*value),
                SymbolData::FastLink(_) => report.add("FASTLINK references", 1),
                _ => {}
            }
            report.add("module symbols", 1);
        }
        if let Some(lines) = &lines {
            module_lines(lines, context, report)?;
        }
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
            LineSubsection::CoffSymbolRvas(rvas) => coff_symbol_rvas(rvas.iter(), context, report)?,
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
    procedures: &Procedures,
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
            if procedures.incomplete.contains(&module) {
                report.add("procedure references into partly decoded modules", 1);
            } else if !procedures.offsets.contains(&(module, reference.offset)) {
                return Err("procedure reference does not name a module procedure".into());
            }
        }
        if matches!(symbol.data(), SymbolData::Unsupported(_)) {
            report.undecoded(format!("shared symbol {:#06x}", symbol.kind()));
        }
        report.add("shared symbols", 1);
    }
    let mut indexed = HashSet::new();
    let globals = report.supported(pdb.global_symbols(dbi), "global symbol index")?;
    if let Some(globals) = globals.flatten() {
        report.claim(dbi.header().global_stream.ok_or("missing global stream")?);
        globals.hash_table().check_records(&offsets)?;
        check_lookup("global", globals.hash_table(), &bytes, &mut indexed, report)?;
        report.add("global index records", globals.hash_table().len());
    }
    let publics = report.supported(pdb.public_symbols(dbi), "public symbol index")?;
    if let Some(publics) = publics.flatten() {
        report.claim(dbi.header().public_stream.ok_or("missing public stream")?);
        publics.check_records(&offsets)?;
        publics.check_address_order(|offset| {
            let symbol = offset.record(&bytes)?.symbol()?;
            let address = symbol.address().ok_or_else(|| {
                pdbrs::Error::new(ErrorKind::InvalidReference, None)
                    .with_context("public address map names a symbol without an address")
            })?;
            Ok((address.section, address.offset))
        })?;
        check_lookup("public", publics.hash_table(), &bytes, &mut indexed, report)?;
        report.add("public index records", publics.hash_table().len());
        join_trampolines(&publics, &procedures.trampolines, report);
    }
    report.add(
        "shared symbols outside both indexes",
        offsets.len() - indexed.len(),
    );
    Ok(())
}

/// Make sure that each COFF symbol RVA is 0, 0xffffffff or inside a section.
///
/// In the MSVC FASTLINK fixture, 0 marks a record without an image address
/// and 0xffffffff marks an external symbol that the linker resolved to an
/// absolute value, for example `__guard_flags` (#13).
fn coff_symbol_rvas(
    rvas: impl Iterator<Item = u32>,
    context: &Context<'_>,
    report: &mut Report,
) -> Outcome {
    for rva in rvas {
        if rva == 0 {
            report.add("COFF symbol RVAs without an address", 1);
        } else if rva == u32::MAX {
            report.add("COFF symbol RVAs of absolute symbols", 1);
        } else if let Some(map) = &context.addresses {
            map.section_offset(Rva(rva))?;
            report.add("COFF symbol RVAs inside a section", 1);
        } else {
            report.add("addresses without section headers", 1);
        }
    }
    Ok(())
}

/// Join the incremental trampolines to the public thunk map.
///
/// Thunk `i` is at `thunk_table_offset + i * thunk_size` in the thunk table
/// section. Thunk map entry `i` is the flat address of the thunk target: the
/// section start (`SO` entry) of the target section plus the target offset.
fn join_trampolines(publics: &PublicSymbolIndex, trampolines: &[Trampoline], report: &mut Report) {
    let header = publics.header();
    if trampolines.is_empty() && header.thunk_count == 0 {
        return;
    }
    let starts: HashMap<u16, u32> = publics
        .section_starts()
        .map(|start| (start.section, start.offset))
        .collect();
    let targets: Vec<u32> = publics.thunk_map().collect();
    let mut covered = HashSet::new();
    let (mut joined, mut mismatched) = (0, 0);
    for trampoline in trampolines {
        if trampoline.kind != TrampolineKind::INCREMENTAL {
            report.add("trampolines that are not incremental thunks", 1);
            continue;
        }
        let target = starts
            .get(&trampoline.target.section)
            .and_then(|start| start.checked_add(trampoline.target.offset));
        match thunk_index(header, trampoline) {
            Some(index) if target.is_some() && targets.get(index) == target.as_ref() => {
                covered.insert(index);
                joined += 1;
            }
            _ => mismatched += 1,
        }
    }
    report.add("trampolines joined to the thunk map", joined);
    report.add("trampolines without a matching thunk map entry", mismatched);
    report.add(
        "thunk map entries without a trampoline",
        targets.len() - covered.len(),
    );
    if mismatched != 0 {
        report.inconsistency(format!(
            "{mismatched} trampolines do not match the public thunk map"
        ));
    }
}

/// Return the index of the trampoline thunk in the thunk table, if the table
/// holds it.
fn thunk_index(header: &PublicHeader, trampoline: &Trampoline) -> Option<usize> {
    if trampoline.thunk.section != header.thunk_table_section
        || u32::from(trampoline.thunk_size) != header.thunk_size
    {
        return None;
    }
    let delta = trampoline
        .thunk
        .offset
        .checked_sub(header.thunk_table_offset)?;
    if delta.checked_rem(header.thunk_size)? != 0 {
        return None;
    }
    usize::try_from(delta / header.thunk_size).ok()
}

/// Make sure that name lookup finds every record of a symbol hash table.
///
/// A record outside the bucket of its name is a producer defect: lookup by
/// name cannot find it. The example reports it and continues. A record of a
/// kind without a decoder has no known name; the shared symbol pass already
/// counts it as undecoded.
fn check_lookup(
    label: &str,
    table: HashTable<'_>,
    bytes: &[u8],
    indexed: &mut HashSet<u32>,
    report: &mut Report,
) -> Outcome {
    for entry in table.records() {
        indexed.insert(entry.symbol.0);
        let symbol = entry.symbol.record(bytes)?.symbol()?;
        let Some(name) = symbol.name() else {
            if matches!(symbol.data(), SymbolData::Unsupported(_)) {
                continue;
            }
            return Err("indexed symbol has no name".into());
        };
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
    }
    Ok(())
}

/// Report present streams that no decoder read.
///
/// An unclaimed stream with the same bytes as a type map stream that `/TMCache` names is an
/// unowned type map: two inputs with the same type section checksum share one `/TMCache` entry,
/// and the entry keeps only the last stream (#13).
fn unclaimed_streams(pdb: &Pdb<FileReader>, report: &mut Report) -> Outcome {
    for index in 0..pdb.msf().stream_count() {
        let id = StreamId(index);
        if pdb.msf().stream(id)?.is_none() || report.claimed.contains(&index) {
            continue;
        }
        let bytes = pdb.stream_bytes(id)?;
        if report.type_maps.contains(&bytes) && TypeMapStream::parse(&bytes, id).is_ok() {
            let reason = "type map stream that /TMCache does not name (same bytes as a named one)";
            *report.unowned.entry(reason.to_owned()).or_default() += 1;
        } else {
            report.undecoded("stream without a known role".to_owned());
        }
    }
    report.add("streams read", report.claimed.len());
    Ok(())
}

fn print(report: &Report) {
    for (name, count) in &report.counts {
        println!("{name}: {count}");
    }
    let undecoded: usize = report.undecoded.values().sum();
    println!("undecoded items: {undecoded}");
    for (item, count) in &report.undecoded {
        println!("  {count} x {item}");
    }
    println!("inconsistencies: {}", report.inconsistencies.len());
    for item in &report.inconsistencies {
        println!("  {item}");
    }
    let unowned: usize = report.unowned.values().sum();
    println!("decoded streams without an owner: {unowned}");
    for (item, count) in &report.unowned {
        println!("  {count} x {item}");
    }
}
