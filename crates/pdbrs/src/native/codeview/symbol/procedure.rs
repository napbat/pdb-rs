//! Procedure, scope, inline site and call records.
//!
//! The layouts follow `ProcSym`, `FrameProcSym`, `BlockSym`, `LabelSym`,
//! `Thunk32Sym`, `InlineSiteSym`, `CallerSym`, `CallSiteInfoSym` and
//! `JumpTableSym` in LLVM `SymbolRecordMapping.cpp`. The reserved field of
//! `S_CALLSITEINFO` comes from `CALLSITEINFO` in `cvinfo.h`.

use core::iter::FusedIterator;

use super::super::Fields;
use super::super::index::{IdIndex, TypeIndex};
use super::{BinaryAnnotations, SectionOffset};
use crate::{Error, ErrorKind, Result};

flags! {
    /// The flags of a procedure or a label (`CV_PROCFLAGS`).
    ProcedureFlags(u8) {
        /// The frame pointer is present.
        HAS_FRAME_POINTER = 1 << 0;
        /// The procedure returns with an interrupt return.
        HAS_INTERRUPT_RETURN = 1 << 1;
        /// The procedure returns with a far return.
        HAS_FAR_RETURN = 1 << 2;
        /// The procedure does not return.
        NO_RETURN = 1 << 3;
        /// The label or procedure is not reached.
        UNREACHABLE = 1 << 4;
        /// The procedure uses a custom calling convention.
        CUSTOM_CALLING_CONVENTION = 1 << 5;
        /// The procedure is marked as no-inline.
        NO_INLINE = 1 << 6;
        /// The procedure has debug information for optimized code.
        OPTIMIZED_DEBUG_INFO = 1 << 7;
    }
}

/// The type of a procedure.
///
/// The `_ID` kinds refer to an `LF_FUNC_ID` or `LF_MFUNC_ID` record in the
/// IPI stream. The other kinds refer to a procedure type in the TPI stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProcedureType {
    /// An index in the TPI stream.
    Type(TypeIndex),
    /// An index in the IPI stream.
    Id(IdIndex),
}

/// A procedure start record (`S_GPROC32`, `S_LPROC32` and their `_ID` kinds).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Procedure<'a> {
    /// The module stream offset of the enclosing scope record, or 0.
    pub parent: u32,
    /// The module stream offset of the record that ends this scope.
    pub end: u32,
    /// The module stream offset of the next procedure record, or 0. The
    /// scope check does not use this value.
    pub next: u32,
    /// The length of the procedure code in bytes.
    pub code_length: u32,
    /// The offset of the end of the prologue, relative to the procedure start.
    pub debug_start: u32,
    /// The offset of the start of the epilogue, relative to the procedure start.
    pub debug_end: u32,
    /// The procedure type or the function ID.
    pub signature: ProcedureType,
    /// The start address.
    pub address: SectionOffset,
    /// The flags.
    pub flags: ProcedureFlags,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> Procedure<'a> {
    /// Read the fields. Set `id` for the `_ID` kinds.
    pub(super) fn read(fields: &mut Fields<'a>, id: bool) -> Result<Self> {
        let parent = fields.u32()?;
        let end = fields.u32()?;
        let next = fields.u32()?;
        let code_length = fields.u32()?;
        let debug_start = fields.u32()?;
        let debug_end = fields.u32()?;
        let signature = if id {
            ProcedureType::Id(fields.id_index()?)
        } else {
            ProcedureType::Type(fields.type_index()?)
        };
        let address = SectionOffset::read(fields)?;
        let flags = ProcedureFlags(fields.u8()?);
        let name = fields.name()?;
        Ok(Self {
            parent,
            end,
            next,
            code_length,
            debug_start,
            debug_end,
            signature,
            address,
            flags,
            name,
        })
    }
}

values! {
    /// An encoded frame pointer register in [`FrameProcedureFlags`].
    ///
    /// The register depends on the CPU. For x64, LLVM `decodeFramePtrReg`
    /// maps [`FramePointer::STACK`] to `RSP`, [`FramePointer::FRAME`] to
    /// `RBP` and [`FramePointer::BASE`] to `R13`.
    FramePointer(u8) {
        /// No register.
        NONE = 0;
        /// The stack pointer.
        STACK = 1;
        /// The frame pointer.
        FRAME = 2;
        /// The base pointer.
        BASE = 3;
    }
}

flags! {
    /// The flags of a frame procedure record (`FrameProcedureOptions` in LLVM).
    ///
    /// Bits 14 and 15 and bits 16 and 17 hold encoded registers. Use
    /// [`FrameProcedureFlags::local_frame_pointer`] and
    /// [`FrameProcedureFlags::parameter_frame_pointer`] to read them.
    FrameProcedureFlags(u32) {
        /// The procedure calls `alloca`.
        HAS_ALLOCA = 1 << 0;
        /// The procedure calls `setjmp`.
        HAS_SETJMP = 1 << 1;
        /// The procedure calls `longjmp`.
        HAS_LONGJMP = 1 << 2;
        /// The procedure contains inline assembly.
        HAS_INLINE_ASSEMBLY = 1 << 3;
        /// The procedure has exception handling.
        HAS_EXCEPTION_HANDLING = 1 << 4;
        /// The procedure is marked inline.
        MARKED_INLINE = 1 << 5;
        /// The procedure has structured exception handling.
        HAS_STRUCTURED_EXCEPTION_HANDLING = 1 << 6;
        /// The procedure is naked.
        NAKED = 1 << 7;
        /// The procedure has buffer security checks.
        SECURITY_CHECKS = 1 << 8;
        /// The procedure was compiled with asynchronous exception handling.
        ASYNCHRONOUS_EXCEPTION_HANDLING = 1 << 9;
        /// The security checks do not order the stack.
        NO_STACK_ORDERING_FOR_SECURITY_CHECKS = 1 << 10;
        /// The procedure was inlined.
        INLINED = 1 << 11;
        /// The procedure has strict security checks.
        STRICT_SECURITY_CHECKS = 1 << 12;
        /// The procedure is marked with safe buffers.
        SAFE_BUFFERS = 1 << 13;
        /// The procedure was compiled with profile-guided optimization.
        PROFILE_GUIDED_OPTIMIZATION = 1 << 18;
        /// The profile counts are valid.
        VALID_PROFILE_COUNTS = 1 << 19;
        /// The procedure was optimized for speed.
        OPTIMIZED_FOR_SPEED = 1 << 20;
        /// The procedure has control flow guard checks.
        GUARD_CFG = 1 << 21;
        /// The procedure has control flow write guard checks.
        GUARD_CFW = 1 << 22;
    }
}

impl FrameProcedureFlags {
    /// Return the encoded register that addresses local variables (bits 14 and 15).
    #[must_use]
    pub const fn local_frame_pointer(self) -> FramePointer {
        FramePointer(self.0.to_le_bytes()[1] >> 6)
    }

    /// Return the encoded register that addresses parameters (bits 16 and 17).
    #[must_use]
    pub const fn parameter_frame_pointer(self) -> FramePointer {
        FramePointer(self.0.to_le_bytes()[2] & 0b11)
    }
}

/// A frame procedure record (`S_FRAMEPROC`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct FrameProcedure {
    /// The size of the stack frame in bytes.
    pub frame_size: u32,
    /// The size of the padding in the frame in bytes.
    pub padding_size: u32,
    /// The offset of the padding, relative to the frame pointer.
    pub padding_offset: u32,
    /// The size of the callee-saved registers in bytes.
    pub callee_saved_size: u32,
    /// The address of the exception handler.
    pub exception_handler: SectionOffset,
    /// The flags and the encoded frame pointer registers.
    pub flags: FrameProcedureFlags,
}

impl FrameProcedure {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let frame_size = fields.u32()?;
        let padding_size = fields.u32()?;
        let padding_offset = fields.u32()?;
        let callee_saved_size = fields.u32()?;
        let exception_handler = SectionOffset::read(fields)?;
        let flags = FrameProcedureFlags(fields.u32()?);
        Ok(Self {
            frame_size,
            padding_size,
            padding_offset,
            callee_saved_size,
            exception_handler,
            flags,
        })
    }
}

/// A block start record (`S_BLOCK32`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Block<'a> {
    /// The module stream offset of the enclosing scope record.
    pub parent: u32,
    /// The module stream offset of the `S_END` record of this block.
    pub end: u32,
    /// The length of the block code in bytes.
    pub code_length: u32,
    /// The start address.
    pub address: SectionOffset,
    /// The name. It is often empty.
    pub name: &'a [u8],
}

impl<'a> Block<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let parent = fields.u32()?;
        let end = fields.u32()?;
        let code_length = fields.u32()?;
        let address = SectionOffset::read(fields)?;
        let name = fields.name()?;
        Ok(Self {
            parent,
            end,
            code_length,
            address,
            name,
        })
    }
}

/// A code label record (`S_LABEL32`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Label<'a> {
    /// The address.
    pub address: SectionOffset,
    /// The flags.
    pub flags: ProcedureFlags,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> Label<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let address = SectionOffset::read(fields)?;
        let flags = ProcedureFlags(fields.u8()?);
        let name = fields.name()?;
        Ok(Self {
            address,
            flags,
            name,
        })
    }
}

values! {
    /// The kind of a thunk (`THUNK_ORDINAL`).
    ThunkOrdinal(u8) {
        /// A standard thunk.
        STANDARD = 0;
        /// A `this` adjustor thunk.
        THIS_ADJUSTOR = 1;
        /// A virtual call thunk.
        VCALL = 2;
        /// A P-code thunk.
        PCODE = 3;
        /// A delay-load thunk.
        UNKNOWN_LOAD = 4;
        /// An incremental-link trampoline.
        TRAMPOLINE_INCREMENTAL = 5;
        /// A branch island.
        BRANCH_ISLAND = 6;
    }
}

/// A thunk start record (`S_THUNK32`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Thunk<'a> {
    /// The module stream offset of the enclosing scope record, or 0.
    pub parent: u32,
    /// The module stream offset of the `S_END` record of this thunk.
    pub end: u32,
    /// The module stream offset of the next record in the chain, or 0.
    pub next: u32,
    /// The start address.
    pub address: SectionOffset,
    /// The length of the thunk code in bytes.
    pub length: u16,
    /// The kind of the thunk.
    pub ordinal: ThunkOrdinal,
    /// The name.
    pub name: &'a [u8],
    /// The raw variant data after the name.
    ///
    /// A [`ThunkOrdinal::STANDARD`] thunk has no variant data, so the value
    /// is empty and the decoder checks the zero padding. For other kinds,
    /// the references do not give the variant length. The value then holds
    /// all bytes after the name, including the record padding.
    pub variant: &'a [u8],
}

impl<'a> Thunk<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let parent = fields.u32()?;
        let end = fields.u32()?;
        let next = fields.u32()?;
        let address = SectionOffset::read(fields)?;
        let length = fields.u16()?;
        let ordinal = ThunkOrdinal(fields.u8()?);
        let name = fields.name()?;
        let variant = if ordinal == ThunkOrdinal::STANDARD {
            &[]
        } else {
            fields.bytes(fields.rest().len())?
        };
        Ok(Self {
            parent,
            end,
            next,
            address,
            length,
            ordinal,
            name,
            variant,
        })
    }
}

/// An inline site start record (`S_INLINESITE`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct InlineSite<'a> {
    /// The module stream offset of the enclosing scope record.
    pub parent: u32,
    /// The module stream offset of the `S_INLINESITE_END` record.
    pub end: u32,
    /// The ID index of the inlined function in the IPI stream.
    pub inlinee: IdIndex,
    /// The binary annotations without the trailing padding.
    pub annotations: BinaryAnnotations<'a>,
}

impl<'a> InlineSite<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let parent = fields.u32()?;
        let end = fields.u32()?;
        let inlinee = fields.id_index()?;
        let annotations = BinaryAnnotations::new(fields.rest())?;
        fields.bytes(fields.rest().len())?;
        Ok(Self {
            parent,
            end,
            inlinee,
            annotations,
        })
    }
}

/// A counted list of ID indices (`S_INLINEES` and `S_CALLEES`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IdList<'a> {
    /// The little-endian 32-bit indices.
    entries: &'a [[u8; 4]],
}

impl<'a> IdList<'a> {
    /// Read a `u32` count and then the indices.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let count = fields.u32()?;
        let length = usize::try_from(count)
            .ok()
            .and_then(|count| count.checked_mul(4))
            .filter(|&length| length <= fields.rest().len())
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("symbol index list count exceeds the record")
            })?;
        let (entries, _) = fields.bytes(length)?.as_chunks();
        Ok(Self { entries })
    }

    /// Return the number of indices.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return `true` if the list has no indices.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return an iterator over the indices.
    #[must_use]
    pub fn iter(&self) -> IdListIter<'a> {
        IdListIter {
            entries: self.entries.iter(),
        }
    }
}

impl<'a> IntoIterator for IdList<'a> {
    type Item = IdIndex;
    type IntoIter = IdListIter<'a>;

    fn into_iter(self) -> IdListIter<'a> {
        self.iter()
    }
}

impl<'a> IntoIterator for &IdList<'a> {
    type Item = IdIndex;
    type IntoIter = IdListIter<'a>;

    fn into_iter(self) -> IdListIter<'a> {
        self.iter()
    }
}

/// An iterator over the indices of an [`IdList`].
#[derive(Clone, Debug)]
pub struct IdListIter<'a> {
    /// The 4-byte indices that are not returned yet.
    entries: core::slice::Iter<'a, [u8; 4]>,
}

impl Iterator for IdListIter<'_> {
    type Item = IdIndex;

    fn next(&mut self) -> Option<IdIndex> {
        self.entries
            .next()
            .map(|&bytes| IdIndex(u32::from_le_bytes(bytes)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.entries.size_hint()
    }
}

impl ExactSizeIterator for IdListIter<'_> {}

impl FusedIterator for IdListIter<'_> {}

/// A call site record (`S_CALLSITEINFO`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct CallSiteInfo {
    /// The address of the call instruction.
    pub address: SectionOffset,
    /// The type index of the function signature in the TPI stream.
    pub type_index: TypeIndex,
}

impl CallSiteInfo {
    /// Read the fields and check the reserved field.
    pub(super) fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let address = SectionOffset::read(fields)?;
        if fields.u16()? != 0 {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("S_CALLSITEINFO reserved field is not zero"));
        }
        let type_index = fields.type_index()?;
        Ok(Self {
            address,
            type_index,
        })
    }
}

values! {
    /// The encoding of jump table entries (`CV_armswitchtype`).
    JumpTableEntry(u16) {
        /// Signed 8-bit offsets.
        INT8 = 0;
        /// Unsigned 8-bit offsets.
        UINT8 = 1;
        /// Signed 16-bit offsets.
        INT16 = 2;
        /// Unsigned 16-bit offsets.
        UINT16 = 3;
        /// Signed 32-bit offsets.
        INT32 = 4;
        /// Unsigned 32-bit offsets.
        UINT32 = 5;
        /// Absolute pointers.
        POINTER = 6;
        /// Unsigned 8-bit offsets that are shifted left.
        UINT8_SHIFT_LEFT = 7;
        /// Unsigned 16-bit offsets that are shifted left.
        UINT16_SHIFT_LEFT = 8;
        /// Signed 8-bit offsets that are shifted left.
        INT8_SHIFT_LEFT = 9;
        /// Signed 16-bit offsets that are shifted left.
        INT16_SHIFT_LEFT = 10;
    }
}

/// A jump table record (`S_ARMSWITCHTABLE`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct JumpTable {
    /// The base address that the entries are relative to.
    pub base: SectionOffset,
    /// The encoding of the entries.
    pub entry: JumpTableEntry,
    /// The address of the branch instruction.
    pub branch: SectionOffset,
    /// The address of the table.
    pub table: SectionOffset,
    /// The number of entries.
    pub entry_count: u32,
}

impl JumpTable {
    /// Read the fields.
    ///
    /// The layout is base offset, base section, entry encoding, branch
    /// offset, table offset, branch section, table section and entry count.
    pub(super) fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let base = SectionOffset::read(fields)?;
        let entry = JumpTableEntry(fields.u16()?);
        let branch_offset = fields.u32()?;
        let table_offset = fields.u32()?;
        let branch_section = fields.u16()?;
        let table_section = fields.u16()?;
        let entry_count = fields.u32()?;
        Ok(Self {
            base,
            entry,
            branch: SectionOffset {
                section: branch_section,
                offset: branch_offset,
            },
            table: SectionOffset {
                section: table_section,
                offset: table_offset,
            },
            entry_count,
        })
    }
}
