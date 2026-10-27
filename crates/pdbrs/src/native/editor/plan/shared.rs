//! The symbol record stream and the global and public symbol indexes of a
//! plan.

use alloc::vec::Vec;

use super::modules::OffsetMap;
use super::patch::Patch;
use crate::native::Producer;
use crate::native::codeview::Records;
use crate::native::codeview::index::{ModuleIndex, ModuleSymbolHandle, ModuleSymbolOffset};
use crate::native::codeview::symbol::{ModuleSymbolTarget, ProcedureReference, Symbol, SymbolData};
use crate::native::editor::record::{SymbolRecord, append_source};
use crate::native::editor::symbols::{IndexEntry, SharedSymbols};
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::native::symbols::{
    BucketCount, GlobalSymbolIndex, HashEntry, PublicSymbolIndex, SymbolOffset,
};
use crate::{Bytes, Error, ErrorKind, Result};

/// The position of the target offset in a reference record: after the
/// length, the kind and the name checksum.
const TARGET_OFFSET: usize = 8;
/// The position of the module number in a reference record.
const TARGET_MODULE: usize = 12;

/// The new positions of the module records.
pub(super) trait ModuleTargets {
    /// Return the offset of a module record of the model.
    fn handle(&self, handle: ModuleSymbolHandle) -> Result<ModuleSymbolOffset>;
    /// Return the offset map of a module, or `None` if the plan keeps the
    /// module stream.
    fn map(&self, module: ModuleIndex) -> Option<&OffsetMap>;
    /// Return the name of the module record that `target` names in the
    /// model, or `None` if its module is not decoded or has no such record.
    fn name(&self, target: ModuleSymbolTarget) -> Option<&[u8]>;
}

/// Return the module of a reference target, if the target gives one.
pub(super) const fn target_module(target: ModuleSymbolTarget) -> Option<ModuleIndex> {
    match target {
        ModuleSymbolTarget::Handle(handle) => Some(ModuleIndex(handle.module)),
        ModuleSymbolTarget::Position { module, .. } => module,
    }
}

/// The encoded symbol record stream.
pub(super) struct EncodedShared {
    /// The stream bytes.
    pub(super) bytes: Vec<u8>,
    /// The offset of each record by its handle, or `None` for a removed
    /// record.
    pub(super) offsets: Vec<Option<SymbolOffset>>,
    /// Pairs of a source offset and a new offset, in source order.
    pub(super) pairs: Vec<(u32, u32)>,
    /// The number of source records.
    pub(super) source_count: usize,
}

impl EncodedShared {
    /// Return `true` if a source record moved or is gone.
    pub(super) fn moved(&self) -> bool {
        self.pairs.len() != self.source_count || self.pairs.iter().any(|(old, new)| old != new)
    }

    /// Return the new offset of the source record at `offset`, or `None` if
    /// the model does not have the record any more.
    pub(super) fn get(&self, offset: SymbolOffset) -> Option<SymbolOffset> {
        self.pairs
            .binary_search_by_key(&offset.0, |&(old, _)| old)
            .ok()
            .map(|position| SymbolOffset(self.pairs[position].1))
    }
}

/// Return the reference of a reference record.
const fn reference<'r>(data: &'r SymbolData<'_>) -> Option<&'r ProcedureReference<'r>> {
    match data {
        SymbolData::ProcedureReference(reference) | SymbolData::SymbolReference(reference) => {
            Some(reference)
        }
        _ => None,
    }
}

/// Return the fields of a reference record with a new target.
fn with_target(data: &SymbolData<'static>, target: ModuleSymbolTarget) -> SymbolData<'static> {
    match data {
        SymbolData::ProcedureReference(reference) => {
            let mut reference = reference.clone();
            reference.target = target;
            SymbolData::ProcedureReference(reference)
        }
        SymbolData::SymbolReference(reference) => {
            let mut reference = reference.clone();
            reference.target = target;
            SymbolData::SymbolReference(reference)
        }
        other => other.clone(),
    }
}

/// Return the error for a reference record whose target is gone.
fn gone(stream: Option<StreamId>, offset: u32) -> Error {
    let error = Error::new(ErrorKind::InvalidReference, None).with_context(
        "a reference record names a module symbol record that the model does not have",
    );
    match stream {
        Some(stream) => error.with_stream(stream, u64::from(offset)),
        None => error,
    }
}

/// Return the new target of a reference, or `None` if it keeps its target.
fn new_target(
    target: ModuleSymbolTarget,
    modules: &dyn ModuleTargets,
    stream: Option<StreamId>,
    offset: u32,
) -> Result<Option<ModuleSymbolTarget>> {
    match target {
        ModuleSymbolTarget::Handle(handle) => Ok(Some(ModuleSymbolTarget::Position {
            offset: modules.handle(handle)?,
            module: Some(handle.module()),
        })),
        ModuleSymbolTarget::Position {
            module: Some(module),
            offset: target,
        } => match modules.map(module) {
            None => Ok(None),
            Some(map) => {
                let new = map.get(target.0).ok_or_else(|| gone(stream, offset))?;
                Ok((new != target.0).then_some(ModuleSymbolTarget::Position {
                    module: Some(module),
                    offset: ModuleSymbolOffset(new),
                }))
            }
        },
        ModuleSymbolTarget::Position { module: None, .. } => Ok(None),
    }
}

/// Append one record of the model to `out`.
fn encode_record(
    record: &SymbolRecord,
    modules: &dyn ModuleTargets,
    stream: Option<StreamId>,
    out: &mut Vec<u8>,
) -> Result<()> {
    let source_offset = record.origin().map_or(0, |origin| origin.record().offset);
    let Some(target) = reference(record.data()).map(|reference| reference.target) else {
        return record.encode(out);
    };
    // A reference record can get a new name only together with its target:
    // the module record holds the same name (#41 rule 4).
    if let (Some(origin), None) = (record.origin(), record.unchanged())
        && origin.name() != record.data().name()
        && modules.name(target) != record.name()
    {
        return Err(Error::new(ErrorKind::UnsupportedEdit, None)
            .with_context(
                "a reference record keeps its name unless its module record gets the same name",
            )
            .with_stream(stream.unwrap_or(StreamId(0)), u64::from(source_offset)));
    }
    let Some(target) = new_target(target, modules, stream, source_offset)? else {
        return record.encode(out);
    };
    match (record.unchanged(), target) {
        (Some(symbol), ModuleSymbolTarget::Position { offset, .. }) => {
            // Keep the source bytes and their padding; write the new offset.
            let start = out.len();
            append_source(symbol, out)?;
            out[start + TARGET_OFFSET..start + TARGET_MODULE]
                .copy_from_slice(&offset.0.to_le_bytes());
            Ok(())
        }
        _ => with_target(record.data(), target).encode(record.kind(), out),
    }
}

/// Encode the records of the model in stream order.
pub(super) fn encode_records(
    model: &SharedSymbols,
    modules: &dyn ModuleTargets,
    stream: Option<StreamId>,
) -> Result<EncodedShared> {
    let mut encoded = EncodedShared {
        bytes: Vec::new(),
        offsets: Vec::new(),
        pairs: Vec::new(),
        source_count: model.source.as_ref().map_or(0, |source| source.count),
    };
    encoded
        .offsets
        .try_reserve_exact(model.records.len())
        .map_err(|_| no_memory())?;
    for record in &model.records {
        let origin = record
            .as_ref()
            .and_then(SymbolRecord::origin)
            .map(|origin| origin.record().offset);
        let Some(record) = record else {
            encoded.offsets.push(None);
            continue;
        };
        let offset = u32::try_from(encoded.bytes.len()).map_err(|_| {
            Error::new(ErrorKind::OffsetOverflow, None)
                .with_context("symbol record offset does not fit in 32 bits")
        })?;
        encode_record(record, modules, stream, &mut encoded.bytes)?;
        encoded.offsets.push(Some(SymbolOffset(offset)));
        if let Some(old) = origin {
            encoded.pairs.try_reserve(1).map_err(|_| no_memory())?;
            encoded.pairs.push((old, offset));
        }
    }
    encoded.pairs.sort_unstable();
    Ok(encoded)
}

/// Return the hash entries of `entries` at their new offsets.
///
/// A record with a new name goes into the bucket of its new name.
fn hash_entries(
    model: &SharedSymbols,
    entries: &[IndexEntry],
    encoded: &EncodedShared,
) -> Result<Vec<HashEntry>> {
    let mut list = Vec::new();
    list.try_reserve_exact(entries.len())
        .map_err(|_| no_memory())?;
    for entry in entries {
        let symbol = encoded
            .offsets
            .get(entry.symbol.0 as usize)
            .copied()
            .flatten()
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidReference, None)
                    .with_context("symbol index entry has no record")
            })?;
        let renamed = model.record(entry.symbol).is_some_and(|record| {
            record
                .origin()
                .is_some_and(|origin| origin.name().map(Bytes::as_bytes) != record.name())
        });
        list.push(HashEntry {
            symbol,
            reference_count: entry.reference_count,
            bucket: if renamed { None } else { entry.bucket },
        });
    }
    Ok(list)
}

/// Encode the global symbol index.
pub(super) fn encode_globals(
    model: &SharedSymbols,
    encoded: &EncodedShared,
    buckets: BucketCount,
    producer: Producer,
) -> Result<Vec<u8>> {
    let entries = hash_entries(model, &model.globals, encoded)?;
    let mut bytes = Vec::new();
    GlobalSymbolIndex::encode_parts(&encoded.bytes, &entries, buckets, producer, &mut bytes)?;
    Ok(bytes)
}

/// Encode the public symbol index.
pub(super) fn encode_publics(
    model: &SharedSymbols,
    encoded: &EncodedShared,
    buckets: BucketCount,
    producer: Producer,
) -> Result<Vec<u8>> {
    let entries = hash_entries(model, &model.publics, encoded)?;
    let mut bytes = Vec::new();
    PublicSymbolIndex::encode_parts(
        &encoded.bytes,
        &entries,
        buckets,
        producer,
        model.thunks.table(),
        &model.section_starts,
        &mut bytes,
    )?;
    Ok(bytes)
}

/// Return `true` if `entries` differ from the source entries or name a
/// record with a new name.
pub(super) fn index_changed(
    model: &SharedSymbols,
    entries: &[IndexEntry],
    source: Option<&[IndexEntry]>,
) -> bool {
    source != Some(entries)
        || entries.iter().any(|entry| {
            model.record(entry.symbol).is_some_and(|record| {
                record
                    .origin()
                    .is_some_and(|origin| origin.name().map(Bytes::as_bytes) != record.name())
            })
        })
}

/// Return the new targets of the reference records of an undecoded symbol
/// record stream as patches (#48). Return an empty list if no target moved.
pub(super) fn patch_targets(
    records: &[u8],
    modules: &dyn ModuleTargets,
    stream: StreamId,
) -> Result<Vec<Patch>> {
    let mut patches = Vec::new();
    for record in Records::new(records) {
        let record = record?;
        let symbol = Symbol::decode(record)?;
        let Some(target) = symbol.reference().map(|reference| reference.target) else {
            continue;
        };
        let Some(ModuleSymbolTarget::Position { offset, .. }) =
            new_target(target, modules, Some(stream), record.offset)?
        else {
            continue;
        };
        patches.try_reserve(1).map_err(|_| no_memory())?;
        patches.push(Patch::new(record.offset as usize + TARGET_OFFSET, offset.0));
    }
    Ok(patches)
}
