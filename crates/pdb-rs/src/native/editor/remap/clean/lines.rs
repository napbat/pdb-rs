//! The clean rewrite of the C13 subsections of a module.
//!
//! The rewrite changes the subsections that hold addresses:
//!
//! - `DEBUG_S_LINES` gets one subsection for each part of its code extent ([`split`]).
//! - `DEBUG_S_IL_LINES` holds IL offsets. Its header start maps as a point.
//! - `DEBUG_S_FRAMEDATA` records move with their extent. A split fails.
//! - `DEBUG_S_COFF_SYMBOL_RVA` entries map as points.
//!
//! A `DEBUG_S_SYMBOLS` subsection, a subsection of an unknown kind and C11 line data are
//! opaque data that can hold an address, so the rewrite fails for them. The other
//! subsections hold no address and stay.
//!
//! The rewrite changes the list of the module only if a value changes. Thus the clean
//! identity map keeps the source bytes of the C13 line region.

use alloc::vec::Vec;

use super::{Context, no_split_form};
use crate::native::RemapItem;
use crate::native::address::{Rva, SectionOffset};
use crate::native::dbi::ModuleStream;
use crate::native::editor::module::ModuleContents;
use crate::native::fpo::FrameData;
use crate::native::lines::{
    CoffSymbolRvas, FrameDataSubsection, LineSubsection, LinesSubsection, SubsectionKind,
};
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::native::remap::clean::Point;
use crate::{Error, ErrorKind, Result};

mod split;

/// The new subsections that replace one subsection, or `None` if the subsection stays.
///
/// An empty list removes the subsection.
type Change = Option<Vec<LineSubsection<'static>>>;

/// Rewrite the addresses of the C13 subsections of `module`.
///
/// # Errors
///
/// - [`ErrorKind::UnsupportedEdit`] for C11 line data, a `DEBUG_S_SYMBOLS` subsection, a
///   subsection of an unknown kind, a record without a split form that a range splits,
///   and (with [`RemovedCode::Fail`](crate::native::RemovedCode::Fail)) a record that holds
///   removed code.
/// - [`ErrorKind::LimitExceeded`] if an allocation fails.
pub(super) fn rewrite(module: &mut ModuleContents, context: &mut Context<'_>) -> Result<()> {
    check_c11(module)?;
    let mut places = Places::new(module);
    let lines = module.lines();
    let mut output: Option<Vec<LineSubsection<'static>>> = None;
    for (index, subsection) in lines.iter().enumerate() {
        let mut location = || places.at(index);
        let change = match subsection {
            LineSubsection::Lines(lines) => split::rewrite(lines, context, &mut location)?,
            LineSubsection::IlLines(lines) => il_lines(lines, context, &mut location)?,
            LineSubsection::FrameData(frames) => frame_data(frames, context, &mut location)?,
            LineSubsection::CoffSymbolRvas(rvas) => coff_symbol_rvas(rvas, context, &mut location)?,
            LineSubsection::Symbols(_) | LineSubsection::Unsupported { .. } => {
                return Err(opaque(subsection.kind(), location()));
            }
            _ => None,
        };
        match change {
            None => {
                if let Some(out) = &mut output {
                    push(out, subsection.clone())?;
                }
            }
            Some(new) => {
                if output.is_none() {
                    output = Some(copy(&lines[..index])?);
                }
                if let Some(out) = &mut output {
                    out.try_reserve(new.len()).map_err(|_| no_memory())?;
                    out.extend(new);
                }
            }
        }
    }
    if let Some(output) = output {
        *module.lines_mut() = output;
    }
    Ok(())
}

/// Return the error for C11 line data of the source module stream.
///
/// The plan copies the C11 line region of the source, so the rewrite cannot change its
/// addresses.
fn check_c11(module: &ModuleContents) -> Result<()> {
    let Some(stream) = module.source().and_then(|source| source.stream.as_ref()) else {
        return Ok(());
    };
    if stream.c11_data().is_empty() {
        return Ok(());
    }
    let offset = stream.symbol_offset() + stream.symbol_data().len();
    Err(Error::new(ErrorKind::UnsupportedEdit, None)
        .with_context("C11 line data can hold addresses, and clean remap output cannot rewrite it")
        .with_stream(stream.stream(), to_u64(offset)))
}

/// Return the error for a subsection of `kind` whose data stays opaque.
fn opaque(kind: SubsectionKind, location: Option<(StreamId, u64)>) -> Error {
    let context = if kind.is_ignored() || kind.is_decoded() {
        "a DEBUG_S_SYMBOLS subsection can hold addresses, and clean remap output cannot rewrite it"
    } else {
        "a C13 subsection of an unknown kind can hold addresses, and clean remap output cannot rewrite it"
    };
    let error = Error::new(ErrorKind::UnsupportedEdit, None).with_context(context);
    match location {
        Some((stream, offset)) => error.with_stream(stream, offset),
        None => error,
    }
}

/// Map the header start of a `DEBUG_S_IL_LINES` subsection as a point.
///
/// The code offsets of the entries are IL offsets, so they do not change.
fn il_lines(
    lines: &LinesSubsection<'_>,
    context: &mut Context<'_>,
    location: &mut impl FnMut() -> Option<(StreamId, u64)>,
) -> Result<Change> {
    let header = lines.header();
    let start = SectionOffset {
        section: header.segment,
        offset: header.code_offset,
    };
    match context.map.point(start) {
        Point::Absent => Ok(None),
        Point::Mapped(new) if new == start => Ok(None),
        Point::Mapped(new) => {
            let moved = split::moved(lines, new)?;
            Ok(Some(single(LineSubsection::IlLines(moved))?))
        }
        Point::Removed(rva) => {
            context.remove(RemapItem::LineBlock, rva, location())?;
            Ok(Some(Vec::new()))
        }
    }
}

/// Move each record of a `DEBUG_S_FRAMEDATA` subsection with its extent.
///
/// A record keeps its position. A start of 0 is not an address.
fn frame_data(
    frames: &FrameDataSubsection<'_>,
    context: &mut Context<'_>,
    location: &mut impl FnMut() -> Option<(StreamId, u64)>,
) -> Result<Change> {
    let records = frames.records();
    let mut out: Vec<FrameData> = Vec::new();
    out.try_reserve_exact(records.len())
        .map_err(|_| no_memory())?;
    let mut changed = false;
    for record in records {
        if record.start == 0 {
            out.push(record);
            continue;
        }
        let parts = context.map.parts(record.start, record.size)?;
        match parts.parts.first() {
            Some(part) if parts.is_whole() => {
                let mut moved = record;
                moved.start = part.rva;
                changed |= moved != record;
                out.push(moved);
            }
            _ if parts.parts.len() > 1 => {
                return Err(no_split_form(
                    RemapItem::ModuleFrameData,
                    record.start,
                    location(),
                ));
            }
            _ => {
                context.remove(RemapItem::ModuleFrameData, record.start, location())?;
                changed = true;
            }
        }
    }
    if !changed {
        return Ok(None);
    }
    let mut data = Vec::new();
    FrameDataSubsection::encode_parts(frames.relocation(), &out, &mut data)?;
    let frames = FrameDataSubsection::parse(&data)?.into_owned();
    Ok(Some(single(LineSubsection::FrameData(frames))?))
}

/// Map each entry of a `DEBUG_S_COFF_SYMBOL_RVA` subsection as a point.
///
/// The values 0 (no image address) and `0xffffffff` (an absolute value) are not addresses.
/// Entry `i` belongs to record `i` of the COFF symbol table, so a removed entry gets the
/// value 0 and keeps its position.
fn coff_symbol_rvas(
    rvas: &CoffSymbolRvas<'_>,
    context: &mut Context<'_>,
    location: &mut impl FnMut() -> Option<(StreamId, u64)>,
) -> Result<Change> {
    let mut out: Vec<Rva> = Vec::new();
    out.try_reserve_exact(rvas.len()).map_err(|_| no_memory())?;
    let mut changed = false;
    for rva in rvas {
        if rva.0 == 0 || rva.0 == u32::MAX {
            out.push(rva);
            continue;
        }
        match context.map.rva_point(rva.0) {
            Point::Mapped(new) => {
                changed |= new != rva.0;
                out.push(Rva(new));
            }
            Point::Removed(old) => {
                context.remove(RemapItem::CoffSymbolRva, old, location())?;
                changed = true;
                out.push(Rva(0));
            }
            Point::Absent => out.push(rva),
        }
    }
    if !changed {
        return Ok(None);
    }
    let mut data = Vec::new();
    CoffSymbolRvas::encode_parts(&out, &mut data)?;
    let rvas = CoffSymbolRvas::parse(&data)?.into_owned();
    Ok(Some(single(LineSubsection::CoffSymbolRvas(rvas))?))
}

/// The module stream locations of the source subsections.
///
/// The locations are known only while the list of the module is the source list.
struct Places<'a> {
    /// The source module stream, if the list is the source list.
    stream: Option<&'a ModuleStream>,
    /// The number of subsections in the list.
    count: usize,
    /// The module stream offset of each subsection header, after the first query.
    offsets: Option<Vec<u64>>,
}

impl<'a> Places<'a> {
    /// Make the locations of the subsections of `module`.
    fn new(module: &'a ModuleContents) -> Self {
        let stream = module
            .source()
            .and_then(|source| source.stream.as_ref())
            .filter(|_| !module.lines_changed());
        Self {
            stream,
            count: module.lines().len(),
            offsets: None,
        }
    }

    /// Return the module stream and the header offset of subsection `index`, if known.
    fn at(&mut self, index: usize) -> Option<(StreamId, u64)> {
        let stream = self.stream?;
        let offsets = self
            .offsets
            .get_or_insert_with(|| header_offsets(stream, self.count));
        offsets.get(index).map(|&offset| (stream.stream(), offset))
    }
}

/// Return the module stream offset of each C13 subsection header of `stream`.
///
/// Return an empty list if the framing fails or does not give `count` subsections.
fn header_offsets(stream: &ModuleStream, count: usize) -> Vec<u64> {
    let base = stream.symbol_offset() + stream.symbol_data().len() + stream.c11_data().len();
    let mut offsets = Vec::new();
    if offsets.try_reserve_exact(count).is_err() {
        return Vec::new();
    }
    let mut subsections = stream.c13_subsections();
    while let Some(Ok((offset, _, _))) = subsections.next_parts() {
        if offsets.len() == count {
            return Vec::new();
        }
        let offset = usize::try_from(offset).map_or(u64::MAX, |offset| to_u64(base + offset));
        offsets.push(offset);
    }
    if offsets.len() == count {
        offsets
    } else {
        Vec::new()
    }
}

/// Return a list with the one subsection `subsection`.
fn single(subsection: LineSubsection<'static>) -> Result<Vec<LineSubsection<'static>>> {
    let mut list = Vec::new();
    push(&mut list, subsection)?;
    Ok(list)
}

/// Return a copy of `lines`.
fn copy(lines: &[LineSubsection<'static>]) -> Result<Vec<LineSubsection<'static>>> {
    let mut list = Vec::new();
    list.try_reserve(lines.len()).map_err(|_| no_memory())?;
    list.extend_from_slice(lines);
    Ok(list)
}

/// Append `value` to `list` with a checked allocation.
fn push<T>(list: &mut Vec<T>, value: T) -> Result<()> {
    list.try_reserve(1).map_err(|_| no_memory())?;
    list.push(value);
    Ok(())
}

/// Return a stream offset as `u64`.
fn to_u64(offset: usize) -> u64 {
    u64::try_from(offset).unwrap_or(u64::MAX)
}
