//! Translation between section-relative addresses and RVAs.
//!
//! Symbols, section contributions and line data give an address as a 1-based
//! section number and a byte offset in that section. The image section
//! headers give the RVA and the virtual size of each section.
//! [`AddressMap`] converts between the two forms with these headers.
//!
//! A PDB with OMAP data maps the addresses of a changed image back to the
//! original image. This crate does not decode OMAP streams. [`AddressMap`]
//! therefore refuses to translate when the DBI stream names an OMAP stream.

use super::dbi::{Dbi, DebugStream};
use super::section::{SectionHeader, SectionHeaders};
use crate::{Error, ErrorKind, Result};

/// A relative virtual address: a byte offset from the image base.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Rva(pub u32);

/// An address as a 1-based section number and a byte offset in the section.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SectionOffset {
    /// The 1-based section number.
    pub section: u16,
    /// The byte offset from the start of the section.
    pub offset: u32,
}

/// Address translation with the section headers of one image.
#[derive(Clone, Copy, Debug)]
pub struct AddressMap<'a> {
    /// The section headers of the image.
    sections: SectionHeaders<'a>,
    /// `true` if the DBI stream names an OMAP stream.
    remapped: bool,
}

impl<'a> AddressMap<'a> {
    /// Create a map from the section headers that `dbi` names.
    ///
    /// Use the stream in the [`DebugStream::SectionHeader`] slot of `dbi`
    /// for `sections`. The map records whether `dbi` names an OMAP stream
    /// in the [`DebugStream::OmapToSource`] or [`DebugStream::OmapFromSource`] slot.
    #[must_use]
    pub fn new(sections: SectionHeaders<'a>, dbi: &Dbi) -> Self {
        let remapped = dbi.debug_stream(DebugStream::OmapToSource).is_some()
            || dbi.debug_stream(DebugStream::OmapFromSource).is_some();
        Self { sections, remapped }
    }

    /// Return the section headers.
    #[must_use]
    pub const fn sections(&self) -> SectionHeaders<'a> {
        self.sections
    }

    /// Convert a section-relative address to an RVA.
    ///
    /// The offset must be less than the virtual size of the section.
    ///
    /// Section 0 and section numbers past the last header do not name an
    /// image section. For example, linkers give absolute symbols such as
    /// `__ImageBase` section 0, or the number of the absolute segment
    /// after the last section in the DBI section map.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedVersion`] if the DBI stream names an OMAP stream.
    /// - [`ErrorKind::InvalidReference`] if no section header has the
    ///   section number, or if the offset is not less than the virtual size.
    /// - [`ErrorKind::OffsetOverflow`] if the RVA does not fit in 32 bits.
    pub fn rva(&self, address: SectionOffset) -> Result<Rva> {
        self.require_direct()?;
        if address.section == 0 {
            return Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("section number 0 does not name an image section"));
        }
        let header = self.sections.section(address.section).ok_or_else(|| {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("section number is past the last section header")
        })?;
        if address.offset >= header.virtual_size {
            return Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("section offset is not less than the virtual size"));
        }
        header
            .virtual_address
            .checked_add(address.offset)
            .map(Rva)
            .ok_or_else(|| {
                Error::new(ErrorKind::OffsetOverflow, None)
                    .with_context("section RVA overflows 32 bits")
            })
    }

    /// Convert an RVA to a section-relative address.
    ///
    /// Return the first section in header order that holds the RVA. Only the
    /// first 65535 headers have a section number.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedVersion`] if the DBI stream names an OMAP stream.
    /// - [`ErrorKind::InvalidReference`] if no section holds the RVA.
    pub fn section_offset(&self, rva: Rva) -> Result<SectionOffset> {
        self.require_direct()?;
        self.sections
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

    /// Return an error if the image addresses need OMAP translation.
    fn require_direct(&self) -> Result<()> {
        if self.remapped {
            Err(Error::new(ErrorKind::UnsupportedVersion, None)
                .with_context("address translation through OMAP is not supported"))
        } else {
            Ok(())
        }
    }
}

/// Return the offset of `rva` in the section, if the section holds it.
fn offset_in(header: &SectionHeader, rva: Rva) -> Option<u32> {
    let offset = rva.0.checked_sub(header.virtual_address)?;
    (offset < header.virtual_size).then_some(offset)
}
