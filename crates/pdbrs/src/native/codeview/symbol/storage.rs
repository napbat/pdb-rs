//! Frame-relative, register and file-static variable records.
//!
//! The layouts follow `BPRelativeSym`, `RegisterSym` and `FileStaticSym` in
//! LLVM `SymbolRecordMapping.cpp` and `BPRELSYM32`, `REGSYM` and
//! `FILESTATICSYM` in `cvinfo.h`.

use super::super::Fields;
use super::super::index::TypeIndex;
use super::LocalFlags;
use crate::Result;

/// A variable relative to the frame pointer (`S_BPREL32`).
///
/// x86 producers write this record for locals and parameters relative to
/// `EBP`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
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
}

/// A variable in a register (`S_REGISTER`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
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
}

/// A static variable of a source file (`S_FILESTATIC`).
///
/// The def-range records that follow give the location of the variable.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
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
}
