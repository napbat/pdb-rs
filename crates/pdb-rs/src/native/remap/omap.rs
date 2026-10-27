//! The OMAP tables of an [`ImageRemap`] and their composition with source tables.
//!
//! A table is a list of linear segments: each segment maps `length` addresses from `start`
//! to `target`. The segments of a table do not overlap. The table records are:
//!
//! - one record `(start, target)` for each segment, in ascending order of `start`;
//! - one record `(end, 0)` after each segment whose end is not the start of the next
//!   segment, also after the last segment.
//!
//! The lookup rules of DIA ([`OmapStream::lookup`]) then map each address of a segment as
//! the segment says, and no other address.
//!
//! The composition of two tables maps each address through the first table, then through
//! the second table. It splits each segment of the first table at the segment ends of the
//! second table.

use alloc::vec::Vec;

use super::ImageRemap;
use crate::native::address::AddressTables;
use crate::native::debug_stream::{OmapRecord, OmapStream};
use crate::{Error, ErrorKind, Result};

/// A linear part of an address map: `length` addresses from `start` map to `target`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Segment {
    /// The first address.
    pub(super) start: u32,
    /// The number of addresses; at least 1.
    pub(super) length: u32,
    /// The target of `start`; not 0.
    pub(super) target: u32,
}

impl Segment {
    /// Return the address after the last address of the segment.
    pub(super) fn end(self) -> u64 {
        u64::from(self.start) + u64::from(self.length)
    }
}

/// Return the address tables of the new image of `remap` for a source with `source`.
pub(super) fn address_tables(remap: &ImageRemap, source: &AddressTables) -> Result<AddressTables> {
    let to_old = segments(remap.by_old.values().map(|range| Segment {
        start: range.old.0,
        length: range.length,
        target: range.new.0,
    }))?;
    let to_new = segments(remap.by_new.values().map(|range| Segment {
        start: range.new.0,
        length: range.length,
        target: range.old.0,
    }))?;
    let (original, from_source, to_source) = match (&source.from_source, &source.to_source) {
        (None, None) => (source.sections.clone(), to_old, to_new),
        (Some(from_source), Some(to_source)) => {
            let original = source
                .original
                .clone()
                .ok_or_else(|| missing("the source has OMAP, but no original section headers"))?;
            let from_source = compose(&table_segments(from_source)?, &to_old)?;
            let to_source = compose(&to_new, &table_segments(to_source)?)?;
            (original, from_source, to_source)
        }
        (None, Some(_)) => return Err(missing("the source has no OMAP from source table")),
        (Some(_), None) => return Err(missing("the source has no OMAP to source table")),
    };
    let mut tables = AddressTables::new(remap.sections.clone());
    tables.original = Some(original);
    tables.from_source = Some(table(&from_source)?);
    tables.to_source = Some(table(&to_source)?);
    Ok(tables)
}

/// Collect `items` into a vector with a checked allocation.
fn segments(items: impl ExactSizeIterator<Item = Segment>) -> Result<Vec<Segment>> {
    let mut out = Vec::new();
    out.try_reserve_exact(items.len())
        .map_err(|_| allocation_failed())?;
    out.extend(items);
    Ok(out)
}

/// Return the segments of the addresses that `table` maps, in ascending order.
///
/// A record maps the addresses up to the `rva` of the next record. The last record maps
/// only its own `rva`. A record with the target 0 maps no address. An address whose target
/// does not fit in 32 bits is not part of a segment.
pub(super) fn table_segments(table: &OmapStream) -> Result<Vec<Segment>> {
    let mut out = Vec::new();
    out.try_reserve_exact(table.len())
        .map_err(|_| allocation_failed())?;
    let mut records = table.iter().peekable();
    while let Some(record) = records.next() {
        let length = records
            .peek()
            .map_or(1, |next| u64::from(next.rva - record.rva));
        let fits = (1_u64 << 32) - u64::from(record.rva_to);
        let length = u32::try_from(length.min(fits)).unwrap_or(u32::MAX);
        if length != 0 && record.rva_to != 0 {
            out.push(Segment {
                start: record.rva,
                length,
                target: record.rva_to,
            });
        }
    }
    Ok(out)
}

/// Map the segments of `first` through the segments of `second`.
///
/// Both lists are in ascending order of `start`, and their segments do not overlap. The
/// result has the same properties. An address of `first` whose target is in no segment of
/// `second` is in no segment of the result.
pub(super) fn compose(first: &[Segment], second: &[Segment]) -> Result<Vec<Segment>> {
    let mut out = Vec::new();
    for &segment in first {
        let low = u64::from(segment.target);
        let high = low + u64::from(segment.length);
        let mut index = second.partition_point(|other| other.end() <= low);
        while let Some(&other) = second.get(index) {
            if u64::from(other.start) >= high {
                break;
            }
            let from = low.max(u64::from(other.start));
            let to = high.min(other.end());
            // `from` is at least `low`, and `to` is at most `high`. The differences thus
            // fit in the 32-bit length of `segment`, and the sums are addresses of it.
            let piece = Segment {
                start: segment.start + narrow(from - low),
                length: narrow(to - from),
                target: other.target + narrow(from - u64::from(other.start)),
            };
            out.try_reserve(1).map_err(|_| allocation_failed())?;
            out.push(piece);
            index += 1;
        }
    }
    Ok(out)
}

/// Return the OMAP table of `segments`.
fn table(segments: &[Segment]) -> Result<OmapStream> {
    let mut records = Vec::new();
    let mut end: Option<u64> = None;
    for segment in segments {
        if let Some(gap) = end.filter(|&end| end < u64::from(segment.start)) {
            push(&mut records, narrow(gap), 0)?;
        }
        push(&mut records, segment.start, segment.target)?;
        end = Some(segment.end());
    }
    if let Some(Ok(gap)) = end.map(u32::try_from) {
        push(&mut records, gap, 0)?;
    }
    OmapStream::from_records(&records)
}

/// Append one record to `records`.
fn push(records: &mut Vec<OmapRecord>, rva: u32, rva_to: u32) -> Result<()> {
    if records.len() >= ImageRemap::MAX_OMAP_RECORDS {
        return Err(Error::new(ErrorKind::LimitExceeded, None)
            .with_context("the OMAP table has more than the maximum number of records"));
    }
    records.try_reserve(1).map_err(|_| allocation_failed())?;
    records.push(OmapRecord { rva, rva_to });
    Ok(())
}

/// Return a value that is known to fit in 32 bits as `u32`.
fn narrow(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Return the error for an absent part of the source tables.
fn missing(context: &'static str) -> Error {
    Error::new(ErrorKind::MissingStream, None).with_context(context)
}

/// Return the error for a failed allocation.
fn allocation_failed() -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context("OMAP table allocation failed")
}
