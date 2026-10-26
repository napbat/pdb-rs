//! Export, discarded, profile, module type reference and PDB map records.
//!
//! The layouts follow `EXPORTSYM`, `DISCARDEDSYM`, `POGOINFO`, `MODTYPEREF`
//! and `PDBMAP` in `cvinfo.h`. The debug type records of `cvinfo.h` in the
//! DIA SDK of Visual Studio 18 (`lib/amd64/diaguids.lib`) confirm
//! `MODTYPEREF` and `PDBMAP`, and they give `SOURCELINKSYM` as the layout
//! of `BUILDINFOSYM`: one ID index.

use super::super::Fields;
use crate::native::msf::StreamId;
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// The kind of an export record (`S_EXPORT`).
pub const S_EXPORT: u16 = 0x1138;
/// The kind of a record that the linker discarded (`S_DISCARDED`).
pub const S_DISCARDED: u16 = 0x113b;
/// The kind of a profile-guided optimization record (`S_POGODATA`).
pub const S_POGODATA: u16 = 0x115c;
/// The kind of a module type reference (`S_MOD_TYPEREF`).
pub const S_MOD_TYPEREF: u16 = 0x115f;
/// The kind of a map from one PDB file name to another (`S_PDBMAP`).
pub const S_PDBMAP: u16 = 0x1161;
/// The kind of a Source Link reference (`S_SOURCELINK`). The record holds
/// an ID index.
pub const S_SOURCELINK: u16 = 0x117e;

flags! {
    /// The flags of an export record.
    ExportFlags(u16) {
        /// The export is a constant (`CONSTANT`).
        CONSTANT = 1 << 0;
        /// The export is data (`DATA`).
        DATA = 1 << 1;
        /// The export is private (`PRIVATE`).
        PRIVATE = 1 << 2;
        /// The export has no name (`NONAME`).
        NO_NAME = 1 << 3;
        /// The ordinal was assigned explicitly.
        ORDINAL = 1 << 4;
        /// The export is a forwarder.
        FORWARDER = 1 << 5;
    }
}

/// An export record ([`S_EXPORT`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Export<'a> {
    /// The ordinal.
    pub ordinal: u16,
    /// The flags. `cvinfo.h` requires bits 6 to 15 to be zero; the value
    /// keeps them.
    pub flags: ExportFlags,
    /// The exported name.
    pub name: &'a [u8],
}

impl<'a> Export<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let ordinal = fields.u16()?;
        let flags = ExportFlags(fields.u16()?);
        let name = fields.name()?;
        Ok(Self {
            ordinal,
            flags,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u16(self.ordinal)?;
        out.u16(self.flags.0)?;
        out.c_string(self.name)
    }
}

values! {
    /// The reason for a discarded record (`CV_DISCARDED_e`).
    DiscardReason(u8) {
        /// The reason is not known.
        UNKNOWN = 0;
        /// The linker did not select the COMDAT.
        NOT_SELECTED = 1;
        /// No code referenced the record.
        NOT_REFERENCED = 2;
    }
}

/// A record that the linker discarded ([`S_DISCARDED`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Discarded<'a> {
    /// The reason in bits 0 to 7 and unused bits 8 to 31, raw.
    pub raw_reason: u32,
    /// The first file ID, if line information is present.
    pub file_id: u32,
    /// The first line number.
    pub line: u32,
    /// The original records with type indices that are not valid, with the
    /// record padding.
    pub data: &'a [u8],
}

impl<'a> Discarded<'a> {
    /// Read the fields and the original records.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let raw_reason = fields.u32()?;
        let file_id = fields.u32()?;
        let line = fields.u32()?;
        let data = fields.bytes(fields.rest().len())?;
        Ok(Self {
            raw_reason,
            file_id,
            line,
            data,
        })
    }

    /// Write the fields and the original records.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.raw_reason)?;
        out.u32(self.file_id)?;
        out.u32(self.line)?;
        out.bytes(self.data)
    }

    /// Return the reason in bits 0 to 7.
    #[must_use]
    pub const fn reason(&self) -> DiscardReason {
        DiscardReason(self.raw_reason.to_le_bytes()[0])
    }
}

/// A profile-guided optimization record ([`S_POGODATA`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProfileData {
    /// The number of calls of the function.
    pub invocations: u32,
    /// The dynamic instruction count. `cvinfo.h` gives a signed `__int64`.
    pub dynamic_instructions: i64,
    /// The static instruction count.
    pub static_instructions: u32,
    /// The static instruction count after inlining.
    pub live_instructions: u32,
}

impl ProfileData {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let invocations = fields.u32()?;
        let dynamic_instructions = fields.u64()?.cast_signed();
        let static_instructions = fields.u32()?;
        let live_instructions = fields.u32()?;
        Ok(Self {
            invocations,
            dynamic_instructions,
            static_instructions,
            live_instructions,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.invocations)?;
        out.u64(self.dynamic_instructions.cast_unsigned())?;
        out.u32(self.static_instructions)?;
        out.u32(self.live_instructions)
    }
}

flags! {
    /// The flags of a module type reference.
    ModuleTypeReferenceFlags(u32) {
        /// The module does not refer to types.
        NONE = 1 << 0;
        /// The module refers to precompiled types of `/Z7`.
        REFERS_TO_PRECOMPILED_TYPES = 1 << 1;
        /// The module holds precompiled types of `/Z7`.
        OWNS_PRECOMPILED_TYPES = 1 << 2;
        /// The module holds type information of `/Z7`.
        OWNS_TYPE_RECORDS = 1 << 3;
        /// The module holds type information of `/Zi` or `/ZI`.
        OWNS_TYPE_MAP = 1 << 4;
        /// The module refers to type information of another module.
        REFERS_TO_TYPE_MAP = 1 << 5;
    }
}

/// A module type reference ([`S_MOD_TYPEREF`]).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ModuleTypeReference {
    /// The flags. Bits 6 to 14 are reserved; the value keeps them.
    pub flags: ModuleTypeReferenceFlags,
    /// The first word: a stream number or a module index, as the flags select.
    pub word0: u16,
    /// The second word: a stream number or a module index, as the flags select.
    pub word1: u16,
}

/// Where the type records of a module are, as an [`S_MOD_TYPEREF`] record
/// tells.
///
/// MSVC `link /DEBUG:LAZY` writes these records in a PDB without type
/// merging (feature `NOTM`). The type and ID indices of the module symbols
/// are then indices of the input object file, not TPI or IPI indices. The
/// meaning of the words comes from `msvc-x64-lazy` (#30): see
/// [`ModuleTypeReference::source`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ModuleTypeSource {
    /// The module refers to no types (`fNone`).
    NoTypes,
    /// The module holds the `.debug$T` records of a `/Z7` object (`fOwnTMR`).
    /// The stream holds the records without the signature. Types and IDs
    /// share one index space from `0x1000`.
    TypeRecords {
        /// The stream with the records (`word0`).
        stream: StreamId,
    },
    /// The module holds the records of a `/Zi` type server (`fOwnTM`). Each
    /// stream starts at index `0x1000`.
    TypeServer {
        /// The stream with the type records (`word0`).
        types: StreamId,
        /// The stream with the ID records (`word1`).
        ids: StreamId,
    },
    /// The module uses the records of another module (`fRefTM`).
    OtherModule {
        /// The index of the module that holds the records (`word0`, 0-based).
        module: u16,
    },
    /// Another combination of flags, for example the precompiled-type flags
    /// `fRefTMPCT` and `fOwnTMPCT`. No fixture shows their words.
    Other(ModuleTypeReference),
}

impl ModuleTypeReference {
    /// Return where the type records of the module are.
    ///
    /// The function uses only the flags of bits 0 to 5. Exactly one of
    /// `fNone`, `fOwnTMR`, `fOwnTM` and `fRefTM` must be set, and no
    /// precompiled-type flag; else the result is [`ModuleTypeSource::Other`].
    #[must_use]
    pub fn source(&self) -> ModuleTypeSource {
        let stream = |word: u16| StreamId(u32::from(word));
        match self.flags.0 & 0x3f {
            0x01 => ModuleTypeSource::NoTypes,
            0x08 => ModuleTypeSource::TypeRecords {
                stream: stream(self.word0),
            },
            0x10 => ModuleTypeSource::TypeServer {
                types: stream(self.word0),
                ids: stream(self.word1),
            },
            0x20 => ModuleTypeSource::OtherModule { module: self.word0 },
            _ => ModuleTypeSource::Other(*self),
        }
    }

    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let flags = ModuleTypeReferenceFlags(fields.u32()?);
        let word0 = fields.u16()?;
        let word1 = fields.u16()?;
        Ok(Self {
            flags,
            word0,
            word1,
        })
    }

    /// Write the fields.
    pub(super) fn write(self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.flags.0)?;
        out.u16(self.word0)?;
        out.u16(self.word1)
    }
}

/// A map from one PDB file name to another ([`S_PDBMAP`]).
///
/// Both names are UTF-16 strings with a NUL code unit at the end. The values
/// hold the little-endian code units without the terminator.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdbMap<'a> {
    /// The source PDB file name.
    pub from: &'a [u8],
    /// The destination PDB file name.
    pub to: &'a [u8],
}

impl<'a> PdbMap<'a> {
    /// Read the two names.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let from = read_utf16(fields)?;
        let to = read_utf16(fields)?;
        Ok(Self { from, to })
    }

    /// Write the two names and their terminators.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        write_utf16(out, self.from)?;
        write_utf16(out, self.to)
    }
}

/// Read UTF-16 code units up to a NUL code unit and consume the terminator.
fn read_utf16<'a>(fields: &mut Fields<'a>) -> Result<&'a [u8]> {
    let end = fields
        .rest()
        .as_chunks::<2>()
        .0
        .iter()
        .position(|unit| *unit == [0, 0])
        .ok_or_else(|| {
            Error::new(ErrorKind::InvalidFormat, None)
                .with_context("S_PDBMAP name has no NUL code unit")
        })?;
    let name = fields.bytes(end * 2)?;
    fields.bytes(2)?;
    Ok(name)
}

/// Write UTF-16 code units and a NUL code unit.
fn write_utf16(out: &mut Output<'_>, units: &[u8]) -> Result<()> {
    let (whole, rest) = units.as_chunks::<2>();
    if !rest.is_empty() || whole.contains(&[0, 0]) {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("S_PDBMAP name is not UTF-16 code units without NUL"));
    }
    out.bytes(units)?;
    out.u16(0)
}
