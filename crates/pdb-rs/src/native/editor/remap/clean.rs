//! Clean output of the address remap.
//!
//! [`PdbEditor::remap_addresses`] with [`RemapMode::Clean`](crate::native::RemapMode::Clean)
//! keeps a copy of the map and the policy. The plan applies them: it copies the
//! address-bearing parts of the model, rewrites the copy and plans from it. The parts are
//! the modules, the symbol record stream with its indexes, the DBI values and the optional
//! debug streams. The copy stays in the editor until the next change of the model, because
//! the plan borrows the streams that it writes.
//!
//! The rewrite of one map runs in this order:
//!
//! 1. The symbols of each module ([`symbols`]): the scope tree, the split forms and the
//!    removed records. The function collects the removed records that other records can
//!    name.
//! 2. The C13 subsections of each module ([`lines`]).
//! 3. The symbol record stream and its indexes ([`shared`]), with the references to the
//!    removed records.
//! 4. The DBI values ([`dbi`]): the section contributions, the section map and the
//!    contribution of each module record.
//! 5. The optional debug streams ([`debug`]): the FPO, new FPO, exception data and fixup
//!    records, the unwind streams of the caller and the section headers. The OMAP streams
//!    and the original section headers go away.

use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use core::cell::OnceCell;

use super::super::model::{ModuleSlot, PdbEditor, missing};
use super::super::report::RemapReport;
use super::super::source;
use super::super::streams::{DebugContent, DebugInfo, DebugStreamList};
use super::super::symbols::SharedSymbols;
use crate::io::ReadAt;
use crate::native::address::{AddressTables, PdbRva};
use crate::native::codeview::index::ModuleIndex;
use crate::native::dbi::DebugStream;
use crate::native::debug_stream::OmapStream;
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::native::remap::clean::CleanMap;
use crate::native::section::SectionHeaderStream;
use crate::native::{ImageRemap, RemapAddress, RemapItem, RemovedCode};
use crate::{Error, ErrorKind, Result};

mod dbi;
mod debug;
mod lines;
mod shared;
mod symbols;

/// The clean remaps of the model and the cache of the rewritten parts.
#[derive(Debug, Default)]
pub(in crate::native::editor) struct CleanRemap {
    /// The maps and policies in call order.
    steps: Vec<(ImageRemap, RemovedCode)>,
    /// The rewritten parts, after the first plan since the last change.
    output: OnceCell<Box<Remapped>>,
}

impl Clone for CleanRemap {
    /// Copy the maps. The copy has no cache.
    fn clone(&self) -> Self {
        Self {
            steps: self.steps.clone(),
            output: OnceCell::new(),
        }
    }
}

impl CleanRemap {
    /// Return `true` if a step gives the unwind streams of its new image.
    pub(in crate::native::editor) fn has_unwind_streams(&self) -> bool {
        self.steps
            .iter()
            .any(|(remap, _)| remap.unwind_streams().is_some())
    }

    /// Add a step and clear the cache.
    pub(in crate::native::editor) fn push(
        &mut self,
        remap: ImageRemap,
        policy: RemovedCode,
    ) -> Result<()> {
        self.steps.try_reserve(1).map_err(|_| no_memory())?;
        self.steps.push((remap, policy));
        self.output = OnceCell::new();
        Ok(())
    }
}

/// The rewritten address-bearing parts of the model.
#[derive(Debug)]
pub(in crate::native::editor) struct Remapped {
    /// The modules, all decoded.
    pub(in crate::native::editor) modules: Vec<ModuleSlot>,
    /// The symbol record stream and its indexes, if the PDB has them.
    pub(in crate::native::editor) symbols: Option<SharedSymbols>,
    /// The DBI values.
    pub(in crate::native::editor) dbi: DebugInfo,
    /// The optional debug streams, all with bytes.
    pub(in crate::native::editor) debug: DebugStreamList,
    /// The counts of the report.
    pub(in crate::native::editor) report: RemapReport,
}

/// The module records that the rewrite removed.
///
/// A reference record of the source names a source record by its module and its module
/// stream offset ([`ModuleSymbolTarget::Position`](crate::native::codeview::symbol::ModuleSymbolTarget)).
/// A reference record of the model names a record by its
/// [`ModuleSymbolHandle`](crate::native::codeview::index::ModuleSymbolHandle): the module and
/// the number of the record in its module (`SymbolRecord::id`).
#[derive(Debug, Default)]
pub(super) struct Removed {
    /// The module index and the record number of each removed record.
    pub(super) records: BTreeSet<(u32, u32)>,
    /// The module index and the source module stream offset of each removed source record.
    pub(super) offsets: BTreeSet<(u32, u32)>,
}

/// The state of the rewrite of one map.
pub(super) struct Context<'m> {
    /// The address map.
    pub(super) map: &'m CleanMap,
    /// The map of the caller.
    pub(super) remap: &'m ImageRemap,
    /// The policy for removed code.
    pub(super) policy: RemovedCode,
    /// The counts of the report.
    pub(super) report: &'m mut RemapReport,
}

impl Context<'_> {
    /// Handle a record `item` at the PDB RVA `address` that holds removed code.
    ///
    /// With [`RemovedCode::Fail`], return the error that names the record. With
    /// [`RemovedCode::Drop`], count the removed record; the caller removes it.
    pub(super) fn remove(
        &mut self,
        item: RemapItem,
        address: u32,
        location: Option<(StreamId, u64)>,
    ) -> Result<()> {
        match self.policy {
            RemovedCode::Drop => {
                self.report.count_removed(item, 1);
                Ok(())
            }
            RemovedCode::Fail => Err(record_error(
                item,
                address,
                location,
                "a record holds removed code: an old address without a remap range",
            )),
        }
    }

    /// Count `count` added parts of `item`.
    pub(super) fn added(&mut self, item: RemapItem, count: usize) {
        self.report.count_added(item, count);
    }
}

/// Return the [`ErrorKind::UnsupportedEdit`] error for the record `item` at the PDB RVA
/// `address`, with the stream location of a source record.
pub(super) fn record_error(
    item: RemapItem,
    address: u32,
    location: Option<(StreamId, u64)>,
    context: &'static str,
) -> Error {
    let error = Error::new(ErrorKind::UnsupportedEdit, None)
        .with_context(context)
        .with_remap_address(RemapAddress::new(item, PdbRva(address)));
    match location {
        Some((stream, offset)) => error.with_stream(stream, offset),
        None => error,
    }
}

/// Return the error for a record that a range splits and that has no split form.
pub(super) fn no_split_form(
    item: RemapItem,
    address: u32,
    location: Option<(StreamId, u64)>,
) -> Error {
    record_error(
        item,
        address,
        location,
        "a remap range splits a record that has no split form",
    )
}

/// Return the section headers and the OMAP tables of a debug stream list whose streams all
/// have bytes.
fn address_tables(list: &DebugStreamList) -> Result<AddressTables> {
    let bytes = |kind: DebugStream| {
        list.entries
            .iter()
            .find(|(slot, _)| *slot == kind)
            .and_then(|(_, content)| match content {
                DebugContent::Bytes(bytes) => Some(bytes.as_slice()),
                DebugContent::Source(_) => None,
            })
    };
    let parse_sections = |bytes: &[u8]| SectionHeaderStream::parse(bytes.to_vec(), StreamId(0));
    let sections = bytes(DebugStream::SectionHeader)
        .ok_or_else(|| missing("the PDB has no section header stream to remap"))?;
    let mut tables = AddressTables::new(parse_sections(sections)?);
    tables.original = bytes(DebugStream::OriginalSectionHeader)
        .map(parse_sections)
        .transpose()?;
    tables.to_source = bytes(DebugStream::OmapToSource)
        .map(|bytes| OmapStream::parse(bytes.to_vec(), StreamId(0)))
        .transpose()?;
    tables.from_source = bytes(DebugStream::OmapFromSource)
        .map(|bytes| OmapStream::parse(bytes.to_vec(), StreamId(0)))
        .transpose()?;
    Ok(tables)
}

impl<S> PdbEditor<'_, S> {
    /// Return the rewritten parts of the clean output, if the plan made them.
    pub(in crate::native::editor) fn remapped(&self) -> Option<&Remapped> {
        self.clean.as_ref()?.output.get().map(Box::as_ref)
    }

    /// Return the modules that the plan writes.
    pub(in crate::native::editor) fn plan_modules(&self) -> Option<&[ModuleSlot]> {
        match self.remapped() {
            Some(remapped) => Some(&remapped.modules),
            None => self.modules.as_deref(),
        }
    }

    /// Return the symbol record stream model that the plan writes.
    pub(in crate::native::editor) fn plan_symbols(&self) -> Option<&SharedSymbols> {
        match self.remapped() {
            Some(remapped) => remapped.symbols.as_ref(),
            None => self.symbols.as_ref(),
        }
    }

    /// Return the DBI values that the plan writes.
    pub(in crate::native::editor) fn plan_dbi(&self) -> Option<&DebugInfo> {
        match self.remapped() {
            Some(remapped) => Some(&remapped.dbi),
            None => self.dbi.as_ref(),
        }
    }

    /// Return the optional debug streams that the plan writes.
    pub(in crate::native::editor) fn plan_debug(&self) -> Option<&DebugStreamList> {
        match self.remapped() {
            Some(remapped) => Some(&remapped.debug),
            None => self.debug.as_ref(),
        }
    }

    /// Clear the rewritten parts after a change of the model.
    pub(in crate::native::editor) fn clear_remap_output(&mut self) {
        if let Some(clean) = &mut self.clean {
            clean.output = OnceCell::new();
        }
    }
}

impl<S: ReadAt> PdbEditor<'_, S> {
    /// Make the rewritten parts of the clean output, if the model has a clean remap and the
    /// cache is empty.
    pub(in crate::native::editor) fn prepare_remap(&self) -> Result<()> {
        let Some(clean) = &self.clean else {
            return Ok(());
        };
        if clean.output.get().is_some() {
            return Ok(());
        }
        let remapped = self.rewrite(&clean.steps)?;
        // The cell is empty, so `set` succeeds.
        let _ = clean.output.set(Box::new(remapped));
        Ok(())
    }

    /// Copy the address-bearing parts of the model and apply each step.
    fn rewrite(&self, steps: &[(ImageRemap, RemovedCode)]) -> Result<Remapped> {
        let mut remapped = self.copy_parts()?;
        for (remap, policy) in steps {
            let tables = address_tables(&remapped.debug)?;
            let map = CleanMap::new(remap, &tables)?;
            let mut context = Context {
                map: &map,
                remap,
                policy: *policy,
                report: &mut remapped.report,
            };
            let mut removed = Removed::default();
            for slot in &mut remapped.modules {
                if let ModuleSlot::Decoded(module) = slot {
                    symbols::rewrite(module, &mut context, &mut removed)?;
                    lines::rewrite(module, &mut context)?;
                }
            }
            if let Some(symbols) = &mut remapped.symbols {
                shared::rewrite(symbols, &removed, &mut context)?;
            }
            dbi::rewrite(&mut remapped.dbi, &mut remapped.modules, &mut context)?;
            debug::rewrite(&mut remapped.debug, &mut context)?;
        }
        Ok(remapped)
    }

    /// Copy the modules, the symbol record stream model, the DBI values and the debug
    /// streams of the model. Decode each part that the model does not have.
    fn copy_parts(&self) -> Result<Remapped> {
        let source = self.source;
        let pdb = || source.ok_or_else(|| missing("the editor has no source for this part"));
        let mut modules = match &self.modules {
            Some(slots) => slots.clone(),
            None => source::module_slots(pdb()?)?,
        };
        for (index, slot) in modules.iter_mut().enumerate() {
            if let ModuleSlot::Source = slot {
                let index = ModuleIndex(u32::try_from(index).unwrap_or(u32::MAX));
                *slot = ModuleSlot::Decoded(Box::new(source::module(pdb()?, index)?));
            }
        }
        let symbols = match &self.symbols {
            Some(symbols) => Some(symbols.clone()),
            None => match source {
                Some(pdb)
                    if pdb
                        .debug_info()?
                        .and_then(|dbi| dbi.header().symbol_stream)
                        .is_some() =>
                {
                    Some(source::shared_symbols(pdb)?)
                }
                _ => None,
            },
        };
        let dbi = match &self.dbi {
            Some(dbi) => dbi.clone(),
            None => DebugInfo::decode(
                pdb()?
                    .debug_info()?
                    .ok_or_else(|| missing("PDB has no DBI stream"))?,
            )?,
        };
        let mut debug = match &self.debug {
            Some(list) => list.clone(),
            None => DebugStreamList::from_dbi(
                pdb()?
                    .debug_info()?
                    .ok_or_else(|| missing("PDB has no DBI stream"))?,
            )?,
        };
        for (_, content) in &mut debug.entries {
            if let DebugContent::Source(id) = content {
                *content = DebugContent::Bytes(pdb()?.stream_bytes(*id)?);
            }
        }
        Ok(Remapped {
            modules,
            symbols,
            dbi,
            debug,
            report: RemapReport::default(),
        })
    }
}
