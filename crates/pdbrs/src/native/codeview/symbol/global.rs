//! Public, procedure reference, section and COFF group records.
//!
//! The layouts follow `PublicSym32`, `ProcRefSym`, `SectionSym` and
//! `CoffGroupSym` in LLVM `SymbolRecordMapping.cpp`. The reserved byte of
//! `S_SECTION` comes from `SECTIONSYM` in `cvinfo.h`.

use super::super::index::{ModuleIndex, ModuleSymbolOffset};
use super::super::{Fields, SymbolKind};
use super::SectionOffset;
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// The kind of a reference to a data symbol in a module (`S_DATAREF`).
pub const S_DATAREF: SymbolKind = SymbolKind(0x1126);
/// The kind of a reference to an annotation symbol in a module (`S_ANNOTATIONREF`).
pub const S_ANNOTATIONREF: SymbolKind = SymbolKind(0x1128);

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
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
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
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.flags.0)?;
        self.address.write(out)?;
        out.c_string(self.name)
    }
}

/// A reference to a procedure in a module (`S_PROCREF` and `S_LPROCREF`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ProcedureReference<'a> {
    /// The `SUC` name checksum. Linkers usually write 0.
    pub name_checksum: u32,
    /// The offset of the procedure record in the module stream. The offset
    /// includes the 4-byte signature at the start of the stream.
    pub offset: ModuleSymbolOffset,
    /// The index of the module in the DBI module list, or `None` if the
    /// record does not give a module. The record stores the index plus 1.
    pub module: Option<ModuleIndex>,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> ProcedureReference<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let name_checksum = fields.u32()?;
        let offset = ModuleSymbolOffset(fields.u32()?);
        let module = fields
            .u16()?
            .checked_sub(1)
            .map(|index| ModuleIndex(u32::from(index)));
        let name = fields.name()?;
        Ok(Self {
            name_checksum,
            offset,
            module,
            name,
        })
    }

    /// Write the fields.
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the module number does not fit
    /// in 16 bits.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        let module = match self.module {
            None => 0,
            Some(index) => index
                .0
                .checked_add(1)
                .and_then(|number| u16::try_from(number).ok())
                .ok_or_else(|| {
                    Error::new(ErrorKind::LimitExceeded, None)
                        .with_context("procedure reference module does not fit in 16 bits")
                })?,
        };
        out.u32(self.name_checksum)?;
        out.u32(self.offset.0)?;
        out.u16(module)?;
        out.c_string(self.name)
    }
}

/// An image section record (`S_SECTION`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
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
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u16(self.number)?;
        out.u8(self.alignment)?;
        out.u8(0)?;
        out.u32(self.rva)?;
        out.u32(self.length)?;
        out.u32(self.characteristics)?;
        out.c_string(self.name)
    }
}

/// A COFF group record (`S_COFFGROUP`).
///
/// A COFF group is a part of an image section, for example `.text$mn`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
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
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.length)?;
        out.u32(self.characteristics)?;
        self.address.write(out)?;
        out.c_string(self.name)
    }
}
