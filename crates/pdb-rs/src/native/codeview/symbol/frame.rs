//! Frame cookie, frame register and heap allocation site records.
//!
//! The layouts of `S_FRAMECOOKIE` and `S_HEAPALLOCSITE` follow
//! `FrameCookieSym` and `HeapAllocationSiteSym` in LLVM
//! `SymbolRecordMapping.cpp` and `FRAMECOOKIE` and `HEAPALLOCSITE` in
//! `cvinfo.h`.
//!
//! No reference gives a layout for `S_FRAMEREG`. The layout comes from the
//! MSVC ARM64 fixture.

use super::super::Fields;
use super::super::index::TypeIndex;
use super::SectionOffset;
use crate::Result;
use crate::native::output::Output;

values! {
    /// The way a frame cookie is encoded (`CV_cookietype_e`).
    CookieKind(u8) {
        /// The cookie is a copy of the security cookie.
        COPY = 0;
        /// The cookie is the security cookie XOR the stack pointer.
        XOR_STACK_POINTER = 1;
        /// The cookie is the security cookie XOR the frame pointer.
        XOR_FRAME_POINTER = 2;
        /// The cookie is the security cookie XOR `R13`.
        XOR_R13 = 3;
    }
}

/// A security cookie record (`S_FRAMECOOKIE`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct FrameCookie {
    /// The offset of the cookie from the register. `cvinfo.h` gives a
    /// signed `CV_off32_t`.
    pub offset: i32,
    /// The base register, as a `CV_HREG_e` number for the CPU of the module.
    pub register: u16,
    /// The encoding of the cookie.
    pub kind: CookieKind,
    /// The raw flags. `cvinfo.h` gives no named bits.
    pub flags: u8,
}

impl FrameCookie {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let offset = fields.i32()?;
        let register = fields.u16()?;
        let kind = CookieKind(fields.u8()?);
        let flags = fields.u8()?;
        Ok(Self {
            offset,
            register,
            kind,
            flags,
        })
    }

    /// Write the fields.
    pub(super) fn write(self, out: &mut Output<'_>) -> Result<()> {
        out.i32(self.offset)?;
        out.u16(self.register)?;
        out.u8(self.kind.0)?;
        out.u8(self.flags)
    }
}

/// A heap allocation call site (`S_HEAPALLOCSITE`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct HeapAllocationSite {
    /// The address of the call instruction.
    pub address: SectionOffset,
    /// The length of the call instruction in bytes.
    pub instruction_length: u16,
    /// The type index of the signature of the called function.
    pub type_index: TypeIndex,
}

impl HeapAllocationSite {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let address = SectionOffset::read(fields)?;
        let instruction_length = fields.u16()?;
        let type_index = fields.type_index()?;
        Ok(Self {
            address,
            instruction_length,
            type_index,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        self.address.write(out)?;
        out.u16(self.instruction_length)?;
        out.type_index(self.type_index)
    }
}
