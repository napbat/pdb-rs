//! Frame-relative, register and file-static variable records.
//!
//! The layouts follow `BPRelativeSym`, `RegisterSym` and `FileStaticSym` in
//! LLVM `SymbolRecordMapping.cpp` and `BPRELSYM32`, `REGSYM` and
//! `FILESTATICSYM` in `cvinfo.h`.
//!
//! The indirect kinds and the `_ENCTMP` kinds of `S_BPREL32` are newer than
//! the pinned `cvinfo.h`. LLVM gives `RegRelativeIndirSym` for
//! `S_REGREL32_INDIR`. The debug type records of `cvinfo.h` in the DIA SDK
//! of Visual Studio 18 (`lib/amd64/diaguids.lib`) give `BPRELSYM32INDIR`
//! and `REGREL32INDIR`, and the type names `BPREL32ENCTMP`,
//! `BPREL32INDIRENCTMP` and `REGREL32INDIRENCTMP` for the layouts of
//! `BPRELSYM32`, `BPRELSYM32INDIR` and `REGREL32INDIR`.

use super::super::Fields;
use super::super::index::TypeIndex;
use super::LocalFlags;
use crate::Result;
use crate::native::output::Output;

/// The kind of an indirect frame-relative variable (`S_BPREL32_INDIR`).
pub const S_BPREL32_INDIR: u16 = 0x1170;
/// The kind of an indirect register-relative variable (`S_REGREL32_INDIR`).
pub const S_REGREL32_INDIR: u16 = 0x1171;
/// The kind of a frame-relative compiler temporary for Edit and Continue
/// (`S_BPREL32_ENCTMP`).
pub const S_BPREL32_ENCTMP: u16 = 0x1178;
/// The kind of an indirect frame-relative compiler temporary for Edit and
/// Continue (`S_BPREL32_INDIR_ENCTMP`).
pub const S_BPREL32_INDIR_ENCTMP: u16 = 0x117a;
/// The kind of an indirect register-relative compiler temporary for Edit
/// and Continue (`S_REGREL32_INDIR_ENCTMP`).
pub const S_REGREL32_INDIR_ENCTMP: u16 = 0x117b;

/// A variable relative to the frame pointer (`S_BPREL32` and [`S_BPREL32_ENCTMP`]).
///
/// x86 producers write this record for locals and parameters relative to
/// `EBP`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BasePointerRelative<'a> {
    /// The offset from the frame pointer. `cvinfo.h` gives a signed
    /// `CV_off32_t`.
    pub offset: i32,
    /// The type index in the TPI stream.
    pub type_index: TypeIndex,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> BasePointerRelative<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let offset = fields.i32()?;
        let type_index = fields.type_index()?;
        let name = fields.name()?;
        Ok(Self {
            offset,
            type_index,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.i32(self.offset)?;
        out.type_index(self.type_index)?;
        out.c_string(self.name)
    }
}

/// A variable in a register (`S_REGISTER`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RegisterVariable<'a> {
    /// The type index in the TPI stream.
    pub type_index: TypeIndex,
    /// The register, as a `CV_HREG_e` number for the CPU of the module.
    pub register: u16,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> RegisterVariable<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let type_index = fields.type_index()?;
        let register = fields.u16()?;
        let name = fields.name()?;
        Ok(Self {
            type_index,
            register,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.type_index(self.type_index)?;
        out.u16(self.register)?;
        out.c_string(self.name)
    }
}

/// A static variable of a source file (`S_FILESTATIC`).
///
/// The def-range records that follow give the location of the variable.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FileStatic<'a> {
    /// The type index in the TPI stream.
    pub type_index: TypeIndex,
    /// The byte offset of the module file name in the `/names` stream.
    pub module_name_offset: u32,
    /// The flags.
    pub flags: LocalFlags,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> FileStatic<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let type_index = fields.type_index()?;
        let module_name_offset = fields.u32()?;
        let flags = LocalFlags(fields.u16()?);
        let name = fields.name()?;
        Ok(Self {
            type_index,
            module_name_offset,
            flags,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.type_index(self.type_index)?;
        out.u32(self.module_name_offset)?;
        out.u16(self.flags.0)?;
        out.c_string(self.name)
    }
}

/// An indirect variable relative to the frame pointer ([`S_BPREL32_INDIR`]
/// and [`S_BPREL32_INDIR_ENCTMP`]).
///
/// The variable is at `*(frame + offset) + offset_in_udt`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BasePointerRelativeIndirect<'a> {
    /// The offset of the pointer from the frame pointer (`off`, a `long`).
    pub offset: i32,
    /// The type index in the TPI stream.
    pub type_index: TypeIndex,
    /// The offset to add after the pointer is read (`fieldoff`, a `long`).
    pub offset_in_udt: i32,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> BasePointerRelativeIndirect<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let offset = fields.i32()?;
        let type_index = fields.type_index()?;
        let offset_in_udt = fields.i32()?;
        let name = fields.name()?;
        Ok(Self {
            offset,
            type_index,
            offset_in_udt,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.i32(self.offset)?;
        out.type_index(self.type_index)?;
        out.i32(self.offset_in_udt)?;
        out.c_string(self.name)
    }
}

/// An indirect register-relative variable ([`S_REGREL32_INDIR`] and
/// [`S_REGREL32_INDIR_ENCTMP`]).
///
/// The variable is at `*(register + offset) + offset_in_udt`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RegisterRelativeIndirect<'a> {
    /// The offset of the pointer from the register.
    pub offset: u32,
    /// The type index in the TPI stream.
    pub type_index: TypeIndex,
    /// The offset to add after the pointer is read.
    pub offset_in_udt: u32,
    /// The base register, as a `CV_HREG_e` number for the CPU of the module.
    pub register: u16,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> RegisterRelativeIndirect<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let offset = fields.u32()?;
        let type_index = fields.type_index()?;
        let offset_in_udt = fields.u32()?;
        let register = fields.u16()?;
        let name = fields.name()?;
        Ok(Self {
            offset,
            type_index,
            offset_in_udt,
            register,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.offset)?;
        out.type_index(self.type_index)?;
        out.u32(self.offset_in_udt)?;
        out.u16(self.register)?;
        out.c_string(self.name)
    }
}
