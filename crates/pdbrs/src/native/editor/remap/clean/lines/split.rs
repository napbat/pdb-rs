//! The clean rewrite of a `DEBUG_S_LINES` subsection.
//!
//! The code extent of the header maps to parts. Each part gets its own subsection, in the
//! order of the old addresses, with the start of the part and the length of the part. Each
//! block keeps its entries in the part, with offsets from the part start. If no entry
//! starts at the part start, the entry that holds the part start gives a first entry at
//! offset 0 to its block. A block without an entry in the part is not in the part.

use alloc::vec::Vec;

use super::super::Context;
use super::{Change, push, single};
use crate::Result;
use crate::native::RemapItem;
use crate::native::address::SectionOffset;
use crate::native::codeview::index::ChecksumOffset;
use crate::native::lines::{
    ColumnEntry, LineBlock, LineEntry, LineSubsection, LinesHeader, LinesSubsection,
};
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::native::remap::clean::Part;

/// The entries of one block, decoded for the split.
struct Block {
    /// The file checksum entry of the block.
    checksum_offset: ChecksumOffset,
    /// The line entries.
    lines: Vec<LineEntry>,
    /// The column entries, if the header has the column flag.
    columns: Option<Vec<ColumnEntry>>,
}

/// Rewrite the code extent of `lines`.
///
/// # Errors
///
/// - [`ErrorKind::UnsupportedEdit`](crate::ErrorKind::UnsupportedEdit) with
///   [`RemovedCode::Fail`](crate::native::RemovedCode::Fail) if a byte of the extent has
///   no range.
/// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if an allocation fails.
pub(super) fn rewrite(
    lines: &LinesSubsection<'_>,
    context: &mut Context<'_>,
    location: &mut impl FnMut() -> Option<(StreamId, u64)>,
) -> Result<Change> {
    let header = *lines.header();
    let start = SectionOffset {
        section: header.segment,
        offset: header.code_offset,
    };
    let Some(old_rva) = context.map.old_rva(start) else {
        return Ok(None);
    };
    let parts = context.map.parts(old_rva, header.code_size)?;
    let mut removed = parts.removed;
    let mut targets: Vec<(Part, SectionOffset)> = Vec::new();
    targets
        .try_reserve_exact(parts.parts.len())
        .map_err(|_| no_memory())?;
    for part in parts.parts {
        // A range lies inside one new section, so each part has a new section.
        match context.map.section_offset(part.rva) {
            Some(new) => targets.push((part, new)),
            None => removed = true,
        }
    }
    if !removed && let [(_, new)] = targets.as_slice() {
        if *new == start {
            return Ok(None);
        }
        return Ok(Some(single(LineSubsection::Lines(moved(lines, *new)?))?));
    }
    if removed {
        let gap = first_gap(&targets, header.code_size);
        context.remove(RemapItem::LineBlock, old_rva.wrapping_add(gap), location())?;
    }
    if targets.len() > 1 {
        context.added(RemapItem::LineBlock, targets.len() - 1);
    }
    let blocks = decode(lines)?;
    let mut out = Vec::new();
    out.try_reserve_exact(targets.len())
        .map_err(|_| no_memory())?;
    for (part, new) in targets {
        out.push(LineSubsection::Lines(split(header, &blocks, part, new)?));
    }
    Ok(Some(out))
}

/// Return `lines` with the code start `new` and the same blocks.
///
/// # Errors
///
/// Returns the errors of the encoders and of [`LinesSubsection::parse`].
pub(super) fn moved(
    lines: &LinesSubsection<'_>,
    new: SectionOffset,
) -> Result<LinesSubsection<'static>> {
    let old = lines.header();
    let header = LinesHeader {
        code_offset: new.offset,
        segment: new.section,
        flags: old.flags,
        code_size: old.code_size,
    };
    let mut data = Vec::new();
    header.encode(&mut data)?;
    for block in lines.blocks() {
        block.encode(&mut data)?;
    }
    Ok(LinesSubsection::parse(&data)?.into_owned())
}

/// Return the offset of the first byte of an extent of `length` bytes that no part of
/// `targets` holds.
fn first_gap(targets: &[(Part, SectionOffset)], length: u32) -> u32 {
    let mut covered = 0;
    for (part, _) in targets {
        if part.offset > covered {
            return covered;
        }
        covered = part.offset + part.length;
    }
    if covered < length { covered } else { 0 }
}

/// Decode the blocks of `lines`.
fn decode(lines: &LinesSubsection<'_>) -> Result<Vec<Block>> {
    let mut blocks = Vec::new();
    blocks
        .try_reserve_exact(lines.block_count())
        .map_err(|_| no_memory())?;
    for block in lines.blocks() {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(block.lines.len())
            .map_err(|_| no_memory())?;
        entries.extend(block.lines.iter());
        let columns = match &block.columns {
            Some(columns) => {
                let mut list = Vec::new();
                list.try_reserve_exact(columns.len())
                    .map_err(|_| no_memory())?;
                list.extend(columns.iter());
                Some(list)
            }
            None => None,
        };
        blocks.push(Block {
            checksum_offset: block.checksum_offset,
            lines: entries,
            columns,
        });
    }
    Ok(blocks)
}

/// Return the block and the entry position of the entry that holds the part start
/// `start`, if no entry starts there.
///
/// The code offsets do not decrease in stream order, so the last entry before `start` holds
/// it.
fn covering(blocks: &[Block], start: u32) -> Option<(usize, usize)> {
    let mut found = None;
    for (index, block) in blocks.iter().enumerate() {
        for (position, entry) in block.lines.iter().enumerate() {
            if entry.offset == start {
                return None;
            }
            if entry.offset < start {
                found = Some((index, position));
            }
        }
    }
    found
}

/// Return the subsection of the part `part` at the new start `new`.
fn split(
    header: LinesHeader,
    blocks: &[Block],
    part: Part,
    new: SectionOffset,
) -> Result<LinesSubsection<'static>> {
    let start = part.offset;
    let end = u64::from(start) + u64::from(part.length);
    let cover = covering(blocks, start);
    let header = LinesHeader {
        code_offset: new.offset,
        segment: new.section,
        flags: header.flags,
        code_size: part.length,
    };
    let mut data = Vec::new();
    header.encode(&mut data)?;
    let mut lines = Vec::new();
    let mut columns = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        lines.clear();
        columns.clear();
        let mut add = |position: usize, offset: u32| -> Result<()> {
            let mut entry = block.lines[position];
            entry.offset = offset;
            push(&mut lines, entry)?;
            if let Some(list) = &block.columns {
                push(&mut columns, list[position])?;
            }
            Ok(())
        };
        if let Some((_, position)) = cover.filter(|&(owner, _)| owner == index) {
            add(position, 0)?;
        }
        for (position, entry) in block.lines.iter().enumerate() {
            if entry.offset >= start && u64::from(entry.offset) < end {
                add(position, entry.offset - start)?;
            }
        }
        if lines.is_empty() {
            continue;
        }
        let block_columns = block.columns.as_ref().map(|_| columns.as_slice());
        LineBlock::encode_parts(block.checksum_offset, &lines, block_columns, &mut data)?;
    }
    Ok(LinesSubsection::parse(&data)?.into_owned())
}
