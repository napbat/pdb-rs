//! The clean rewrite of the DBI values (#43, decisions 4 to 6).
//!
//! - Each section contribution gets one entry for each part of its extent. The list is
//!   sorted again by section and offset if an entry changes.
//! - The module record of each module gets the first contribution of its module.
//! - The section map gets new entries from the new section headers, as Microsoft
//!   `link.exe` writes them, if the section headers change.
//!
//! A contribution whose section, offset or size changes gets the data CRC 0 and the
//! relocation CRC 0 (decision 6).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use super::Context;
use crate::native::RemapItem;
use crate::native::address::SectionOffset;
use crate::native::codeview::index::ModuleIndex;
use crate::native::dbi::{SectionContribution, SectionMap, SectionMapEntry, SegmentFlags};
use crate::native::editor::model::ModuleSlot;
use crate::native::editor::streams::DebugInfo;
use crate::native::output::no_memory;
use crate::native::remap::clean::Part;
use crate::native::section::SectionHeader;
use crate::{Error, ErrorKind, Result};

/// `IMAGE_SCN_MEM_16BIT`: the section has 16-bit code.
const SCN_MEM_16BIT: u32 = 0x0002_0000;
/// `IMAGE_SCN_MEM_EXECUTE`: the section is executable.
const SCN_MEM_EXECUTE: u32 = 0x2000_0000;
/// `IMAGE_SCN_MEM_READ`: the section is readable.
const SCN_MEM_READ: u32 = 0x4000_0000;
/// `IMAGE_SCN_MEM_WRITE`: the section is writable.
const SCN_MEM_WRITE: u32 = 0x8000_0000;

/// The module record contribution of a module without a contribution, as Microsoft
/// `link.exe` writes it: section `0xffff`, size -1 and module `0xffff`.
const NO_CONTRIBUTION: SectionContribution = SectionContribution {
    section: 0xffff,
    padding1: 0,
    offset: 0,
    size: -1,
    characteristics: 0,
    module: ModuleIndex(0xffff),
    padding2: 0,
    data_crc: Some(0),
    reloc_crc: Some(0),
    coff_section: None,
};

/// Rewrite the section contributions, the section map and the contribution of each module
/// record.
pub(super) fn rewrite(
    dbi: &mut DebugInfo,
    modules: &mut [ModuleSlot],
    context: &mut Context<'_>,
) -> Result<()> {
    let changes = contributions(dbi, context)?;
    module_records(dbi, modules, &changes, context)?;
    section_map(dbi, context)
}

/// The modules of the section contributions.
#[derive(Debug, Default)]
struct Changes {
    /// The modules with a contribution in the source list.
    present: BTreeSet<u32>,
    /// The modules with a changed, split or removed contribution.
    changed: BTreeSet<u32>,
}

/// The new place of a source extent.
enum Extent {
    /// The extent is not an address, or it does not move: keep the value.
    Keep,
    /// The parts of the extent, as new section, offset and size, in old address order.
    /// An empty list means that the extent is removed. `removed` is `true` if a byte of
    /// the extent has no range.
    Parts {
        /// The PDB RVA of the extent start.
        rva: u32,
        /// The new places.
        parts: Vec<(SectionOffset, u32)>,
        /// `true` if a byte of the extent has no range.
        removed: bool,
    },
}

/// Map the extent of `contribution` to the new image.
fn extent(contribution: &SectionContribution, context: &Context<'_>) -> Result<Extent> {
    let (Ok(offset), Ok(size)) = (
        u32::try_from(contribution.offset),
        u32::try_from(contribution.size),
    ) else {
        // A negative offset or size is not an address.
        return Ok(Extent::Keep);
    };
    let start = SectionOffset {
        section: contribution.section,
        offset,
    };
    let (Some(rva), Some(found)) = (
        context.map.old_rva(start),
        context.map.section_parts(start, size)?,
    ) else {
        return Ok(Extent::Keep);
    };
    let mut parts = Vec::new();
    parts
        .try_reserve_exact(found.parts.len())
        .map_err(|_| no_memory())?;
    let mut removed = found.removed;
    for &Part { length, rva, .. } in &found.parts {
        // A range lies inside one new section. A part without a section is removed code.
        match context.map.section_offset(rva) {
            Some(place) => parts.push((place, length)),
            None => removed = true,
        }
    }
    if !removed
        && let [(place, length)] = parts.as_slice()
        && *place == start
        && *length == size
    {
        return Ok(Extent::Keep);
    }
    Ok(Extent::Parts {
        rva,
        parts,
        removed,
    })
}

/// Return `contribution` at the new place `place` with `size` bytes.
///
/// The contribution gets the data CRC 0 and the relocation CRC 0, if it has CRCs.
fn moved(
    contribution: &SectionContribution,
    place: SectionOffset,
    size: u32,
    context: &mut Context<'_>,
) -> Result<SectionContribution> {
    let mut out = *contribution;
    out.section = place.section;
    out.offset = signed(place.offset)?;
    out.size = signed(size)?;
    if out.data_crc.is_some() || out.reloc_crc.is_some() {
        out.data_crc = out.data_crc.map(|_| 0);
        out.reloc_crc = out.reloc_crc.map(|_| 0);
        context.report.count_cleared_crcs(1);
    }
    Ok(out)
}

/// Return a new offset or size of a contribution as the `i32` of the record.
fn signed(value: u32) -> Result<i32> {
    i32::try_from(value).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("a remapped section contribution does not fit in 31 bits")
    })
}

/// Rewrite the section contribution list. Return the modules of the list and the modules
/// whose contributions change.
fn contributions(dbi: &mut DebugInfo, context: &mut Context<'_>) -> Result<Changes> {
    let source = dbi.section_contributions();
    let mut changes = Changes::default();
    let mut out = Vec::new();
    out.try_reserve(source.len()).map_err(|_| no_memory())?;
    for contribution in source {
        let module = contribution.module.0;
        changes.present.insert(module);
        let Extent::Parts {
            rva,
            parts,
            removed,
        } = extent(contribution, context)?
        else {
            out.push(*contribution);
            continue;
        };
        changes.changed.insert(module);
        if removed {
            context.remove(RemapItem::SectionContribution, rva, None)?;
        }
        out.try_reserve(parts.len()).map_err(|_| no_memory())?;
        for &(place, size) in &parts {
            out.push(moved(contribution, place, size, context)?);
        }
        context.added(
            RemapItem::SectionContribution,
            parts.len().saturating_sub(1),
        );
    }
    if !changes.changed.is_empty() {
        // The sort is stable, so the contributions at one address keep their order.
        out.sort_by_key(|contribution| (contribution.section, contribution.offset));
        let version = dbi.section_contribution_version();
        dbi.set_section_contributions(version, out);
    }
    Ok(changes)
}

/// Set the contribution of each module record.
///
/// - A module with a changed contribution gets its first contribution in the new list, or
///   the value of a module without a contribution.
/// - A module without a contribution in the list keeps a record value that is not an
///   address. Else its record value maps as an extent, and the record gets the first part.
/// - Each other record does not change.
fn module_records(
    dbi: &DebugInfo,
    modules: &mut [ModuleSlot],
    changes: &Changes,
    context: &mut Context<'_>,
) -> Result<()> {
    let mut first = BTreeMap::new();
    if !changes.changed.is_empty() {
        for contribution in dbi.section_contributions() {
            if changes.changed.contains(&contribution.module.0) {
                first.entry(contribution.module.0).or_insert(*contribution);
            }
        }
    }
    for (index, slot) in modules.iter_mut().enumerate() {
        let ModuleSlot::Decoded(module) = slot else {
            continue;
        };
        let index = u32::try_from(index).unwrap_or(u32::MAX);
        let record = &mut module.record_mut().section_contribution;
        let new = if changes.changed.contains(&index) {
            first.get(&index).copied().unwrap_or(NO_CONTRIBUTION)
        } else if changes.present.contains(&index) {
            continue;
        } else {
            match record_extent(record, context)? {
                Some(new) => new,
                None => continue,
            }
        };
        set_record(record, new, context);
    }
    Ok(())
}

/// Map the record value `record` of a module without a contribution in the list. Return
/// the new value, or `None` if the value does not change.
fn record_extent(
    record: &SectionContribution,
    context: &mut Context<'_>,
) -> Result<Option<SectionContribution>> {
    let Extent::Parts {
        rva,
        parts,
        removed,
    } = extent(record, context)?
    else {
        return Ok(None);
    };
    if removed {
        context.remove(RemapItem::SectionContribution, rva, None)?;
    }
    let Some(&(place, size)) = parts.iter().min_by_key(|(place, _)| *place) else {
        return Ok(Some(NO_CONTRIBUTION));
    };
    let mut new = *record;
    new.section = place.section;
    new.offset = signed(place.offset)?;
    new.size = signed(size)?;
    Ok(Some(new))
}

/// Set the module record value `record` to `new` in the layout of a module record. A value
/// with a new section, offset or size gets the data CRC 0 and the relocation CRC 0.
fn set_record(
    record: &mut SectionContribution,
    new: SectionContribution,
    context: &mut Context<'_>,
) {
    if (record.section, record.offset, record.size) == (new.section, new.offset, new.size) {
        return;
    }
    *record = SectionContribution {
        data_crc: Some(0),
        reloc_crc: Some(0),
        coff_section: None,
        ..new
    };
    context.report.count_cleared_crcs(1);
}

/// Write the section map of the new section headers, if the section headers change.
///
/// A DBI stream without a section map keeps none.
fn section_map(dbi: &mut DebugInfo, context: &Context<'_>) -> Result<()> {
    let sections = context.map.new_sections();
    if sections == context.map.source_sections() {
        return Ok(());
    }
    let Some(source) = dbi.section_map() else {
        return Ok(());
    };
    let map = derive_section_map(sections, source)?;
    dbi.set_section_map(Some(map));
    Ok(())
}

/// Return the `OMFSegDescFlags` of a section with the characteristics `characteristics`,
/// as Microsoft `link.exe` and LLVM `lld-link` write them.
fn segment_flags(characteristics: u32) -> SegmentFlags {
    let mut flags = SegmentFlags::IS_SELECTOR.0;
    if characteristics & SCN_MEM_16BIT == 0 {
        flags |= SegmentFlags::ADDRESS_IS_32_BIT.0;
    }
    for (bit, flag) in [
        (SCN_MEM_READ, SegmentFlags::READ),
        (SCN_MEM_WRITE, SegmentFlags::WRITE),
        (SCN_MEM_EXECUTE, SegmentFlags::EXECUTE),
    ] {
        if characteristics & bit != 0 {
            flags |= flag.0;
        }
    }
    SegmentFlags(flags)
}

/// Return the section map of `sections`: one entry for each section and a last entry for
/// the absolute symbols.
///
/// The last entry has frame 0, as Microsoft `link.exe` writes it. LLVM `lld-link` writes the
/// number of the entry, but then DIA finds no address in the last section. The logical
/// count is the number of entries if the source does so, else the source value.
fn derive_section_map(sections: &[SectionHeader], source: &SectionMap) -> Result<SectionMap> {
    let count = u16::try_from(sections.len()).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("the new image has more than 65534 section headers")
    })?;
    let absolute_frame = count.checked_add(1).ok_or_else(|| {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("the new image has more than 65534 section headers")
    })?;
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(sections.len() + 1)
        .map_err(|_| no_memory())?;
    for (frame, header) in (1..=count).zip(sections) {
        entries.push(SectionMapEntry {
            flags: segment_flags(header.characteristics),
            overlay: 0,
            group: 0,
            frame,
            section_name: 0xffff,
            class_name: 0xffff,
            offset: 0,
            length: header.virtual_size,
        });
    }
    entries.push(SectionMapEntry {
        flags: SegmentFlags(
            SegmentFlags::IS_ABSOLUTE_ADDRESS.0 | SegmentFlags::ADDRESS_IS_32_BIT.0,
        ),
        overlay: 0,
        group: 0,
        frame: 0,
        section_name: 0xffff,
        class_name: 0xffff,
        offset: 0,
        length: u32::MAX,
    });
    let logical_count = if usize::from(source.logical_count) == source.entries.len() {
        absolute_frame
    } else {
        source.logical_count
    };
    Ok(SectionMap {
        logical_count,
        entries,
    })
}
