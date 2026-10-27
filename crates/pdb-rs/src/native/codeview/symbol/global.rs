//! Public, procedure reference, section and COFF group records.
//!
//! The layouts follow `PublicSym32`, `ProcRefSym`, `SectionSym` and
//! `CoffGroupSym` in LLVM `SymbolRecordMapping.cpp`. The reserved byte of
//! `S_SECTION` comes from `SECTIONSYM` in `cvinfo.h`.

use super::super::index::{ModuleIndex, ModuleSymbolHandle, ModuleSymbolOffset};
use super::super::{Fields, SymbolKind};
use super::SectionOffset;
use crate::bytes::into_owned;
use crate::native::output::Output;
use crate::{Bytes, Error, ErrorKind, Result};

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
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Public<'a> {
    /// The flags.
    pub flags: PublicFlags,
    /// The address.
    pub address: SectionOffset,
    /// The name. It is usually a mangled linker name.
    pub name: Bytes<'a>,
}

into_owned!(struct Public { name; flags, address });

impl<'a> Public<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let flags = PublicFlags(fields.u32()?);
        let address = SectionOffset::read(fields)?;
        let name = Bytes::new(fields.name()?);
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
        out.c_string(&self.name)
    }
}

/// The module symbol record that a reference record names.
///
/// A decoded record gives a [`ModuleSymbolTarget::Position`]. A builder
/// record can give a [`ModuleSymbolTarget::Handle`]: the write plan then
/// writes the module index and the module stream offset of the handle.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ModuleSymbolTarget {
    /// A position in the module streams of a PDB.
    Position {
        /// The index of the module in the DBI module list, or `None` if the
        /// record does not give a module. The record stores the index
        /// plus 1.
        module: Option<ModuleIndex>,
        /// The offset of the record in the module stream. The offset
        /// includes the 4-byte signature at the start of the stream.
        offset: ModuleSymbolOffset,
    },
    /// A module symbol record of a builder.
    ///
    /// Only a write plan resolves the handle. [`SymbolData::encode`](super::SymbolData::encode)
    /// gives [`ErrorKind::InvalidReference`] for it.
    Handle(ModuleSymbolHandle),
}

impl Default for ModuleSymbolTarget {
    /// Return a position without a module and with offset 0.
    fn default() -> Self {
        Self::Position {
            module: None,
            offset: ModuleSymbolOffset(0),
        }
    }
}

/// A reference to a record in a module: `S_PROCREF`, `S_LPROCREF`,
/// `S_TOKENREF`, `S_DATAREF` and `S_ANNOTATIONREF`.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ProcedureReference<'a> {
    /// The `SUC` name checksum. Linkers usually write 0.
    pub name_checksum: u32,
    /// The module symbol record that the reference names.
    pub target: ModuleSymbolTarget,
    /// The name.
    pub name: Bytes<'a>,
}

into_owned!(struct ProcedureReference { name; name_checksum, target });

impl<'a> ProcedureReference<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let name_checksum = fields.u32()?;
        let offset = ModuleSymbolOffset(fields.u32()?);
        let module = fields
            .u16()?
            .checked_sub(1)
            .map(|index| ModuleIndex(u32::from(index)));
        let name = Bytes::new(fields.name()?);
        Ok(Self {
            name_checksum,
            target: ModuleSymbolTarget::Position { module, offset },
            name,
        })
    }

    /// Write the fields.
    ///
    /// Returns [`ErrorKind::InvalidReference`] for a
    /// [`ModuleSymbolTarget::Handle`] and [`ErrorKind::LimitExceeded`] if the
    /// module number does not fit in 16 bits.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        let ModuleSymbolTarget::Position { module, offset } = self.target else {
            return Err(Error::new(ErrorKind::InvalidReference, None).with_context(
                "reference target is a module symbol handle; only a write plan resolves it",
            ));
        };
        let module = match module {
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
        out.u32(offset.0)?;
        out.u16(module)?;
        out.c_string(&self.name)
    }
}

/// An image section record (`S_SECTION`).
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
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
    pub name: Bytes<'a>,
}

into_owned!(struct Section { name; number, alignment, rva, length, characteristics });

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
        let name = Bytes::new(fields.name()?);
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
        out.c_string(&self.name)
    }
}

/// A COFF group record (`S_COFFGROUP`).
///
/// A COFF group is a part of an image section, for example `.text$mn`.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct CoffGroup<'a> {
    /// The length of the group in bytes.
    pub length: u32,
    /// The COFF section characteristics (`IMAGE_SCN_*`).
    pub characteristics: u32,
    /// The start address.
    pub address: SectionOffset,
    /// The group name.
    pub name: Bytes<'a>,
}

into_owned!(struct CoffGroup { name; length, characteristics, address });

impl<'a> CoffGroup<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let length = fields.u32()?;
        let characteristics = fields.u32()?;
        let address = SectionOffset::read(fields)?;
        let name = Bytes::new(fields.name()?);
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
        out.c_string(&self.name)
    }
}
