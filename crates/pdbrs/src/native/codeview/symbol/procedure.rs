//! Procedure, scope, inline site and call records.
//!
//! The layouts follow `ProcSym`, `FrameProcSym`, `BlockSym`, `LabelSym`,
//! `Thunk32Sym`, `InlineSiteSym`, `CallerSym`, `CallSiteInfoSym` and
//! `JumpTableSym` in LLVM `SymbolRecordMapping.cpp`. The reserved field of
//! `S_CALLSITEINFO` comes from `CALLSITEINFO` in `cvinfo.h`.
//! `INLINESITESYM2` and `FUNCTIONLIST` in `cvinfo.h` give the layouts of
//! `S_INLINESITE2` and `S_CALLERS`.
//!
//! No pinned reference gives the layout of the `S_GPROC32EX` kinds. The
//! DIA SDK of Visual Studio 18 has debug type records for its `cvinfo.h` in
//! `lib/amd64/diaguids.lib`. They give `PROCSYM32EX`: the `PROCSYM32`
//! layout with a 2-byte `CV_EXPROCFLAGS` in place of the 1-byte
//! `CV_PROCFLAGS`.

use super::super::id_record::IdList;
use super::super::index::{IdIndex, ModuleSymbolOffset, TypeIndex};
use super::super::{Fields, SymbolKind};
use super::{BinaryAnnotations, SectionOffset};
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// The kind of a module-local data-parallel C++ procedure start (`S_LPROC32_DPC`).
pub const S_LPROC32_DPC: SymbolKind = SymbolKind(0x1155);
/// The kind of a module-local data-parallel C++ procedure start with an ID
/// index (`S_LPROC32_DPC_ID`).
pub const S_LPROC32_DPC_ID: SymbolKind = SymbolKind(0x1156);
/// The kind of a list of callers (`S_CALLERS`).
pub const S_CALLERS: SymbolKind = SymbolKind(0x115b);
/// The kind of an inline site start with an invocation count (`S_INLINESITE2`).
pub const S_INLINESITE2: SymbolKind = SymbolKind(0x115d);
/// The kind of a global procedure start with extended flags (`S_GPROC32EX`).
pub const S_GPROC32EX: SymbolKind = SymbolKind(0x1172);
/// The kind of a module-local procedure start with extended flags (`S_LPROC32EX`).
pub const S_LPROC32EX: SymbolKind = SymbolKind(0x1173);
/// The kind of a global procedure start with extended flags and an ID index
/// (`S_GPROC32EX_ID`).
pub const S_GPROC32EX_ID: SymbolKind = SymbolKind(0x1174);
/// The kind of a module-local procedure start with extended flags and an ID
/// index (`S_LPROC32EX_ID`).
pub const S_LPROC32EX_ID: SymbolKind = SymbolKind(0x1175);

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

flags! {
    /// The second flags byte of an extended procedure (`CV_EXPROCFLAGS`).
    ///
    /// The names come from the bit fields of `CV_EXPROCFLAGS` in the debug
    /// type records of the DIA SDK. No reference describes the bits.
    ExtendedProcedureFlags(u8) {
        /// `CV_PFLAG_COMPGEN`: the compiler generated the procedure.
        COMPILER_GENERATED = 1 << 0;
        /// `CV_PFLAG_CXXMOD`: the procedure belongs to a C++ module.
        CXX_MODULE = 1 << 1;
        /// `CV_PFLAG_CXXMODEXP`: a C++ module exports the procedure.
        CXX_MODULE_EXPORT = 1 << 2;
    }
}

/// A procedure start record.
///
/// The kinds are [`S_GPROC32`](super::S_GPROC32),
/// [`S_LPROC32`](super::S_LPROC32), [`S_LPROC32_DPC`], the `_ID` kinds of
/// these and the `EX` kinds, for example [`S_GPROC32EX`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Procedure<'a> {
    /// The module stream offset of the enclosing scope record, or 0.
    pub parent: ModuleSymbolOffset,
    /// The module stream offset of the record that ends this scope.
    pub end: ModuleSymbolOffset,
    /// The module stream offset of the next procedure record, or 0. The
    /// scope check does not use this value.
    pub next: ModuleSymbolOffset,
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
    /// The second flags byte of the `EX` kinds, or `None` for the other kinds.
    pub extended_flags: Option<ExtendedProcedureFlags>,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> Procedure<'a> {
    /// Make a record with `signature`. The other fields are zero or empty.
    #[must_use]
    pub const fn new(signature: ProcedureType) -> Self {
        Self {
            parent: ModuleSymbolOffset(0),
            end: ModuleSymbolOffset(0),
            next: ModuleSymbolOffset(0),
            code_length: 0,
            debug_start: 0,
            debug_end: 0,
            signature,
            address: SectionOffset {
                section: 0,
                offset: 0,
            },
            flags: ProcedureFlags(0),
            extended_flags: None,
            name: &[],
        }
    }

    /// Read the fields. Set `id` for the `_ID` kinds and `extended` for the
    /// `EX` kinds.
    pub(super) fn read(fields: &mut Fields<'a>, id: bool, extended: bool) -> Result<Self> {
        let parent = ModuleSymbolOffset(fields.u32()?);
        let end = ModuleSymbolOffset(fields.u32()?);
        let next = ModuleSymbolOffset(fields.u32()?);
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
        let extended_flags = if extended {
            Some(ExtendedProcedureFlags(fields.u8()?))
        } else {
            None
        };
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
            extended_flags,
            name,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.parent.0)?;
        out.u32(self.end.0)?;
        out.u32(self.next.0)?;
        out.u32(self.code_length)?;
        out.u32(self.debug_start)?;
        out.u32(self.debug_end)?;
        self.signature.write(out)?;
        self.address.write(out)?;
        out.u8(self.flags.0)?;
        if let Some(flags) = self.extended_flags {
            out.u8(flags.0)?;
        }
        out.c_string(self.name)
    }
}

impl ProcedureType {
    /// Write the index as a `u32`.
    pub(super) fn write(self, out: &mut Output<'_>) -> Result<()> {
        match self {
            Self::Type(index) => out.type_index(index),
            Self::Id(index) => out.id_index(index),
        }
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
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
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

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.frame_size)?;
        out.u32(self.padding_size)?;
        out.u32(self.padding_offset)?;
        out.u32(self.callee_saved_size)?;
        self.exception_handler.write(out)?;
        out.u32(self.flags.0)
    }
}

/// A block start record (`S_BLOCK32`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Block<'a> {
    /// The module stream offset of the enclosing scope record.
    pub parent: ModuleSymbolOffset,
    /// The module stream offset of the `S_END` record of this block.
    pub end: ModuleSymbolOffset,
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
        let parent = ModuleSymbolOffset(fields.u32()?);
        let end = ModuleSymbolOffset(fields.u32()?);
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

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.parent.0)?;
        out.u32(self.end.0)?;
        out.u32(self.code_length)?;
        self.address.write(out)?;
        out.c_string(self.name)
    }
}

/// A code label record (`S_LABEL32`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
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

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        self.address.write(out)?;
        out.u8(self.flags.0)?;
        out.c_string(self.name)
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
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Thunk<'a> {
    /// The module stream offset of the enclosing scope record, or 0.
    pub parent: ModuleSymbolOffset,
    /// The module stream offset of the `S_END` record of this thunk.
    pub end: ModuleSymbolOffset,
    /// The module stream offset of the next record in the chain, or 0.
    pub next: ModuleSymbolOffset,
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
        let parent = ModuleSymbolOffset(fields.u32()?);
        let end = ModuleSymbolOffset(fields.u32()?);
        let next = ModuleSymbolOffset(fields.u32()?);
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

    /// Write the fields.
    ///
    /// A [`ThunkOrdinal::STANDARD`] thunk with variant data gives
    /// [`ErrorKind::InvalidFormat`], because the decoder rejects it.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        if self.ordinal == ThunkOrdinal::STANDARD && !self.variant.is_empty() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("standard S_THUNK32 record has variant data"));
        }
        out.u32(self.parent.0)?;
        out.u32(self.end.0)?;
        out.u32(self.next.0)?;
        self.address.write(out)?;
        out.u16(self.length)?;
        out.u8(self.ordinal.0)?;
        out.c_string(self.name)?;
        out.bytes(self.variant)
    }
}

/// An inline site start record (`S_INLINESITE` and [`S_INLINESITE2`]).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct InlineSite<'a> {
    /// The module stream offset of the enclosing scope record.
    pub parent: ModuleSymbolOffset,
    /// The module stream offset of the `S_INLINESITE_END` record.
    pub end: ModuleSymbolOffset,
    /// The ID index of the inlined function in the IPI stream.
    pub inlinee: IdIndex,
    /// The entry count of an [`S_INLINESITE2`] record, or `None` for an
    /// `S_INLINESITE` record.
    pub invocations: Option<u32>,
    /// The binary annotations without the trailing padding.
    pub annotations: BinaryAnnotations<'a>,
}

impl<'a> InlineSite<'a> {
    /// Read the fields. Set `counted` for [`S_INLINESITE2`].
    pub(super) fn read(fields: &mut Fields<'a>, counted: bool) -> Result<Self> {
        let parent = ModuleSymbolOffset(fields.u32()?);
        let end = ModuleSymbolOffset(fields.u32()?);
        let inlinee = fields.id_index()?;
        let invocations = if counted { Some(fields.u32()?) } else { None };
        let annotations = BinaryAnnotations::parse(fields.rest())?;
        fields.bytes(fields.rest().len())?;
        Ok(Self {
            parent,
            end,
            inlinee,
            invocations,
            annotations,
        })
    }

    /// Write the fields.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.parent.0)?;
        out.u32(self.end.0)?;
        out.id_index(self.inlinee)?;
        if let Some(invocations) = self.invocations {
            out.u32(invocations)?;
        }
        out.bytes(self.annotations.bytes())
    }
}

/// A list of functions with optional invocation counts (`S_CALLEES` and
/// [`S_CALLERS`]).
///
/// The counts parallel the functions. `cvinfo.h` says that the count array
/// can be shorter than the function list: a missing count is zero.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct FunctionList<'a> {
    /// The ID indices of the functions.
    functions: IdList<'a>,
    /// The little-endian 32-bit invocation counts.
    invocations: &'a [[u8; 4]],
}

impl<'a> FunctionList<'a> {
    /// Read the function list and the counts that follow it.
    ///
    /// The counts fill whole 4-byte units of the rest of the record, up to
    /// the number of functions.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let functions = read_id_list(fields)?;
        let count = functions.len().min(fields.rest().len() / 4);
        let (invocations, _) = fields.bytes(count * 4)?.as_chunks();
        Ok(Self {
            functions,
            invocations,
        })
    }

    /// Create a list from functions and little-endian 32-bit counts.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the length of `invocations`
    /// is not a multiple of 4 or if it holds more counts than functions.
    pub fn new(functions: IdList<'a>, invocations: &'a [u8]) -> Result<Self> {
        let (invocations, rest) = invocations.as_chunks();
        if !rest.is_empty() || invocations.len() > functions.len() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("function list has invalid invocation counts"));
        }
        Ok(Self {
            functions,
            invocations,
        })
    }

    /// Return the ID indices of the functions.
    #[must_use]
    pub const fn functions(&self) -> IdList<'a> {
        self.functions
    }

    /// Return the stored invocation counts. A function without a count has
    /// the count 0.
    #[must_use]
    pub fn invocations(&self) -> Invocations<'a> {
        Invocations(self.invocations.iter())
    }

    /// Write the count, the functions and the invocation counts.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        write_id_list(&self.functions, out)?;
        for entry in self.invocations {
            out.bytes(entry)?;
        }
        Ok(())
    }
}

/// Read a `u32` count and then the ID indices (`S_INLINEES` and the
/// functions of a [`FunctionList`]).
pub(super) fn read_id_list<'a>(fields: &mut Fields<'a>) -> Result<IdList<'a>> {
    let count = fields.u32()?;
    let length = usize::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(4))
        .filter(|&length| length <= fields.rest().len())
        .ok_or_else(|| {
            Error::new(ErrorKind::InvalidFormat, None)
                .with_context("symbol index list count exceeds the record")
        })?;
    IdList::parse(fields.bytes(length)?)
}

/// Write the `u32` count and then the ID indices.
pub(super) fn write_id_list(list: &IdList<'_>, out: &mut Output<'_>) -> Result<()> {
    out.u32(count_u32(list.len())?)?;
    out.bytes(list.as_bytes())
}

/// A call site record (`S_CALLSITEINFO`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
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

    /// Write the fields and a zero reserved field.
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        self.address.write(out)?;
        out.u16(0)?;
        out.type_index(self.type_index)
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
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
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

    /// Write the fields in the order of [`JumpTable::read`].
    pub(super) fn write(&self, out: &mut Output<'_>) -> Result<()> {
        self.base.write(out)?;
        out.u16(self.entry.0)?;
        out.u32(self.branch.offset)?;
        out.u32(self.table.offset)?;
        out.u16(self.branch.section)?;
        out.u16(self.table.section)?;
        out.u32(self.entry_count)
    }
}

/// Convert a list length to a `u32` count field.
pub(super) fn count_u32(count: usize) -> Result<u32> {
    u32::try_from(count).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("symbol list count does not fit in 32 bits")
    })
}

/// An iterator over the invocation counts of a [`FunctionList`].
#[derive(Clone, Debug)]
pub struct Invocations<'a>(core::slice::Iter<'a, [u8; 4]>);

impl Iterator for Invocations<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        self.0.next().map(|x| u32::from_le_bytes(*x))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl ExactSizeIterator for Invocations<'_> {}

impl core::iter::FusedIterator for Invocations<'_> {}
