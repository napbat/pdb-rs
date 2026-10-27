//! The DBI values, the optional debug streams and the streams without a
//! name of the editor model.

use alloc::vec::Vec;

use crate::native::dbi::{
    Dbi, DbiBuilder, DbiHeader, DebugStream, SectionContribution, SectionContributionVersion,
    SectionMap, TypeServer,
};
use crate::native::msf::StreamId;
use crate::native::names::{HashVersion, StringTableBuilder};
use crate::native::output::no_memory;
use crate::{Error, ErrorKind, Result};

/// The DBI values that the plan does not compute.
///
/// [`PdbEditor::debug_info_mut`](super::PdbEditor::debug_info_mut) gives
/// the values. The plan sets the age, the stream fields of the header, the
/// module records, the source info substream and the stream IDs of the
/// optional debug header.
#[derive(Clone, Debug)]
pub struct DebugInfo {
    /// The DBI values. In the copy-on-write layout, the module records, the
    /// source info and the debug header slots are the source values.
    pub(super) dbi: DbiBuilder,
    /// The smallest number of slots in the optional debug header.
    pub(super) debug_slots: usize,
}

impl DebugInfo {
    /// Make the values of a PDB without a source, as Microsoft `link.exe`
    /// writes them: an empty EC name table and no debug header slots.
    pub(super) fn new() -> Result<Self> {
        let mut dbi = DbiBuilder::new(DbiHeader::default());
        // Microsoft `link.exe` writes an empty EC name table, and LLVM reads
        // it for the name indices of the module records.
        dbi.ec_names = Some(StringTableBuilder::new(HashVersion::V1)?);
        Ok(Self {
            dbi,
            debug_slots: 0,
        })
    }

    /// Decode the values of a source DBI stream.
    pub(super) fn decode(source: &Dbi) -> Result<Self> {
        let dbi = DbiBuilder::from_dbi(source)?;
        let debug_slots = dbi.debug_streams.len();
        Ok(Self { dbi, debug_slots })
    }

    /// Return the header values.
    #[must_use]
    pub const fn header(&self) -> &DbiHeader {
        &self.dbi.header
    }

    /// Return the header values for a change.
    ///
    /// The plan replaces the age and the global, public and symbol record
    /// stream fields.
    pub const fn header_mut(&mut self) -> &mut DbiHeader {
        &mut self.dbi.header
    }

    /// Return the layout of the section contribution substream.
    #[must_use]
    pub const fn section_contribution_version(&self) -> SectionContributionVersion {
        self.dbi.section_contribution_version
    }

    /// Return the section contributions.
    #[must_use]
    pub fn section_contributions(&self) -> &[SectionContribution] {
        &self.dbi.section_contributions
    }

    /// Set the section contributions and the layout of their substream.
    pub fn set_section_contributions(
        &mut self,
        version: SectionContributionVersion,
        contributions: Vec<SectionContribution>,
    ) {
        self.dbi.section_contribution_version = version;
        self.dbi.section_contributions = contributions;
    }

    /// Return the section map.
    #[must_use]
    pub const fn section_map(&self) -> Option<&SectionMap> {
        self.dbi.section_map.as_ref()
    }

    /// Set the section map. `None` writes an empty substream.
    pub fn set_section_map(&mut self, map: Option<SectionMap>) {
        self.dbi.section_map = map;
    }

    /// Return the type server map.
    #[must_use]
    pub fn type_servers(&self) -> &[TypeServer] {
        &self.dbi.type_servers
    }

    /// Set the type server map.
    pub fn set_type_servers(&mut self, servers: Vec<TypeServer>) {
        self.dbi.type_servers = servers;
    }

    /// Return the EC name table.
    #[must_use]
    pub const fn ec_names(&self) -> Option<&StringTableBuilder> {
        self.dbi.ec_names.as_ref()
    }

    /// Set the EC name table. `None` writes an empty substream.
    pub fn set_ec_names(&mut self, names: Option<StringTableBuilder>) {
        self.dbi.ec_names = names;
    }

    /// Return the smallest number of slots in the optional debug header.
    #[must_use]
    pub const fn debug_slot_count(&self) -> usize {
        self.debug_slots
    }

    /// Set the smallest number of slots in the optional debug header.
    ///
    /// The header has a slot for each debug stream, and at least `count`
    /// slots. Microsoft and LLVM writers write 11 slots.
    pub const fn set_debug_slot_count(&mut self, count: usize) {
        self.debug_slots = count;
    }
}

/// The contents of an optional debug stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum DebugContent {
    /// The source stream.
    Source(StreamId),
    /// New bytes.
    Bytes(Vec<u8>),
}

/// The optional debug streams by kind, in slot order.
///
/// [`PdbEditor::debug_streams_mut`](super::PdbEditor::debug_streams_mut)
/// gives the list. Use the encoder of the stream family for the bytes, for
/// example [`RecordStream::to_bytes`](crate::native::debug_stream::RecordStream::to_bytes).
#[derive(Clone, Debug, Default)]
pub struct DebugStreamList {
    /// The streams by slot, in slot order.
    pub(super) entries: Vec<(DebugStream, DebugContent)>,
}

impl DebugStreamList {
    /// Make the list of the debug streams of a source DBI stream. A slot
    /// after the named slots stays a source slot of the DBI stream.
    pub(super) fn from_dbi(dbi: &Dbi) -> Result<Self> {
        let mut entries = Vec::new();
        for (slot, id) in dbi.debug_streams().enumerate() {
            let (Some(id), Some(kind)) = (id, kind_of(slot)) else {
                continue;
            };
            entries.try_reserve(1).map_err(|_| no_memory())?;
            entries.push((kind, DebugContent::Source(id)));
        }
        Ok(Self { entries })
    }

    /// Set the bytes of the debug stream of `kind`.
    ///
    /// In the copy-on-write layout, a replaced stream keeps its stream ID
    /// and a new stream gets a new stream ID.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn set(&mut self, kind: DebugStream, bytes: Vec<u8>) -> Result<()> {
        match self
            .entries
            .binary_search_by_key(&kind.index(), |(slot, _)| slot.index())
        {
            Ok(position) => self.entries[position].1 = DebugContent::Bytes(bytes),
            Err(position) => {
                self.entries.try_reserve(1).map_err(|_| no_memory())?;
                self.entries
                    .insert(position, (kind, DebugContent::Bytes(bytes)));
            }
        }
        Ok(())
    }

    /// Remove the debug stream of `kind`, if the list has it.
    ///
    /// The plan removes the stream and empties its slot in the optional debug
    /// header of the DBI stream. In the copy-on-write layout, the source
    /// stream becomes a nil stream. Return `true` if the list had the stream.
    pub fn remove(&mut self, kind: DebugStream) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(slot, _)| *slot != kind);
        self.entries.len() != before
    }

    /// Return `true` if the list has a stream of `kind`.
    #[must_use]
    pub fn contains(&self, kind: DebugStream) -> bool {
        self.entries.iter().any(|(slot, _)| *slot == kind)
    }

    /// Return the kinds of the streams, in slot order.
    pub fn kinds(&self) -> impl Iterator<Item = DebugStream> + '_ {
        self.entries.iter().map(|(kind, _)| *kind)
    }
}

/// The optional debug stream kinds in slot order.
pub(super) const DEBUG_KINDS: [DebugStream; 11] = [
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

/// Return the kind of debug header slot `slot`.
pub(super) fn kind_of(slot: usize) -> Option<DebugStream> {
    DEBUG_KINDS.get(slot).copied()
}

/// The streams without a name.
///
/// [`PdbEditor::unnamed_streams_mut`](super::PdbEditor::unnamed_streams_mut)
/// gives the list. Another stream refers to such a stream by its stream ID,
/// for example a type map stream that `/TMCache` names, or the module type
/// stream of an `S_MOD_TYPEREF` record.
///
/// [`UnnamedStreams::push`] returns the stream ID in the editor model. In
/// the copy-on-write layout, it is the output stream ID. In the new layout,
/// the plan assigns new stream IDs and writes them into the `S_MOD_TYPEREF`
/// records and the `/TMCache` stream.
#[derive(Clone, Debug, Default)]
pub struct UnnamedStreams {
    /// The streams with their model stream IDs, in ID order.
    pub(super) entries: Vec<(StreamId, Vec<u8>)>,
    /// The model stream ID of the next stream.
    pub(super) next: u32,
    /// New bytes of source streams without a name in the copy-on-write
    /// layout: the module type streams after
    /// [`PdbEditor::remap_source_paths`](super::PdbEditor::remap_source_paths)
    /// (#46). Each source stream keeps its stream ID.
    pub(super) replaced: Vec<(StreamId, Vec<u8>)>,
}

impl UnnamedStreams {
    /// Make an empty list whose first stream gets the ID `first`.
    pub(super) const fn new(first: u32) -> Self {
        Self {
            entries: Vec::new(),
            next: first,
            replaced: Vec::new(),
        }
    }

    /// Add a stream with `bytes` and return its stream ID in the editor
    /// model.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the stream ID does not fit in
    /// 32 bits or an allocation fails.
    pub fn push(&mut self, bytes: Vec<u8>) -> Result<StreamId> {
        let id = StreamId(self.next);
        let next = self.next.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::LimitExceeded, None).with_context("PDB has too many streams")
        })?;
        self.entries.try_reserve(1).map_err(|_| no_memory())?;
        self.entries.push((id, bytes));
        self.next = next;
        Ok(id)
    }

    /// Return the bytes of the stream `id`.
    #[must_use]
    pub fn get(&self, id: StreamId) -> Option<&[u8]> {
        self.entries
            .iter()
            .find(|(stream, _)| *stream == id)
            .map(|(_, bytes)| bytes.as_slice())
    }

    /// Return the number of streams.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return `true` if the list has no stream.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
