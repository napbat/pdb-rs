//! The module streams of a plan: the scope tree, the line subsections and
//! the global references.

use alloc::borrow::Cow;
use alloc::vec::Vec;

use crate::native::codeview::SymbolKind;
use crate::native::codeview::index::ModuleSymbolOffset;
use crate::native::codeview::symbol::{ModuleSymbolWriter, SymbolData};
use crate::native::dbi::{Module, ModuleStream, ModuleStreamParts};
use crate::native::editor::module::{ModuleContents, SYMBOL_BASE, Symbols, symbol_base};
use crate::native::editor::record::SymbolRecord;
use crate::native::editor::tree;
use crate::native::lines::ModuleLines;
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::native::symbols::SymbolOffset;
use crate::{Bytes, Error, ErrorKind, Result};

/// The new offsets of the source records of one module.
#[derive(Clone, Debug, Default)]
pub(super) struct OffsetMap {
    /// Pairs of a source offset and a new offset, sorted by source offset.
    /// `None` for a module whose records keep their offsets.
    pairs: Option<Vec<(u32, u32)>>,
}

impl OffsetMap {
    /// Return the map of a module whose records keep their offsets.
    pub(super) const fn identity() -> Self {
        Self { pairs: None }
    }

    /// Return the new offset of the source record at `offset`, or `None` if
    /// the model does not have the record any more.
    pub(super) fn get(&self, offset: u32) -> Option<u32> {
        match &self.pairs {
            None => Some(offset),
            Some(pairs) => pairs
                .binary_search_by_key(&offset, |&(old, _)| old)
                .ok()
                .map(|position| pairs[position].1),
        }
    }

    /// Return `true` if a record moved or is gone.
    pub(super) fn moved(&self, source_count: usize) -> bool {
        self.pairs.as_ref().is_some_and(|pairs| {
            pairs.len() != source_count || pairs.iter().any(|(old, new)| old != new)
        })
    }
}

/// The encoded regions of one module stream before its global references
/// resolve.
pub(super) struct EncodedModule {
    /// The symbol records without the signature, or `None`.
    pub(super) symbols: Option<Vec<u8>>,
    /// The C11 line region of the source.
    pub(super) c11: Vec<u8>,
    /// The C13 line region.
    pub(super) c13: Vec<u8>,
    /// The module stream offset of each record by its number.
    pub(super) ids: Vec<Option<u32>>,
    /// The new offsets of the source records.
    pub(super) map: OffsetMap,
    /// The number of source records.
    pub(super) source_count: usize,
}

/// Return the fields of an `S_MOD_TYPEREF` record with mapped stream IDs, or
/// `None` if the record is another record or keeps its stream IDs.
fn mapped_type_reference(
    record: &SymbolRecord,
    map_stream: &dyn Fn(StreamId) -> Result<StreamId>,
) -> Result<Option<SymbolData<'static>>> {
    let SymbolData::ModuleTypeReference(reference) = record.data() else {
        return Ok(None);
    };
    let mut mapped = *reference;
    let mut changed = false;
    mapped.map_streams(|id| {
        let new = map_stream(id)?;
        changed |= new != id;
        Ok(new)
    })?;
    Ok(changed.then_some(SymbolData::ModuleTypeReference(mapped)))
}

/// Encode one record of a module.
fn push_record(
    record: &SymbolRecord,
    writer: &mut ModuleSymbolWriter,
    map_stream: &dyn Fn(StreamId) -> Result<StreamId>,
) -> Result<ModuleSymbolOffset> {
    match mapped_type_reference(record, map_stream)? {
        Some(data) => writer.push(record.kind(), &data),
        None => record.push_module(writer),
    }
}

/// Encode the symbol tree and the lines of a module.
///
/// `map_stream` gives the output stream ID of a model stream ID, for the
/// `S_MOD_TYPEREF` records.
pub(super) fn encode_regions(
    module: &ModuleContents,
    map_stream: &dyn Fn(StreamId) -> Result<StreamId>,
) -> Result<EncodedModule> {
    if module.has_open_scope() {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("module has a scope without an end record"));
    }
    let source_stream = module.source().and_then(|source| source.stream.as_ref());
    let base = source_stream.map_or(SYMBOL_BASE, symbol_base);
    let mut ids = Vec::new();
    let mut pairs = Vec::new();
    let mut source_count = 0;
    let symbols = match module.symbol_region() {
        Symbols::Absent => None,
        Symbols::Opaque => {
            let stream = source_stream.ok_or_else(|| {
                Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("opaque module has no source stream")
            })?;
            Some(copy(stream.symbol_data())?)
        }
        Symbols::Tree(nodes) => {
            let mut writer = ModuleSymbolWriter::new(ModuleSymbolOffset(SYMBOL_BASE));
            tree::walk(nodes, &mut |record| {
                let offset = push_record(record, &mut writer, map_stream)?;
                if let Some(id) = record.id() {
                    let slot = id as usize;
                    if ids.len() <= slot {
                        ids.try_reserve(slot + 1 - ids.len())
                            .map_err(|_| no_memory())?;
                        ids.resize(slot + 1, None);
                    }
                    ids[slot] = Some(offset.0);
                }
                if let Some(origin) = record.origin() {
                    pairs.try_reserve(1).map_err(|_| no_memory())?;
                    pairs.push((origin.record().offset + base, offset.0));
                }
                Ok(())
            })?;
            Some(writer.finish()?)
        }
    };
    if let Some(stream) = source_stream
        && let Ok(count) = crate::native::codeview::Records::new(stream.symbol_data())
            .try_fold(0_usize, |count, record| record.map(|_| count + 1))
    {
        source_count = count;
    }
    pairs.sort_unstable();
    let map = if matches!(module.symbol_region(), Symbols::Tree(_)) {
        OffsetMap { pairs: Some(pairs) }
    } else {
        OffsetMap::identity()
    };
    let c13 = match source_stream {
        Some(stream) if !module.lines_changed() => copy(stream.c13_data())?,
        _ => {
            let mut c13 = Vec::new();
            for subsection in module.lines() {
                subsection.encode_framed(&mut c13)?;
            }
            c13
        }
    };
    let c11 = match source_stream {
        Some(stream) => copy(stream.c11_data())?,
        None => Vec::new(),
    };
    Ok(EncodedModule {
        symbols,
        c11,
        c13,
        ids,
        map,
        source_count,
    })
}

/// Copy `bytes` with a checked allocation.
fn copy(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(bytes.len())
        .map_err(|_| no_memory())?;
    copy.extend_from_slice(bytes);
    Ok(copy)
}

/// Join the regions of a module with its global references.
///
/// Set the stream, the region sizes and the source file count of the module
/// record, and return the record and the stream bytes.
pub(super) fn join(
    module: &ModuleContents,
    regions: &EncodedModule,
    refs: &[SymbolOffset],
    stream: StreamId,
) -> Result<(Module<'static>, Vec<u8>)> {
    let mut record = module.record().clone();
    record.source_file_count = u16::try_from(module.files().len()).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("module has more than 65535 source files")
    })?;
    record.stream = Some(stream);
    let mut bytes = Vec::new();
    let sizes = ModuleStreamParts {
        symbols: regions.symbols.as_deref().map(Bytes::new),
        c11: Bytes::new(&regions.c11),
        c13: Bytes::new(&regions.c13),
        global_refs: Cow::Borrowed(refs),
    }
    .encode(&mut bytes)
    .map_err(|error| error.with_stream(stream, 0))?;
    record.symbol_bytes = sizes.symbol_bytes;
    record.c11_bytes = sizes.c11_bytes;
    record.c13_bytes = sizes.c13_bytes;
    let parsed = ModuleStream::parse(bytes, &record)?;
    ModuleLines::parse(&parsed)?.check()?;
    Ok((record, parsed.into_bytes()))
}

/// Return the record of a module without a module stream.
pub(super) fn streamless(module: &ModuleContents) -> Result<Module<'static>> {
    let has_symbols = match module.symbol_region() {
        Symbols::Absent => false,
        Symbols::Tree(nodes) => !nodes.is_empty(),
        Symbols::Opaque => true,
    };
    if has_symbols || !module.lines().is_empty() || module.global_ref_count() != 0 {
        return Err(Error::new(ErrorKind::InvalidFormat, None).with_context(
            "module without a module stream has symbols, subsections or references",
        ));
    }
    let mut record = module.record().clone();
    record.source_file_count = u16::try_from(module.files().len()).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("module has more than 65535 source files")
    })?;
    record.stream = None;
    record.symbol_bytes = 0;
    record.c11_bytes = 0;
    record.c13_bytes = 0;
    Ok(record)
}

/// Return the kind of each record of the encoded symbols of a module, by
/// module stream offset.
pub(super) fn record_kinds(symbols: &[u8]) -> Result<Vec<(u32, SymbolKind)>> {
    let mut kinds = Vec::new();
    for record in crate::native::codeview::Records::new(symbols) {
        let record = record?;
        kinds.try_reserve(1).map_err(|_| no_memory())?;
        kinds.push((record.offset + SYMBOL_BASE, SymbolKind(record.kind)));
    }
    Ok(kinds)
}
