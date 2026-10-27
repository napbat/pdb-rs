//! The report of a write plan: how the plan writes each stream (rule 10 of #42).

use alloc::vec::Vec;

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
}

impl PlanReport {
    /// Make a report from its entries.
    pub(super) const fn new(streams: Vec<StreamReport>) -> Self {
        Self { streams }
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
}
