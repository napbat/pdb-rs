//! Procedure start records of the MIPS and IA64 architectures.
//!
//! The layouts follow `PROCSYMMIPS` and `PROCSYMIA64` in `cvinfo.h`. The
//! `_ID` kinds use the same layouts with an ID index in place of the type
//! index, as `S_GPROC32_ID` does for `PROCSYM32`.

use super::super::index::ModuleSymbolOffset;
use super::super::{Fields, SymbolKind};
use super::{ProcedureFlags, ProcedureType, SectionOffset};
use crate::Result;
use crate::native::output::Output;

/// The kind of a module-local MIPS procedure start (`S_LPROCMIPS`).
pub const S_LPROCMIPS: SymbolKind = SymbolKind(0x1114);
/// The kind of a global MIPS procedure start (`S_GPROCMIPS`).
pub const S_GPROCMIPS: SymbolKind = SymbolKind(0x1115);
/// The kind of a module-local IA64 procedure start (`S_LPROCIA64`).
pub const S_LPROCIA64: SymbolKind = SymbolKind(0x1118);
/// The kind of a global IA64 procedure start (`S_GPROCIA64`).
pub const S_GPROCIA64: SymbolKind = SymbolKind(0x1119);
/// The kind of a module-local MIPS procedure start with an ID index (`S_LPROCMIPS_ID`).
pub const S_LPROCMIPS_ID: SymbolKind = SymbolKind(0x1148);
/// The kind of a global MIPS procedure start with an ID index (`S_GPROCMIPS_ID`).
pub const S_GPROCMIPS_ID: SymbolKind = SymbolKind(0x1149);
/// The kind of a module-local IA64 procedure start with an ID index (`S_LPROCIA64_ID`).
pub const S_LPROCIA64_ID: SymbolKind = SymbolKind(0x114a);
/// The kind of a global IA64 procedure start with an ID index (`S_GPROCIA64_ID`).
pub const S_GPROCIA64_ID: SymbolKind = SymbolKind(0x114b);

/// The links and the code range that all procedure records start with.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ProcedureHeader {
    /// The module stream offset of the enclosing scope record, or 0.
    pub parent: ModuleSymbolOffset,
    /// The module stream offset of the record that ends this scope.
    pub end: ModuleSymbolOffset,
    /// The module stream offset of the next procedure record, or 0.
    pub next: ModuleSymbolOffset,
    /// The length of the procedure code in bytes.
    pub code_length: u32,
    /// The offset of the end of the prologue, relative to the procedure start.
    pub debug_start: u32,
    /// The offset of the start of the epilogue, relative to the procedure start.
    pub debug_end: u32,
}

impl ProcedureHeader {
    /// A header with all fields zero.
    const ZERO: Self = Self {
        parent: ModuleSymbolOffset(0),
        end: ModuleSymbolOffset(0),
        next: ModuleSymbolOffset(0),
        code_length: 0,
        debug_start: 0,
        debug_end: 0,
    };

    /// Read the six `u32` fields.
    fn read(fields: &mut Fields<'_>) -> Result<Self> {
        Ok(Self {
            parent: ModuleSymbolOffset(fields.u32()?),
            end: ModuleSymbolOffset(fields.u32()?),
            next: ModuleSymbolOffset(fields.u32()?),
            code_length: fields.u32()?,
            debug_start: fields.u32()?,
            debug_end: fields.u32()?,
        })
    }

    /// Write the six `u32` fields.
    fn write(self, out: &mut Output<'_>) -> Result<()> {
        for value in [
            self.parent.0,
            self.end.0,
            self.next.0,
            self.code_length,
            self.debug_start,
            self.debug_end,
        ] {
            out.u32(value)?;
        }
        Ok(())
    }
}

/// Read the type index, or the ID index if `id` is set.
fn read_signature(fields: &mut Fields<'_>, id: bool) -> Result<ProcedureType> {
    Ok(if id {
        ProcedureType::Id(fields.id_index()?)
    } else {
        ProcedureType::Type(fields.type_index()?)
    })
}

/// A MIPS procedure start record ([`S_GPROCMIPS`], [`S_LPROCMIPS`] and their
/// `_ID` kinds).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct MipsProcedure<'a> {
    /// The links and the code range.
    pub header: ProcedureHeader,
    /// The save mask of the integer registers.
    pub integer_save_mask: u32,
    /// The save mask of the floating-point registers.
    pub float_save_mask: u32,
    /// The save offset of the integer registers.
    pub integer_save_offset: u32,
    /// The save offset of the floating-point registers.
    pub float_save_offset: u32,
    /// The procedure type or the function ID.
    pub signature: ProcedureType,
    /// The start address.
    pub address: SectionOffset,
    /// The register of the return value.
    pub return_register: u8,
    /// The frame pointer register.
    pub frame_register: u8,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> MipsProcedure<'a> {
    /// Make a record with `signature`. The other fields are zero or empty.
    #[must_use]
    pub const fn new(signature: ProcedureType) -> Self {
        Self {
            header: ProcedureHeader::ZERO,
            integer_save_mask: 0,
            float_save_mask: 0,
            integer_save_offset: 0,
            float_save_offset: 0,
            signature,
            address: SectionOffset {
                section: 0,
                offset: 0,
            },
            return_register: 0,
            frame_register: 0,
            name: &[],
        }
    }

    /// Read the fields. Set `id` for the `_ID` kinds.
    pub(super) fn read(fields: &mut Fields<'a>, id: bool) -> Result<Self> {
        let header = ProcedureHeader::read(fields)?;
        let integer_save_mask = fields.u32()?;
        let float_save_mask = fields.u32()?;
        let integer_save_offset = fields.u32()?;
        let float_save_offset = fields.u32()?;
        let signature = read_signature(fields, id)?;
        let address = SectionOffset::read(fields)?;
        let return_register = fields.u8()?;
        let frame_register = fields.u8()?;
        let name = fields.name()?;
        Ok(Self {
            header,
            integer_save_mask,
            float_save_mask,
            integer_save_offset,
            float_save_offset,
            signature,
            address,
            return_register,
            frame_register,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        self.header.write(out)?;
        out.u32(self.integer_save_mask)?;
        out.u32(self.float_save_mask)?;
        out.u32(self.integer_save_offset)?;
        out.u32(self.float_save_offset)?;
        self.signature.write(out)?;
        self.address.write(out)?;
        out.u8(self.return_register)?;
        out.u8(self.frame_register)?;
        out.c_string(self.name)
    }
}

/// An IA64 procedure start record ([`S_GPROCIA64`], [`S_LPROCIA64`] and
/// their `_ID` kinds).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Ia64Procedure<'a> {
    /// The links and the code range.
    pub header: ProcedureHeader,
    /// The procedure type or the function ID.
    pub signature: ProcedureType,
    /// The start address.
    pub address: SectionOffset,
    /// The register of the return value.
    pub return_register: u16,
    /// The flags.
    pub flags: ProcedureFlags,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> Ia64Procedure<'a> {
    /// Make a record with `signature`. The other fields are zero or empty.
    #[must_use]
    pub const fn new(signature: ProcedureType) -> Self {
        Self {
            header: ProcedureHeader::ZERO,
            signature,
            address: SectionOffset {
                section: 0,
                offset: 0,
            },
            return_register: 0,
            flags: ProcedureFlags(0),
            name: &[],
        }
    }

    /// Read the fields. Set `id` for the `_ID` kinds.
    pub(super) fn read(fields: &mut Fields<'a>, id: bool) -> Result<Self> {
        let header = ProcedureHeader::read(fields)?;
        let signature = read_signature(fields, id)?;
        let address = SectionOffset::read(fields)?;
        let return_register = fields.u16()?;
        let flags = ProcedureFlags(fields.u8()?);
        let name = fields.name()?;
        Ok(Self {
            header,
            signature,
            address,
            return_register,
            flags,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        self.header.write(out)?;
        self.signature.write(out)?;
        self.address.write(out)?;
        out.u16(self.return_register)?;
        out.u8(self.flags.0)?;
        out.c_string(self.name)
    }
}
