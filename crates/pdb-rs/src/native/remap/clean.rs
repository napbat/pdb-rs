//! The address map of clean output.
//!
//! A [`CleanMap`] maps an address of the source PDB to the new image. A source address is a
//! section and an offset in the section headers that the records use: the original section
//! headers of a source with OMAP, else the image section headers. Its PDB RVA is the virtual
//! address of the section plus the offset. The map translates the PDB RVA through the source
//! OMAP (if present) and the ranges, as the `from_source` table of
//! [`ImageRemap::address_tables`] does. The section headers of the new image then give the
//! new section and offset.
//!
//! An extent (a start and a length) maps to parts: each part is a piece that one segment of
//! the map moves. Two pieces that continue each other in the old and in the new image are one
//! part. A piece without a segment is removed code.

use alloc::vec::Vec;

use super::ImageRemap;
use super::omap::{Segment, compose, table_segments};
use crate::native::address::{AddressTables, SectionOffset};
use crate::native::section::SectionHeader;
use crate::{Error, ErrorKind, Result};

/// The answer of [`CleanMap::point`] for one address.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Point<T> {
    /// The value is not an address: section 0 or a section after the last header. It keeps
    /// its value.
    Absent,
    /// The address has no range: its code is removed. The value is the PDB RVA.
    Removed(u32),
    /// The new address.
    Mapped(T),
}

/// A part of an extent that one segment of the map moves.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Part {
    /// The offset of the first byte of the part from the start of the extent.
    pub(crate) offset: u32,
    /// The number of bytes in the part; at least 1.
    pub(crate) length: u32,
    /// The new RVA of the first byte.
    pub(crate) rva: u32,
}

/// The parts of one extent, in the order of the old addresses.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Parts {
    /// The mapped parts.
    pub(crate) parts: Vec<Part>,
    /// `true` if a byte of the extent has no range.
    pub(crate) removed: bool,
}

impl Parts {
    /// Return `true` if the whole extent maps to one part.
    pub(crate) fn is_whole(&self) -> bool {
        !self.removed && self.parts.len() == 1
    }
}

/// The map from the addresses of a source PDB to the new image of an [`ImageRemap`].
#[derive(Clone, Debug)]
pub(crate) struct CleanMap {
    /// The virtual addresses of the section headers that the source addresses use.
    old_sections: Vec<u32>,
    /// The image section headers of the source (slot 5).
    source_sections: Vec<SectionHeader>,
    /// The section headers of the new image.
    new_sections: Vec<SectionHeader>,
    /// The segments from PDB RVAs to new RVAs, in ascending order of the start.
    segments: Vec<Segment>,
}

impl CleanMap {
    /// Make the map of `remap` for a source with the address tables `source`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the source has OMAP, but no original section headers
    ///   or only one of the two OMAP tables.
    /// - [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub(crate) fn new(remap: &ImageRemap, source: &AddressTables) -> Result<Self> {
        let ranges = remap.by_old.values().map(|range| Segment {
            start: range.old.0,
            length: range.length,
            target: range.new.0,
        });
        let mut to_new = Vec::new();
        to_new
            .try_reserve_exact(ranges.len())
            .map_err(|_| no_memory())?;
        to_new.extend(ranges);
        let (headers, segments) = match (&source.from_source, &source.to_source) {
            (None, None) => (source.sections.headers(), to_new),
            (Some(from_source), Some(_)) => {
                let original = source.original.as_ref().ok_or_else(|| {
                    missing("the source has OMAP, but no original section headers")
                })?;
                let composed = compose(&table_segments(from_source)?, &to_new)?;
                (original.headers(), composed)
            }
            (None, Some(_)) => return Err(missing("the source has no OMAP from source table")),
            (Some(_), None) => return Err(missing("the source has no OMAP to source table")),
        };
        let mut old_sections = Vec::new();
        old_sections
            .try_reserve_exact(headers.len())
            .map_err(|_| no_memory())?;
        old_sections.extend(headers.iter().map(|header| header.virtual_address));
        let mut source_sections = Vec::new();
        source_sections
            .try_reserve_exact(source.sections.headers().len())
            .map_err(|_| no_memory())?;
        source_sections.extend(source.sections.headers().iter());
        let mut new_sections = Vec::new();
        new_sections
            .try_reserve_exact(remap.sections().len())
            .map_err(|_| no_memory())?;
        new_sections.extend(remap.sections().iter());
        Ok(Self {
            old_sections,
            source_sections,
            new_sections,
            segments: merge(segments),
        })
    }

    /// Return the number of section headers that the source addresses use.
    pub(crate) fn old_section_count(&self) -> usize {
        self.old_sections.len()
    }

    /// Return the image section headers of the source (slot 5 of the source).
    pub(crate) fn source_sections(&self) -> &[SectionHeader] {
        &self.source_sections
    }

    /// Return the section headers of the new image.
    pub(crate) fn new_sections(&self) -> &[SectionHeader] {
        &self.new_sections
    }

    /// Return the PDB RVA of a source address, or `None` if the value is not an address.
    pub(crate) fn old_rva(&self, address: SectionOffset) -> Option<u32> {
        let index = usize::from(address.section.checked_sub(1)?);
        self.old_sections
            .get(index)
            .and_then(|start| start.checked_add(address.offset))
    }

    /// Return the new RVA of the PDB RVA `rva`, or `None` if it has no range.
    pub(crate) fn rva(&self, rva: u32) -> Option<u32> {
        let index = self
            .segments
            .partition_point(|segment| segment.start <= rva);
        let segment = self.segments.get(index.checked_sub(1)?)?;
        let delta = rva - segment.start;
        (delta < segment.length).then(|| segment.target + delta)
    }

    /// Return the section and offset of the new RVA `rva` in the new section headers.
    ///
    /// A range lies inside one new section, so each new RVA of a range has a section.
    pub(crate) fn section_offset(&self, rva: u32) -> Option<SectionOffset> {
        self.new_sections
            .iter()
            .position(|header| {
                header.virtual_address <= rva
                    && u64::from(rva)
                        < u64::from(header.virtual_address) + u64::from(header.virtual_size)
            })
            .map(|index| SectionOffset {
                section: u16::try_from(index + 1).unwrap_or(u16::MAX),
                offset: rva - self.new_sections[index].virtual_address,
            })
    }

    /// Map the source address `address`.
    pub(crate) fn point(&self, address: SectionOffset) -> Point<SectionOffset> {
        let Some(old) = self.old_rva(address) else {
            return Point::Absent;
        };
        match self.rva(old).and_then(|rva| self.section_offset(rva)) {
            Some(new) => Point::Mapped(new),
            None => Point::Removed(old),
        }
    }

    /// Map the PDB RVA `rva` to a new RVA.
    pub(crate) fn rva_point(&self, rva: u32) -> Point<u32> {
        self.rva(rva).map_or(Point::Removed(rva), Point::Mapped)
    }

    /// Return the parts of the extent of `length` bytes from the PDB RVA `start`.
    ///
    /// An extent of 0 bytes maps as the point `start`: it has one part of length 0 if the
    /// point maps.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub(crate) fn parts(&self, start: u32, length: u32) -> Result<Parts> {
        let mut out = Parts::default();
        if length == 0 {
            match self.rva(start) {
                Some(rva) => push(
                    &mut out.parts,
                    Part {
                        offset: 0,
                        length: 0,
                        rva,
                    },
                )?,
                None => out.removed = true,
            }
            return Ok(out);
        }
        let low = u64::from(start);
        let high = low + u64::from(length);
        let mut covered = low;
        let mut index = self
            .segments
            .partition_point(|segment| segment.end() <= low);
        while let Some(&segment) = self.segments.get(index) {
            if u64::from(segment.start) >= high {
                break;
            }
            let from = low.max(u64::from(segment.start));
            let to = high.min(segment.end());
            out.removed |= from > covered;
            covered = to;
            // `from` and `to` lie inside the extent, so the differences fit in 32 bits.
            let part = Part {
                offset: narrow(from - low),
                length: narrow(to - from),
                rva: segment.target + narrow(from - u64::from(segment.start)),
            };
            match out.parts.last_mut() {
                Some(last)
                    if last.offset + last.length == part.offset
                        && u64::from(last.rva) + u64::from(last.length) == u64::from(part.rva) =>
                {
                    last.length += part.length;
                }
                _ => push(&mut out.parts, part)?,
            }
            index += 1;
        }
        out.removed |= covered < high;
        Ok(out)
    }

    /// Return the parts of the extent of `length` bytes from the source address `start`, or
    /// `None` if `start` is not an address.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub(crate) fn section_parts(&self, start: SectionOffset, length: u32) -> Result<Option<Parts>> {
        self.old_rva(start)
            .map(|rva| self.parts(rva, length))
            .transpose()
    }
}

/// Join the segments that continue each other in the old and in the new addresses.
fn merge(mut segments: Vec<Segment>) -> Vec<Segment> {
    let mut kept: usize = 0;
    for index in 0..segments.len() {
        let segment = segments[index];
        if let Some(last) = kept.checked_sub(1).map(|last| &mut segments[last])
            && last.end() == u64::from(segment.start)
            && u64::from(last.target) + u64::from(last.length) == u64::from(segment.target)
        {
            // The old range of `last` ends at the start of `segment`, which fits in 32
            // bits, and RVA 0 has no range. Thus the sum fits in 32 bits.
            last.length += segment.length;
            continue;
        }
        segments[kept] = segment;
        kept += 1;
    }
    segments.truncate(kept);
    segments
}

/// Append `value` with a checked allocation.
fn push<T>(list: &mut Vec<T>, value: T) -> Result<()> {
    list.try_reserve(1).map_err(|_| no_memory())?;
    list.push(value);
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
fn no_memory() -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context("clean remap allocation failed")
}
