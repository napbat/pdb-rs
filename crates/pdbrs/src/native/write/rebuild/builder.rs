//! The conversion of decoded PDB contents to a builder.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::{NamedContent, PdbContents};
use crate::native::Identity;
use crate::native::auxiliary::{
    LinkInfo, NamedStreamKind, SourceLink, SourceServer, TmCache, TmCacheBuilder, TypeMapStream,
    UdtSourceLineUndone,
};
use crate::native::codeview::Records;
use crate::native::codeview::id_record::IdRecord;
use crate::native::codeview::symbol::{
    ProcedureReference, S_ANNOTATIONREF, S_DATAREF, S_LPROCREF, S_PROCREF, S_TOKENREF, Symbol,
    SymbolData,
};
use crate::native::codeview::type_record::TypeLeaf;
use crate::native::dbi::OwnedModule;
use crate::native::hash_table::HashTableProducer;
use crate::native::info::IdentityPolicy;
use crate::native::injected_source::SourceHeaderBlock;
use crate::native::lines::ModuleLines;
use crate::native::msf::StreamId;
use crate::native::names::{StringTable, StringTableBuilder};
use crate::native::symbols::{IndexProducer, SymbolOffset, ThunkTable};
use crate::native::types::{HashLayout, TypeHashes, TypeStream};
use crate::native::write::{
    BuildOptions, IdHandle, ModuleSymbolHandle, PdbBuilder, StreamHandle, SymbolHandle, TypeHandle,
};
use crate::{Error, ErrorKind, Result};

/// The handles of the module symbol records by module index and module
/// stream offset.
type ModuleSymbols = BTreeMap<(u16, u32), ModuleSymbolHandle>;

/// Return the error for data that the rebuild cannot decode.
fn unsupported(context: &'static str) -> Error {
    Error::new(ErrorKind::UnsupportedVersion, None).with_context(context)
}

/// Return the position of a record with `index` in a stream from `begin`.
fn position(index: u32, begin: u32) -> Result<u32> {
    index.checked_sub(begin).ok_or_else(|| {
        Error::new(ErrorKind::InvalidReference, None)
            .with_context("record index is below the first index of its stream")
    })
}

impl PdbContents {
    /// Return the options that keep the producer conventions of the source.
    ///
    /// The TPI header selects the hash layout and the bucket count. A source
    /// with the LLVM hash layout also gets the LLVM conventions for the
    /// symbol indexes and the named-stream map.
    #[must_use]
    pub fn build_options(&self) -> BuildOptions {
        let header = self.tpi.0.header();
        let layout = HashLayout::of(header);
        let (index_producer, info_producer) = match layout {
            HashLayout::Llvm => (IndexProducer::Llvm, HashTableProducer::Llvm),
            HashLayout::Microsoft => (IndexProducer::Microsoft, HashTableProducer::Microsoft),
        };
        BuildOptions {
            hash_layout: layout,
            hash_buckets: header.hash_bucket_count,
            index_producer,
            info_producer,
            ..BuildOptions::default()
        }
    }

    /// Decode every record and give the decoded values to a new builder.
    ///
    /// `policy` sets the identity of the new PDB (see [`IdentityPolicy`]).
    /// Read the matching values for the image with
    /// [`PdbWritePlan::pe_match`](crate::native::write::PdbWritePlan::pe_match).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedVersion`] for data that the rebuild cannot
    ///   decode: a C11 line region, a named stream of an unknown kind or an
    ///   Info stream without a GUID identity.
    /// - [`ErrorKind::InvalidReference`] for a reference that does not
    ///   identify a record of the source.
    /// - The errors of the record decoders and of the builder.
    pub fn to_builder(&self, policy: IdentityPolicy) -> Result<PdbBuilder<'_>> {
        let Identity::Modern { guid, age } = self.info.identity() else {
            return Err(unsupported("PDB Info stream has no GUID identity"));
        };
        let mut builder = PdbBuilder::new(self.info.signature(), guid, age)?;
        let features: Vec<_> = self.info.typed_features().collect();
        builder.set_features(&features)?;
        builder.set_dbi_age(self.dbi.header().age);
        builder.apply_identity(policy)?;
        builder.set_names(match &self.names {
            Some(bytes) => Some(StringTableBuilder::from_table(&StringTable::parse(bytes)?)?),
            None => None,
        });
        self.add_types(&mut builder)?;
        self.add_dbi(&mut builder)?;
        let module_symbols = self.add_modules(&mut builder)?;
        let shared = self.add_shared_symbols(&mut builder, &module_symbols)?;
        self.add_global_refs(&mut builder, &shared)?;
        self.add_indexes(&mut builder, &shared)?;
        for (kind, bytes) in &self.debug {
            builder.set_debug_stream(*kind, bytes.clone())?;
        }
        self.add_named_streams(&mut builder)?;
        Ok(builder)
    }

    /// Add the TPI and IPI records and their hash adjusters.
    fn add_types<'a>(&'a self, builder: &mut PdbBuilder<'a>) -> Result<()> {
        let (tpi, tpi_hash) = &self.tpi;
        for record in tpi.records() {
            builder.add_type(TypeLeaf::decode(record?.record)?)?;
        }
        for adjuster in adjusters(tpi, tpi_hash.as_deref())? {
            let handle = TypeHandle(position(adjuster.index, tpi.header().index_begin)?);
            builder.add_type_hash_adjuster(handle, adjuster.name_offset)?;
        }
        if let Some((ipi, ipi_hash)) = &self.ipi {
            for record in ipi.records() {
                builder.add_id(IdRecord::decode(record?.record)?)?;
            }
            for adjuster in adjusters(ipi, ipi_hash.as_deref())? {
                let handle = IdHandle(position(adjuster.index, ipi.header().index_begin)?);
                builder.add_id_hash_adjuster(handle, adjuster.name_offset)?;
            }
        }
        Ok(())
    }

    /// Add the DBI header values and the substreams that the plan keeps.
    fn add_dbi(&self, builder: &mut PdbBuilder<'_>) -> Result<()> {
        let dbi = &self.dbi;
        builder.set_dbi_header(*dbi.header());
        if let Some(contributions) = dbi.section_contributions()? {
            let mut list = Vec::new();
            list.try_reserve_exact(contributions.len())
                .map_err(|_| crate::native::output::no_memory())?;
            list.extend(contributions.iter());
            builder.set_section_contributions(contributions.version(), list);
        }
        if let Some(map) = dbi.section_map()? {
            builder.set_section_map(Some(crate::native::dbi::OwnedSectionMap::from_map(&map)?));
        }
        builder.set_type_servers(dbi.type_server_map()?);
        if let Some(names) = dbi.ec_names()? {
            builder.set_ec_names(Some(StringTableBuilder::from_table(&names)?));
        }
        builder.set_debug_slot_count(dbi.debug_streams().count());
        Ok(())
    }

    /// Add the modules with their symbols, subsections and source files.
    fn add_modules<'a>(&'a self, builder: &mut PdbBuilder<'a>) -> Result<ModuleSymbols> {
        let mut handles = ModuleSymbols::new();
        let sources = self.dbi.source_info()?;
        let mut files = sources.map(|info| info.modules());
        for (module, stream) in self.dbi.modules().zip(&self.modules) {
            let module = module?;
            let index = u16::try_from(module.index)
                .map_err(|_| unsupported("PDB has more than 65535 modules"))?;
            let target = builder.add_module(OwnedModule::from_module(&module)?)?;
            if let Some(list) = files.as_mut().and_then(Iterator::next) {
                for name in list.file_names() {
                    target.push_file(name?)?;
                }
            }
            let Some(stream) = stream else {
                target.set_stream(false);
                continue;
            };
            if !stream.c11_data().is_empty() {
                return Err(unsupported("C11 line data is a legacy format (#16)"));
            }
            if stream.symbol_offset() == 0 {
                target.set_symbol_region(false)?;
            }
            let base = u32::try_from(stream.symbol_offset()).unwrap_or(u32::MAX);
            for record in stream.symbols() {
                let record = record?;
                let handle = target.push_symbol(record.symbol()?)?;
                handles.insert((index, record.offset + base), handle);
            }
            for subsection in ModuleLines::parse(stream)?.subsections() {
                target.push_subsection(subsection.data.clone())?;
            }
        }
        Ok(handles)
    }

    /// Add the records of the symbol record stream.
    ///
    /// Return the handle of each record by its source offset.
    fn add_shared_symbols<'a>(
        &'a self,
        builder: &mut PdbBuilder<'a>,
        modules: &ModuleSymbols,
    ) -> Result<BTreeMap<SymbolOffset, SymbolHandle>> {
        let mut handles = BTreeMap::new();
        for record in Records::new(&self.symbols) {
            let record = record?;
            let symbol = record.symbol()?;
            let handle = match module_target(&symbol, modules) {
                Some((reference, target)) => {
                    let kind = symbol.kind();
                    let wide = matches!(symbol.data(), SymbolData::SymbolReference(_));
                    builder.add_symbol_with(move |resolver, out| {
                        let mut reference = reference;
                        reference.offset = resolver.module_symbol_offset(target)?;
                        reference.module = resolver.module_number(target.module())?;
                        let data = if wide {
                            SymbolData::SymbolReference(reference)
                        } else {
                            SymbolData::ProcedureReference(reference)
                        };
                        data.encode(kind, out)
                    })?
                }
                None => builder.add_symbol(symbol)?,
            };
            handles.insert(SymbolOffset(record.offset), handle);
        }
        Ok(handles)
    }

    /// Add the global references of the module streams.
    fn add_global_refs(
        &self,
        builder: &mut PdbBuilder<'_>,
        shared: &BTreeMap<SymbolOffset, SymbolHandle>,
    ) -> Result<()> {
        for (index, stream) in self.modules.iter().enumerate() {
            let Some(stream) = stream else { continue };
            let handle =
                crate::native::write::ModuleHandle(u32::try_from(index).unwrap_or(u32::MAX));
            let module = builder.module_mut(handle)?;
            for offset in &stream.global_refs()? {
                module.push_global_ref(symbol_handle(shared, offset)?)?;
            }
        }
        Ok(())
    }

    /// Add the global and public index entries and the public index tables.
    fn add_indexes<'a>(
        &'a self,
        builder: &mut PdbBuilder<'a>,
        shared: &BTreeMap<SymbolOffset, SymbolHandle>,
    ) -> Result<()> {
        if let Some(globals) = &self.globals {
            for record in globals.hash_table().records() {
                builder.index_global(
                    symbol_handle(shared, record.symbol)?,
                    record.reference_count,
                )?;
            }
        }
        if let Some(publics) = &self.publics {
            for record in publics.hash_table().records() {
                builder.index_public(symbol_handle(shared, record.symbol)?)?;
            }
            let header = publics.header();
            builder.set_public_thunks(ThunkTable {
                thunk_size: header.thunk_size,
                section: header.thunk_table_section,
                offset: header.thunk_table_offset,
                targets: &self.thunk_targets,
            });
            builder.set_section_starts(self.section_starts.clone());
        }
        Ok(())
    }

    /// Decode each named stream and add it again.
    fn add_named_streams<'a>(&'a self, builder: &mut PdbBuilder<'a>) -> Result<()> {
        let mut maps = Vec::new();
        for (id, bytes) in &self.type_maps {
            let mut encoded = Vec::new();
            TypeMapStream::parse(bytes, *id)?.encode(&mut encoded)?;
            maps.push((*id, builder.add_stream(encoded)?));
        }
        for content in &self.named {
            let NamedContent {
                name,
                stream,
                bytes,
            } = content;
            let mut encoded = Vec::new();
            match NamedStreamKind::classify(name) {
                NamedStreamKind::LinkInfo => {
                    if let Some(link) = LinkInfo::parse(bytes, *stream)? {
                        link.encode(&mut encoded)?;
                    }
                }
                NamedStreamKind::TmCache => {
                    let cache = TmCache::parse(bytes, *stream)?;
                    if !cache.is_empty() {
                        let handles = cache_handles(&cache, &maps)?;
                        builder.add_named_stream_with(name, move |resolver, out| {
                            encode_cache(&cache, &handles, resolver, out)
                        })?;
                        continue;
                    }
                }
                NamedStreamKind::UdtSourceLineUndone => {
                    UdtSourceLineUndone::parse(bytes, *stream)?.encode(&mut encoded)?;
                }
                NamedStreamKind::SourceLink => {
                    SourceLink::parse(bytes, *stream)?.encode(&mut encoded)?;
                }
                NamedStreamKind::SourceServer => {
                    SourceServer::parse(bytes, *stream)?.encode(&mut encoded)?;
                }
                NamedStreamKind::SourceHeaderBlock => {
                    if let Some(block) = SourceHeaderBlock::parse(bytes, *stream)? {
                        block.encode(&mut encoded)?;
                    }
                }
                // The stream holds the bytes of an injected source file.
                NamedStreamKind::SourceFile { .. } => encoded.clone_from(bytes),
                _ => return Err(unsupported("named stream has no decoder")),
            }
            builder.add_named_stream(name, encoded)?;
        }
        Ok(())
    }
}

/// Decode the hash adjusters of a type stream.
fn adjusters(
    stream: &TypeStream,
    hash: Option<&[u8]>,
) -> Result<Vec<crate::native::types::HashAdjuster>> {
    match hash {
        Some(bytes) => Ok(TypeHashes::parse(stream.header(), bytes)?
            .adjusters()
            .to_vec()),
        None => Ok(Vec::new()),
    }
}

/// Return the reference and the target handle of a shared record that
/// names a module record.
fn module_target<'a>(
    symbol: &Symbol<'a>,
    modules: &ModuleSymbols,
) -> Option<(ProcedureReference<'a>, ModuleSymbolHandle)> {
    let (SymbolData::ProcedureReference(reference) | SymbolData::SymbolReference(reference)) =
        *symbol.data()
    else {
        return None;
    };
    if !matches!(
        symbol.kind(),
        S_PROCREF | S_LPROCREF | S_TOKENREF | S_DATAREF | S_ANNOTATIONREF
    ) {
        return None;
    }
    let module = reference.module_index()?;
    let target = *modules.get(&(module, reference.offset))?;
    Some((reference, target))
}

/// Return the handle of the shared record at `offset` of the source.
fn symbol_handle(
    shared: &BTreeMap<SymbolOffset, SymbolHandle>,
    offset: SymbolOffset,
) -> Result<SymbolHandle> {
    shared.get(&offset).copied().ok_or_else(|| {
        Error::new(ErrorKind::InvalidReference, None)
            .with_context("symbol offset does not identify a record of the symbol record stream")
    })
}

/// Return the handle of each stream slot of a `/TMCache` stream.
fn cache_handles(
    cache: &TmCache<'_>,
    maps: &[(StreamId, StreamHandle)],
) -> Result<Vec<Option<StreamHandle>>> {
    cache
        .streams()
        .map(|slot| {
            slot.map(|id| {
                maps.iter()
                    .find(|(source, _)| *source == id)
                    .map(|(_, handle)| *handle)
                    .ok_or_else(|| {
                        Error::new(ErrorKind::InvalidReference, None)
                            .with_context("/TMCache names a stream that the rebuild did not read")
                    })
            })
            .transpose()
        })
        .collect()
}

/// Encode a `/TMCache` stream with the new stream IDs of its type maps.
fn encode_cache(
    cache: &TmCache<'_>,
    handles: &[Option<StreamHandle>],
    resolver: &crate::native::write::Resolver<'_>,
    out: &mut Vec<u8>,
) -> Result<()> {
    let mut builder = TmCacheBuilder::new();
    for handle in handles {
        builder.push_stream(handle.map(|handle| resolver.stream(handle)).transpose()?)?;
    }
    for entry in cache.entries() {
        builder.push_entry(entry.slot, entry.checksum)?;
    }
    builder.encode(out)
}
