//! The report of a write plan: the change of each metadata stream.

use alloc::vec::Vec;

use crate::portable::metadata::MetadataStream;

/// The streams of the output in file order.
pub(super) const STREAMS: [MetadataStream; 6] = [
    MetadataStream::Pdb,
    MetadataStream::Tables,
    MetadataStream::Strings,
    MetadataStream::UserStrings,
    MetadataStream::Guid,
    MetadataStream::Blob,
];

/// Why the plan wrote a stream again that the model does not change.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum RebuildReason {
    /// The PDB ID changed: only the first 20 bytes of `#Pdb` are different.
    Identity,
}

/// The change of one stream in the output, compared with the source.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum StreamChange {
    /// The output stream has the bytes of the source stream. This includes a stream that the
    /// plan encoded again with the same bytes.
    Copied,
    /// The output stream has other bytes, because the model changed.
    Edited,
    /// The output stream has other bytes for a reason outside the model.
    Rebuilt(RebuildReason),
    /// The source has no such stream, or the plan has no source.
    Added,
}

/// The change of each stream of a [`PdbWritePlan`](crate::portable::PdbWritePlan).
///
/// The report lists `#Pdb`, `#~`, `#Strings`, `#US`, `#GUID` and `#Blob` in file order. The
/// plan of an editor with a source compares each output stream with the source stream. A
/// plan without a source ([`PdbEditor::new`](crate::portable::PdbEditor::new),
/// [`PdbParts::plan`](crate::portable::write::PdbParts::plan)) reports each stream as
/// [`StreamChange::Added`].
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PlanReport {
    streams: Vec<(MetadataStream, StreamChange)>,
}

impl PlanReport {
    /// Make a report with one change for each stream of the output, in file order.
    pub(super) fn new(changes: [StreamChange; 6]) -> Self {
        Self {
            streams: STREAMS.into_iter().zip(changes).collect(),
        }
    }

    /// Return each stream of the output with its change, in file order.
    #[must_use]
    pub fn streams(&self) -> &[(MetadataStream, StreamChange)] {
        &self.streams
    }

    /// Return the change of a stream, or `None` if the output has no such stream.
    #[must_use]
    pub fn stream(&self, stream: MetadataStream) -> Option<StreamChange> {
        self.streams
            .iter()
            .find_map(|&(name, change)| (name == stream).then_some(change))
    }
}
