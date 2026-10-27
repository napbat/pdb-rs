//! The report of a write plan: how the plan writes each stream (rule 10 of #42).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::native::RemapItem;
use crate::native::msf::StreamId;

/// A space of values that records use to refer to other records.
///
/// An edit can change the values of a space, for example when a symbol record
/// moves. The plan then writes the new values into each decoded reference.
/// An error of a rejected edit and a [`StreamChange::Remapped`] stream name
/// the space.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum IndexSpace {
    /// The type indices of the TPI stream.
    TypeIndex,
    /// The ID indices of the IPI stream.
    IdIndex,
    /// The offsets of the records in the symbol record stream.
    SymbolOffset,
    /// The offsets of the symbol records in a module stream.
    ModuleSymbolOffset,
    /// The stream IDs of the MSF stream directory.
    StreamId,
    /// The offsets of the names in `/names`.
    NameOffset,
}

/// The reason why the plan wrote a stream that the caller did not change.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RebuildReason {
    /// The identity changed: the Info stream or the DBI age.
    Identity,
    /// The named-stream map or the feature codes of the Info stream changed.
    NamedStreamMap,
    /// The records of the TPI or IPI stream changed, so its hash stream changed.
    TypeHashes,
    /// The records of the symbol record stream changed, so the global or the
    /// public symbol index changed.
    SymbolIndexes,
    /// A module changed, so the DBI module records or substreams changed.
    ModuleRecords,
    /// A debug stream got a new slot, so the optional debug header of the DBI
    /// stream changed.
    DebugHeader,
    /// The strip of private symbols ([`EditOptions::with_strip_private`](super::EditOptions::with_strip_private)).
    Strip,
    /// The plan wrote all streams in a new layout
    /// ([`PdbEditor::decode_all`](super::PdbEditor::decode_all)).
    Layout,
    /// The previous stream directory (stream 0): the plan writes it empty,
    /// because its pages describe the source layout.
    Container,
}

/// How the plan writes one stream.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum StreamChange {
    /// The output stream has the bytes of the source stream. This is also
    /// true for a stream that the plan encoded again with the same bytes.
    Copied,
    /// The caller changed the records of the stream.
    Edited,
    /// The plan derived new bytes from other changes.
    Rebuilt(RebuildReason),
    /// The plan wrote new values of references to records that moved.
    Remapped(IndexSpace),
    /// The source has no such stream, or the editor has no source.
    Added,
    /// The output does not keep the source stream. In the copy-on-write
    /// layout, the output stream is a nil stream.
    Removed,
}

/// The change of one stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct StreamReport {
    /// The source stream, or `None` for a new stream.
    pub source: Option<StreamId>,
    /// The output stream, or `None` for a source stream that the output does
    /// not keep.
    pub output: Option<StreamId>,
    /// The change.
    pub change: StreamChange,
}

/// The report of a [`PdbWritePlan`](crate::native::PdbWritePlan): one entry for each
/// output stream and for each source stream that the output does not keep.
///
/// The entries of the output streams come first, in output stream order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PlanReport {
    /// The entries.
    streams: Vec<StreamReport>,
    /// The counts of clean remap output, if the plan wrote it.
    remap: Option<RemapReport>,
}

impl PlanReport {
    /// Make a report from its entries.
    pub(super) const fn new(streams: Vec<StreamReport>, remap: Option<RemapReport>) -> Self {
        Self { streams, remap }
    }

    /// Return all entries.
    #[must_use]
    pub fn streams(&self) -> &[StreamReport] {
        &self.streams
    }

    /// Return the entry of the output stream `id`.
    #[must_use]
    pub fn output(&self, id: StreamId) -> Option<&StreamReport> {
        self.streams.iter().find(|entry| entry.output == Some(id))
    }

    /// Return the output streams after stream 0 that do not have the bytes of
    /// their source stream, in output stream order.
    ///
    /// The list has the edited, rebuilt, remapped, added and removed streams.
    /// A removed stream of the copy-on-write layout is a nil stream in the
    /// output, so the list has it.
    pub fn changed(&self) -> impl Iterator<Item = StreamId> + '_ {
        self.streams.iter().filter_map(|entry| {
            entry
                .output
                .filter(|id| id.0 != 0 && entry.change != StreamChange::Copied)
        })
    }

    /// Return the source streams that the output does not keep.
    ///
    /// After [`PdbEditor::decode_all`](super::PdbEditor::decode_all), these
    /// are the source streams that no other stream names.
    pub fn dropped(&self) -> impl Iterator<Item = StreamId> + '_ {
        self.streams
            .iter()
            .filter(|entry| entry.output.is_none())
            .filter_map(|entry| entry.source)
    }

    /// Return the counts of clean remap output, or `None` if the plan did not
    /// write clean output ([`RemapMode::Clean`](crate::native::RemapMode::Clean)).
    #[must_use]
    pub const fn remap(&self) -> Option<&RemapReport> {
        self.remap.as_ref()
    }
}

/// The counts of clean remap output (decision 13 of #43).
///
/// [`PlanReport::remap`] gives the counts. A record that holds removed code
/// counts as removed with [`RemovedCode::Drop`](crate::native::RemovedCode::Drop).
/// A record that a range splits counts each part after the first one as an
/// added part, for example an `S_SEPCODE` scope of a procedure or a line
/// block of a `DEBUG_S_LINES` subsection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RemapReport {
    /// The number of removed records of each kind.
    removed: BTreeMap<RemapItem, usize>,
    /// The number of added parts of each kind.
    added: BTreeMap<RemapItem, usize>,
    /// The number of section contributions with new CRC values.
    cleared_crcs: usize,
}

impl RemapReport {
    /// Return the number of removed records of `item`.
    #[must_use]
    pub fn removed(&self, item: RemapItem) -> usize {
        self.removed.get(&item).copied().unwrap_or(0)
    }

    /// Return the kinds with removed records and their counts, in order of the kind.
    pub fn removed_items(&self) -> impl Iterator<Item = (RemapItem, usize)> + '_ {
        self.removed.iter().map(|(item, count)| (*item, *count))
    }

    /// Return the number of added parts of `item`.
    #[must_use]
    pub fn added(&self, item: RemapItem) -> usize {
        self.added.get(&item).copied().unwrap_or(0)
    }

    /// Return the kinds with added parts and their counts, in order of the kind.
    pub fn added_items(&self) -> impl Iterator<Item = (RemapItem, usize)> + '_ {
        self.added.iter().map(|(item, count)| (*item, *count))
    }

    /// Return the number of section contributions (also in the DBI module
    /// records) that got the data CRC 0 and the relocation CRC 0, because
    /// their address or size changed (decision 6 of #43).
    #[must_use]
    pub const fn cleared_crcs(&self) -> usize {
        self.cleared_crcs
    }

    /// Count `count` removed records of `item`.
    pub(in crate::native::editor) fn count_removed(&mut self, item: RemapItem, count: usize) {
        add(&mut self.removed, item, count);
    }

    /// Count `count` added parts of `item`.
    pub(in crate::native::editor) fn count_added(&mut self, item: RemapItem, count: usize) {
        add(&mut self.added, item, count);
    }

    /// Count `count` contributions with new CRC values.
    pub(in crate::native::editor) const fn count_cleared_crcs(&mut self, count: usize) {
        self.cleared_crcs += count;
    }
}

/// Add `count` to the entry of `item`. A count of 0 adds no entry.
///
/// The map has at most one entry for each item kind, so it stays small.
fn add(counts: &mut BTreeMap<RemapItem, usize>, item: RemapItem, count: usize) {
    if count != 0 {
        *counts.entry(item).or_insert(0) += count;
    }
}
