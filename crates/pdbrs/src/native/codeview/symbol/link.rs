//! Records that the linker writes: trampolines and mini PDB references.
//!
//! The layout of `S_TRAMPOLINE` follows `TrampolineSym` in LLVM
//! `SymbolRecordMapping.cpp` and `TRAMPOLINESYM` in `cvinfo.h`.
//!
//! `S_REF_MINIPDB` follows `REFMINIPDB` in `cvinfo.h`. No pinned reference
//! gives a layout for `S_FASTLINK`. LLVM calls the kind `S_REF_MINIPDB2`.
//! The layout is `REFMINIPDB` of `cvinfo.h` without the module index. The
//! MSVC 14.44 FASTLINK fixture of #12 and its object files confirm it. The
//! decision is in #13. The debug type records of `cvinfo.h` in the DIA SDK
//! of Visual Studio 18 (`lib/amd64/diaguids.lib`) give `REFMINIPDB2` with
//! the same layout, and the flag bits `fThunk` (bit 5) and `fNamespace`
//! (bit 6).

use super::super::index::TypeIndex;
use super::super::{Fields, SymbolKind};
use super::SectionOffset;
use crate::bytes::into_owned;
use crate::native::output::Output;
use crate::{Bytes, Result};

/// The kind of a reference to a record in a mini PDB (`S_REF_MINIPDB`).
pub const S_REF_MINIPDB: SymbolKind = SymbolKind(0x1160);

values! {
    /// The kind of a trampoline.
    TrampolineKind(u16) {
        /// An incremental link thunk. The public thunk map has its target.
        INCREMENTAL = 0;
        /// A branch island.
        BRANCH_ISLAND = 1;
    }
}

/// A trampoline thunk record (`S_TRAMPOLINE`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Trampoline {
    /// The kind of the trampoline.
    pub kind: TrampolineKind,
    /// The size of the thunk in bytes.
    pub thunk_size: u16,
    /// The address of the thunk.
    pub thunk: SectionOffset,
    /// The address of the target of the thunk.
    pub target: SectionOffset,
}

impl Trampoline {
    /// Read the fields.
    ///
    /// The record stores both offsets before both section numbers.
    pub(super) fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let kind = TrampolineKind(fields.u16()?);
        let thunk_size = fields.u16()?;
        let thunk_offset = fields.u32()?;
        let target_offset = fields.u32()?;
        let thunk_section = fields.u16()?;
        let target_section = fields.u16()?;
        Ok(Self {
            kind,
            thunk_size,
            thunk: SectionOffset {
                section: thunk_section,
                offset: thunk_offset,
            },
            target: SectionOffset {
                section: target_section,
                offset: target_offset,
            },
        })
    }

    /// Write the fields in the order of [`Trampoline::read`].
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u16(self.kind.0)?;
        out.u16(self.thunk_size)?;
        out.u32(self.thunk.offset)?;
        out.u32(self.target.offset)?;
        out.u16(self.thunk.section)?;
        out.u16(self.target.section)
    }
}

flags! {
    /// The flags of a mini PDB reference.
    ///
    /// Bits 0 to 4 come from `REFMINIPDB` in `cvinfo.h`. Bits 5 and 6 come
    /// from `REFMINIPDB2` in the DIA SDK. The FASTLINK fixture of #12 sets
    /// bit 6 only on namespace names.
    FastLinkFlags(u16) {
        /// The name is local to the object file.
        LOCAL = 1 << 0;
        /// The name is data, not a function.
        DATA = 1 << 1;
        /// The name is a user-defined type. The value is a type index.
        UDT = 1 << 2;
        /// The name is a label.
        LABEL = 1 << 3;
        /// The name is a constant.
        CONSTANT = 1 << 4;
        /// The name is a thunk (`fThunk`).
        THUNK = 1 << 5;
        /// The name is a namespace (`fNamespace`).
        NAMESPACE = 1 << 6;
    }
}

/// A reference from a mini PDB to a record in an object file
/// (`S_FASTLINK` and [`S_REF_MINIPDB`]).
///
/// A FASTLINK PDB keeps the full records in the object files. This record
/// gives the name and the place of the full record.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct FastLink<'a> {
    /// The raw value (`isectCoff` or `typind` in `REFMINIPDB`).
    ///
    /// With [`FastLinkFlags::UDT`], the value is a type index in the type
    /// records of the object file, not in the TPI stream. Otherwise, the
    /// fixture shows the 1-based COFF section number of the `.debug$S`
    /// section that holds the full record. Use [`FastLink::type_index`]
    /// and [`FastLink::section`].
    pub value: u32,
    /// The raw module index (`imod`) of an [`S_REF_MINIPDB`] record, or
    /// `None` for an `S_FASTLINK` record. The base of the value (0 or 1) is
    /// not known.
    pub module: Option<u16>,
    /// The flags.
    pub flags: FastLinkFlags,
    /// The name. The first record of a module has an empty name.
    pub name: Bytes<'a>,
}

into_owned!(struct FastLink { name; value, module, flags });

impl<'a> FastLink<'a> {
    /// Read the fields. Set `module` for [`S_REF_MINIPDB`].
    pub(super) fn read(fields: &mut Fields<'a>, module: bool) -> Result<Self> {
        let value = fields.u32()?;
        let module = if module { Some(fields.u16()?) } else { None };
        let flags = FastLinkFlags(fields.u16()?);
        let name = Bytes::new(fields.name()?);
        Ok(Self {
            value,
            module,
            flags,
            name,
        })
    }

    /// Return the type index in the object file, if the UDT bit is set.
    #[must_use]
    pub const fn type_index(&self) -> Option<TypeIndex> {
        if self.flags.contains(FastLinkFlags::UDT) {
            Some(TypeIndex(self.value))
        } else {
            None
        }
    }

    /// Return the COFF section number in the object file, if the UDT bit is
    /// clear.
    #[must_use]
    pub const fn section(&self) -> Option<u32> {
        if self.flags.contains(FastLinkFlags::UDT) {
            None
        } else {
            Some(self.value)
        }
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.value)?;
        if let Some(module) = self.module {
            out.u16(module)?;
        }
        out.u16(self.flags.0)?;
        out.c_string(&self.name)
    }
}
