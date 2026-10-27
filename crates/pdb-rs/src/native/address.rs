//! Translation between section-relative addresses and RVAs.
//!
//! Symbols, section contributions and line data give an address as a 1-based
//! section number and a byte offset in that section. The section headers give
//! the RVA and the virtual size of each section. [`AddressMap`] converts
//! between the two forms.
//!
//! # OMAP
//!
//! A tool that rewrites an image after the link moves code and data. The PDB
//! then keeps the addresses of the original image: a PDB address is a
//! [`PdbRva`] or a [`SectionOffset`] in the original sections. The image uses
//! other addresses: an image address is an [`Rva`]. The DBI optional debug
//! header names the streams that connect the two address spaces:
//!
//! | Slot | Stream | Use |
//! | --- | --- | --- |
//! | [`DebugStream::SectionHeader`] | Image section headers | [`Rva`] to and from image sections |
//! | [`DebugStream::OriginalSectionHeader`] | Original section headers | [`SectionOffset`] to and from [`PdbRva`] |
//! | [`DebugStream::OmapFromSource`] | [`OmapStream`] | [`PdbRva`] to [`Rva`] |
//! | [`DebugStream::OmapToSource`] | [`OmapStream`] | [`Rva`] to [`PdbRva`] |
//!
//! [`OmapStream::lookup`] gives the OMAP semantics. An address without a
//! mapping gives [`Translation::Unmapped`] with an [`Unmapped`] reason. The
//! map does not return the input address as a fallback.
//!
//! Without OMAP, a PDB address is an image address. The image section headers
//! then resolve section offsets, and a [`PdbRva`] has the value of its [`Rva`].
//!
//! [`OmapStream`]: super::debug_stream::OmapStream
//! [`OmapStream::lookup`]: super::debug_stream::OmapStream::lookup

use super::dbi::{Dbi, DebugStream};
use super::debug_stream::{OmapRecord, OmapStream};
use super::section::{SectionHeader, SectionHeaderStream, SectionHeaders};
use crate::{Error, ErrorKind, Result};

/// A relative virtual address in the image: a byte offset from the image base.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Rva(pub u32);

/// A relative virtual address in the original image that the PDB describes.
///
/// Without OMAP, the original image is the image.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PdbRva(pub u32);

/// An address as a section number and a byte offset in the section.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SectionOffset {
    /// The 1-based section number. Symbols can use 0 for an absolute or
    /// discarded address; such an address has no RVA.
    pub section: u16,
    /// The byte offset from the start of the section.
    pub offset: u32,
}

/// The result of an address translation through OMAP.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Translation<T> {
    /// The address has a mapping.
    Mapped(T),
    /// The address has no mapping.
    Unmapped(Unmapped),
}

impl<T> Translation<T> {
    /// Return the mapped address, or `None` if the address has no mapping.
    #[must_use]
    pub fn mapped(self) -> Option<T> {
        match self {
            Self::Mapped(address) => Some(address),
            Self::Unmapped(_) => None,
        }
    }

    /// Apply `map` to a mapped address.
    #[must_use]
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> Translation<U> {
        match self {
            Self::Mapped(address) => Translation::Mapped(map(address)),
            Self::Unmapped(reason) => Translation::Unmapped(reason),
        }
    }
}

/// The reason why an OMAP table does not map an address.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Unmapped {
    /// The address is before the first record, or the table has no records.
    BeforeFirstRecord,
    /// The record that holds the address has `rvaTo == 0`. The rewrite
    /// removed the address range of this record.
    NoTarget(OmapRecord),
    /// The address is after the `rva` of the last record. DIA maps only the
    /// `rva` of the last record.
    AfterLastRecord(OmapRecord),
}

/// The address remapping state of an [`AddressMap`].
#[derive(Clone, Copy, Debug)]
enum Remap<'a> {
    /// The PDB has no OMAP: PDB addresses are image addresses.
    Direct,
    /// The DBI stream names OMAP streams, but the map has no tables.
    NotLoaded,
    /// The OMAP tables and the original section headers.
    Omap {
        /// The original section headers (slot 10), if the PDB has them.
        original: Option<SectionHeaders<'a>>,
        /// The image to original table (slot 3), if the PDB has it.
        to_source: Option<&'a OmapStream>,
        /// The original to image table (slot 4), if the PDB has it.
        from_source: Option<&'a OmapStream>,
    },
}

/// Address translation with the section headers and OMAP tables of one PDB.
#[derive(Clone, Copy, Debug)]
pub struct AddressMap<'a> {
    /// The image section headers.
    sections: SectionHeaders<'a>,
    /// The address remapping.
    remap: Remap<'a>,
}

impl<'a> AddressMap<'a> {
    /// Create a map from the image section headers without OMAP tables.
    ///
    /// Use the stream in the [`DebugStream::SectionHeader`] slot of `dbi`
    /// for `sections`. If `dbi` names an OMAP stream in the
    /// [`DebugStream::OmapToSource`] or [`DebugStream::OmapFromSource`] slot,
    /// the translation functions return [`ErrorKind::MissingStream`]. For such
    /// a PDB, use [`AddressMap::with_omap`] or [`AddressTables::map`].
    #[must_use]
    pub fn new(sections: SectionHeaders<'a>, dbi: &Dbi) -> Self {
        let remapped = dbi.debug_stream(DebugStream::OmapToSource).is_some()
            || dbi.debug_stream(DebugStream::OmapFromSource).is_some();
        Self {
            sections,
            remap: if remapped {
                Remap::NotLoaded
            } else {
                Remap::Direct
            },
        }
    }

    /// Create a map for a PDB with OMAP.
    ///
    /// - `sections` are the image section headers ([`DebugStream::SectionHeader`]).
    /// - `original` are the original section headers ([`DebugStream::OriginalSectionHeader`]).
    /// - `to_source` is the [`DebugStream::OmapToSource`] table.
    /// - `from_source` is the [`DebugStream::OmapFromSource`] table.
    ///
    /// A translation that needs an absent part returns [`ErrorKind::MissingStream`].
    #[must_use]
    pub const fn with_omap(
        sections: SectionHeaders<'a>,
        original: Option<SectionHeaders<'a>>,
        to_source: Option<&'a OmapStream>,
        from_source: Option<&'a OmapStream>,
    ) -> Self {
        Self {
            sections,
            remap: Remap::Omap {
                original,
                to_source,
                from_source,
            },
        }
    }

    /// Return the image section headers.
    #[must_use]
    pub const fn sections(&self) -> SectionHeaders<'a> {
        self.sections
    }

    /// Return `true` if the PDB addresses need OMAP translation.
    #[must_use]
    pub const fn is_remapped(&self) -> bool {
        !matches!(self.remap, Remap::Direct)
    }

    /// Convert a PDB section offset to an image RVA.
    ///
    /// The function converts the section offset to a [`PdbRva`] with
    /// [`AddressMap::pdb_rva`], then to an [`Rva`] with [`AddressMap::pdb_rva_to_rva`].
    ///
    /// # Errors
    ///
    /// Returns the errors of [`AddressMap::pdb_rva`] and [`AddressMap::pdb_rva_to_rva`].
    pub fn rva(&self, address: SectionOffset) -> Result<Translation<Rva>> {
        let pdb_rva = self.pdb_rva(address)?;
        self.pdb_rva_to_rva(pdb_rva)
    }

    /// Convert an image RVA to a PDB section offset.
    ///
    /// The function converts the RVA to a [`PdbRva`] with
    /// [`AddressMap::rva_to_pdb_rva`], then to a section offset with
    /// [`AddressMap::pdb_section_offset`].
    ///
    /// # Errors
    ///
    /// Returns the errors of [`AddressMap::rva_to_pdb_rva`] and
    /// [`AddressMap::pdb_section_offset`].
    pub fn section_offset(&self, rva: Rva) -> Result<Translation<SectionOffset>> {
        match self.rva_to_pdb_rva(rva)? {
            Translation::Mapped(pdb_rva) => {
                self.pdb_section_offset(pdb_rva).map(Translation::Mapped)
            }
            Translation::Unmapped(reason) => Ok(Translation::Unmapped(reason)),
        }
    }

    /// Convert a PDB section offset to a PDB RVA.
    ///
    /// The offset can be at most the virtual size of the section. An offset
    /// equal to the virtual size is the end of the section: linkers give it to
    /// end markers such as `__enc$textbss$end` and to empty COFF groups.
    ///
    /// Section 0 and section numbers past the last header do not name a
    /// section. For example, linkers give absolute symbols such as
    /// `__ImageBase` section 0, or the number of the absolute segment
    /// after the last section in the DBI section map.
    ///
    /// With OMAP, the original section headers resolve the section. Without
    /// OMAP, the image section headers resolve it.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the PDB has OMAP and the map has no
    ///   OMAP tables or no original section headers.
    /// - [`ErrorKind::InvalidReference`] if no section header has the
    ///   section number, or if the offset is greater than the virtual size.
    /// - [`ErrorKind::OffsetOverflow`] if the RVA does not fit in 32 bits.
    pub fn pdb_rva(&self, address: SectionOffset) -> Result<PdbRva> {
        section_rva(self.pdb_sections()?, address).map(PdbRva)
    }

    /// Convert a PDB RVA to a PDB section offset.
    ///
    /// Return the first section in header order that holds the RVA. The end
    /// of a section does not belong to it. Only the first 65535 headers have
    /// a section number. The function uses the same headers as [`AddressMap::pdb_rva`].
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the PDB has OMAP and the map has no
    ///   OMAP tables or no original section headers.
    /// - [`ErrorKind::InvalidReference`] if no section holds the RVA.
    pub fn pdb_section_offset(&self, rva: PdbRva) -> Result<SectionOffset> {
        rva_section(self.pdb_sections()?, rva.0)
    }

    /// Convert a PDB RVA to an image RVA with the [`DebugStream::OmapFromSource`] table.
    ///
    /// Without OMAP, the result is the same value.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the PDB has OMAP and the map has no
    ///   [`DebugStream::OmapFromSource`] table.
    /// - [`ErrorKind::OffsetOverflow`] if the mapped RVA does not fit in 32 bits.
    pub fn pdb_rva_to_rva(&self, rva: PdbRva) -> Result<Translation<Rva>> {
        match self.remap {
            Remap::Direct => Ok(Translation::Mapped(Rva(rva.0))),
            Remap::NotLoaded => Err(not_loaded()),
            Remap::Omap { from_source, .. } => Ok(from_source
                .ok_or_else(|| missing("the OMAP from source table is missing"))?
                .lookup(rva.0)?
                .map(Rva)),
        }
    }

    /// Convert an image RVA to a PDB RVA with the [`DebugStream::OmapToSource`] table.
    ///
    /// Without OMAP, the result is the same value.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the PDB has OMAP and the map has no
    ///   [`DebugStream::OmapToSource`] table.
    /// - [`ErrorKind::OffsetOverflow`] if the mapped RVA does not fit in 32 bits.
    pub fn rva_to_pdb_rva(&self, rva: Rva) -> Result<Translation<PdbRva>> {
        match self.remap {
            Remap::Direct => Ok(Translation::Mapped(PdbRva(rva.0))),
            Remap::NotLoaded => Err(not_loaded()),
            Remap::Omap { to_source, .. } => Ok(to_source
                .ok_or_else(|| missing("the OMAP to source table is missing"))?
                .lookup(rva.0)?
                .map(PdbRva)),
        }
    }

    /// Convert an image section offset to an image RVA with the image section headers.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if no image section header has the
    ///   section number, or if the offset is greater than the virtual size.
    /// - [`ErrorKind::OffsetOverflow`] if the RVA does not fit in 32 bits.
    pub fn image_section_rva(&self, address: SectionOffset) -> Result<Rva> {
        section_rva(self.sections, address).map(Rva)
    }

    /// Convert an image RVA to an image section offset with the image section headers.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if no image section holds the RVA.
    pub fn image_section_offset(&self, rva: Rva) -> Result<SectionOffset> {
        rva_section(self.sections, rva.0)
    }

    /// Return the section headers of PDB addresses.
    fn pdb_sections(&self) -> Result<SectionHeaders<'a>> {
        match self.remap {
            Remap::Direct => Ok(self.sections),
            Remap::NotLoaded => Err(not_loaded()),
            Remap::Omap { original, .. } => {
                original.ok_or_else(|| missing("the original section headers are missing"))
            }
        }
    }
}

/// The owned section headers and OMAP tables of one PDB.
///
/// [`Pdb::address_tables`] loads them. [`AddressTables::map`] gives the
/// [`AddressMap`].
///
/// [`Pdb::address_tables`]: crate::native::Pdb::address_tables
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct AddressTables {
    /// The image section headers ([`DebugStream::SectionHeader`]).
    pub sections: SectionHeaderStream,
    /// The original section headers ([`DebugStream::OriginalSectionHeader`]), if present.
    pub original: Option<SectionHeaderStream>,
    /// The image to original table ([`DebugStream::OmapToSource`]), if present.
    pub to_source: Option<OmapStream>,
    /// The original to image table ([`DebugStream::OmapFromSource`]), if present.
    pub from_source: Option<OmapStream>,
}

impl AddressTables {
    /// Return tables with the image section headers `sections` and no original section
    /// headers or OMAP tables.
    #[must_use]
    pub const fn new(sections: SectionHeaderStream) -> Self {
        Self {
            sections,
            original: None,
            to_source: None,
            from_source: None,
        }
    }

    /// Return the address map of the tables.
    ///
    /// If no OMAP table is present, the map has no OMAP and ignores the
    /// original section headers. Otherwise the map uses [`AddressMap::with_omap`].
    #[must_use]
    pub fn map(&self) -> AddressMap<'_> {
        let sections = self.sections.headers();
        if self.to_source.is_none() && self.from_source.is_none() {
            return AddressMap {
                sections,
                remap: Remap::Direct,
            };
        }
        AddressMap::with_omap(
            sections,
            self.original.as_ref().map(SectionHeaderStream::headers),
            self.to_source.as_ref(),
            self.from_source.as_ref(),
        )
    }
}

/// Convert a section offset to an RVA with `sections`.
fn section_rva(sections: SectionHeaders<'_>, address: SectionOffset) -> Result<u32> {
    if address.section == 0 {
        return Err(Error::new(ErrorKind::InvalidReference, None)
            .with_context("section number 0 does not name a section"));
    }
    let header = sections.section(address.section).ok_or_else(|| {
        Error::new(ErrorKind::InvalidReference, None)
            .with_context("section number is past the last section header")
    })?;
    if address.offset > header.virtual_size {
        return Err(Error::new(ErrorKind::InvalidReference, None)
            .with_context("section offset is greater than the virtual size"));
    }
    header
        .virtual_address
        .checked_add(address.offset)
        .ok_or_else(|| {
            Error::new(ErrorKind::OffsetOverflow, None)
                .with_context("section RVA overflows 32 bits")
        })
}

/// Convert an RVA to a section offset with `sections`.
fn rva_section(sections: SectionHeaders<'_>, rva: u32) -> Result<SectionOffset> {
    sections
        .iter()
        .zip(1..=u16::MAX)
        .find_map(|(header, section)| {
            offset_in(&header, rva).map(|offset| SectionOffset { section, offset })
        })
        .ok_or_else(|| {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("RVA is not inside a section")
        })
}

/// Return the offset of `rva` in the section, if the section holds it.
fn offset_in(header: &SectionHeader, rva: u32) -> Option<u32> {
    let offset = rva.checked_sub(header.virtual_address)?;
    (offset < header.virtual_size).then_some(offset)
}

/// Return the error for a map without the OMAP tables that the DBI stream names.
fn not_loaded() -> Error {
    Error::new(ErrorKind::MissingStream, None)
        .with_context("the DBI stream names OMAP streams, but the address map has no OMAP tables")
}

/// Return the error for an absent part of an OMAP map.
fn missing(context: &'static str) -> Error {
    Error::new(ErrorKind::MissingStream, None).with_context(context)
}
