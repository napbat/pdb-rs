//! The range sets of `verify-remap` (#43): identity, section shift, procedure reorder,
//! procedure split and procedure removal.
//!
//! Each set gives the section headers of a new image and the ranges from the old image to the
//! new image. The sets use the procedures of the source PDB ([`Procedure`]). A code section is
//! a section with `IMAGE_SCN_CNT_CODE` or `IMAGE_SCN_MEM_EXECUTE`. The sets cut each code
//! section into pieces: the procedures and the gaps between them.
//!
//! - Identity: one range for each section, to the same RVA.
//! - Section shift: each code section moves by the same amount, to a place after the end of
//!   the image. The other sections stay.
//! - Procedure reorder: the pieces of each code section come in reverse order.
//! - Procedure split: each procedure of 2 or more bytes splits into 2 ranges at a place that
//!   changes from procedure to procedure. The first parts and the gaps keep their order; the
//!   second parts follow them at the end of the section.
//! - Procedure removal: the removable procedures ([`Procedure::removable`]) have no range.
//!   The other pieces stay.
//!
//! OMAP cannot map RVA 0 (decision 1 of #43). A range that starts at the old or the new RVA 0
//! thus starts 1 byte later. The HLSL and the .NET fixtures have a section at RVA 0. In the
//! reorder set of the HLSL fixtures, the procedure moves to RVA 0, so its first byte has no
//! mapping.

use pdbrs::native::section::SectionHeader;

/// The `IMAGE_SCN_CNT_CODE` and `IMAGE_SCN_MEM_EXECUTE` flags.
const CODE: u32 = 0x0000_0020 | 0x2000_0000;

/// The alignment of the section shift.
const SECTION_ALIGNMENT: u32 = 0x1000;

/// The kind of a range set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SetKind {
    /// The identity map.
    Identity,
    /// A shift of each code section by a fixed amount.
    SectionShift,
    /// A new order of the procedures in each code section.
    Reorder,
    /// A split of each procedure into 2 ranges.
    Split,
    /// The removal of the procedures that no other record refers to.
    Removal,
}

impl SetKind {
    /// All kinds, in the order of the command output.
    pub(crate) const ALL: [Self; 5] = [
        Self::Identity,
        Self::SectionShift,
        Self::Reorder,
        Self::Split,
        Self::Removal,
    ];

    /// The name of the kind in the output and in the file names.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::SectionShift => "section-shift",
            Self::Reorder => "reorder",
            Self::Split => "split",
            Self::Removal => "removal",
        }
    }
}

/// One range: `length` bytes from the old RVA `old` to the new RVA `new`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Range {
    /// The old RVA.
    pub old: u32,
    /// The length in bytes.
    pub length: u32,
    /// The new RVA.
    pub new: u32,
}

/// A procedure of the source image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Procedure {
    /// The RVA of the first byte.
    pub start: u32,
    /// The length in bytes (more than 0).
    pub length: u32,
    /// `true` if no other record refers to the procedure.
    pub removable: bool,
}

/// The section headers of a new image and the ranges to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RangeSet {
    /// The kind.
    pub kind: SetKind,
    /// The section headers of the new image, in RVA order.
    pub sections: Vec<SectionHeader>,
    /// The ranges, sorted by the old RVA.
    pub ranges: Vec<Range>,
}

impl RangeSet {
    /// Return the new RVA of the old RVA `rva`, or `None` if no range holds it.
    pub(crate) fn map(&self, rva: u32) -> Option<u32> {
        let index = self.ranges.partition_point(|range| range.old <= rva);
        let range = self.ranges.get(index.checked_sub(1)?)?;
        let offset = rva - range.old;
        (offset < range.length).then(|| range.new + offset)
    }

    /// Return the number of procedures that the set removes.
    pub(crate) fn removed(&self, procedures: &[Procedure]) -> usize {
        procedures
            .iter()
            .filter(|procedure| self.map(procedure.start).is_none())
            .count()
    }
}

/// A part of a code section: a procedure or a gap.
#[derive(Clone, Copy, Debug)]
struct Piece {
    /// The old RVA.
    old: u32,
    /// The length in bytes.
    length: u32,
    /// The procedure of the piece, or `None` for a gap.
    procedure: Option<Procedure>,
}

/// Return `true` for a code section.
fn is_code(section: &SectionHeader) -> bool {
    section.characteristics & CODE != 0
}

/// Return the end of a section, or `None` if it does not fit in 32 bits.
fn end(section: &SectionHeader) -> Option<u32> {
    section.virtual_address.checked_add(section.virtual_size)
}

/// Keep the procedures that are inside one code section of `sections` and that do not overlap
/// a kept procedure, sorted by RVA.
pub(crate) fn select(sections: &[SectionHeader], mut candidates: Vec<Procedure>) -> Vec<Procedure> {
    candidates.sort_by_key(|procedure| (procedure.start, procedure.length));
    let mut kept: Vec<Procedure> = Vec::new();
    for procedure in candidates {
        let Some(procedure_end) = procedure.start.checked_add(procedure.length) else {
            continue;
        };
        let inside = sections.iter().any(|section| {
            is_code(section)
                && section.virtual_address <= procedure.start
                && end(section).is_some_and(|end| procedure_end <= end)
        });
        let after = kept
            .last()
            .is_none_or(|last| last.start + last.length <= procedure.start);
        if procedure.length > 0 && inside && after {
            kept.push(procedure);
        }
    }
    kept
}

/// Cut a code section into pieces: the procedures inside it and the gaps between them.
fn pieces(section: &SectionHeader, procedures: &[Procedure]) -> Vec<Piece> {
    let start = section.virtual_address;
    let section_end = end(section).unwrap_or(start);
    let mut pieces = Vec::new();
    let mut cursor = start;
    for procedure in procedures
        .iter()
        .filter(|procedure| (start..section_end).contains(&procedure.start))
    {
        if procedure.start > cursor {
            pieces.push(Piece {
                old: cursor,
                length: procedure.start - cursor,
                procedure: None,
            });
        }
        pieces.push(Piece {
            old: procedure.start,
            length: procedure.length,
            procedure: Some(*procedure),
        });
        cursor = procedure.start + procedure.length;
    }
    if section_end > cursor {
        pieces.push(Piece {
            old: cursor,
            length: section_end - cursor,
            procedure: None,
        });
    }
    pieces
}

/// Put `pieces` one after the other from the RVA `start` and return their ranges.
fn lay_out(start: u32, pieces: impl IntoIterator<Item = (u32, u32)>) -> Vec<Range> {
    let mut cursor = start;
    let mut ranges = Vec::new();
    for (old, length) in pieces {
        ranges.push(Range {
            old,
            length,
            new: cursor,
        });
        cursor += length;
    }
    ranges
}

/// Return the place of the split of the procedure `index` with `length` bytes: an offset from
/// 1 to `length - 1`.
fn split_offset(index: usize, length: u32) -> u32 {
    let spread = u64::try_from(index)
        .unwrap_or(0)
        .wrapping_mul(0x9e37_79b9)
        .wrapping_add(7);
    1 + u32::try_from(spread % u64::from(length - 1)).unwrap_or(0)
}

/// Return the ranges of one code section for the set `kind`.
fn code_ranges(kind: SetKind, section: &SectionHeader, procedures: &[Procedure]) -> Vec<Range> {
    let start = section.virtual_address;
    let pieces = pieces(section, procedures);
    let parts = pieces.iter().map(|piece| (piece.old, piece.length));
    match kind {
        SetKind::Identity | SetKind::SectionShift => lay_out(start, parts),
        SetKind::Reorder => lay_out(start, parts.rev()),
        SetKind::Split => {
            let mut first = Vec::new();
            let mut second = Vec::new();
            for (index, piece) in pieces.iter().enumerate() {
                if piece.procedure.is_some() && piece.length >= 2 {
                    let offset = split_offset(index, piece.length);
                    first.push((piece.old, offset));
                    second.push((piece.old + offset, piece.length - offset));
                } else {
                    first.push((piece.old, piece.length));
                }
            }
            lay_out(start, first.into_iter().chain(second))
        }
        SetKind::Removal => pieces
            .iter()
            .filter(|piece| !piece.procedure.is_some_and(|procedure| procedure.removable))
            .map(|piece| Range {
                old: piece.old,
                length: piece.length,
                new: piece.old,
            })
            .collect(),
    }
}

/// Return the amount of the section shift: the code sections then start after the end of the
/// image. Return 0 for an image without a code section.
fn shift(sections: &[SectionHeader]) -> Option<u32> {
    let Some(first_code) = sections
        .iter()
        .filter(|section| is_code(section))
        .map(|section| section.virtual_address)
        .min()
    else {
        return Some(0);
    };
    let image_end = sections.iter().map(end).max().flatten()?;
    image_end
        .checked_sub(first_code)?
        .checked_next_multiple_of(SECTION_ALIGNMENT)
}

/// Make the range set `kind` for an image with `sections` and the selected `procedures` (see
/// [`select`]).
///
/// # Errors
///
/// Returns an error if a section or a shifted section does not fit in 32 bits.
pub(crate) fn range_set(
    kind: SetKind,
    sections: &[SectionHeader],
    procedures: &[Procedure],
) -> Result<RangeSet, String> {
    let shift = match kind {
        SetKind::SectionShift => {
            shift(sections).ok_or("the section shift does not fit in 32 bits")?
        }
        _ => 0,
    };
    let mut new_sections = Vec::new();
    let mut ranges = Vec::new();
    for section in sections {
        let mut moved = *section;
        if is_code(section) {
            moved.virtual_address = section
                .virtual_address
                .checked_add(shift)
                .filter(|_| end(section).is_some())
                .ok_or("a shifted section does not fit in 32 bits")?;
        }
        if end(&moved).is_none() {
            return Err("a section does not fit in 32 bits".to_owned());
        }
        new_sections.push(moved);
        if section.virtual_size == 0 {
            continue;
        }
        if is_code(section) {
            for mut range in code_ranges(kind, section, procedures) {
                range.new += shift;
                ranges.push(range);
            }
        } else {
            ranges.push(Range {
                old: section.virtual_address,
                length: section.virtual_size,
                new: section.virtual_address,
            });
        }
    }
    new_sections.sort_by_key(|section| section.virtual_address);
    // OMAP cannot map RVA 0 (decision 1 of #43), so a range at RVA 0 starts 1 byte later.
    let mut ranges: Vec<Range> = ranges
        .into_iter()
        .filter_map(|range| {
            if range.old != 0 && range.new != 0 {
                return Some(range);
            }
            (range.length > 1).then(|| Range {
                old: range.old + 1,
                length: range.length - 1,
                new: range.new + 1,
            })
        })
        .collect();
    ranges.sort();
    Ok(RangeSet {
        kind,
        sections: new_sections,
        ranges,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(virtual_address: u32, virtual_size: u32, characteristics: u32) -> SectionHeader {
        let mut section = SectionHeader::new(*b".s\0\0\0\0\0\0", virtual_size, virtual_address);
        section.characteristics = characteristics;
        section
    }

    /// `.text` at `0x1000` (0x100 bytes) and `.data` at `0x2000` (0x80 bytes).
    fn sections() -> Vec<SectionHeader> {
        vec![
            section(0x1000, 0x100, 0x6000_0020),
            section(0x2000, 0x80, 0xc000_0040),
        ]
    }

    fn procedure(start: u32, length: u32, removable: bool) -> Procedure {
        Procedure {
            start,
            length,
            removable,
        }
    }

    fn procedures() -> Vec<Procedure> {
        select(
            &sections(),
            vec![
                procedure(0x1040, 0x20, true),
                procedure(0x1010, 0x20, false),
                // Overlaps the procedure at 0x1010.
                procedure(0x1020, 0x8, false),
                // Outside a code section.
                procedure(0x2000, 0x10, false),
                procedure(0x1060, 1, false),
            ],
        )
    }

    /// Check that the ranges of `set` do not overlap and that each new range is inside one
    /// new section.
    fn check(set: &RangeSet) {
        for pair in set.ranges.windows(2) {
            assert!(pair[0].old + pair[0].length <= pair[1].old, "{set:?}");
        }
        let mut by_new = set.ranges.clone();
        by_new.sort_by_key(|range| range.new);
        for pair in by_new.windows(2) {
            assert!(pair[0].new + pair[0].length <= pair[1].new, "{set:?}");
        }
        for range in &set.ranges {
            assert!(
                set.sections.iter().any(|section| {
                    section.virtual_address <= range.new
                        && range.new + range.length
                            <= section.virtual_address + section.virtual_size
                }),
                "{range:?}"
            );
        }
    }

    #[test]
    fn selection_keeps_procedures_in_code_without_overlap() {
        let starts: Vec<u32> = procedures().iter().map(|p| p.start).collect();
        assert_eq!(starts, [0x1010, 0x1040, 0x1060]);
    }

    #[test]
    fn identity_maps_each_section_to_itself() {
        let set = range_set(SetKind::Identity, &sections(), &procedures()).unwrap();
        check(&set);
        assert_eq!(set.sections, sections());
        for rva in [0x1000, 0x1010, 0x10ff, 0x2000, 0x207f] {
            assert_eq!(set.map(rva), Some(rva));
        }
        assert_eq!(set.map(0x1100), None);
        assert_eq!(set.map(0x2080), None);
    }

    #[test]
    fn section_shift_moves_code_after_the_image() {
        let set = range_set(SetKind::SectionShift, &sections(), &procedures()).unwrap();
        check(&set);
        assert_eq!(set.sections[0].virtual_address, 0x2000);
        assert_eq!(set.sections[1].virtual_address, 0x3000);
        assert_eq!(set.map(0x1010), Some(0x3010));
        assert_eq!(set.map(0x2010), Some(0x2010));
    }

    #[test]
    fn reorder_reverses_the_pieces() {
        let set = range_set(SetKind::Reorder, &sections(), &procedures()).unwrap();
        check(&set);
        // Pieces: gap 0x1000+0x10, 0x1010+0x20, gap 0x1030+0x10, 0x1040+0x20, 0x1060+1,
        // gap 0x1061+0x9f. Reversed: the last gap comes first.
        assert_eq!(set.map(0x1061), Some(0x1000));
        assert_eq!(set.map(0x1060), Some(0x109f));
        assert_eq!(set.map(0x1040), Some(0x10a0));
        assert_eq!(set.map(0x1000), Some(0x10f0));
    }

    #[test]
    fn split_moves_the_second_parts_to_the_end() {
        let set = range_set(SetKind::Split, &sections(), &procedures()).unwrap();
        check(&set);
        // Each procedure of 2 or more bytes has 2 ranges.
        let text: Vec<&Range> = set.ranges.iter().filter(|r| r.old < 0x2000).collect();
        assert_eq!(text.len(), 8);
        // The first parts keep their place; the second parts are at the end, in order.
        assert_eq!(set.map(0x1010), Some(0x1010));
        let mut by_new = text.clone();
        by_new.sort_by_key(|range| range.new);
        let last: Vec<u32> = by_new[6..].iter().map(|range| range.old).collect();
        assert!(last[0] > 0x1010 && last[0] < 0x1030, "{last:x?}");
        assert!(last[1] > 0x1040 && last[1] < 0x1060, "{last:x?}");
        assert_eq!(by_new[7].new + by_new[7].length, 0x1100);
    }

    #[test]
    fn removal_removes_the_removable_procedures() {
        let procedures = procedures();
        let set = range_set(SetKind::Removal, &sections(), &procedures).unwrap();
        check(&set);
        assert_eq!(set.map(0x1040), None);
        assert_eq!(set.map(0x105f), None);
        assert_eq!(set.map(0x1060), Some(0x1060));
        assert_eq!(set.removed(&procedures), 1);
    }
}
