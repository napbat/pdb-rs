//! The address remap of the editor model.

use alloc::vec::Vec;

use super::model::{PdbEditor, missing};
use super::plan::unsupported;
use super::streams::DebugContent;
use crate::Result;
use crate::io::ReadAt;
use crate::native::address::AddressTables;
use crate::native::dbi::DebugStream;
use crate::native::debug_stream::OmapStream;
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::native::remap::clean::CleanMap;
use crate::native::section::SectionHeaderStream;
use crate::native::{ImageRemap, RemapMode};

pub(super) mod clean;

/// The debug streams that OMAP output writes, in slot order.
const OMAP_STREAMS: [DebugStream; 4] = [
    DebugStream::OmapToSource,
    DebugStream::OmapFromSource,
    DebugStream::SectionHeader,
    DebugStream::OriginalSectionHeader,
];

impl<S: ReadAt> PdbEditor<'_, S> {
    /// Remap the addresses of the PDB to the rewritten image of `remap`.
    ///
    /// The old RVAs of `remap` are RVAs of the image that the model describes.
    /// If the model already has OMAP tables, the function composes them with
    /// the ranges. Records that other edits add or change, before or after
    /// the call, use the addresses of the source PDB, as the other records do.
    ///
    /// # OMAP output
    ///
    /// With [`RemapMode::Omap`], the function replaces 4 optional debug
    /// streams of the model (see [`PdbEditor::debug_streams_mut`]):
    ///
    /// - [`DebugStream::SectionHeader`]: the section headers of the new image.
    /// - [`DebugStream::OriginalSectionHeader`]: the section headers that the
    ///   addresses of the records use.
    /// - [`DebugStream::OmapFromSource`] and [`DebugStream::OmapToSource`]: the
    ///   OMAP tables of [`ImageRemap::address_tables`].
    ///
    /// The records keep their addresses, so the plan copies every other
    /// stream. In the copy-on-write layout, the report then lists only the
    /// Info stream (for the identity policy), the DBI stream and the 4 debug
    /// streams. [`AddressMap`](crate::native::address::AddressMap) and DIA
    /// translate each address exactly as the ranges say. An address without a
    /// range has no mapping. The new tables map each address of the records
    /// directly to the new image, so a second call describes a second rewrite
    /// of the image.
    ///
    /// # Clean output
    ///
    /// With [`RemapMode::Clean`], the function keeps a copy of `remap` and the
    /// policy. The plan writes the new address into each record that holds an
    /// address: the module symbols, the C13 line blocks,
    /// frame data and COFF symbol RVAs, the symbol record stream with the
    /// address map of the public index, the DBI section contributions and
    /// section map, and the FPO, new FPO, exception data and fixup streams.
    /// [`DebugStream::SectionHeader`] gets the section headers of the new
    /// image. The output has no OMAP stream and no
    /// [`DebugStream::OriginalSectionHeader`] stream.
    ///
    /// - A record that a range splits gets the split form of its kind, for
    ///   example an `S_SEPCODE` scope for each part of a procedure after the
    ///   first one, or a `DEBUG_S_LINES` subsection for each part of a line
    ///   block. A kind without a split form, for example `S_BLOCK32` or an FPO
    ///   record, makes the plan fail with [`ErrorKind::UnsupportedEdit`].
    /// - A record that holds an old address without a range holds removed
    ///   code. [`RemovedCode::Fail`] makes the plan fail;
    ///   [`RemovedCode::Drop`] removes the record and the references to it.
    /// - The error of a rejected record names the record and its address
    ///   ([`Error::remap_address`](crate::Error::remap_address)).
    /// - [`PlanReport::remap`](crate::native::PlanReport::remap) counts the
    ///   removed records and the added parts.
    ///
    /// A second clean call describes a second rewrite: the plan applies the
    /// maps in call order.
    ///
    /// Caution: the new image matches the output only after the caller writes
    /// [`PdbWritePlan::identity`](crate::native::PdbWritePlan::identity) into
    /// its `RSDS` debug entry. Use [`IdentityPolicy::IncrementAge`] or
    /// [`IdentityPolicy::Replace`]: a remapped PDB describes another image.
    ///
    /// The function is atomic: an error leaves the model unchanged.
    ///
    /// # Usage
    ///
    /// ```no_run
    /// use pdbrs::native::address::Rva;
    /// use pdbrs::native::section::SectionHeaderStream;
    /// use pdbrs::native::{EditOptions, IdentityPolicy, ImageRemap, Pdb, RemapMode};
    ///
    /// # #[cfg(feature = "std")]
    /// # fn main() -> pdbrs::Result<()> {
    /// # let new_section_headers = SectionHeaderStream::from_records(&[])?;
    /// // The caller gives the section headers of the new image. pdbrs has no PE parser.
    /// let remap = ImageRemap::new(new_section_headers)
    ///     .with_range(Rva(0x1000), 0x2a0, Rva(0x6000))? // old RVA, length, new RVA
    ///     .with_range(Rva(0x12a0), 0x40, Rva(0x1000))?;
    ///
    /// let pdb = Pdb::open("app.pdb")?;
    /// let mut edit = pdb.edit(IdentityPolicy::IncrementAge);
    /// edit.remap_addresses(&remap, RemapMode::Omap)?; // or RemapMode::Clean(RemovedCode::Fail)
    /// let plan = edit.plan(EditOptions::default())?;
    /// plan.write_file("app.rewritten.pdb")?;
    /// // Write the new identity into the RSDS debug entry of the new image.
    /// let identity = plan.identity()?;
    /// # Ok(())
    /// # }
    /// # #[cfg(not(feature = "std"))]
    /// # fn main() {}
    /// ```
    ///
    /// [`IdentityPolicy::IncrementAge`]: crate::native::IdentityPolicy::IncrementAge
    /// [`IdentityPolicy::Replace`]: crate::native::IdentityPolicy::Replace
    /// [`ErrorKind::UnsupportedEdit`]: crate::ErrorKind::UnsupportedEdit
    /// [`ErrorKind::MissingStream`]: crate::ErrorKind::MissingStream
    /// [`ErrorKind::LimitExceeded`]: crate::ErrorKind::LimitExceeded
    /// [`RemovedCode::Fail`]: crate::native::RemovedCode::Fail
    /// [`RemovedCode::Drop`]: crate::native::RemovedCode::Drop
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedEdit`] for OMAP output after a clean call,
    ///   and for clean output of a model with a [`DebugStream::Xdata`] or
    ///   [`DebugStream::Pdata`] stream if `remap` has no unwind streams
    ///   ([`ImageRemap::with_unwind_streams`]).
    /// - [`ErrorKind::MissingStream`] if the model has no
    ///   [`DebugStream::SectionHeader`] stream, or OMAP without the original
    ///   section headers or without one of the two OMAP tables.
    /// - [`ErrorKind::LimitExceeded`] if a table has more than
    ///   [`ImageRemap::MAX_OMAP_RECORDS`] records or an allocation fails.
    /// - The errors of [`PdbEditor::debug_streams_mut`], of the stream readers
    ///   and of the section header and OMAP decoders.
    ///
    /// The plan returns the errors of clean output for the records.
    pub fn remap_addresses(&mut self, remap: &ImageRemap, mode: RemapMode) -> Result<()> {
        match mode {
            RemapMode::Omap => self.remap_omap(remap),
            RemapMode::Clean(policy) => self.remap_clean(remap, policy),
        }
    }

    /// Check the input of clean output and keep a copy of the map.
    fn remap_clean(
        &mut self,
        remap: &ImageRemap,
        policy: crate::native::RemovedCode,
    ) -> Result<()> {
        let pending = self.clean.as_ref();
        let unwind_before = pending.is_some_and(clean::CleanRemap::has_unwind_streams);
        if pending.is_none() {
            CleanMap::new(remap, &self.model_address_tables()?)?;
        }
        let list = self.debug_streams_mut()?;
        let unwind =
            unwind_before || list.contains(DebugStream::Xdata) || list.contains(DebugStream::Pdata);
        if unwind && remap.unwind_streams().is_none() {
            return Err(unsupported(
                "clean remap output needs the .pdata and .xdata streams of the new image; use ImageRemap::with_unwind_streams",
            ));
        }
        self.clean
            .get_or_insert_with(clean::CleanRemap::default)
            .push(remap.clone(), policy)
    }

    /// Write the OMAP tables and the section headers of the new image.
    fn remap_omap(&mut self, remap: &ImageRemap) -> Result<()> {
        if self.clean.is_some() {
            return Err(unsupported(
                "OMAP remap output cannot follow clean remap output; use RemapMode::Clean",
            ));
        }
        let source = self.model_address_tables()?;
        let tables = remap.address_tables(&source)?;
        let original = tables
            .original
            .as_ref()
            .ok_or_else(|| missing("the remap tables have no original section headers"))?;
        let to_source = tables
            .to_source
            .as_ref()
            .ok_or_else(|| missing("the remap tables have no OMAP to source table"))?;
        let from_source = tables
            .from_source
            .as_ref()
            .ok_or_else(|| missing("the remap tables have no OMAP from source table"))?;
        let streams = [
            to_source.to_bytes()?,
            from_source.to_bytes()?,
            tables.sections.to_bytes()?,
            original.to_bytes()?,
        ];
        let list = self.debug_streams_mut()?;
        // With the capacity reserved, `set` does not fail, so the change is atomic.
        list.entries
            .try_reserve(OMAP_STREAMS.len())
            .map_err(|_| no_memory())?;
        for (kind, bytes) in OMAP_STREAMS.into_iter().zip(streams) {
            list.set(kind, bytes)?;
        }
        Ok(())
    }

    /// Return the section headers and the OMAP tables of the model.
    fn model_address_tables(&mut self) -> Result<AddressTables> {
        let (bytes, id) = self
            .debug_stream(DebugStream::SectionHeader)?
            .ok_or_else(|| missing("the PDB has no section header stream to remap"))?;
        let mut tables = AddressTables::new(SectionHeaderStream::parse(bytes, id)?);
        tables.original = self
            .debug_stream(DebugStream::OriginalSectionHeader)?
            .map(|(bytes, id)| SectionHeaderStream::parse(bytes, id))
            .transpose()?;
        tables.to_source = self
            .debug_stream(DebugStream::OmapToSource)?
            .map(|(bytes, id)| OmapStream::parse(bytes, id))
            .transpose()?;
        tables.from_source = self
            .debug_stream(DebugStream::OmapFromSource)?
            .map(|(bytes, id)| OmapStream::parse(bytes, id))
            .transpose()?;
        Ok(tables)
    }

    /// Return the bytes of the debug stream `kind` of the model, with the
    /// stream ID for the error location, or `None` if the model has no such
    /// stream.
    ///
    /// The location of new bytes is the source stream of the slot. A slot
    /// without a source stream has no stream ID yet, so its location is
    /// stream 0.
    fn debug_stream(&mut self, kind: DebugStream) -> Result<Option<(Vec<u8>, StreamId)>> {
        let source = self.source;
        let list = self.debug_streams_mut()?;
        let Some((_, content)) = list.entries.iter().find(|(slot, _)| *slot == kind) else {
            return Ok(None);
        };
        match content {
            DebugContent::Source(id) => {
                let pdb = source.ok_or_else(|| missing("the editor has no source stream"))?;
                Ok(Some((pdb.stream_bytes(*id)?, *id)))
            }
            DebugContent::Bytes(bytes) => {
                let mut copy = Vec::new();
                copy.try_reserve_exact(bytes.len())
                    .map_err(|_| no_memory())?;
                copy.extend_from_slice(bytes);
                let slot = match source {
                    Some(pdb) => pdb.debug_info()?.and_then(|dbi| dbi.debug_stream(kind)),
                    None => None,
                };
                Ok(Some((copy, slot.unwrap_or_default())))
            }
        }
    }
}
