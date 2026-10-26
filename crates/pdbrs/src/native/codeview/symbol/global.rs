//! Public, procedure reference, section and COFF group records.
//!
//! The layouts follow `PublicSym32`, `ProcRefSym`, `SectionSym` and
//! `CoffGroupSym` in LLVM `SymbolRecordMapping.cpp`. The reserved byte of
//! `S_SECTION` comes from `SECTIONSYM` in `cvinfo.h`.

use super::super::Fields;
use super::super::writer::FieldWriter;
use super::SectionOffset;
use crate::{Error, ErrorKind, Result};

/// The kind of a reference to a data symbol in a module (`S_DATAREF`).
pub const S_DATAREF: u16 = 0x1126;
/// The kind of a reference to an annotation symbol in a module (`S_ANNOTATIONREF`).
pub const S_ANNOTATIONREF: u16 = 0x1128;

flags! {
    /// The flags of a public symbol (`CV_PUBSYMFLAGS`).
    PublicFlags(u32) {
        /// The symbol is in code.
        CODE = 1 << 0;
        /// The symbol is a function.
        FUNCTION = 1 << 1;
        /// The symbol is in managed code.
        MANAGED = 1 << 2;
        /// The symbol is in MSIL code.
        MSIL = 1 << 3;
    }
}

/// A public symbol record (`S_PUB32`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Public<'a> {
    /// The flags.
    pub flags: PublicFlags,
    /// The address.
    pub address: SectionOffset,
    /// The name. It is usually a mangled linker name.
    pub name: &'a [u8],
}

impl<'a> Public<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let flags = PublicFlags(fields.u32()?);
        let address = SectionOffset::read(fields)?;
        let name = fields.name()?;
        Ok(Self {
            flags,
            address,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut FieldWriter<'_>) -> Result<()> {
        out.u32(self.flags.0)?;
        self.address.write(out)?;
        out.name(self.name)
    }
}

/// A reference to a procedure in a module (`S_PROCREF` and `S_LPROCREF`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ProcedureReference<'a> {
    /// The `SUC` name checksum. Linkers usually write 0.
    pub name_checksum: u32,
    /// The offset of the procedure record in the module stream. The offset
    /// includes the 4-byte signature at the start of the stream.
    pub offset: u32,
    /// The raw module number. It is the module index plus 1.
    pub module: u16,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> ProcedureReference<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let name_checksum = fields.u32()?;
        let offset = fields.u32()?;
        let module = fields.u16()?;
        let name = fields.name()?;
        Ok(Self {
            name_checksum,
            offset,
            module,
            name,
        })
    }

    /// Return the index of the module in the DBI module list.
    ///
    /// The index is [`ProcedureReference::module`] minus 1. The raw value 0
    /// gives `None`.
    #[must_use]
    pub const fn module_index(&self) -> Option<u16> {
        self.module.checked_sub(1)
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut FieldWriter<'_>) -> Result<()> {
        out.u32(self.name_checksum)?;
        out.u32(self.offset)?;
        out.u16(self.module)?;
        out.name(self.name)
    }
}

/// An image section record (`S_SECTION`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Section<'a> {
    /// The section number.
    pub number: u16,
    /// The alignment as a power-of-two exponent, for example 12 for 4096 bytes.
    pub alignment: u8,
    /// The relative virtual address of the section.
    pub rva: u32,
    /// The length of the section in bytes.
    pub length: u32,
    /// The COFF section characteristics (`IMAGE_SCN_*`).
    pub characteristics: u32,
    /// The section name.
    pub name: &'a [u8],
}

impl<'a> Section<'a> {
    /// Read the fields and check the reserved byte.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let number = fields.u16()?;
        let alignment = fields.u8()?;
        if fields.u8()? != 0 {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("S_SECTION reserved byte is not zero"));
        }
        let rva = fields.u32()?;
        let length = fields.u32()?;
        let characteristics = fields.u32()?;
        let name = fields.name()?;
        Ok(Self {
            number,
            alignment,
            rva,
            length,
            characteristics,
            name,
        })
    }

    /// Write the fields and a zero reserved byte.
    pub(super) fn write(&self, out: &mut FieldWriter<'_>) -> Result<()> {
        out.u16(self.number)?;
        out.u8(self.alignment)?;
        out.u8(0)?;
        out.u32(self.rva)?;
        out.u32(self.length)?;
        out.u32(self.characteristics)?;
        out.name(self.name)
    }
}

/// A COFF group record (`S_COFFGROUP`).
///
/// A COFF group is a part of an image section, for example `.text$mn`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct CoffGroup<'a> {
    /// The length of the group in bytes.
    pub length: u32,
    /// The COFF section characteristics (`IMAGE_SCN_*`).
    pub characteristics: u32,
    /// The start address.
    pub address: SectionOffset,
    /// The group name.
    pub name: &'a [u8],
}

impl<'a> CoffGroup<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let length = fields.u32()?;
        let characteristics = fields.u32()?;
        let address = SectionOffset::read(fields)?;
        let name = fields.name()?;
        Ok(Self {
            length,
            characteristics,
            address,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut FieldWriter<'_>) -> Result<()> {
        out.u32(self.length)?;
        out.u32(self.characteristics)?;
        self.address.write(out)?;
        out.name(self.name)
    }
}
