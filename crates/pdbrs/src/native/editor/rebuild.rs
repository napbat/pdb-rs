//! [`PdbEditor::decode_all`]: the semantic rebuild of a native PDB.
//!
//! `decode_all` decodes every stream that another stream names. The plan
//! then writes the new layout: it encodes each stream from the decoded
//! values and assigns new stream IDs. Records that name a stream or a
//! symbol record by its on-disk value get the new value from the plan:
//!
//! - the module streams of the DBI module records,
//! - the global references of the module streams,
//! - the module offsets of `S_PROCREF`, `S_LPROCREF`, `S_DATAREF`,
//!   `S_ANNOTATIONREF` and `S_TOKENREF` records (they get a
//!   [`ModuleSymbolTarget::Handle`]),
//! - the type map streams of `/TMCache`,
//! - the module type streams that an `S_MOD_TYPEREF` record names in a PDB
//!   without type merging (`NOTM`). The rebuild decodes and encodes their
//!   records.
//!
//! The plan also builds again what it computes: the TPI and IPI hash
//! streams, the global and public indexes, the DBI source info and the scope
//! links of module symbols. The rebuild does not keep the previous stream
//! directory (stream 0) or a stream that no other stream names; the report
//! gives such a stream as removed.

use alloc::vec::Vec;

use super::module::{GlobalRef, ModuleContents, Symbols};
use super::named::{Content, NamedEntry, NamedStreams};
use super::streams::{DebugContent, DebugInfo, DebugStreamList, UnnamedStreams, kind_of};
use super::symbols::{IndexEntry, SharedSymbols};
use super::tree;
use super::types::RecordList;
use super::{IdList, InfoModel, ModuleSlot, Names, PdbEditor, TypeList, missing, source};
use crate::io::ReadAt;
use crate::native::auxiliary::{
    LinkInfo, NamedStreamKind, SourceLink, SourceServer, TmCache, TypeMapStream,
    UdtSourceLineUndone,
};
use crate::native::codeview::Records;
use crate::native::codeview::id_record::IdRecord;
use crate::native::codeview::index::{ModuleIndex, ModuleSymbolHandle, SymbolHandle};
use crate::native::codeview::symbol::{
    ModuleSymbolTarget, ModuleTypeReference, ModuleTypeSource, S_ANNOTATIONREF, S_DATAREF,
    S_LPROCREF, S_PROCREF, S_TOKENREF, SymbolData,
};
use crate::native::codeview::type_record::TypeLeaf;
use crate::native::dbi::{Dbi, DbiBuilder, DebugStream, SectionContributionVersion};
use crate::native::injected_source::SourceHeaderBlock;
use crate::native::msf::StreamId;
use crate::native::names::{HashVersion, StringTableBuilder};
use crate::native::output::no_memory;
use crate::native::{Identity, Pdb, Producer};
use crate::{Error, ErrorKind, Result};

/// The conventions of the source that the new layout keeps by default.
#[derive(Clone, Copy, Debug)]
pub(super) struct Conventions {
    /// The page size of the source container.
    pub(super) page_size: u32,
    /// The producer of the TPI hash stream layout.
    pub(super) producer: Producer,
    /// The hash bucket count of the TPI stream.
    pub(super) hash_buckets: u32,
    /// `true` if the source has a TPI hash stream.
    pub(super) type_hash_stream: bool,
    /// `true` if the source has an IPI hash stream or no IPI stream.
    pub(super) id_hash_stream: bool,
}

/// The source streams of the parts of the new layout.
#[derive(Clone, Debug, Default)]
pub(super) struct SourceIds {
    /// The number of source streams.
    pub(super) count: u32,
    /// The IPI stream, if the source has one.
    pub(super) ipi: Option<StreamId>,
    /// The TPI hash stream.
    pub(super) tpi_hash: Option<StreamId>,
    /// The IPI hash stream.
    pub(super) ipi_hash: Option<StreamId>,
    /// The global symbol index.
    pub(super) globals: Option<StreamId>,
    /// The public symbol index.
    pub(super) publics: Option<StreamId>,
    /// The symbol record stream.
    pub(super) symbols: Option<StreamId>,
    /// The `/names` stream.
    pub(super) names: Option<StreamId>,
    /// The optional debug streams.
    pub(super) debug: Vec<(DebugStream, StreamId)>,
}

/// The state of an editor after [`PdbEditor::decode_all`].
#[derive(Clone, Debug)]
pub(super) struct Rebuild {
    /// The conventions of the source.
    pub(super) conventions: Conventions,
    /// The source streams of the parts.
    pub(super) sources: SourceIds,
    /// The present source streams that no other stream names.
    pub(super) unclaimed: Vec<StreamId>,
}

/// Return the error for data that the rebuild cannot decode.
fn unsupported(context: &'static str) -> Error {
    Error::new(ErrorKind::UnsupportedVersion, None).with_context(context)
}

/// Push a value with a checked allocation.
fn push<T>(list: &mut Vec<T>, value: T) -> Result<()> {
    list.try_reserve(1).map_err(|_| no_memory())?;
    list.push(value);
    Ok(())
}

/// The streams that other streams name.
struct Claims {
    /// The claimed stream IDs.
    claimed: Vec<StreamId>,
}

impl Claims {
    /// Claim a stream.
    fn claim(&mut self, id: StreamId) -> Result<()> {
        push(&mut self.claimed, id)
    }

    /// Return the present source streams that no claim names.
    fn unclaimed<S: ReadAt>(&self, pdb: &Pdb<S>) -> Result<Vec<StreamId>> {
        let mut unclaimed = Vec::new();
        for id in 0..pdb.container().stream_count() {
            let id = StreamId(id);
            if !self.claimed.contains(&id) && pdb.container().stream(id)?.is_some() {
                push(&mut unclaimed, id)?;
            }
        }
        Ok(unclaimed)
    }
}

impl<S: ReadAt> PdbEditor<'_, S> {
    /// Decode every stream of the source.
    ///
    /// The plan then writes the new layout (see [`PdbEditor`](PdbEditor#layouts)):
    /// it encodes each stream again from the decoded values and assigns new
    /// stream IDs. The rebuild keeps the record order of each stream, so the
    /// type indices, the ID indices, the `/names` offsets, the module order
    /// and the symbol record order stay. A part that a `*_mut` call decoded
    /// keeps its changes.
    ///
    /// The function does nothing for an editor in the new layout. It is
    /// atomic: an error leaves the model unchanged.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the PDB has no TPI or DBI stream.
    /// - [`ErrorKind::UnsupportedVersion`] for data that the rebuild cannot
    ///   decode: C11 line data, a named stream of an unknown kind, an
    ///   optional debug header slot after the slots of [`DebugStream`] with a
    ///   stream, an `S_MOD_TYPEREF` record with flags of unknown meaning, or
    ///   an Info stream without a GUID identity.
    /// - [`ErrorKind::InvalidReference`] for a reference that does not
    ///   identify a record of the source.
    /// - The errors of the stream readers and the record decoders.
    pub fn decode_all(&mut self) -> Result<()> {
        if self.is_new_layout() {
            return Ok(());
        }
        self.clear_remap_output();
        let pdb = self.cow_source()?;
        let mut claims = Claims {
            claimed: Vec::new(),
        };
        for id in 0..5 {
            claims.claim(StreamId(id))?;
        }
        let info = pdb.info();
        let Identity::Modern { guid, age } = info.identity() else {
            return Err(unsupported("PDB Info stream has no GUID identity"));
        };
        let tpi = pdb
            .types()?
            .ok_or_else(|| missing("PDB has no TPI stream"))?;
        let ipi = pdb.ids()?;
        let dbi = pdb
            .debug_info()?
            .ok_or_else(|| missing("PDB has no DBI stream"))?;
        let header = *dbi.header();
        let mut sources = SourceIds {
            count: pdb.container().stream_count(),
            ipi: ipi.as_ref().map(|_| StreamId(4)),
            tpi_hash: tpi.header().hash_stream,
            ipi_hash: ipi.as_ref().and_then(|ipi| ipi.header().hash_stream),
            globals: header.global_stream,
            publics: header.public_stream,
            symbols: header.symbol_stream,
            names: None,
            debug: Vec::new(),
        };
        for id in [sources.tpi_hash, sources.ipi_hash]
            .into_iter()
            .chain([
                header.global_stream,
                header.public_stream,
                header.symbol_stream,
            ])
            .flatten()
        {
            claims.claim(id)?;
        }
        let conventions = Conventions {
            page_size: pdb.output_page_size(),
            producer: tpi.header().producer(),
            hash_buckets: tpi.header().hash_bucket_count,
            type_hash_stream: tpi.header().hash_stream.is_some(),
            id_hash_stream: ipi
                .as_ref()
                .is_none_or(|ipi| ipi.header().hash_stream.is_some()),
        };
        let types = match &self.types {
            Some(types) => types.list.clone(),
            None => source::type_list(pdb)?,
        };
        let ids = match (&self.ids, ipi.is_some()) {
            (Some(ids), _) => ids.list.clone(),
            (None, true) => source::id_list(pdb)?,
            (None, false) => RecordList::new(),
        };
        let mut unnamed = UnnamedStreams::new(sources.count);
        let modules = self.decode_modules(pdb, dbi, &mut claims, &mut unnamed)?;
        let shared = match &self.symbols {
            Some(shared) => shared.clone(),
            None if header.symbol_stream.is_some() => source::shared_symbols(pdb)?,
            None => SharedSymbols::default(),
        };
        let debug = self.decode_debug(pdb, dbi, &mut claims, &mut sources)?;
        let (named, table) = self.decode_named(pdb, &mut claims, &mut unnamed, &mut sources)?;
        if let Some(added) = &self.unnamed {
            for entry in &added.entries {
                push(&mut unnamed.entries, entry.clone())?;
            }
            unnamed.next = unnamed.next.max(added.next);
        }
        let dbi_values = match &self.dbi {
            Some(values) => values.clone(),
            None => rebuild_dbi(dbi)?,
        };
        let (modules, shared) = link(modules, shared)?;
        let unclaimed = claims.unclaimed(pdb)?;
        let features = self.feature_codes();
        let mut types = types;
        types.forget_source();
        let mut ids = ids;
        ids.forget_source();
        self.info = Some(InfoModel {
            signature: info.signature(),
            guid,
            age,
            dbi_age: header.age,
            features,
        });
        self.features = None;
        self.types = Some(TypeList::from_list(types));
        self.ids = Some(IdList::from_list(ids));
        self.modules = Some(modules);
        self.symbols = Some(shared);
        self.debug = Some(debug);
        self.named = Some(named);
        self.names = Some(table);
        self.dbi = Some(dbi_values);
        self.unnamed = Some(unnamed);
        self.rebuild = Some(Rebuild {
            conventions,
            sources,
            unclaimed,
        });
        Ok(())
    }

    /// Decode each module and the module type streams of `S_MOD_TYPEREF`
    /// records.
    fn decode_modules(
        &self,
        pdb: &Pdb<S>,
        dbi: &Dbi,
        claims: &mut Claims,
        unnamed: &mut UnnamedStreams,
    ) -> Result<Vec<ModuleSlot>> {
        let decoded = self.modules.as_deref().unwrap_or_default();
        let mut modules = Vec::new();
        let mut index = 0_u32;
        for module in dbi.modules() {
            let (_, record) = module?;
            if let Some(id) = record.stream {
                claims.claim(id)?;
            }
            let mut module = match decoded.get(index as usize) {
                Some(ModuleSlot::Decoded(module)) => (**module).clone(),
                _ => source::module(pdb, ModuleIndex(index))?,
            };
            if let Some(stream) = module.source().and_then(|source| source.stream.as_ref()) {
                if !stream.c11_data().is_empty() {
                    return Err(unsupported("C11 line data is a legacy format (#16)"));
                }
                let replaced = self
                    .unnamed
                    .as_ref()
                    .map_or(&[][..], |unnamed| unnamed.replaced.as_slice());
                module_types(pdb, stream.symbol_data(), replaced, claims, unnamed)?;
            }
            prepare_module(&mut module, unnamed)?;
            push(
                &mut modules,
                ModuleSlot::Decoded(alloc::boxed::Box::new(module)),
            )?;
            index = index.checked_add(1).ok_or_else(|| {
                Error::new(ErrorKind::LimitExceeded, None).with_context("PDB has too many modules")
            })?;
        }
        // Modules that the caller added after the source modules.
        for slot in decoded.iter().skip(modules.len()) {
            push(&mut modules, slot.clone())?;
        }
        Ok(modules)
    }

    /// Decode each optional debug stream and encode it again.
    fn decode_debug(
        &self,
        pdb: &Pdb<S>,
        dbi: &Dbi,
        claims: &mut Claims,
        sources: &mut SourceIds,
    ) -> Result<DebugStreamList> {
        let mut list = self.debug.clone().unwrap_or_default();
        for (slot, id) in dbi.debug_streams().enumerate() {
            let Some(id) = id else { continue };
            claims.claim(id)?;
            let kind = kind_of(slot).ok_or_else(|| {
                unsupported("optional debug header slot has no known stream kind")
            })?;
            push(&mut sources.debug, (kind, id))?;
            if self.debug.is_none() {
                list.set(kind, debug_bytes(pdb, kind)?)?;
            }
        }
        for entry in &mut list.entries {
            if let (kind, DebugContent::Source(_)) = entry {
                entry.1 = DebugContent::Bytes(debug_bytes(pdb, *kind)?);
            }
        }
        Ok(list)
    }

    /// Decode each named stream and encode it again. Return the named
    /// streams and `/names`.
    fn decode_named(
        &self,
        pdb: &Pdb<S>,
        claims: &mut Claims,
        unnamed: &mut UnnamedStreams,
        sources: &mut SourceIds,
    ) -> Result<(NamedStreams, Names)> {
        let mut names = Names::Absent;
        let mut maps = Vec::new();
        for entry in pdb.info().named_streams() {
            claims.claim(entry.stream)?;
            match NamedStreamKind::classify(&entry.name) {
                NamedStreamKind::Names => {
                    sources.names = Some(entry.stream);
                    names = match &self.names {
                        Some(Names::Table(table)) => Names::Table(table.clone()),
                        _ => source::names(pdb)?,
                    };
                }
                NamedStreamKind::TmCache => {
                    let bytes = pdb.stream_bytes(entry.stream)?;
                    for id in TmCache::parse(&bytes, entry.stream)?.streams().flatten() {
                        if maps.contains(&id) {
                            continue;
                        }
                        claims.claim(id)?;
                        push(&mut maps, id)?;
                    }
                }
                _ => {}
            }
        }
        for id in maps {
            let bytes = pdb.stream_bytes(id)?;
            let mut encoded = Vec::new();
            TypeMapStream::parse(&bytes, id)?.encode(&mut encoded)?;
            push(&mut unnamed.entries, (id, encoded))?;
        }
        let mut model = match &self.named {
            Some(decoded) => decoded.clone(),
            None => NamedStreams::from_info(pdb.info())?,
        };
        model.restricted = false;
        for entry in &mut model.entries {
            decode_named_entry(pdb, entry)?;
        }
        Ok((model, names))
    }
}

/// Decode a source named stream and encode it again.
fn decode_named_entry<S: ReadAt>(pdb: &Pdb<S>, entry: &mut NamedEntry) -> Result<()> {
    let (Content::Source, Some(stream)) = (&entry.content, entry.source) else {
        return Ok(());
    };
    let bytes = pdb.stream_bytes(stream)?;
    let mut encoded = Vec::new();
    match NamedStreamKind::classify(&entry.name) {
        NamedStreamKind::LinkInfo => {
            if let Some(link) = LinkInfo::parse(&bytes, stream)? {
                link.encode(&mut encoded)?;
            }
        }
        NamedStreamKind::TmCache => {
            // The plan writes the new stream IDs of the type maps.
            if !TmCache::parse(&bytes, stream)?.is_empty() {
                encoded = bytes;
            }
        }
        NamedStreamKind::UdtSourceLineUndone => {
            UdtSourceLineUndone::parse(&bytes, stream)?.encode(&mut encoded)?;
        }
        NamedStreamKind::SourceLink => {
            SourceLink::parse(&bytes, stream)?.encode(&mut encoded)?;
        }
        NamedStreamKind::NumberedSourceLink { .. } => {
            SourceLink::parse_numbered(&bytes, stream)?.encode(&mut encoded)?;
        }
        NamedStreamKind::SourceServer => {
            SourceServer::parse(&bytes, stream)?.encode(&mut encoded)?;
        }
        NamedStreamKind::SourceHeaderBlock => {
            if let Some(block) = SourceHeaderBlock::parse(&bytes, stream)? {
                block.encode(&mut encoded)?;
            }
        }
        // The stream holds the bytes of an injected source file.
        NamedStreamKind::SourceFile { .. } => encoded = bytes,
        _ => return Err(unsupported("named stream has no decoder")),
    }
    entry.content = Content::Bytes(encoded);
    Ok(())
}

/// Decode one optional debug stream and encode it again.
fn debug_bytes<S: ReadAt>(pdb: &Pdb<S>, kind: DebugStream) -> Result<Vec<u8>> {
    let missing = || missing("optional debug stream is missing");
    match kind {
        DebugStream::Fpo => pdb.fpo()?.ok_or_else(missing)?.to_bytes(),
        DebugStream::NewFpo => pdb.frame_data()?.ok_or_else(missing)?.to_bytes(),
        DebugStream::Exception => pdb.exception_data()?.ok_or_else(missing)?.to_bytes(),
        DebugStream::Fixup => pdb.fixups()?.ok_or_else(missing)?.to_bytes(),
        DebugStream::OmapToSource | DebugStream::OmapFromSource => {
            pdb.omap(kind)?.ok_or_else(missing)?.to_bytes()
        }
        DebugStream::SectionHeader | DebugStream::OriginalSectionHeader => {
            pdb.section_headers(kind)?.ok_or_else(missing)?.to_bytes()
        }
        DebugStream::TokenRidMap => pdb.token_rid_map()?.ok_or_else(missing)?.to_bytes(),
        DebugStream::Xdata | DebugStream::Pdata => {
            pdb.debug_blob(kind)?.ok_or_else(missing)?.to_bytes()
        }
    }
}

/// Read the streams that the `S_MOD_TYPEREF` record of a module names,
/// decode their records and encode them again. A stream of `replaced` (the
/// new bytes of a path remap, #46) replaces its source stream.
fn module_types<S: ReadAt>(
    pdb: &Pdb<S>,
    symbols: &[u8],
    replaced: &[(StreamId, Vec<u8>)],
    claims: &mut Claims,
    unnamed: &mut UnnamedStreams,
) -> Result<()> {
    let Some(reference) = ModuleTypeReference::find(symbols)? else {
        return Ok(());
    };
    let source = reference.source();
    if let ModuleTypeSource::Other(_) = source {
        return Err(unsupported(
            "S_MOD_TYPEREF record has flags with words of unknown meaning",
        ));
    }
    for id in source.streams() {
        if unnamed.entries.iter().any(|(existing, _)| *existing == id) {
            continue;
        }
        claims.claim(id)?;
        let bytes = match replaced.iter().find(|(stream, _)| *stream == id) {
            Some((_, bytes)) => bytes.clone(),
            None => pdb.stream_bytes(id)?,
        };
        let mut encoded = Vec::new();
        encoded
            .try_reserve_exact(bytes.len())
            .map_err(|_| no_memory())?;
        for record in Records::new(&bytes) {
            let record = record?;
            match IdRecord::decode(record)? {
                IdRecord::Unsupported(_) => TypeLeaf::decode(record)?.encode(&mut encoded)?,
                id => id.encode(&mut encoded)?,
            }
        }
        push(&mut unnamed.entries, (id, encoded))?;
    }
    Ok(())
}

/// Prepare a decoded module for the new layout: the plan encodes its lines
/// and the `S_MOD_TYPEREF` record with streams from their fields.
fn prepare_module(module: &mut ModuleContents, unnamed: &UnnamedStreams) -> Result<()> {
    module.set_lines_changed();
    let Symbols::Tree(nodes) = module.symbol_region_mut() else {
        return Ok(());
    };
    tree::walk_mut(nodes, &mut |record| {
        if let SymbolData::ModuleTypeReference(reference) = record.data() {
            let mut streams = reference.source().streams().peekable();
            if streams.peek().is_some() {
                for id in streams {
                    if !unnamed.entries.iter().any(|(existing, _)| *existing == id) {
                        return Err(Error::new(ErrorKind::InvalidReference, None).with_context(
                            "S_MOD_TYPEREF names a stream that the rebuild did not read",
                        ));
                    }
                }
                record.forget_source();
            }
        }
        Ok(())
    })
}

/// Return the DBI values of the rebuild.
fn rebuild_dbi(dbi: &Dbi) -> Result<DebugInfo> {
    let mut builder = DbiBuilder::new(*dbi.header());
    if let Some(contributions) = dbi.section_contributions()? {
        builder.section_contribution_version = contributions.version();
        builder
            .section_contributions
            .try_reserve_exact(contributions.len())
            .map_err(|_| no_memory())?;
        builder.section_contributions.extend(contributions.iter());
    } else {
        builder.section_contribution_version = SectionContributionVersion::V60;
    }
    builder.section_map = dbi.section_map()?;
    builder.type_servers = dbi.type_server_map()?;
    // A rebuild without EC names keeps the empty table of a new PDB.
    builder.ec_names = Some(match dbi.ec_names()? {
        Some(names) => StringTableBuilder::from_table(&names)?,
        None => StringTableBuilder::new(HashVersion::V1)?,
    });
    Ok(DebugInfo {
        dbi: builder,
        debug_slots: dbi.debug_streams().count(),
    })
}

/// Give the shared records handles to their module records, and the
/// modules handles to their global references. Build the index entries of
/// the rebuild: in record order, with the bucket of the name.
fn link(
    mut modules: Vec<ModuleSlot>,
    mut shared: SharedSymbols,
) -> Result<(Vec<ModuleSlot>, SharedSymbols)> {
    let mut handles = Vec::<((u32, u32), ModuleSymbolHandle)>::new();
    for slot in &modules {
        let ModuleSlot::Decoded(module) = slot else {
            continue;
        };
        let Some(base) = module
            .source()
            .and_then(|source| source.stream.as_ref())
            .map(super::module::symbol_base)
        else {
            continue;
        };
        if let Symbols::Tree(nodes) = module.symbol_region() {
            tree::walk(nodes, &mut |record| {
                if let (Some(origin), Some(handle)) = (record.origin(), module.handle(record)) {
                    push(
                        &mut handles,
                        ((module.index().0, origin.record().offset + base), handle),
                    )?;
                }
                Ok(())
            })?;
        }
    }
    handles.sort_unstable_by_key(|(key, _)| *key);
    let mut offsets = Vec::new();
    for (position, record) in (0_u32..).zip(&mut shared.records) {
        let Some(record) = record else {
            continue;
        };
        if let Some(origin) = record.origin() {
            push(&mut offsets, (origin.record().offset, position))?;
        }
        let kind = record.kind();
        if !matches!(
            kind,
            S_PROCREF | S_LPROCREF | S_TOKENREF | S_DATAREF | S_ANNOTATIONREF
        ) {
            continue;
        }
        let target = match record.data() {
            SymbolData::ProcedureReference(reference) | SymbolData::SymbolReference(reference) => {
                reference.target
            }
            _ => continue,
        };
        let ModuleSymbolTarget::Position {
            module: Some(module),
            offset,
        } = target
        else {
            continue;
        };
        if u16::try_from(module.0).is_err() {
            continue;
        }
        let Ok(position) = handles.binary_search_by_key(&(module.0, offset.0), |(key, _)| *key)
        else {
            continue;
        };
        let handle = handles[position].1;
        record.forget_source();
        if let SymbolData::ProcedureReference(reference) | SymbolData::SymbolReference(reference) =
            record.data_mut()
        {
            reference.target = ModuleSymbolTarget::Handle(handle);
        }
    }
    offsets.sort_unstable();
    for slot in &mut modules {
        let ModuleSlot::Decoded(module) = slot else {
            continue;
        };
        for reference in module.global_refs_mut() {
            if let GlobalRef::Offset(offset) = *reference {
                let position = offsets
                    .binary_search_by_key(&offset.0, |&(source, _)| source)
                    .map(|found| offsets[found].1)
                    .map_err(|_| {
                        Error::new(ErrorKind::InvalidReference, None).with_context(
                            "symbol offset does not identify a record of the symbol record stream",
                        )
                    })?;
                *reference = GlobalRef::Handle(SymbolHandle(position));
            }
        }
    }
    rebuild_entries(&mut shared)?;
    Ok((modules, shared))
}

/// Make the index entries of the rebuild: each named record once, in record
/// order, with the bucket of its name. A global entry keeps the reference
/// count of the source; a public entry gets 1.
fn rebuild_entries(shared: &mut SharedSymbols) -> Result<()> {
    let count = shared.records.len();
    let mut global = Vec::new();
    global.try_reserve_exact(count).map_err(|_| no_memory())?;
    global.resize(count, None);
    let mut public = Vec::new();
    public.try_reserve_exact(count).map_err(|_| no_memory())?;
    public.resize(count, false);
    for entry in &shared.globals {
        if let Some(slot) = global.get_mut(entry.symbol.0 as usize) {
            *slot = Some(entry.reference_count);
        }
    }
    for entry in &shared.publics {
        if let Some(slot) = public.get_mut(entry.symbol.0 as usize) {
            *slot = true;
        }
    }
    shared.globals.clear();
    shared.publics.clear();
    for (position, (global, public)) in global.into_iter().zip(public).enumerate() {
        let symbol = SymbolHandle(u32::try_from(position).unwrap_or(u32::MAX));
        if let Some(reference_count) = global {
            push(
                &mut shared.globals,
                IndexEntry {
                    symbol,
                    reference_count,
                    bucket: None,
                },
            )?;
        }
        if public {
            push(
                &mut shared.publics,
                IndexEntry {
                    symbol,
                    reference_count: 1,
                    bucket: None,
                },
            )?;
        }
    }
    shared.source = None;
    Ok(())
}
