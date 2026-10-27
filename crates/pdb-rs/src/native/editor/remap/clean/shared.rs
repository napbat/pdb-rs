//! The clean rewrite of the symbol record stream and its indexes.
//!
//! The rewrite changes:
//!
//! - the points of the records (publics, data, thread data and the other kinds of
//!   [`points`]);
//! - the reference records (`S_PROCREF`, `S_LPROCREF`, `S_DATAREF`, `S_TOKENREF` and
//!   `S_ANNOTATIONREF`) to removed module records: they go away with their index entries;
//! - the thunk table, the thunk targets and the section starts of the public index.
//!
//! The plan makes the address map of the public index from the remaining public records
//! again. It also drops the module global references to the removed records of the
//! stream (`plan::layout::global_refs`), so this rewrite does not change the modules.

use alloc::vec::Vec;

use super::{Context, Removed, record_error};
use crate::native::RemapItem;
use crate::native::address::SectionOffset;
use crate::native::codeview::index::SymbolHandle;
use crate::native::codeview::symbol::{ModuleSymbolTarget, SymbolData};
use crate::native::editor::record::SymbolRecord;
use crate::native::editor::symbols::{SharedSymbols, Thunks};
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::native::remap::clean::Point;
use crate::native::symbols::SectionStart;
use crate::{Error, ErrorKind, Result};

mod points;

use points::Fields;

/// Rewrite the addresses of the records of `symbols`, and remove the references to the
/// removed module records.
///
/// The records keep their source bytes if their values do not change.
///
/// # Errors
///
/// - [`ErrorKind::UnsupportedEdit`] for a record without a decoder, for a record with a
///   code range and an address, and with `RemovedCode::Fail` for the first record whose
///   address has no range. The error names the record and its address.
/// - [`ErrorKind::LimitExceeded`] if an allocation fails.
pub(super) fn rewrite(
    symbols: &mut SharedSymbols,
    removed: &Removed,
    context: &mut Context<'_>,
) -> Result<()> {
    let stream = symbols.source.as_ref().map(|source| source.stream);
    let mut gone = Vec::new();
    for (position, slot) in symbols.records.iter_mut().enumerate() {
        let Some(record) = slot else {
            continue;
        };
        let handle = SymbolHandle(u32::try_from(position).unwrap_or(u32::MAX));
        let location = stream.zip(
            record
                .origin()
                .map(|origin| u64::from(origin.record().offset)),
        );
        let remove = if names_removed(record.data(), removed) {
            context
                .report
                .count_removed(RemapItem::Symbol(record.kind()), 1);
            true
        } else {
            rewrite_record(record, location, context)?
        };
        if remove {
            gone.try_reserve(1).map_err(|_| no_memory())?;
            gone.push(handle);
        }
    }
    for handle in gone {
        symbols.remove(handle)?;
    }
    rewrite_thunks(&mut symbols.thunks, context)?;
    rewrite_section_starts(&mut symbols.section_starts, context)
}

/// Return `true` if `data` is a reference record whose module record the rewrite removed.
fn names_removed(data: &SymbolData<'_>, removed: &Removed) -> bool {
    let (SymbolData::ProcedureReference(reference) | SymbolData::SymbolReference(reference)) = data
    else {
        return false;
    };
    match reference.target {
        ModuleSymbolTarget::Handle(handle) => {
            removed.records.contains(&(handle.module, handle.position))
        }
        ModuleSymbolTarget::Position {
            module: Some(module),
            offset,
        } => removed.offsets.contains(&(module.0, offset.0)),
        ModuleSymbolTarget::Position { module: None, .. } => false,
    }
}

/// Rewrite the addresses of one record. Return `true` if the record goes away: its
/// address has no range and the policy is `RemovedCode::Drop`.
fn rewrite_record(
    record: &mut SymbolRecord,
    location: Option<(StreamId, u64)>,
    context: &mut Context<'_>,
) -> Result<bool> {
    let item = RemapItem::Symbol(record.kind());
    let map = context.map;
    match points::fields(record.data()) {
        Fields::None => Ok(false),
        Fields::Unsupported => {
            let error = Error::new(ErrorKind::UnsupportedEdit, None).with_context(
                "a record of the symbol record stream has no decoder, so its address cannot change",
            );
            Err(match location {
                Some((stream, offset)) => error.with_stream(stream, offset),
                None => error,
            })
        }
        Fields::Range(start) => match map.old_rva(start) {
            Some(address) => Err(record_error(
                item,
                address,
                location,
                "a record with a code range in the symbol record stream cannot get a new address",
            )),
            None => Ok(false),
        },
        Fields::Section(rva) => {
            if rva == 0 {
                return Ok(false);
            }
            match map.rva_point(rva) {
                Point::Removed(address) => {
                    context.remove(item, address, location)?;
                    Ok(true)
                }
                Point::Mapped(new) => {
                    let number = map.section_offset(new).map(|address| address.section);
                    if let SymbolData::Section(section) = record.data()
                        && (section.rva != new || number.is_some_and(|n| n != section.number))
                        && let SymbolData::Section(section) = record.data_mut()
                    {
                        section.rva = new;
                        if let Some(number) = number {
                            section.number = number;
                        }
                    }
                    Ok(false)
                }
                Point::Absent => Ok(false),
            }
        }
        Fields::Points(mut points) => {
            let mut changed = false;
            for value in points.values_mut() {
                match map.point(*value) {
                    Point::Absent => {}
                    Point::Removed(address) => {
                        context.remove(item, address, location)?;
                        return Ok(true);
                    }
                    Point::Mapped(new) => {
                        changed |= new != *value;
                        *value = new;
                    }
                }
            }
            if changed {
                points::set_points(record.data_mut(), &points);
            }
            Ok(false)
        }
    }
}

/// Rewrite the thunk table of the public index: its address and the thunk targets.
///
/// The address of the table is a point. A thunk target is a PDB RVA; the value 0 means no
/// target. With `RemovedCode::Drop`, a removed table goes away with its targets, and a
/// removed target becomes 0, so that the other thunks keep their positions.
fn rewrite_thunks(thunks: &mut Thunks, context: &mut Context<'_>) -> Result<()> {
    let table = SectionOffset {
        section: thunks.section,
        offset: thunks.offset,
    };
    match context.map.point(table) {
        Point::Absent => {}
        Point::Removed(address) => {
            context.remove(RemapItem::Thunk, address, None)?;
            *thunks = Thunks::default();
            return Ok(());
        }
        Point::Mapped(new) => {
            thunks.section = new.section;
            thunks.offset = new.offset;
        }
    }
    for target in &mut thunks.targets {
        if *target == 0 {
            continue;
        }
        match context.map.rva_point(*target) {
            Point::Absent => {}
            Point::Removed(address) => {
                context.remove(RemapItem::Thunk, address, None)?;
                *target = 0;
            }
            Point::Mapped(new) => *target = new,
        }
    }
    Ok(())
}

/// Rewrite the section starts of the public index.
///
/// If each value is the start of its section in the old section headers, each value
/// becomes the start of the same section in the new section headers. Else each value maps
/// as a point. With `RemovedCode::Drop`, a value without a range goes away.
fn rewrite_section_starts(starts: &mut Vec<SectionStart>, context: &mut Context<'_>) -> Result<()> {
    let map = context.map;
    let sections = map.new_sections();
    let headers = starts.iter().all(|start| {
        usize::from(start.section) <= sections.len()
            && map.old_rva(SectionOffset {
                section: start.section,
                offset: 0,
            }) == Some(start.offset)
    });
    if headers {
        for start in starts.iter_mut() {
            // `old_rva` gives a value only for a section from 1, and the check above keeps
            // the section inside the new headers.
            start.offset = sections[usize::from(start.section) - 1].virtual_address;
        }
        return Ok(());
    }
    let count = map.old_section_count();
    let mut kept = 0;
    for index in 0..starts.len() {
        let mut start = starts[index];
        if start.offset != 0 && (1..=count).contains(&usize::from(start.section)) {
            match map.rva_point(start.offset) {
                Point::Absent => {}
                Point::Removed(address) => {
                    context.remove(RemapItem::Thunk, address, None)?;
                    continue;
                }
                Point::Mapped(new) => {
                    start.offset = new;
                    if let Some(address) = map.section_offset(new) {
                        start.section = address.section;
                    }
                }
            }
        }
        starts[kept] = start;
        kept += 1;
    }
    starts.truncate(kept);
    Ok(())
}
