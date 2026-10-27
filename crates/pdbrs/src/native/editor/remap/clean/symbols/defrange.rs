//! Def-ranges: one record for each part of the range.
//!
//! Each record gets the gaps inside its part, with offsets from the new part start. With
//! [`RemovedCode::Drop`](crate::native::RemovedCode::Drop), the parts without a remap range
//! go away, and the report counts one removal for each record that lost bytes.

use alloc::vec::Vec;

use super::classify;
use super::{Walker, push};
use crate::Result;
use crate::native::RemapItem;
use crate::native::codeview::symbol::{AddressGaps, AddressRange};
use crate::native::editor::record::SymbolRecord;
use crate::native::editor::tree::ModuleSymbol;
use crate::native::output::no_memory;
use crate::native::remap::clean::Part;

/// Rewrite the def-range `record` with the range `range` and append its records to `out`.
pub(super) fn rewrite(
    walker: &mut Walker<'_, '_>,
    mut record: SymbolRecord,
    range: AddressRange,
    out: &mut Vec<ModuleSymbol>,
) -> Result<()> {
    let map = walker.context.map;
    let Some(parts) = map.section_parts(range.start, u32::from(range.length))? else {
        return push(out, ModuleSymbol::Record(record));
    };
    let rva = walker.rva(range.start);
    if parts.parts.is_empty() {
        walker.remove(&record, rva)?;
        walker.forget(&record);
        return Ok(());
    }
    if parts.is_whole() {
        let start = walker.new_address(parts.parts[0].rva)?;
        if start != range.start {
            classify::set_def_range(
                record.data_mut(),
                AddressRange {
                    start,
                    length: range.length,
                },
                None,
            );
        }
        return push(out, ModuleSymbol::Record(record));
    }
    if parts.removed {
        walker.remove(&record, rva)?;
    }
    let mut pieces = Vec::new();
    pieces
        .try_reserve_exact(parts.parts.len())
        .map_err(|_| no_memory())?;
    for part in &parts.parts {
        let piece = AddressRange {
            start: walker.new_address(part.rva)?,
            // A part lies inside the range, so its length fits in 16 bits.
            length: u16::try_from(part.length).unwrap_or(u16::MAX),
        };
        pieces.push((piece, clip(classify::gaps(record.data()), part)?));
    }
    out.try_reserve(pieces.len()).map_err(|_| no_memory())?;
    let kind = record.kind();
    let mut pieces = pieces.into_iter();
    let Some((first_range, first_gaps)) = pieces.next() else {
        return Ok(());
    };
    let mut others = Vec::new();
    others
        .try_reserve_exact(pieces.len())
        .map_err(|_| no_memory())?;
    for (piece, gaps) in pieces {
        let mut data = record.data().clone();
        classify::set_def_range(&mut data, piece, Some(gaps));
        others.push(ModuleSymbol::Record(SymbolRecord::new(kind, data)));
    }
    walker.context.added(RemapItem::Symbol(kind), others.len());
    classify::set_def_range(record.data_mut(), first_range, Some(first_gaps));
    out.push(ModuleSymbol::Record(record));
    out.extend(others);
    Ok(())
}

/// Return the gaps of `gaps` inside `part`, with offsets from the start of the part.
fn clip(gaps: Option<&AddressGaps<'_>>, part: &Part) -> Result<AddressGaps<'static>> {
    let mut bytes = Vec::new();
    let end = part.offset + part.length;
    for gap in gaps.into_iter().flatten() {
        let low = u32::from(gap.start).max(part.offset);
        let high = (u32::from(gap.start) + u32::from(gap.length)).min(end);
        if low < high {
            // The values lie inside a range of at most 0xffff bytes.
            let start = u16::try_from(low - part.offset).unwrap_or(u16::MAX);
            let length = u16::try_from(high - low).unwrap_or(u16::MAX);
            bytes.try_reserve(4).map_err(|_| no_memory())?;
            bytes.extend_from_slice(&start.to_le_bytes());
            bytes.extend_from_slice(&length.to_le_bytes());
        }
    }
    AddressGaps::parse(bytes)
}
