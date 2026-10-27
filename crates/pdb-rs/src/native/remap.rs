//! Address remapping for a rewritten image.
//!
//! A binary rewriter moves, splits or removes code after the link. The PDB must then
//! describe the new image. [`ImageRemap`] holds the section headers of the new image and a
//! list of [`RemapRange`] values. Each range maps an RVA of the old image, with a length, to
//! an RVA of the new image.
//!
//! - The map is linear inside a range: each address keeps its distance from the range start.
//! - An old address outside all ranges has no new address: the rewriter removed its code.
//! - An old RVA is an address of the image that the source PDB matches ([`Rva`]), not a
//!   [`PdbRva`](super::address::PdbRva).
//!
//! [`ImageRemap::address_tables`] makes the OMAP tables and the section header streams of
//! the new image. [`AddressTables::map`] and DIA then translate each address exactly as the
//! ranges say. Clean output (`clean`) writes the new addresses into the records.

pub(crate) mod clean;
mod omap;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;

use super::address::{AddressTables, PdbRva, Rva};
use super::codeview::SymbolKind;
use super::debug_stream::RvaVaBlob;
use super::section::{SectionHeaderStream, SectionHeaders};
use crate::{Error, ErrorKind, Result};

/// One range of an [`ImageRemap`].
///
/// The range maps the old RVAs `old .. old + length` to the new RVAs
/// `new .. new + length`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub struct RemapRange {
    /// The first RVA of the range in the old image.
    pub old: Rva,
    /// The number of bytes in the range.
    pub length: u32,
    /// The first RVA of the range in the new image.
    pub new: Rva,
}

impl RemapRange {
    /// Make a range that maps `length` bytes from `old` to `new`.
    #[must_use]
    pub const fn new(old: Rva, length: u32, new: Rva) -> Self {
        Self { old, length, new }
    }

    /// Return the old RVA after the last byte of the range.
    ///
    /// The value is `None` if it does not fit in 32 bits.
    #[must_use]
    pub const fn old_end(&self) -> Option<u32> {
        self.old.0.checked_add(self.length)
    }

    /// Return the new RVA after the last byte of the range.
    ///
    /// The value is `None` if it does not fit in 32 bits.
    #[must_use]
    pub const fn new_end(&self) -> Option<u32> {
        self.new.0.checked_add(self.length)
    }
}

/// The address map from an old image to a rewritten image.
///
/// [`ImageRemap::with_range`] checks each range when it adds it. The checks make sure that
/// the OMAP tables of [`ImageRemap::address_tables`] give one answer for each address in
/// each direction.
///
/// The map keeps the ranges in address order. Thus the same set of ranges gives the same
/// tables, in any order of the calls.
///
/// # Example
///
/// ```
/// use pdb_rs::native::ImageRemap;
/// use pdb_rs::native::address::Rva;
/// use pdb_rs::native::section::{SectionHeader, SectionHeaderStream};
///
/// let text = SectionHeader::new(*b".text\0\0\0", 0x2000, 0x1000);
/// let sections = SectionHeaderStream::from_records(&[text])?;
/// let remap = ImageRemap::new(sections)
///     .with_range(Rva(0x1000), 0x2a0, Rva(0x2000))?
///     .with_range(Rva(0x12a0), 0x40, Rva(0x1000))?;
/// assert_eq!(remap.len(), 2);
/// # Ok::<(), pdb_rs::Error>(())
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ImageRemap {
    /// The section headers of the new image.
    sections: SectionHeaderStream,
    /// The ranges by old RVA.
    by_old: BTreeMap<u32, RemapRange>,
    /// The ranges by new RVA.
    by_new: BTreeMap<u32, RemapRange>,
    /// The `.pdata` and `.xdata` streams of the new image, if the caller gave them.
    unwind: Option<Box<(RvaVaBlob, RvaVaBlob)>>,
}

impl ImageRemap {
    /// The maximum number of OMAP records in one table.
    ///
    /// A table of this size has 64 MiB, the default stream limit of
    /// [`ReadOptions`](super::ReadOptions). Thus pdb-rs reads each table that it makes with
    /// the default options.
    pub const MAX_OMAP_RECORDS: usize = 8 * 1024 * 1024;

    /// The maximum number of ranges.
    ///
    /// Without OMAP in the source, each range gives at most 2 records in each table: the
    /// record of the range and one record for the gap after it.
    pub const MAX_RANGES: usize = Self::MAX_OMAP_RECORDS / 2;

    /// Make a map without ranges for a new image with the section headers `sections`.
    ///
    /// pdb-rs has no PE parser. The caller gives the section headers of the new image.
    #[must_use]
    pub fn new(sections: SectionHeaderStream) -> Self {
        Self {
            sections,
            by_old: BTreeMap::new(),
            by_new: BTreeMap::new(),
            unwind: None,
        }
    }

    /// Add a range that maps `length` bytes from the old RVA `old` to the new RVA `new`.
    ///
    /// # Errors
    ///
    /// Each error names the range ([`Error::remap_range`]).
    ///
    /// - [`ErrorKind::InvalidFormat`] if the range is empty, if `old` or `new` is 0 (OMAP
    ///   uses the target 0 for an address without a mapping), or if the range overlaps an
    ///   old range or a new range of the map.
    /// - [`ErrorKind::OffsetOverflow`] if the end of the old range or of the new range does
    ///   not fit in 32 bits.
    /// - [`ErrorKind::InvalidReference`] if no section of the new image holds all bytes of
    ///   the new range.
    /// - [`ErrorKind::LimitExceeded`] if the map already has [`ImageRemap::MAX_RANGES`]
    ///   ranges.
    pub fn with_range(mut self, old: Rva, length: u32, new: Rva) -> Result<Self> {
        let range = RemapRange::new(old, length, new);
        let fail = |kind, context| {
            Err(Error::new(kind, None)
                .with_context(context)
                .with_remap_range(range))
        };
        if length == 0 {
            return fail(ErrorKind::InvalidFormat, "the remap range is empty");
        }
        if old.0 == 0 || new.0 == 0 {
            return fail(
                ErrorKind::InvalidFormat,
                "the remap range starts at RVA 0, which OMAP cannot map",
            );
        }
        let (Some(old_end), Some(new_end)) = (range.old_end(), range.new_end()) else {
            return fail(
                ErrorKind::OffsetOverflow,
                "the end of the remap range does not fit in 32 bits",
            );
        };
        if !section_holds(self.sections.headers(), new.0, new_end) {
            return fail(
                ErrorKind::InvalidReference,
                "no section of the new image holds the new remap range",
            );
        }
        if overlaps(&self.by_old, old.0, old_end, RemapRange::old_end) {
            return fail(
                ErrorKind::InvalidFormat,
                "the old remap range overlaps the old range of another remap range",
            );
        }
        if overlaps(&self.by_new, new.0, new_end, RemapRange::new_end) {
            return fail(
                ErrorKind::InvalidFormat,
                "the new remap range overlaps the new range of another remap range",
            );
        }
        if self.by_old.len() >= Self::MAX_RANGES {
            return fail(
                ErrorKind::LimitExceeded,
                "the image remap has the maximum number of ranges",
            );
        }
        self.by_old.insert(old.0, range);
        self.by_new.insert(new.0, range);
        Ok(self)
    }

    /// Return the section headers of the new image.
    #[must_use]
    pub fn sections(&self) -> SectionHeaders<'_> {
        self.sections.headers()
    }

    /// Return the number of ranges.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_old.len()
    }

    /// Return `true` if the map has no ranges.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_old.is_empty()
    }

    /// Return the ranges in ascending order of the old RVA.
    pub fn ranges(&self) -> impl ExactSizeIterator<Item = RemapRange> + '_ {
        self.by_old.values().copied()
    }

    /// Give the `.pdata` and `.xdata` streams of the new image.
    ///
    /// pdb-rs does not decode the unwind information in `.xdata`, so it cannot rewrite the
    /// two streams. Clean output ([`RemapMode::Clean`]) writes these streams into the
    /// [`DebugStream::Pdata`](super::dbi::DebugStream::Pdata) and
    /// [`DebugStream::Xdata`](super::dbi::DebugStream::Xdata) slots. OMAP output ignores
    /// them and copies the source streams.
    #[must_use]
    pub fn with_unwind_streams(mut self, pdata: RvaVaBlob, xdata: RvaVaBlob) -> Self {
        self.unwind = Some(Box::new((pdata, xdata)));
        self
    }

    /// Return the `.pdata` and `.xdata` streams of the new image, if the caller gave them.
    #[must_use]
    pub fn unwind_streams(&self) -> Option<(&RvaVaBlob, &RvaVaBlob)> {
        self.unwind.as_deref().map(|(pdata, xdata)| (pdata, xdata))
    }

    /// Return the address tables of the new image for a source PDB with the address tables
    /// `source`.
    ///
    /// The result has these parts:
    ///
    /// - `sections`: the section headers of the new image.
    /// - `original`: the section headers that the addresses of the source PDB use. These
    ///   are the original section headers of a source with OMAP, else the image section
    ///   headers of the source.
    /// - `from_source`: a table that maps each [`PdbRva`](super::address::PdbRva) of the
    ///   source to the new image.
    /// - `to_source`: a table that maps each RVA of the new image to a
    ///   [`PdbRva`](super::address::PdbRva) of the source.
    ///
    /// Without OMAP in the source, a PDB RVA is an old image RVA, and the tables hold the
    /// ranges. If the source has OMAP, the function composes the source tables with the
    /// ranges. The `from_source` table then maps each PDB RVA directly to the new image. The
    /// `to_source` table maps each new RVA to the PDB RVA that the source `to_source` table
    /// gives for the old RVA.
    ///
    /// Each table has one record for each range or part of a range. A record with the
    /// target 0 follows each part that the next part does not continue. Thus an address
    /// without a range has no mapping, also after the last range. The
    /// [`OmapStream`](super::debug_stream::OmapStream) documentation gives the lookup rules.
    ///
    /// If the target of an address in a source table does not fit in 32 bits, the address
    /// has no mapping in the result.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the source has OMAP, but no original section
    ///   headers or only one of the two OMAP tables.
    /// - [`ErrorKind::LimitExceeded`] if a table has more than
    ///   [`ImageRemap::MAX_OMAP_RECORDS`] records or an allocation fails.
    pub fn address_tables(&self, source: &AddressTables) -> Result<AddressTables> {
        omap::address_tables(self, source)
    }
}

/// The output mode of [`PdbEditor::remap_addresses`](super::PdbEditor::remap_addresses).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum RemapMode {
    /// Write the OMAP tables and the section headers of the new image, and keep the
    /// records. A consumer translates each address through OMAP, as DIA does.
    Omap,
    /// Write the new address into each record. The output has no OMAP stream and no
    /// original section headers. The policy selects what the plan does with a record that
    /// holds removed code.
    Clean(RemovedCode),
}

/// The policy of [`RemapMode::Clean`] for a record that holds removed code: an old address
/// without a range.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum RemovedCode {
    /// Fail with [`ErrorKind::UnsupportedEdit`], and name the first record that holds
    /// removed code.
    #[default]
    Fail,
    /// Remove each record that holds removed code, and the references to it.
    Drop,
}

/// The kind of a record that holds an address, for [`RemapAddress`] and the report of clean
/// output.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RemapItem {
    /// A symbol record of a module stream or of the symbol record stream.
    Symbol(SymbolKind),
    /// A line block of a `DEBUG_S_LINES` subsection.
    LineBlock,
    /// A record of a `DEBUG_S_FRAMEDATA` subsection.
    ModuleFrameData,
    /// An RVA of a `DEBUG_S_COFF_SYMBOL_RVA` subsection.
    CoffSymbolRva,
    /// A DBI section contribution.
    SectionContribution,
    /// A record of the FPO stream.
    Fpo,
    /// A record of the new FPO stream.
    NewFpo,
    /// A record of the exception data stream.
    ExceptionData,
    /// A record of the fixup stream.
    Fixup,
    /// A thunk of the incremental thunk table of the public symbol index.
    Thunk,
}

/// The record and the address of a clean remap that the plan rejects.
///
/// [`Error::remap_address`] gives it. The error also names the stream and the offset of a
/// source record ([`Error::stream_location`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct RemapAddress {
    /// The kind of the record.
    pub item: RemapItem,
    /// The address of the record in the address space of the PDB: the original image of a
    /// PDB with OMAP.
    pub address: PdbRva,
}

impl RemapAddress {
    /// Make a value for the record `item` at `address`.
    #[must_use]
    pub const fn new(item: RemapItem, address: PdbRva) -> Self {
        Self { item, address }
    }
}

/// Return `true` if one section of `sections` holds all RVAs from `start` to `end`.
///
/// A section holds the RVAs from its virtual address to its virtual address plus its
/// virtual size.
fn section_holds(sections: SectionHeaders<'_>, start: u32, end: u32) -> bool {
    sections.iter().any(|header| {
        let section_end = u64::from(header.virtual_address) + u64::from(header.virtual_size);
        header.virtual_address <= start && u64::from(end) <= section_end
    })
}

/// Return `true` if the range `start .. end` overlaps a range of `ranges`.
///
/// `ranges` maps the start of each range to the range. `range_end` gives the end of a
/// range of `ranges`.
fn overlaps(
    ranges: &BTreeMap<u32, RemapRange>,
    start: u32,
    end: u32,
    range_end: impl Fn(&RemapRange) -> Option<u32>,
) -> bool {
    let before = ranges
        .range(..=start)
        .next_back()
        .and_then(|(_, other)| range_end(other))
        .is_some_and(|other_end| other_end > start);
    let after = ranges
        .range(start..)
        .next()
        .is_some_and(|(&other_start, _)| other_start < end);
    before || after
}
