//! The strip of private symbols, as Microsoft `link /PDBSTRIPPED` does.
//!
//! The rules come from the stripped PDB that `link.exe` 14.51 writes next to
//! the full PDB of the `msvc-x64-stripped` fixture (#27, #34). The
//! documentation of [`PdbEditor`](super::PdbEditor) lists them.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::opaque::{self, Scope};
use super::plan::{Changes, Context, missing};
use super::{IPI_STREAM, TPI_STREAM, push, unsupported};
use crate::io::ReadAt;
use crate::native::Pdb;
use crate::native::auxiliary::{NamedStreamKind, TmCache};
use crate::native::codeview::index::{ModuleIndex, ModuleSymbolOffset, TypeIndex};
use crate::native::codeview::symbol::{
    ModuleSymbolWriter, Procedure, ProcedureReference, ProcedureType, S_END, S_LMANPROC, S_LPROC32,
    S_LPROC32_DPC, S_LPROC32_DPC_ID, S_LPROC32_ID, S_LPROCREF, S_PUB32, Symbol, SymbolData,
};
use crate::native::codeview::{Records, SymbolKind};
use crate::native::dbi::{DbiBuilder, DebugStream, ModuleStreamParts, OwnedSourceInfo};
use crate::native::info::{Feature, InfoBuilder};
use crate::native::msf::StreamId;
use crate::native::names::{HashVersion, StringTableBuilder};
use crate::native::output::no_memory;
use crate::native::symbols::{
    GlobalSymbolIndex, HashEntry, PublicSymbolIndex, SymbolOffset, ThunkTable,
};
use crate::native::types::{TypeHeader, TypeStreamOptions, encode_type_stream};
use crate::{Error, ErrorKind, Result};

/// The module stream offset of the first symbol record: after the signature.
const SYMBOL_BASE: ModuleSymbolOffset = ModuleSymbolOffset(4);
/// The DBI header flag of a stripped PDB (`fStripped`).
const STRIPPED_FLAG: u16 = 1 << 1;

/// A record of the source symbol record stream that the strip keeps.
#[derive(Clone, Copy, Debug)]
struct Kept {
    /// The bucket of the record in its index.
    bucket: u32,
    /// The reference count of the record in its index.
    reference_count: u32,
    /// `true` for an `S_LPROCREF` record of the global index, `false` for an
    /// `S_PUB32` record of the public index.
    global: bool,
}

/// Return the error for a reference that does not resolve.
fn unresolved(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidReference, None).with_context(context)
}

/// Strip the TPI or IPI stream `id`: no records and an empty hash stream.
fn strip_types(id: StreamId, header: &TypeHeader, changes: &mut Changes) -> Result<()> {
    let options = TypeStreamOptions::from_header(header, &[]);
    let written = encode_type_stream(&[], &options)?;
    if let (Some(hash), Some(stream)) = (written.hash_stream, header.hash_stream) {
        changes.replace(stream, hash)?;
    }
    if let Some(stream) = header.hash_aux_stream {
        changes.replace(stream, Vec::new())?;
    }
    changes.replace(id, written.stream)
}

/// Return the local procedure record of a stripped module: type 0, no
/// links, and the kind without an ID index.
fn stripped_procedure<'a>(symbol: &Symbol<'a>) -> Result<(SymbolKind, SymbolData<'a>)> {
    match (symbol.kind(), *symbol.data()) {
        (
            S_LPROC32 | S_LPROC32_ID | S_LPROC32_DPC | S_LPROC32_DPC_ID,
            SymbolData::Procedure(procedure),
        ) => {
            let kind = if matches!(symbol.kind(), S_LPROC32 | S_LPROC32_ID) {
                S_LPROC32
            } else {
                S_LPROC32_DPC
            };
            Ok((
                kind,
                SymbolData::Procedure(Procedure {
                    parent: ModuleSymbolOffset(0),
                    end: ModuleSymbolOffset(0),
                    next: ModuleSymbolOffset(0),
                    signature: ProcedureType::Type(TypeIndex::NONE),
                    ..procedure
                }),
            ))
        }
        (S_LMANPROC, SymbolData::ManagedProcedure(procedure)) => Ok((
            S_LMANPROC,
            SymbolData::ManagedProcedure(crate::native::codeview::symbol::ManagedProcedure {
                parent: ModuleSymbolOffset(0),
                end: ModuleSymbolOffset(0),
                next: ModuleSymbolOffset(0),
                ..procedure
            }),
        )),
        _ => Err(unsupported(
            "an S_LPROCREF record names a record that the strip does not support",
        )),
    }
}

/// Return the procedure reference of an `S_LPROCREF` record.
fn local_reference<'a>(symbol: &Symbol<'a>) -> Option<ProcedureReference<'a>> {
    match (symbol.kind(), *symbol.data()) {
        (S_LPROCREF, SymbolData::ProcedureReference(reference)) => Some(reference),
        _ => None,
    }
}

/// Return the 0-based module index of a reference as the `u16` key of the
/// strip maps, or `None` for module number 0.
fn reference_module(reference: &ProcedureReference<'_>) -> Option<u16> {
    reference
        .module
        .map(|ModuleIndex(index)| u16::try_from(index).unwrap_or(u16::MAX))
}

/// Collect the records that the strip keeps, by source offset.
fn kept_records(
    records: &[u8],
    globals: Option<&GlobalSymbolIndex>,
    publics: Option<&PublicSymbolIndex>,
) -> Result<BTreeMap<u32, Kept>> {
    let mut kept = BTreeMap::new();
    if let Some(publics) = publics {
        for bucket in publics.hash_table().buckets() {
            for record in bucket.records {
                kept.insert(
                    record.symbol.0,
                    Kept {
                        bucket: bucket.index,
                        reference_count: record.reference_count,
                        global: false,
                    },
                );
            }
        }
    }
    if let Some(globals) = globals {
        for bucket in globals.hash_table().buckets() {
            for record in bucket.records {
                let symbol = Symbol::decode(record.symbol.record(records)?)?;
                if local_reference(&symbol).is_some() {
                    kept.insert(
                        record.symbol.0,
                        Kept {
                            bucket: bucket.index,
                            reference_count: record.reference_count,
                            global: true,
                        },
                    );
                }
            }
        }
    }
    Ok(kept)
}

/// The stripped symbol regions of the modules and the new offset of each
/// kept procedure, by module index and source module offset.
struct ModuleRegions {
    /// The symbol records of each module, or `None` for a module without a
    /// kept procedure.
    regions: Vec<Option<Vec<u8>>>,
    /// The new module offset of each kept procedure.
    offsets: BTreeMap<(u16, u32), ModuleSymbolOffset>,
}

/// Build the stripped symbol region of each module.
fn module_regions<S: ReadAt>(
    pdb: &Pdb<S>,
    context: &Context<'_>,
    records: &[u8],
    kept: &BTreeMap<u32, Kept>,
) -> Result<ModuleRegions> {
    let mut targets = BTreeMap::<u16, Vec<u32>>::new();
    for (&offset, entry) in kept {
        if !entry.global {
            continue;
        }
        let symbol = Symbol::decode(SymbolOffset(offset).record(records)?)?;
        let reference = local_reference(&symbol)
            .ok_or_else(|| unresolved("kept global record is not an S_LPROCREF record"))?;
        let module = reference_module(&reference)
            .ok_or_else(|| unresolved("S_LPROCREF record has module number 0"))?;
        let list = targets.entry(module).or_default();
        push(list, reference.offset.0)?;
    }
    let mut regions = Vec::new();
    let mut offsets = BTreeMap::new();
    for module in context.dbi.modules() {
        let module = module?;
        let index = u16::try_from(module.index.0)
            .map_err(|_| unsupported("PDB has more than 65535 modules"))?;
        let Some(mut list) = targets.remove(&index) else {
            push(&mut regions, None)?;
            continue;
        };
        list.sort_unstable();
        list.dedup();
        let stream = pdb
            .load_module(&module)?
            .ok_or_else(|| unresolved("S_LPROCREF record names a module without a stream"))?;
        let base = u32::try_from(stream.symbol_offset()).unwrap_or(u32::MAX);
        let mut writer = ModuleSymbolWriter::new(SYMBOL_BASE);
        for offset in list {
            let record = offset
                .checked_sub(base)
                .and_then(|start| stream.symbol_data().get(start as usize..))
                .and_then(|rest| Records::new(rest).next())
                .ok_or_else(|| unresolved("S_LPROCREF offset is outside the module symbols"))??;
            let (kind, data) = stripped_procedure(&Symbol::decode(record)?)?;
            let new = writer.push(kind, &data)?;
            writer.push(S_END, &SymbolData::ScopeEnd)?;
            offsets.insert((index, offset), new);
        }
        push(&mut regions, Some(writer.finish()?))?;
    }
    if !targets.is_empty() {
        return Err(unresolved(
            "S_LPROCREF record names a module that the DBI does not have",
        ));
    }
    Ok(ModuleRegions { regions, offsets })
}

/// The stripped symbol record stream and its hash entries.
struct SharedRecords {
    /// The stream.
    bytes: Vec<u8>,
    /// The entries of the global index.
    globals: Vec<HashEntry>,
    /// The entries of the public index.
    publics: Vec<HashEntry>,
    /// The new offsets of the kept `S_LPROCREF` records of each module, in
    /// stream order.
    module_refs: BTreeMap<u16, Vec<SymbolOffset>>,
}

/// Copy the kept records in source order. Give each `S_LPROCREF` record the
/// new offset of its procedure.
fn shared_records(
    records: &[u8],
    kept: &BTreeMap<u32, Kept>,
    modules: &ModuleRegions,
) -> Result<SharedRecords> {
    let mut shared = SharedRecords {
        bytes: Vec::new(),
        globals: Vec::new(),
        publics: Vec::new(),
        module_refs: BTreeMap::new(),
    };
    for record in Records::new(records) {
        let record = record?;
        let Some(entry) = kept.get(&record.offset) else {
            continue;
        };
        let offset = SymbolOffset(u32::try_from(shared.bytes.len()).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("symbol record stream does not fit in 32 bits")
        })?);
        let symbol = Symbol::decode(record)?;
        let hash = HashEntry {
            symbol: offset,
            reference_count: entry.reference_count,
            bucket: Some(entry.bucket),
        };
        if entry.global {
            let mut reference = local_reference(&symbol)
                .ok_or_else(|| unresolved("kept global record is not an S_LPROCREF record"))?;
            let module = reference_module(&reference).unwrap_or(u16::MAX);
            reference.offset = *modules
                .offsets
                .get(&(module, reference.offset.0))
                .ok_or_else(|| unresolved("S_LPROCREF record names no kept procedure"))?;
            SymbolData::ProcedureReference(reference).encode(S_LPROCREF, &mut shared.bytes)?;
            push(&mut shared.globals, hash)?;
            push(shared.module_refs.entry(module).or_default(), offset)?;
        } else {
            if symbol.kind() != S_PUB32 {
                return Err(unresolved(
                    "public index names a record that is not S_PUB32",
                ));
            }
            let start = record.offset as usize;
            let bytes = &records[start..start + 4 + record.data.len()];
            shared
                .bytes
                .try_reserve(bytes.len())
                .map_err(|_| no_memory())?;
            shared.bytes.extend_from_slice(bytes);
            push(&mut shared.publics, hash)?;
        }
    }
    Ok(shared)
}

/// Strip the PDB. Return the DBI stream values; the caller sets the age and
/// encodes the stream.
pub(super) fn plan<S: ReadAt>(
    pdb: &Pdb<S>,
    context: &Context<'_>,
    info: &mut InfoBuilder,
    changes: &mut Changes,
) -> Result<DbiBuilder> {
    // The strip removes all type records, so it removes the type maps
    // without an owner as it removes the type maps that `/TMCache` names.
    // `link /DEBUG:LAZY /PDBSTRIPPED` also writes no module type streams,
    // and the stripped PDB has no `NOTM` feature code.
    let streams = opaque::check(pdb, context, Scope::Streams)?;
    for stream in streams.type_maps.into_iter().chain(streams.module_types) {
        changes.remove(stream)?;
    }
    if info
        .features()
        .any(|feature| feature == Feature::NO_TYPE_MERGE)
    {
        let features: Vec<Feature> = info
            .features()
            .filter(|&feature| feature != Feature::NO_TYPE_MERGE)
            .collect();
        info.set_features(&features)?;
    }
    strip_types(TPI_STREAM, context.tpi.header(), changes)?;
    if let Some(ipi) = pdb.ids()? {
        strip_types(IPI_STREAM, ipi.header(), changes)?;
    }
    let header = context.dbi.header();
    let symbol_stream = header
        .symbol_stream
        .ok_or_else(|| missing("PDB has no symbol record stream"))?;
    let records = pdb
        .symbol_records()?
        .ok_or_else(|| missing("PDB has no symbol record stream"))?;
    let globals = pdb.globals()?.map(|globals| globals.index());
    let publics = pdb.publics()?.map(|publics| publics.index());
    let kept = kept_records(records, globals, publics)?;
    let modules = module_regions(pdb, context, records, &kept)?;
    let mut shared = shared_records(records, &kept, &modules)?;
    let buckets = pdb.symbol_bucket_count();
    if let Some(id) = header.global_stream {
        let mut bytes = Vec::new();
        GlobalSymbolIndex::encode_parts(
            &shared.bytes,
            &shared.globals,
            buckets,
            context.index_producer,
            &mut bytes,
        )?;
        changes.replace(id, bytes)?;
    }
    if let (Some(id), Some(publics)) = (header.public_stream, publics) {
        let targets: Vec<u32> = publics.thunk_map().collect();
        let starts: Vec<_> = publics.section_starts().collect();
        let source = publics.header();
        let thunks = ThunkTable {
            thunk_size: source.thunk_size,
            section: source.thunk_table_section,
            offset: source.thunk_table_offset,
            targets: &targets,
        };
        let mut bytes = Vec::new();
        PublicSymbolIndex::encode_parts(
            &shared.bytes,
            &shared.publics,
            buckets,
            context.index_producer,
            thunks,
            &starts,
            &mut bytes,
        )?;
        changes.replace(id, bytes)?;
    }
    let mut builder = DbiBuilder::from_dbi(context.dbi)?;
    builder.header.flags |= STRIPPED_FLAG;
    for ((index, record), region) in (0_u16..).zip(&mut builder.modules).zip(modules.regions) {
        let source = record.stream;
        record.c11_bytes = 0;
        record.c13_bytes = 0;
        record.source_file_count = 0;
        record.source_file_name_index = 0;
        record.pdb_file_path_index = 0;
        zero_crcs(&mut record.section_contribution);
        let (Some(region), Some(stream)) = (region, source) else {
            if let Some(stream) = source {
                changes.remove(stream)?;
            }
            record.stream = None;
            record.symbol_bytes = 0;
            continue;
        };
        let refs = shared.module_refs.remove(&index).unwrap_or_default();
        let mut bytes = Vec::new();
        let sizes = ModuleStreamParts {
            symbols: Some(&region),
            c11: &[],
            c13: &[],
            global_refs: &refs,
        }
        .encode(&mut bytes)?;
        record.symbol_bytes = sizes.symbol_bytes;
        changes.replace(stream, bytes)?;
    }
    for contribution in &mut builder.section_contributions {
        zero_crcs(contribution);
    }
    if builder.source_info.is_some() {
        builder.source_info = Some(OwnedSourceInfo::from_file_names(
            builder.modules.iter().map(|_| core::iter::empty::<&[u8]>()),
        )?);
    }
    if builder.ec_names.is_some() {
        // The EC names are source and PDB paths of the modules.
        builder.ec_names = Some(StringTableBuilder::new(HashVersion::V1)?);
    }
    changes.replace(symbol_stream, shared.bytes)?;
    strip_named(
        pdb,
        context.dbi.debug_stream(DebugStream::NewFpo).is_some(),
        info,
        changes,
    )?;
    Ok(builder)
}

/// Set the CRCs of a section contribution to 0, as `link /PDBSTRIPPED` does.
fn zero_crcs(contribution: &mut crate::native::dbi::SectionContribution) {
    if contribution.data_crc.is_some() {
        contribution.data_crc = Some(0);
    }
    if contribution.reloc_crc.is_some() {
        contribution.reloc_crc = Some(0);
    }
}

/// Remove `/names` and the injected source streams, and empty `/TMCache`
/// and `/UDTSRCLINEUNDONE`.
///
/// `/names` stays if `keep_names` is set: the frame programs of a new FPO
/// stream refer to it.
fn strip_named<S: ReadAt>(
    pdb: &Pdb<S>,
    keep_names: bool,
    info: &mut InfoBuilder,
    changes: &mut Changes,
) -> Result<()> {
    for named in pdb.info().named_streams() {
        let stream = named.stream;
        let remove = match NamedStreamKind::classify(named.name) {
            NamedStreamKind::Names => !keep_names,
            NamedStreamKind::SourceHeaderBlock | NamedStreamKind::SourceFile { .. } => true,
            NamedStreamKind::TmCache => {
                let bytes = pdb.stream_bytes(stream)?;
                for map in TmCache::parse(&bytes, stream)?.streams().flatten() {
                    changes.remove(map)?;
                }
                if !bytes.is_empty() {
                    changes.replace(stream, Vec::new())?;
                }
                false
            }
            NamedStreamKind::UdtSourceLineUndone => {
                if pdb.container().stream(stream)?.is_some_and(|reader| {
                    crate::io::ReadAt::len(&reader).is_ok_and(|length| length != 0)
                }) {
                    changes.replace(stream, Vec::new())?;
                }
                false
            }
            _ => false,
        };
        if remove {
            info.remove_named_stream(named.name)?;
            changes.remove(stream)?;
        }
    }
    Ok(())
}
