//! The conversion of decoded PDB contents to a builder.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::super::{
    BuildOptions, IdHandle, ModuleHandle, ModuleSymbolHandle, PdbBuilder, Resolver, StreamHandle,
    SymbolHandle, TypeHandle,
};
use super::{NamedContent, PdbContents};
use crate::native::Identity;
use crate::native::IdentityPolicy;
use crate::native::auxiliary::{
    LinkInfo, NamedStreamKind, SourceLink, SourceServer, TmCache, TmCacheBuilder, TypeMapStream,
    UdtSourceLineUndone,
};
use crate::native::codeview::Records;
use crate::native::codeview::id_record::IdRecord;
use crate::native::codeview::symbol::{
    ModuleTypeReference, ProcedureReference, S_ANNOTATIONREF, S_DATAREF, S_LPROCREF, S_MOD_TYPEREF,
    S_PROCREF, S_TOKENREF, Symbol, SymbolData,
};
use crate::native::codeview::type_record::TypeLeaf;
use crate::native::injected_source::SourceHeaderBlock;
use crate::native::lines::ModuleLines;
use crate::native::msf::{MsfWriteOptions, StreamId};
use crate::native::names::{StringTable, StringTableBuilder};
use crate::native::symbols::{SymbolOffset, ThunkTable};
use crate::native::types::{TypeHashes, TypeStream};
use crate::{Error, ErrorKind, Result};

/// The handles of the module symbol records by module index and module
/// stream offset.
type ModuleSymbols = BTreeMap<(u16, u32), ModuleSymbolHandle>;

/// The handles of the module type streams by source stream ID.
type TypeStreams = Vec<(StreamId, StreamHandle)>;

/// The source stream and the handle of each stream of an `S_MOD_TYPEREF`
/// record.
type StreamPairs = [Option<(StreamId, StreamHandle)>; 2];

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
    /// symbol indexes and the named-stream map. The container keeps the page
    /// size of the source.
    ///
    /// To select another page size, replace the container options, for
    /// example with [`BuildOptions::with_msf`] and
    /// [`MsfWriteOptions::with_page_size`]. The plan rejects a source page
    /// size of 65536 bytes (see [`MsfWriteOptions::with_page_size`]).
    #[must_use]
    pub fn build_options(&self) -> BuildOptions {
        let header = self.tpi.0.header();
        let producer = header.producer();
        BuildOptions::default()
            .with_msf(MsfWriteOptions::default().with_page_size(self.page_size))
            .with_type_hash_producer(producer)
            .with_hash_buckets(header.hash_bucket_count)
            .with_index_producer(producer)
            .with_info_producer(producer)
            // A source without a hash stream keeps none, for example the
            // .NET fixture of #14.
            .with_type_hash_stream(header.hash_stream.is_some())
            .with_id_hash_stream(
                self.ipi
                    .as_ref()
                    .is_none_or(|(ipi, _)| ipi.header().hash_stream.is_some()),
            )
    }

    /// Decode every record and give the decoded values to a new builder.
    ///
    /// `policy` sets the identity of the new PDB (see [`IdentityPolicy`]).
    /// Read the identity for the image with
    /// [`PdbWritePlan::identity`](crate::native::PdbWritePlan::identity).
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
        let features: Vec<_> = self.info.features().collect();
        builder.set_features(&features)?;
        builder.set_dbi_age(self.dbi.header().age);
        builder.apply_identity(policy)?;
        builder.set_names(match &self.names {
            Some(bytes) => Some(StringTableBuilder::from_table(&StringTable::parse(bytes)?)?),
            None => None,
        });
        self.add_types(&mut builder)?;
        self.add_dbi(&mut builder)?;
        let type_streams = self.add_module_types(&mut builder)?;
        let module_symbols = self.add_modules(&mut builder, &type_streams)?;
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
        builder.set_section_map(dbi.section_map()?);
        builder.set_type_servers(dbi.type_server_map()?);
        if let Some(names) = dbi.ec_names()? {
            builder.set_ec_names(Some(StringTableBuilder::from_table(&names)?));
        }
        builder.set_debug_slot_count(dbi.debug_streams().count());
        Ok(())
    }

    /// Decode the records of each module type stream, encode them again and
    /// add the stream. Return the handle of each stream by source stream ID.
    fn add_module_types(&self, builder: &mut PdbBuilder<'_>) -> Result<TypeStreams> {
        let mut handles = Vec::new();
        handles
            .try_reserve_exact(self.module_types.len())
            .map_err(|_| crate::native::output::no_memory())?;
        for (id, bytes) in &self.module_types {
            let mut encoded = Vec::new();
            encoded
                .try_reserve_exact(bytes.len())
                .map_err(|_| crate::native::output::no_memory())?;
            for record in Records::new(bytes) {
                let record = record?;
                match IdRecord::decode(record)? {
                    IdRecord::Unsupported(_) => TypeLeaf::decode(record)?.encode(&mut encoded)?,
                    id => id.encode(&mut encoded)?,
                }
            }
            handles.push((*id, builder.add_stream(encoded)?));
        }
        Ok(handles)
    }

    /// Add the modules with their symbols, subsections and source files.
    ///
    /// An `S_MOD_TYPEREF` record gets the new numbers of the streams in
    /// `type_streams`.
    fn add_modules<'a>(
        &'a self,
        builder: &mut PdbBuilder<'a>,
        type_streams: &TypeStreams,
    ) -> Result<ModuleSymbols> {
        let mut handles = ModuleSymbols::new();
        let mut files = self
            .sources
            .iter()
            .flat_map(|info| info.modules.iter().map(move |list| (info, list)));
        for (module, stream) in self.dbi.modules().zip(&self.modules) {
            let (index, module) = module?;
            let index = u16::try_from(index.0)
                .map_err(|_| unsupported("PDB has more than 65535 modules"))?;
            let target = builder.add_module(module.into_owned())?;
            if let Some((info, list)) = files.next() {
                for name in info.file_names(list) {
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
                let symbol = Symbol::decode(record)?;
                let handle = match module_type_reference(&symbol, type_streams)? {
                    Some((reference, streams)) => {
                        target.push_symbol_with(move |resolver, out| {
                            encode_module_type_reference(reference, &streams, resolver, out)
                        })?
                    }
                    None => target.push_symbol(symbol)?,
                };
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
            let symbol = Symbol::decode(record)?;
            let handle = match module_target(&symbol, modules) {
                Some((reference, target)) => {
                    let kind = symbol.kind();
                    let wide = matches!(symbol.data(), SymbolData::SymbolReference(_));
                    builder.add_symbol_with(move |resolver, out| {
                        let mut reference = reference.clone();
                        reference.offset = resolver.module_symbol_offset(target)?;
                        reference.module = Some(resolver.module_index(target.module())?);
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
            let handle = ModuleHandle(u32::try_from(index).unwrap_or(u32::MAX));
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
                NamedStreamKind::NumberedSourceLink { .. } => {
                    SourceLink::parse_numbered(bytes, *stream)?.encode(&mut encoded)?;
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
        symbol.data()
    else {
        return None;
    };
    if !matches!(
        symbol.kind(),
        S_PROCREF | S_LPROCREF | S_TOKENREF | S_DATAREF | S_ANNOTATIONREF
    ) {
        return None;
    }
    let module = u16::try_from(reference.module?.0).ok()?;
    let target = *modules.get(&(module, reference.offset.0))?;
    Some((reference.clone(), target))
}

/// Return the record and the stream handles of an `S_MOD_TYPEREF` record
/// that names streams.
///
/// Return `None` for another record and for a record without streams.
fn module_type_reference(
    symbol: &Symbol<'_>,
    type_streams: &TypeStreams,
) -> Result<Option<(ModuleTypeReference, StreamPairs)>> {
    let SymbolData::ModuleTypeReference(reference) = *symbol.data() else {
        return Ok(None);
    };
    let mut handles = [None; 2];
    for (slot, id) in handles.iter_mut().zip(reference.source().streams()) {
        let handle = type_streams
            .iter()
            .find(|(source, _)| *source == id)
            .map(|&(_, handle)| handle)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidReference, None)
                    .with_context("S_MOD_TYPEREF names a stream that the rebuild did not read")
            })?;
        *slot = Some((id, handle));
    }
    Ok(handles[0].is_some().then_some((reference, handles)))
}

/// Encode an `S_MOD_TYPEREF` record with the new numbers of its streams.
fn encode_module_type_reference(
    mut reference: ModuleTypeReference,
    handles: &StreamPairs,
    resolver: &Resolver<'_>,
    out: &mut Vec<u8>,
) -> Result<()> {
    reference.map_streams(|id| {
        let (_, handle) = handles
            .iter()
            .flatten()
            .find(|(source, _)| *source == id)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidReference, None)
                    .with_context("S_MOD_TYPEREF stream has no handle")
            })?;
        resolver.stream(*handle)
    })?;
    SymbolData::ModuleTypeReference(reference).encode(S_MOD_TYPEREF, out)
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
    resolver: &Resolver<'_>,
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
