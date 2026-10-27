//! The clean rewrite of the optional debug streams (#43, decisions 4, 5 and 7).
//!
//! - The FPO, new FPO and exception data records move with their range. A split fails,
//!   because the records describe the prolog and the frame from the function start. The
//!   streams are sorted again by the start.
//! - The fixups map their two addresses as points and keep their order.
//! - The `.pdata` and `.xdata` streams of the caller replace slots 8 and 7.
//! - Slot 5 gets the section headers of the new image. The OMAP streams and the original
//!   section headers go away.
//!
//! The function writes a record stream only if a record changes.

use alloc::vec::Vec;

use super::{Context, no_split_form};
use crate::Result;
use crate::native::RemapItem;
use crate::native::dbi::DebugStream;
use crate::native::debug_stream::{DebugRecord, Fixup, FunctionEntry};
use crate::native::editor::model::missing;
use crate::native::editor::plan::unsupported;
use crate::native::editor::streams::{DebugContent, DebugStreamList};
use crate::native::fpo::{FpoData, FrameData};
use crate::native::output::no_memory;
use crate::native::remap::clean::{Part, Point};
use crate::{Error, ErrorKind};

/// Rewrite the debug streams: the address records, the unwind streams of the caller and the
/// section headers. Remove the OMAP streams and the original section headers.
pub(super) fn rewrite(list: &mut DebugStreamList, context: &mut Context<'_>) -> Result<()> {
    rewrite_stream::<FpoData>(list, DebugStream::Fpo, context, |record, context| {
        let Some(part) = extent(RemapItem::Fpo, record.start, record.size, context)? else {
            return Ok(None);
        };
        Ok(Some(FpoData {
            start: part.rva,
            ..*record
        }))
    })?;
    rewrite_stream::<FrameData>(list, DebugStream::NewFpo, context, |record, context| {
        let Some(part) = extent(RemapItem::NewFpo, record.start, record.size, context)? else {
            return Ok(None);
        };
        Ok(Some(FrameData {
            start: part.rva,
            ..*record
        }))
    })?;
    rewrite_stream::<FunctionEntry>(list, DebugStream::Exception, context, function_entry)?;
    rewrite_stream::<Fixup>(list, DebugStream::Fixup, context, fixup)?;
    unwind_streams(list, context)?;
    section_headers(list, context)?;
    for kind in [
        DebugStream::OmapToSource,
        DebugStream::OmapFromSource,
        DebugStream::OriginalSectionHeader,
    ] {
        list.remove(kind);
    }
    Ok(())
}

/// A record kind with a start address that the streams sort by.
trait Start {
    /// Return `true` if the stream is sorted by the start address.
    const SORTED: bool;

    /// Return the start address.
    fn start(&self) -> u32;
}

impl Start for FpoData {
    const SORTED: bool = true;

    fn start(&self) -> u32 {
        self.start
    }
}

impl Start for FrameData {
    const SORTED: bool = true;

    fn start(&self) -> u32 {
        self.start
    }
}

impl Start for FunctionEntry {
    const SORTED: bool = true;

    fn start(&self) -> u32 {
        self.start
    }
}

impl Start for Fixup {
    const SORTED: bool = false;

    fn start(&self) -> u32 {
        self.rva
    }
}

/// Return the bytes of the stream of `kind`, if the list has it.
fn stream_bytes(list: &DebugStreamList, kind: DebugStream) -> Result<Option<&[u8]>> {
    match list.entries.iter().find(|(slot, _)| *slot == kind) {
        None => Ok(None),
        Some((_, DebugContent::Bytes(bytes))) => Ok(Some(bytes)),
        Some((_, DebugContent::Source(_))) => {
            Err(missing("the clean remap has no bytes of a debug stream"))
        }
    }
}

/// Map each record of the stream of `kind` with `map` and write the stream if a record
/// changes.
///
/// `map` returns the new record, or `None` to remove the record. The stream is sorted
/// again by the start address if the kind is sorted.
fn rewrite_stream<R>(
    list: &mut DebugStreamList,
    kind: DebugStream,
    context: &mut Context<'_>,
    mut map: impl FnMut(&R, &mut Context<'_>) -> Result<Option<R>>,
) -> Result<()>
where
    R: DebugRecord + Start + PartialEq,
{
    let Some(bytes) = stream_bytes(list, kind)? else {
        return Ok(());
    };
    if !bytes.len().is_multiple_of(R::SIZE) {
        return Err(Error::new(ErrorKind::UnexpectedEof, None).with_context(R::TRUNCATED));
    }
    let mut records = Vec::new();
    records
        .try_reserve_exact(bytes.len() / R::SIZE)
        .map_err(|_| no_memory())?;
    let mut changed = false;
    for chunk in bytes.chunks_exact(R::SIZE) {
        let record = R::decode(chunk.as_chunks::<4>().0);
        match map(&record, context)? {
            Some(new) => {
                changed |= new != record;
                records.push(new);
            }
            None => changed = true,
        }
    }
    if !changed {
        return Ok(());
    }
    if R::SORTED {
        // The sort is stable, so the records at one address keep their order.
        records.sort_by_key(Start::start);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(records.len().saturating_mul(R::SIZE))
        .map_err(|_| no_memory())?;
    for record in &records {
        record.encode(&mut out)?;
    }
    list.set(kind, out)
}

/// Map the extent of `length` bytes from the PDB RVA `start` of a record of `item` without
/// a split form.
///
/// Return the one part of the extent, or `None` for a removed record. A start of 0 is not
/// an address: the record keeps it (one part at RVA 0).
///
/// # Errors
///
/// - [`ErrorKind::UnsupportedEdit`] if a range splits the extent, or with
///   [`RemovedCode::Fail`](crate::native::RemovedCode::Fail) if the extent is removed.
fn extent(
    item: RemapItem,
    start: u32,
    length: u32,
    context: &mut Context<'_>,
) -> Result<Option<Part>> {
    if start == 0 {
        return Ok(Some(Part {
            offset: 0,
            length,
            rva: 0,
        }));
    }
    let parts = context.map.parts(start, length)?;
    match parts.parts.as_slice() {
        [] => {
            context.remove(item, start, None)?;
            Ok(None)
        }
        [part] if !parts.removed => Ok(Some(*part)),
        _ => Err(no_split_form(item, start, None)),
    }
}

/// Map an exception data record: the extent from the start to the end, and the end of the
/// prolog in the same part.
fn function_entry(
    record: &FunctionEntry,
    context: &mut Context<'_>,
) -> Result<Option<FunctionEntry>> {
    let length = record.end.saturating_sub(record.start);
    let Some(part) = extent(RemapItem::ExceptionData, record.start, length, context)? else {
        return Ok(None);
    };
    if record.start == 0 {
        return Ok(Some(*record));
    }
    let end_of_prologue = if record.end_of_prologue == 0 {
        0
    } else if (record.start..=record.end).contains(&record.end_of_prologue) {
        part.rva + (record.end_of_prologue - record.start)
    } else {
        // The end of the prolog lies outside the function: it maps as a point.
        match context.map.rva_point(record.end_of_prologue) {
            Point::Mapped(rva) => rva,
            Point::Absent => record.end_of_prologue,
            Point::Removed(_) => {
                context.remove(RemapItem::ExceptionData, record.start, None)?;
                return Ok(None);
            }
        }
    };
    Ok(Some(FunctionEntry {
        start: part.rva,
        end: part.rva + length,
        end_of_prologue,
    }))
}

/// Map a fixup: the RVA and the target map as points. A value 0 is not an address.
fn fixup(record: &Fixup, context: &mut Context<'_>) -> Result<Option<Fixup>> {
    let mut out = *record;
    for value in [&mut out.rva, &mut out.target] {
        if *value == 0 {
            continue;
        }
        match context.map.rva_point(*value) {
            Point::Mapped(rva) => *value = rva,
            Point::Absent => {}
            Point::Removed(_) => {
                context.remove(RemapItem::Fixup, record.rva, None)?;
                return Ok(None);
            }
        }
    }
    Ok(Some(out))
}

/// Write the `.pdata` and `.xdata` streams of the caller into slots 8 and 7.
fn unwind_streams(list: &mut DebugStreamList, context: &Context<'_>) -> Result<()> {
    match context.remap.unwind_streams() {
        Some((pdata, xdata)) => {
            list.set(DebugStream::Pdata, pdata.to_bytes()?)?;
            list.set(DebugStream::Xdata, xdata.to_bytes()?)
        }
        None if list.contains(DebugStream::Pdata) || list.contains(DebugStream::Xdata) => {
            Err(unsupported(
                "clean remap output needs the .pdata and .xdata streams of the new image; use ImageRemap::with_unwind_streams",
            ))
        }
        None => Ok(()),
    }
}

/// Write the section headers of the new image into slot 5, if they change.
fn section_headers(list: &mut DebugStreamList, context: &Context<'_>) -> Result<()> {
    let mut bytes = Vec::new();
    context.remap.sections().encode(&mut bytes)?;
    if stream_bytes(list, DebugStream::SectionHeader)? == Some(bytes.as_slice()) {
        return Ok(());
    }
    list.set(DebugStream::SectionHeader, bytes)
}
