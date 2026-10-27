//! The semantic rebuild of a native PDB.
//!
//! [`Pdb::contents`] reads the streams of a PDB and decodes their headers.
//! [`PdbContents::to_builder`] decodes every record and gives the decoded
//! values to a [`PdbBuilder`](super::PdbBuilder). The plan of that builder
//! encodes each stream again from the decoded values. It does not copy the
//! bytes of a stream.
//!
//! The rebuild keeps the record order of each stream. Thus the type indices,
//! the ID indices, the `/names` offsets, the module order and the symbol
//! record order stay the same. The stream IDs change: the plan assigns them
//! in its own order. Records that name a stream or a symbol record by its
//! on-disk value get handles, so the plan writes the new values:
//!
//! - the module streams of the DBI module records,
//! - the global references of the module streams,
//! - the module offsets of `S_PROCREF`, `S_LPROCREF`, `S_DATAREF`,
//!   `S_ANNOTATIONREF` and `S_TOKENREF` records,
//! - the type map streams of `/TMCache`,
//! - the module type streams that an `S_MOD_TYPEREF` record names in a PDB
//!   without type merging (`NOTM`). The rebuild decodes and encodes their
//!   records, and the record gets the new stream numbers,
//! - the stream fields of the DBI header, the TPI and IPI headers and the
//!   optional debug header, and the named-stream map.
//!
//! The rebuild also builds again what the plan computes: the TPI and IPI
//! hash streams, the global and public indexes, the DBI source info and the
//! scope links of module symbols.
//!
//! The rebuild does not keep the previous stream directory (stream 0) or a
//! stream that no other stream names. [`PdbContents::unclaimed_streams`]
//! lists those streams.

use alloc::vec::Vec;

use crate::io::ReadAt;
use crate::native::Pdb;
use crate::native::auxiliary::{NamedStreamKind, TmCache};
use crate::native::codeview::symbol::{ModuleTypeReference, ModuleTypeSource};
use crate::native::dbi::{Dbi, DebugStream, ModuleStream};
use crate::native::info::Info;
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::native::symbols::{GlobalSymbolIndex, PublicSymbolIndex, SectionStart};
use crate::native::types::TypeStream;
use crate::{Error, ErrorKind, Result};

mod builder;

/// The optional debug stream kinds in slot order.
const DEBUG_KINDS: [DebugStream; 11] = [
    DebugStream::Fpo,
    DebugStream::Exception,
    DebugStream::Fixup,
    DebugStream::OmapToSource,
    DebugStream::OmapFromSource,
    DebugStream::SectionHeader,
    DebugStream::TokenRidMap,
    DebugStream::Xdata,
    DebugStream::Pdata,
    DebugStream::NewFpo,
    DebugStream::OriginalSectionHeader,
];

/// A named stream of the source.
#[derive(Debug)]
struct NamedContent {
    /// The name.
    name: Vec<u8>,
    /// The stream ID in the source.
    stream: StreamId,
    /// The stream bytes.
    bytes: Vec<u8>,
}

/// The streams of a native PDB for a semantic rebuild.
///
/// The value owns the stream bytes. [`PdbContents::to_builder`] borrows them.
#[derive(Debug)]
pub struct PdbContents {
    /// The Info stream.
    info: Info,
    /// The `/names` stream, if present.
    names: Option<Vec<u8>>,
    /// The TPI stream and its hash stream.
    tpi: (TypeStream, Option<Vec<u8>>),
    /// The IPI stream and its hash stream.
    ipi: Option<(TypeStream, Option<Vec<u8>>)>,
    /// The DBI stream.
    dbi: Dbi,
    /// The module stream of each module, or `None`.
    modules: Vec<Option<ModuleStream>>,
    /// The symbol record stream. It is empty if the PDB has none.
    symbols: Vec<u8>,
    /// The global symbol index.
    globals: Option<GlobalSymbolIndex>,
    /// The public symbol index.
    publics: Option<PublicSymbolIndex>,
    /// The thunk targets of the public symbol index.
    thunk_targets: Vec<u32>,
    /// The section starts of the public symbol index.
    section_starts: Vec<SectionStart>,
    /// The decoded and encoded optional debug streams by slot.
    debug: Vec<(DebugStream, Vec<u8>)>,
    /// The named streams other than `/names`.
    named: Vec<NamedContent>,
    /// The type map streams that `/TMCache` names, by source stream ID.
    type_maps: Vec<(StreamId, Vec<u8>)>,
    /// The module type streams that `S_MOD_TYPEREF` records name, by source
    /// stream ID.
    module_types: Vec<(StreamId, Vec<u8>)>,
    /// The present streams that no other stream names.
    unclaimed: Vec<StreamId>,
    /// The page size of the source container.
    page_size: u32,
}

impl PdbContents {
    /// Return the present streams of the source that no other stream names.
    ///
    /// The rebuild does not keep them, because no record gives their role.
    /// Stream 0 (the previous stream directory) is not in the list.
    #[must_use]
    pub fn unclaimed_streams(&self) -> &[StreamId] {
        &self.unclaimed
    }
}

/// Return the error for a stream that the rebuild needs.
fn missing(context: &'static str) -> Error {
    Error::new(ErrorKind::MissingStream, None).with_context(context)
}

/// Push a value with a checked allocation.
fn push<T>(list: &mut Vec<T>, value: T) -> Result<()> {
    list.try_reserve(1).map_err(|_| no_memory())?;
    list.push(value);
    Ok(())
}

/// Copy `bytes` with a checked allocation.
fn copy_bytes(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(bytes.len())
        .map_err(|_| no_memory())?;
    copy.extend_from_slice(bytes);
    Ok(copy)
}

impl<S: ReadAt> Pdb<S> {
    /// Read all streams of the PDB for a semantic rebuild.
    ///
    /// The function decodes the stream headers and each optional debug
    /// stream. [`PdbContents::to_builder`] decodes the records.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the PDB has no TPI or DBI stream.
    /// - [`ErrorKind::UnsupportedVersion`] for an optional debug header slot
    ///   after the slots that [`DebugStream`] names, if it holds a stream,
    ///   and for an `S_MOD_TYPEREF` record with flags whose words have no
    ///   known meaning ([`ModuleTypeSource::Other`]).
    /// - The errors of the stream readers and decoders.
    pub fn contents(&self) -> Result<PdbContents> {
        let mut claimed = Vec::new();
        let mut claim = |id: StreamId| push(&mut claimed, id);
        for id in 0..5 {
            claim(StreamId(id))?;
        }
        let info = Info::parse(self.stream_bytes(StreamId(1))?)?;
        let tpi = self
            .types()?
            .ok_or_else(|| missing("PDB has no TPI stream"))?;
        let tpi_hash = self.hash_stream(&tpi, &mut claim)?;
        let ipi = match self.ids()? {
            Some(ipi) => {
                let hash = self.hash_stream(&ipi, &mut claim)?;
                Some((ipi, hash))
            }
            None => None,
        };
        let dbi = self
            .debug_info()?
            .ok_or_else(|| missing("PDB has no DBI stream"))?;
        let mut modules = Vec::new();
        let mut module_types = Vec::new();
        for module in dbi.modules() {
            let module = module?;
            if let Some(id) = module.stream {
                claim(id)?;
            }
            let stream = self.load_module(&module)?;
            if let Some(stream) = &stream {
                self.module_types(stream, &mut module_types, &mut claim)?;
            }
            push(&mut modules, stream)?;
        }
        let header = *dbi.header();
        let symbols = match header.symbol_stream {
            Some(id) => {
                claim(id)?;
                copy_bytes(self.symbol_records()?.unwrap_or_default())?
            }
            None => Vec::new(),
        };
        for id in [header.global_stream, header.public_stream]
            .into_iter()
            .flatten()
        {
            claim(id)?;
        }
        let globals = self.globals()?.map(|globals| globals.index().clone());
        let publics = self.publics()?.map(|publics| publics.index().clone());
        let mut thunk_targets = Vec::new();
        let mut section_starts = Vec::new();
        if let Some(publics) = &publics {
            for target in publics.thunk_map() {
                push(&mut thunk_targets, target)?;
            }
            for start in publics.section_starts() {
                push(&mut section_starts, start)?;
            }
        }
        let debug = self.debug_streams(dbi, &mut claim)?;
        let mut names_bytes = None;
        let mut named = Vec::new();
        let mut type_maps = Vec::new();
        for entry in info.named_streams() {
            claim(entry.stream)?;
            let bytes = self.stream_bytes(entry.stream)?;
            match NamedStreamKind::classify(entry.name) {
                NamedStreamKind::Names => names_bytes = Some(bytes),
                kind => {
                    if kind == NamedStreamKind::TmCache {
                        self.type_maps(&bytes, entry.stream, &mut type_maps, &mut claim)?;
                    }
                    let mut name = Vec::new();
                    name.try_reserve_exact(entry.name.len())
                        .map_err(|_| no_memory())?;
                    name.extend_from_slice(entry.name);
                    push(
                        &mut named,
                        NamedContent {
                            name,
                            stream: entry.stream,
                            bytes,
                        },
                    )?;
                }
            }
        }
        let mut unclaimed = Vec::new();
        for id in 0..self.container().stream_count() {
            let id = StreamId(id);
            if !claimed.contains(&id) && self.container().stream(id)?.is_some() {
                push(&mut unclaimed, id)?;
            }
        }
        Ok(PdbContents {
            info,
            names: names_bytes,
            tpi: (tpi, tpi_hash),
            ipi,
            dbi: dbi.clone(),
            modules,
            symbols,
            globals,
            publics,
            thunk_targets,
            section_starts,
            debug,
            named,
            type_maps,
            module_types,
            unclaimed,
            page_size: self.output_page_size(),
        })
    }

    /// Read the hash stream of a type stream.
    fn hash_stream(
        &self,
        stream: &TypeStream,
        claim: &mut impl FnMut(StreamId) -> Result<()>,
    ) -> Result<Option<Vec<u8>>> {
        let Some(id) = stream.header().hash_stream else {
            return Ok(None);
        };
        claim(id)?;
        self.stream_bytes(id).map(Some)
    }

    /// Decode each optional debug stream and encode it again.
    fn debug_streams(
        &self,
        dbi: &Dbi,
        claim: &mut impl FnMut(StreamId) -> Result<()>,
    ) -> Result<Vec<(DebugStream, Vec<u8>)>> {
        let mut streams = Vec::new();
        let missing = || missing("optional debug stream is missing");
        for (slot, id) in dbi.debug_streams().enumerate() {
            let Some(id) = id else { continue };
            claim(id)?;
            let kind = *DEBUG_KINDS.get(slot).ok_or_else(|| {
                Error::new(ErrorKind::UnsupportedVersion, None)
                    .with_context("optional debug header slot has no known stream kind")
            })?;
            let bytes = match kind {
                DebugStream::Fpo => self.fpo()?.ok_or_else(missing)?.to_bytes()?,
                DebugStream::NewFpo => self.frame_data()?.ok_or_else(missing)?.to_bytes()?,
                DebugStream::Exception => self.exception_data()?.ok_or_else(missing)?.to_bytes()?,
                DebugStream::Fixup => self.fixups()?.ok_or_else(missing)?.to_bytes()?,
                DebugStream::OmapToSource | DebugStream::OmapFromSource => {
                    self.omap(kind)?.ok_or_else(missing)?.to_bytes()?
                }
                DebugStream::SectionHeader | DebugStream::OriginalSectionHeader => self
                    .section_headers(kind)?
                    .ok_or_else(missing)?
                    .to_bytes()?,
                DebugStream::TokenRidMap => {
                    self.token_rid_map()?.ok_or_else(missing)?.to_bytes()?
                }
                DebugStream::Xdata | DebugStream::Pdata => {
                    self.debug_blob(kind)?.ok_or_else(missing)?.to_bytes()?
                }
            };
            push(&mut streams, (kind, bytes))?;
        }
        Ok(streams)
    }

    /// Read the streams that the `S_MOD_TYPEREF` record of a module names.
    fn module_types(
        &self,
        stream: &ModuleStream,
        streams: &mut Vec<(StreamId, Vec<u8>)>,
        claim: &mut impl FnMut(StreamId) -> Result<()>,
    ) -> Result<()> {
        let Some(reference) = ModuleTypeReference::find(stream.symbol_data())? else {
            return Ok(());
        };
        let source = reference.source();
        if let ModuleTypeSource::Other(_) = source {
            return Err(Error::new(ErrorKind::UnsupportedVersion, None)
                .with_context("S_MOD_TYPEREF record has flags with words of unknown meaning"));
        }
        for id in source.streams() {
            if streams.iter().any(|(existing, _)| *existing == id) {
                continue;
            }
            claim(id)?;
            push(streams, (id, self.stream_bytes(id)?))?;
        }
        Ok(())
    }

    /// Read the type map streams that a `/TMCache` stream names.
    fn type_maps(
        &self,
        cache: &[u8],
        stream: StreamId,
        maps: &mut Vec<(StreamId, Vec<u8>)>,
        claim: &mut impl FnMut(StreamId) -> Result<()>,
    ) -> Result<()> {
        let cache = TmCache::parse(cache, stream)?;
        for id in cache.streams().flatten() {
            if maps.iter().any(|(existing, _)| *existing == id) {
                continue;
            }
            claim(id)?;
            push(maps, (id, self.stream_bytes(id)?))?;
        }
        Ok(())
    }
}
