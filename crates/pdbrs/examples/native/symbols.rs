//! The module streams, the shared symbol records and the global and public indexes.

use std::collections::{HashMap, HashSet};

use pdbrs::ErrorKind;
use pdbrs::io::ReadAt;
use pdbrs::native::Pdb;
use pdbrs::native::address::Rva;
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::id_record::IdRecord;
use pdbrs::native::codeview::index::{ChecksumOffset, ModuleSymbolOffset};
use pdbrs::native::codeview::symbol::{
    Annotation, AttributeValue, CustomDebugInfoData, ModuleSymbolTarget, ModuleSymbolWriter,
    ModuleSymbols, Oem, Symbol, SymbolData, Trampoline, TrampolineKind,
};
use pdbrs::native::dbi::{Dbi, ModuleStream};
use pdbrs::native::lines::{LineSubsection, ModuleLines};
use pdbrs::native::symbols::{
    HashTable, PublicHeader, PublicSymbolIndex, RecordOffsets, SymbolRecordWriter,
};

use crate::dbi::translate;
use crate::symbol_encoding::{reencode, rewrite_module};
use crate::{Context, Outcome, Report};

/// The deepest scope nesting that the example accepts in a module.
const MAX_SCOPE_DEPTH: usize = 256;

/// Check the type, ID and address of a symbol.
fn check_symbol(symbol: &Symbol<'_>, context: &Context<'_>, report: &mut Report) -> Outcome {
    if let Some(index) = symbol.type_index()
        && context.indices_checked
        && !(index.primitive()?.is_some() || index < context.tpi_end)
    {
        return Err("symbol refers to a missing type".into());
    }
    // `/DEBUG:LAZY` funclets have ID 0 ("no ID"); the library check accepts it too.
    if let Some(id) = symbol.id_index()
        && context.indices_checked
        && id.0 != 0
        && context.ids.record(id).is_none()
    {
        return Err(format!(
            "symbol {:#06x} refers to a missing ID {:#x}",
            symbol.kind(),
            id.0
        )
        .into());
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
pub(crate) struct Procedures {
    /// The module index and module-stream offset of each decoded procedure.
    offsets: HashSet<(u16, u32)>,
    /// Modules whose symbol walk stopped at a scope that is not decoded.
    incomplete: HashSet<u16>,
    /// The `S_TRAMPOLINE` records of all modules.
    trampolines: Vec<Trampoline>,
    /// The record kinds of the module streams for the library checks.
    records: crate::symbol_validation::ModuleRecords,
}

/// Decode every module stream and return the procedure offsets per module.
///
/// In a PDB without type merging (`NOTM`), the type and ID indices of a module resolve against
/// the records that its `S_MOD_TYPEREF` record names, not against TPI and IPI.
pub(crate) fn modules(
    pdb: &Pdb<impl ReadAt>,
    dbi: &Dbi,
    context: &Context<'_>,
    report: &mut Report,
) -> Outcome<Procedures> {
    let mut procedures = Procedures::default();
    for module in dbi.modules() {
        let (module_index, _) = module?;
        let index = u16::try_from(module_index.0)?;
        let Some(stream) = pdb.module(module_index)? else {
            continue;
        };
        report.claim(stream.stream());
        match crate::module_types::streams(pdb, &stream, report)? {
            Some(types) => crate::module_types::with_context(&types, context, |context| {
                module_symbols(&stream, index, context, &mut procedures, report)
            })?,
            None => module_symbols(&stream, index, context, &mut procedures, report)?,
        }
    }
    Ok(procedures)
}

/// Decode and check the symbols and the line data of one module stream.
fn module_symbols(
    stream: &ModuleStream,
    index: u16,
    context: &Context<'_>,
    procedures: &mut Procedures,
    report: &mut Report,
) -> Outcome {
    let lines = report.supported(ModuleLines::parse(stream), "C13 line data")?;
    if let Some(lines) = &lines {
        lines.check()?;
    }
    let checksums = lines.as_ref().and_then(ModuleLines::checksums);
    let base = ModuleSymbolOffset(u32::try_from(stream.symbol_offset())?);
    crate::symbol_validation::module(stream, index, context, &mut procedures.records, report)?;
    let symbols = ModuleSymbols::new(stream.symbol_data(), stream.stream(), base, MAX_SCOPE_DEPTH);
    let mut writer = Some(ModuleSymbolWriter::new(base));
    for scoped in symbols {
        let Some(scoped) = report.supported(scoped, "module symbol scope")? else {
            procedures.incomplete.insert(index);
            break;
        };
        let symbol = scoped.symbol;
        if let Some(links) = symbol.links()
            && links.parent != scoped.scope.unwrap_or_default()
        {
            return Err("a scope parent link is not the enclosing scope".into());
        }
        check_symbol(&symbol, context, report)?;
        reencode(&symbol, report)?;
        if let Some(writer) = &mut writer {
            writer.push_symbol(&symbol)?;
        }
        match symbol.data() {
            SymbolData::Unsupported(_) => {
                report.undecoded(format!("module symbol {:#06x}", symbol.kind()));
            }
            SymbolData::Procedure(_) | SymbolData::ManagedProcedure(_) => {
                procedures.offsets.insert((index, scoped.offset.0));
            }
            SymbolData::InlineSite(site) => {
                for annotation in &site.annotations {
                    if let Annotation::ChangeFile(offset) = annotation
                        && !checksums.is_some_and(|table| table.contains(ChecksumOffset(offset)))
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
            SymbolData::Oem(oem) => oem_attribute(oem, report)?,
            _ => {}
        }
        report.add("module symbols", 1);
    }
    if !procedures.incomplete.contains(&index) {
        rewrite_module(writer.take(), stream.symbol_data(), report)?;
    }
    if let Some(lines) = &lines {
        module_lines(lines, context, report)?;
    }
    Ok(())
}

/// Decode the symbol attribute of an `S_OEM` record and its Roslyn value.
fn oem_attribute(oem: &Oem<'_>, report: &mut Report) -> Outcome {
    let Some(attribute) = oem.attribute()? else {
        report.undecoded("S_OEM record with an unknown OEM ID".to_owned());
        return Ok(());
    };
    match report.supported(attribute.decode(), "symbol attribute")? {
        Some(AttributeValue::CustomDebugInfo(info)) => {
            for record in info.records() {
                let data = report.supported(record.decode(), "custom debug information")?;
                if let Some(CustomDebugInfoData::Unknown(_)) = data {
                    report.undecoded(format!("custom debug information kind {}", record.kind.0));
                }
                report.add("custom debug information records", 1);
            }
        }
        Some(AttributeValue::AsyncMethodInfo(info)) => {
            report.add("async methods", 1);
            report.add("async method steps", info.steps.len());
        }
        Some(_) => {
            let name: Vec<u16> = attribute.name_units().collect();
            report.undecoded(format!(
                "symbol attribute {}",
                String::from_utf16_lossy(&name)
            ));
        }
        None => {}
    }
    Ok(())
}

/// Resolve the file names and inlinees of one module's line data.
fn module_lines(lines: &ModuleLines<'_>, context: &Context<'_>, report: &mut Report) -> Outcome {
    let checksums = lines.checksums();
    let file_name = |offset: ChecksumOffset| -> Outcome {
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
                    if context.indices_checked {
                        match context.ids.leaf(entry.inlinee)? {
                            IdRecord::Function(_) | IdRecord::MemberFunction(_) => {}
                            _ => {
                                return Err(
                                    "inlinee line names an ID that is not a function".into()
                                );
                            }
                        }
                    }
                    file_name(entry.checksum_offset)?;
                    for extra in &entry.extra_files {
                        file_name(extra)?;
                    }
                    report.add("inlinee lines", 1);
                }
            }
            LineSubsection::CoffSymbolRvas(rvas) => coff_symbol_rvas(rvas.iter(), context, report)?,
            other => crate::lines::other_subsection(other, &file_name, context, report)?,
        }
        report.add("C13 subsections", 1);
    }
    Ok(())
}

/// Decode the shared symbol records and the global and public indexes.
pub(crate) fn shared_symbols(
    pdb: &Pdb<impl ReadAt>,
    dbi: &Dbi,
    context: &Context<'_>,
    procedures: &Procedures,
    report: &mut Report,
) -> Outcome {
    let Some(id) = dbi.header().symbol_stream else {
        return Ok(());
    };
    report.claim(id);
    let bytes = pdb
        .symbol_records()?
        .ok_or("missing symbol record stream")?;
    let offsets = RecordOffsets::frame(bytes, id)?;
    crate::symbol_validation::shared(bytes, id, context, &procedures.records, report)?;
    let mut writer = SymbolRecordWriter::new();
    for record in Records::new(bytes) {
        let record = record?;
        let symbol = Symbol::decode(record)
            .map_err(|error| error.with_stream(id, u64::from(record.offset)))?;
        check_symbol(&symbol, context, report)?;
        reencode(&symbol, report)?;
        if writer.push_symbol(&symbol)?.0 != record.offset {
            return Err("a rewritten symbol record moves to another offset".into());
        }
        if let Some(reference) = symbol.procedure_reference() {
            let ModuleSymbolTarget::Position { module, offset } = reference.target else {
                return Err("a decoded procedure reference has a handle target".into());
            };
            let module = module
                .and_then(|module| u16::try_from(module.0).ok())
                .filter(|&module| usize::from(module) < context.module_count)
                .ok_or("procedure reference names a missing module")?;
            if procedures.incomplete.contains(&module) {
                report.add("procedure references into partly decoded modules", 1);
            } else if !procedures.offsets.contains(&(module, offset.0)) {
                return Err("procedure reference does not name a module procedure".into());
            }
        }
        if matches!(symbol.data(), SymbolData::Unsupported(_)) {
            report.undecoded(format!("shared symbol {:#06x}", symbol.kind()));
        }
        report.add("shared symbols", 1);
    }
    let mut indexed = HashSet::new();
    let globals = report.supported(pdb.globals(), "global symbol index")?;
    if let Some(globals) = globals.flatten().map(|globals| globals.index()) {
        report.claim(dbi.header().global_stream.ok_or("missing global stream")?);
        globals.hash_table().check_records(&offsets)?;
        check_lookup("global", globals.hash_table(), bytes, &mut indexed, report)?;
        report.add("global index records", globals.hash_table().len());
    }
    let publics = report.supported(pdb.publics(), "public symbol index")?;
    if let Some(publics) = publics.flatten().map(|publics| publics.index()) {
        report.claim(dbi.header().public_stream.ok_or("missing public stream")?);
        publics.check_records(&offsets)?;
        publics.check_address_order(|offset| {
            let symbol = Symbol::decode(offset.record(bytes)?)?;
            let address = symbol.address().ok_or_else(|| {
                pdbrs::Error::new(ErrorKind::InvalidReference, None)
                    .with_context("public address map names a symbol without an address")
            })?;
            Ok((address.section, address.offset))
        })?;
        check_lookup("public", publics.hash_table(), bytes, &mut indexed, report)?;
        report.add("public index records", publics.hash_table().len());
        join_trampolines(publics, &procedures.trampolines, report);
    }
    report.add(
        "shared symbols outside both indexes",
        offsets.len() - indexed.len(),
    );
    let rebuilt = writer.finish();
    report.add("symbol record streams rewritten", 1);
    // The run stops at the first difference, so a finished run prints 0.
    report.add("symbol record streams rewritten with different bytes", 0);
    if rebuilt.bytes != bytes {
        return Err("the symbol record stream rewrites to different bytes".into());
    }
    // The index builders read the names from the rewritten stream.
    crate::symbol_indexes::symbol_indexes(pdb, dbi, &rebuilt.bytes, report)?;
    Ok(())
}

/// Make sure that each COFF symbol RVA is 0, 0xffffffff or inside a section.
///
/// In the MSVC FASTLINK fixture, 0 marks a record without an image address
/// and 0xffffffff marks an external symbol that the linker resolved to an
/// absolute value, for example `__guard_flags` (#13).
fn coff_symbol_rvas(
    rvas: impl Iterator<Item = Rva>,
    context: &Context<'_>,
    report: &mut Report,
) -> Outcome {
    for rva in rvas {
        if rva == Rva(0) {
            report.add("COFF symbol RVAs without an address", 1);
        } else if rva == Rva(u32::MAX) {
            report.add("COFF symbol RVAs of absolute symbols", 1);
        } else if let Some(map) = &context.addresses {
            map.section_offset(rva)?;
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
        let symbol = Symbol::decode(entry.symbol.record(bytes)?)?;
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
