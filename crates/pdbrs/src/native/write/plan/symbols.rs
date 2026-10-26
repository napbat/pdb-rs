//! The module streams, the symbol record stream and the symbol indexes of a
//! write plan.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::super::PdbBuilder;
use super::super::handle::Resolver;
use super::super::module::ModuleBuilder;
use super::types::{PlannedTypes, id_end};
use super::{BuildOptions, StreamIds};
use crate::native::codeview::Records;
use crate::native::codeview::id_record::IdTable;
use crate::native::codeview::index::{IdIndex, TypeIndex};
use crate::native::codeview::symbol::ModuleSymbolWriter;
use crate::native::codeview::symbol::validate::{
    ModuleRecord, Rule, ValidationContext, ValidationReport, validate_module_symbols,
    validate_shared_symbols,
};
use crate::native::dbi::{ModuleStream, ModuleStreamParts, OwnedModule};
use crate::native::info::Feature;
use crate::native::lines::ModuleLines;
use crate::native::output::no_memory;
use crate::native::symbols::{
    BucketCount, GlobalSymbolIndex, HashEntry, PublicSymbolIndex, SymbolOffset, SymbolRecordWriter,
};
use crate::{Error, ErrorKind, Result};

/// The module stream offset of the first symbol record: after the signature.
const SYMBOL_BASE: u32 = 4;

/// A planned module: its record and its stream.
pub(in crate::native::write) struct PlannedModule {
    /// The module record with the stream and the region sizes.
    pub(in crate::native::write) record: OwnedModule,
    /// The module stream, or `None`.
    pub(in crate::native::write) bytes: Option<Vec<u8>>,
}

/// The encoded symbol data of a plan.
pub(in crate::native::write) struct EncodedSymbols {
    /// The modules in module order.
    pub(in crate::native::write) modules: Vec<PlannedModule>,
    /// The module stream offset of each symbol record of each module.
    pub(in crate::native::write) module_offsets: Vec<Vec<u32>>,
    /// The offset of each record of the symbol record stream.
    pub(in crate::native::write) symbol_offsets: Vec<SymbolOffset>,
    /// The symbol record stream.
    pub(in crate::native::write) symbols: Vec<u8>,
    /// The global symbol index stream.
    pub(in crate::native::write) globals: Vec<u8>,
    /// The public symbol index stream.
    pub(in crate::native::write) publics: Vec<u8>,
}

/// The symbol records and the C13 region of one module before its global
/// references resolve.
struct ModuleRegions {
    /// The symbol records without the signature, or `None`.
    symbols: Option<Vec<u8>>,
    /// The C13 region.
    c13: Vec<u8>,
}

/// Return `true` if the plan rejects a violation of `rule`.
///
/// The plan rejects broken framing and references that do not resolve. The
/// other rules describe producer conventions; the plan keeps such records.
const fn rejected(rule: Rule) -> bool {
    matches!(
        rule,
        Rule::Walk
            | Rule::TypeIndexRange
            | Rule::IdIndexRange
            | Rule::ModuleIndex
            | Rule::ModuleRecordMissing
    )
}

/// Return the first rejected violation of `report` as an error.
fn check(report: &ValidationReport) -> Result<()> {
    match report
        .violations
        .iter()
        .find(|violation| rejected(violation.rule))
    {
        Some(violation) => Err(violation.error()),
        None => Ok(()),
    }
}

/// Return an error for a module without a stream that has stream contents.
fn streamless() -> Error {
    Error::new(ErrorKind::InvalidFormat, None)
        .with_context("module without a module stream has symbols, subsections or references")
}

/// Encode the symbols and the C13 subsections of a module.
///
/// Return the regions and the module stream offset of each symbol record.
fn module_regions(
    module: &ModuleBuilder<'_>,
    resolver: &Resolver<'_>,
) -> Result<(ModuleRegions, Vec<u32>)> {
    let mut offsets = Vec::new();
    let symbols = match &module.symbols {
        None => None,
        Some(entries) => {
            offsets
                .try_reserve_exact(entries.len())
                .map_err(|_| no_memory())?;
            let mut writer = ModuleSymbolWriter::new(SYMBOL_BASE);
            for entry in entries {
                offsets.push(entry.push_module(&mut writer, resolver)?);
            }
            Some(writer.finish()?)
        }
    };
    let mut c13 = Vec::new();
    for subsection in &module.subsections {
        super::super::entry::encode_subsection(subsection, resolver, &mut c13)?;
    }
    Ok((ModuleRegions { symbols, c13 }, offsets))
}

/// Return the kinds of the module records by module index and offset.
fn record_kinds(regions: &[Option<ModuleRegions>]) -> Result<BTreeMap<(u16, u32), u16>> {
    let mut kinds = BTreeMap::new();
    for (index, region) in regions.iter().enumerate() {
        let Some(symbols) = region.as_ref().and_then(|region| region.symbols.as_ref()) else {
            continue;
        };
        let module = u16::try_from(index).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("PDB has more than 65535 modules")
        })?;
        for record in Records::new(symbols) {
            let record = record?;
            kinds.insert((module, record.offset + SYMBOL_BASE), record.kind);
        }
    }
    Ok(kinds)
}

impl PdbBuilder<'_> {
    /// Encode the module streams, the symbol record stream and the indexes.
    pub(super) fn encode_symbols(
        &self,
        resolver: &Resolver<'_>,
        ids: &StreamIds,
        tpi: &PlannedTypes,
        ipi: &PlannedTypes,
        options: &BuildOptions,
    ) -> Result<EncodedSymbols> {
        let id_table = IdTable::new(&ipi.parsed)?;
        let id_kind = |id: IdIndex| id_table.get(id).map(|record| record.kind);
        let base = ValidationContext::new(TypeIndex(tpi.index_end()), id_end(ipi))
            .with_id_kinds(&id_kind)
            .with_module_count(self.modules.len());
        let mut regions = Vec::new();
        let mut module_offsets = Vec::new();
        regions
            .try_reserve_exact(self.modules.len())
            .map_err(|_| no_memory())?;
        module_offsets
            .try_reserve_exact(self.modules.len())
            .map_err(|_| no_memory())?;
        for (module, stream) in self.modules.iter().zip(&ids.modules) {
            if stream.is_none() {
                if module
                    .symbols
                    .as_ref()
                    .is_some_and(|symbols| !symbols.is_empty())
                    || !module.subsections.is_empty()
                    || !module.global_refs.is_empty()
                {
                    return Err(streamless());
                }
                regions.push(None);
                module_offsets.push(Vec::new());
                continue;
            }
            let (region, offsets) = module_regions(module, resolver)?;
            regions.push(Some(region));
            module_offsets.push(offsets);
        }
        self.check_modules(&regions, ids, &base)?;
        let kinds = record_kinds(&regions)?;
        let module_record = |module: u16, offset: u32| {
            kinds
                .get(&(module, offset))
                .map_or(ModuleRecord::Missing, |&kind| ModuleRecord::Kind(kind))
        };
        let shared = Resolver {
            module_offsets: Some(&module_offsets),
            ..*resolver
        };
        let mut writer = SymbolRecordWriter::new();
        for symbol in &self.symbols {
            symbol.entry.push_shared(&mut writer, &shared)?;
        }
        let stream = writer.finish();
        check(&validate_shared_symbols(
            &stream.bytes,
            ids.symbols,
            &base.with_module_records(&module_record),
        )?)?;
        let modules = self.join_modules(regions, ids, &stream.offsets)?;
        let (globals, publics) = self.symbol_indexes(&stream.bytes, &stream.offsets, options)?;
        Ok(EncodedSymbols {
            modules,
            module_offsets,
            symbol_offsets: stream.offsets,
            symbols: stream.bytes,
            globals,
            publics,
        })
    }

    /// Check the references of the module symbols.
    ///
    /// The type and ID indices of a module with an `S_MOD_TYPEREF` record
    /// resolve against the records that it names, not against TPI and IPI.
    fn check_modules(
        &self,
        regions: &[Option<ModuleRegions>],
        ids: &StreamIds,
        base: &ValidationContext<'_>,
    ) -> Result<()> {
        let mut symbols = Vec::new();
        symbols
            .try_reserve_exact(regions.len())
            .map_err(|_| no_memory())?;
        symbols.extend(
            regions
                .iter()
                .map(|region| region.as_ref().and_then(|region| region.symbols.as_deref())),
        );
        let (types, streams) = self.module_types(ids, &symbols)?;
        for ((records, types), stream) in symbols.iter().zip(types).zip(&ids.modules) {
            let (Some(records), Some(stream)) = (records, stream) else {
                continue;
            };
            streams.with_context(types, base, self.modules.len(), |context| {
                check(&validate_module_symbols(
                    records,
                    *stream,
                    SYMBOL_BASE,
                    context,
                )?)
            })?;
        }
        Ok(())
    }

    /// Join the module streams with their global references and set the
    /// module records.
    fn join_modules(
        &self,
        regions: Vec<Option<ModuleRegions>>,
        ids: &StreamIds,
        symbol_offsets: &[SymbolOffset],
    ) -> Result<Vec<PlannedModule>> {
        let mut planned = Vec::new();
        planned
            .try_reserve_exact(self.modules.len())
            .map_err(|_| no_memory())?;
        for ((module, region), stream) in self.modules.iter().zip(regions).zip(&ids.modules) {
            let mut record = module.record.clone();
            record.source_file_count = u16::try_from(module.files.len()).map_err(|_| {
                Error::new(ErrorKind::LimitExceeded, None)
                    .with_context("module has more than 65535 source files")
            })?;
            record.stream = *stream;
            let (Some(region), Some(stream)) = (region, *stream) else {
                record.symbol_bytes = 0;
                record.c11_bytes = 0;
                record.c13_bytes = 0;
                planned.push(PlannedModule {
                    record,
                    bytes: None,
                });
                continue;
            };
            let refs = global_refs(module, symbol_offsets)?;
            let encoded = ModuleStreamParts {
                symbols: region.symbols.as_deref(),
                c11: &[],
                c13: &region.c13,
                global_refs: &refs,
            }
            .encode()
            .map_err(|error| error.with_stream(stream.0, 0))?;
            record.symbol_bytes = encoded.sizes.symbol_bytes;
            record.c11_bytes = encoded.sizes.c11_bytes;
            record.c13_bytes = encoded.sizes.c13_bytes;
            let parsed = ModuleStream::parse(encoded.bytes, &record.as_module())?;
            ModuleLines::parse(&parsed)?.validate()?;
            planned.push(PlannedModule {
                record,
                bytes: Some(parsed.into_bytes()),
            });
        }
        Ok(planned)
    }

    /// Build the global and public symbol index streams.
    fn symbol_indexes(
        &self,
        records: &[u8],
        offsets: &[SymbolOffset],
        options: &BuildOptions,
    ) -> Result<(Vec<u8>, Vec<u8>)> {
        let buckets = BucketCount::from_features(self.info.features().map(Feature::code));
        let mut globals = Vec::new();
        let mut publics = Vec::new();
        for (symbol, &offset) in self.symbols.iter().zip(offsets) {
            if let Some(reference_count) = symbol.global {
                globals.try_reserve(1).map_err(|_| no_memory())?;
                globals.push(HashEntry {
                    symbol: offset,
                    reference_count,
                    bucket: None,
                });
            }
            if symbol.public {
                publics.try_reserve(1).map_err(|_| no_memory())?;
                publics.push(HashEntry::new(offset));
            }
        }
        let globals = GlobalSymbolIndex::build(records, &globals, buckets, options.index_producer)?;
        let publics = PublicSymbolIndex::build(
            records,
            &publics,
            buckets,
            options.index_producer,
            self.thunks,
            &self.section_starts,
        )?;
        Ok((globals, publics))
    }
}

/// Resolve the global references of a module.
fn global_refs(module: &ModuleBuilder<'_>, offsets: &[SymbolOffset]) -> Result<Vec<SymbolOffset>> {
    let mut refs = Vec::new();
    refs.try_reserve_exact(module.global_refs.len())
        .map_err(|_| no_memory())?;
    for handle in &module.global_refs {
        refs.push(
            usize::try_from(handle.0)
                .ok()
                .and_then(|index| offsets.get(index))
                .copied()
                .ok_or_else(|| {
                    super::super::handle::unresolved("global reference has no symbol record")
                })?,
        );
    }
    Ok(refs)
}
