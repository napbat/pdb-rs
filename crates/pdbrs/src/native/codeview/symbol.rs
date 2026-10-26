//! Typed decoding of `CodeView` symbol records in PDB symbol streams.
//!
//! [`Symbol::decode`] turns one framed [`Record`] into a [`Symbol`]. The
//! symbol keeps the record and a [`SymbolData`] value with the typed fields
//! of its kind. A kind that this crate does not decode gives
//! [`SymbolData::Unsupported`] with the raw payload.
//!
//! The kind values come from `CodeViewSymbols.def` and the layouts come from
//! `SymbolRecord.h` and `SymbolRecordMapping.cpp` in LLVM (revision
//! `b9f2235f`). The Microsoft `cvinfo.h` header gives the reserved fields.
//!
//! In a PDB stream, each symbol record ends with zero bytes that align the
//! next record to four bytes. The decoder makes sure that these bytes are
//! zero. Names are NUL-terminated byte strings. They are not decoded as text
//! and not demangled.
//!
//! [`ModuleSymbols`] walks a module symbol stream and checks the scope links.
//!
//! [`SymbolData::encode`] and [`Symbol::encode`] write a record again. A
//! decoded record encodes to its original bytes.

use super::id_record::IdList;
use super::index::{IdIndex, ModuleSymbolOffset, TypeIndex};
use super::{Fields, Record, SymbolKind};
use crate::native::address::SectionOffset;
use crate::native::output::Output;
use crate::{ErrorKind, Result};

/// Define a newtype for a bit set with named bits.
macro_rules! flags {
    (
        $(#[$meta:meta])*
        $name:ident($raw:ty) {
            $($(#[$flag_meta:meta])* $flag:ident = $value:expr;)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
        pub struct $name(pub $raw);

        impl $name {
            $($(#[$flag_meta])* pub const $flag: Self = Self($value);)*

            /// Return `true` if all bits of `other` are set in `self`.
            #[must_use]
            pub const fn contains(self, other: Self) -> bool {
                self.0 & other.0 == other.0
            }
        }
    };
}

/// Define a newtype for a raw value with named known values.
macro_rules! values {
    (
        $(#[$meta:meta])*
        $name:ident($raw:ty) {
            $($(#[$value_meta:meta])* $value_name:ident = $value:expr;)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
        pub struct $name(pub $raw);

        impl $name {
            $($(#[$value_meta])* pub const $value_name: Self = Self($value);)*
        }
    };
}

mod access;
mod annotation;
mod architecture;
mod attributed;
mod code;
mod compile;
mod cpu;
mod defrange;
mod encode;
mod frame;
mod global;
mod hlsl;
mod link;
mod linker;
mod managed;
mod procedure;
mod scope;
mod storage;
pub mod validate;
mod variable;

pub(crate) use annotation::code::{ChunkRange, CodeError, CodeState, separated_chunks};
pub use annotation::{Annotation, BinaryAnnotations, BinaryAnnotationsIter};
pub use architecture::{
    Ia64Procedure, MipsProcedure, ProcedureHeader, S_GPROCIA64, S_GPROCIA64_ID, S_GPROCMIPS,
    S_GPROCMIPS_ID, S_LPROCIA64, S_LPROCIA64_ID, S_LPROCMIPS, S_LPROCMIPS_ID,
};
pub use attributed::{
    AttributedFrameRelative, AttributedRegister, AttributedRegisterRelative, ManyRegisters,
    RegisterList, RegisterListIter, S_ATTR_FRAMEREL, S_ATTR_MANYREG, S_ATTR_REGISTER,
    S_ATTR_REGREL, S_GMANDATA, S_LMANDATA, S_LOCALSLOT, S_MANFRAMEREL, S_MANMANYREG, S_MANMANYREG2,
    S_MANREGISTER, S_MANREGREL, S_MANTYPREF, S_MANYREG, S_MANYREG2, S_PARAMSLOT, Slot,
    VariableAttributes,
};
pub use code::{
    AnnotationStrings, AnnotationStringsIter, Association, AssociationKind, HotPatchFunction,
    HybridRange, S_ANNOTATION, S_ASSOCIATION, S_HOTPATCHFUNC, S_HYBRIDRANGE, S_SEPCODE,
    S_STATICLOCAL, S_VFTABLE32, S_WITH32, SeparatedCode, SeparatedCodeFlags, StaticLocal,
    VirtualFunctionTable, With,
};
pub use compile::{
    Compile2, Compile3, CompileFlags, EnvironmentBlock, ObjectName, SourceLanguage, StringList,
    StringListIter,
};
pub use cpu::CpuType;
use defrange::read_global_on_entry;
pub use defrange::{
    DefRangeProgram, EntryValues, EntryValuesIter, S_DEFRANGE, S_DEFRANGE_CONSTVAL_ON_ENTRY,
    S_DEFRANGE_GLOBALSYM_ON_ENTRY, S_DEFRANGE_SUBFIELD,
};
pub use frame::{CookieKind, FrameCookie, HeapAllocationSite};
pub use global::{
    CoffGroup, ProcedureReference, Public, PublicFlags, S_ANNOTATIONREF, S_DATAREF, Section,
};
pub use hlsl::{
    DefRangeHlsl, DpcGroupSharedLocal, DpcTag, DpcTagMap, DpcTagMapIter, HlslData, HlslDataEx,
    S_DEFRANGE_DPC_PTR_TAG, S_DEFRANGE_HLSL, S_DPC_SYM_TAG_MAP, S_GDATA_HLSL, S_GDATA_HLSL32,
    S_GDATA_HLSL32_EX, S_LDATA_HLSL, S_LDATA_HLSL32, S_LDATA_HLSL32_EX, S_LOCAL_DPC_GROUPSHARED,
};
pub use link::{FastLink, FastLinkFlags, S_REF_MINIPDB, Trampoline, TrampolineKind};
pub use linker::{
    DiscardReason, Discarded, Export, ExportFlags, ModuleTypeReference, ModuleTypeReferenceFlags,
    ModuleTypeSource, PdbMap, ProfileData, S_DISCARDED, S_EXPORT, S_MOD_TYPEREF, S_PDBMAP,
    S_POGODATA, S_SOURCELINK,
};
pub use managed::{
    ASYNC_METHOD_INFO_ATTRIBUTE, AsyncMethodInfo, AsyncStep, AsyncSteps, AsyncStepsIter,
    AttributeValue, CUSTOM_DEBUG_INFO_VERSION, CustomDebugInfo, CustomDebugInfoData,
    CustomDebugInfoKind, CustomDebugInfoRecord, CustomDebugInfoRecords, DynamicLocal,
    DynamicLocals, DynamicLocalsIter, HoistedLocalScope, HoistedLocalScopes,
    HoistedLocalScopesIter, MANAGED_ATTRIBUTE_OEM_ID, MD2_ATTRIBUTE, ManagedConstant,
    ManagedProcedure, ManagedSlot, Oem, S_GMANPROC, S_LMANPROC, S_MANCONSTANT, S_MANSLOT, S_OEM,
    S_TOKENREF, SymbolAttribute, TupleElementNames, TupleElementNamesEntry, TupleElementNamesIter,
    UsingGroups, UsingGroupsIter, Utf8Names, Utf8NamesIter,
};
pub use procedure::{
    Block, CallSiteInfo, ExtendedProcedureFlags, FramePointer, FrameProcedure, FrameProcedureFlags,
    FunctionList, InlineSite, JumpTable, JumpTableEntry, Label, Procedure, ProcedureFlags,
    ProcedureType, S_CALLERS, S_GPROC32EX, S_GPROC32EX_ID, S_INLINESITE2, S_LPROC32_DPC,
    S_LPROC32_DPC_ID, S_LPROC32EX, S_LPROC32EX_ID, Thunk, ThunkOrdinal,
};
pub use scope::{ModuleSymbolWriter, ModuleSymbols, ScopedSymbol};
pub use storage::{
    BasePointerRelative, BasePointerRelativeIndirect, FileStatic, RegisterRelativeIndirect,
    RegisterVariable, S_BPREL32_ENCTMP, S_BPREL32_INDIR, S_BPREL32_INDIR_ENCTMP, S_REGREL32_INDIR,
    S_REGREL32_INDIR_ENCTMP,
};
pub use variable::{
    AddressGap, AddressGaps, AddressGapsIter, AddressRange, Constant, Data,
    DefRangeFramePointerRelative, DefRangeRegister, DefRangeRegisterRelative,
    DefRangeRegisterRelativeIndirect, DefRangeSubfieldRegister, Local, LocalFlags,
    RegisterRelative, S_COBOLUDT, UserDefinedType,
};

/// The kind of a filler record that reserves space (`S_SKIP`).
pub const S_SKIP: SymbolKind = SymbolKind(0x0007);
/// The kind of the end of a procedure, block or thunk scope (`S_END`).
pub const S_END: SymbolKind = SymbolKind(0x0006);
/// The kind of a frame procedure record (`S_FRAMEPROC`).
pub const S_FRAMEPROC: SymbolKind = SymbolKind(0x1012);
/// The kind of an object file name (`S_OBJNAME`).
pub const S_OBJNAME: SymbolKind = SymbolKind(0x1101);
/// The kind of a thunk start (`S_THUNK32`).
pub const S_THUNK32: SymbolKind = SymbolKind(0x1102);
/// The kind of a block start (`S_BLOCK32`).
pub const S_BLOCK32: SymbolKind = SymbolKind(0x1103);
/// The kind of a code label (`S_LABEL32`).
pub const S_LABEL32: SymbolKind = SymbolKind(0x1105);
/// The kind of a register variable (`S_REGISTER`).
pub const S_REGISTER: SymbolKind = SymbolKind(0x1106);
/// The kind of a constant (`S_CONSTANT`).
pub const S_CONSTANT: SymbolKind = SymbolKind(0x1107);
/// The kind of a user-defined type name (`S_UDT`).
pub const S_UDT: SymbolKind = SymbolKind(0x1108);
/// The kind of a frame-pointer-relative variable (`S_BPREL32`).
pub const S_BPREL32: SymbolKind = SymbolKind(0x110b);
/// The kind of a module-local data symbol (`S_LDATA32`).
pub const S_LDATA32: SymbolKind = SymbolKind(0x110c);
/// The kind of a global data symbol (`S_GDATA32`).
pub const S_GDATA32: SymbolKind = SymbolKind(0x110d);
/// The kind of a public symbol (`S_PUB32`).
pub const S_PUB32: SymbolKind = SymbolKind(0x110e);
/// The kind of a module-local procedure start (`S_LPROC32`).
pub const S_LPROC32: SymbolKind = SymbolKind(0x110f);
/// The kind of a global procedure start (`S_GPROC32`).
pub const S_GPROC32: SymbolKind = SymbolKind(0x1110);
/// The kind of a register-relative variable (`S_REGREL32`).
pub const S_REGREL32: SymbolKind = SymbolKind(0x1111);
/// The kind of a module-local thread storage symbol (`S_LTHREAD32`).
pub const S_LTHREAD32: SymbolKind = SymbolKind(0x1112);
/// The kind of a global thread storage symbol (`S_GTHREAD32`).
pub const S_GTHREAD32: SymbolKind = SymbolKind(0x1113);
/// The kind of a compile record, version 2 (`S_COMPILE2`).
pub const S_COMPILE2: SymbolKind = SymbolKind(0x1116);
/// The kind of a using-namespace record (`S_UNAMESPACE`).
pub const S_UNAMESPACE: SymbolKind = SymbolKind(0x1124);
/// The kind of a reference to a global procedure (`S_PROCREF`).
pub const S_PROCREF: SymbolKind = SymbolKind(0x1125);
/// The kind of a reference to a module-local procedure (`S_LPROCREF`).
pub const S_LPROCREF: SymbolKind = SymbolKind(0x1127);
/// The kind of a trampoline thunk (`S_TRAMPOLINE`).
pub const S_TRAMPOLINE: SymbolKind = SymbolKind(0x112c);
/// The kind of an image section record (`S_SECTION`).
pub const S_SECTION: SymbolKind = SymbolKind(0x1136);
/// The kind of a COFF group record (`S_COFFGROUP`).
pub const S_COFFGROUP: SymbolKind = SymbolKind(0x1137);
/// The kind of a call site record (`S_CALLSITEINFO`).
pub const S_CALLSITEINFO: SymbolKind = SymbolKind(0x1139);
/// The kind of a security cookie record (`S_FRAMECOOKIE`).
pub const S_FRAMECOOKIE: SymbolKind = SymbolKind(0x113a);
/// The kind of a compile record, version 3 (`S_COMPILE3`).
pub const S_COMPILE3: SymbolKind = SymbolKind(0x113c);
/// The kind of a build environment block (`S_ENVBLOCK`).
pub const S_ENVBLOCK: SymbolKind = SymbolKind(0x113d);
/// The kind of a local variable (`S_LOCAL`).
pub const S_LOCAL: SymbolKind = SymbolKind(0x113e);
/// The kind of a register def-range (`S_DEFRANGE_REGISTER`).
pub const S_DEFRANGE_REGISTER: SymbolKind = SymbolKind(0x1141);
/// The kind of a frame-pointer-relative def-range (`S_DEFRANGE_FRAMEPOINTER_REL`).
pub const S_DEFRANGE_FRAMEPOINTER_REL: SymbolKind = SymbolKind(0x1142);
/// The kind of a register def-range for a part of a variable
/// (`S_DEFRANGE_SUBFIELD_REGISTER`).
pub const S_DEFRANGE_SUBFIELD_REGISTER: SymbolKind = SymbolKind(0x1143);
/// The kind of a full-scope frame-pointer-relative def-range
/// (`S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE`).
pub const S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE: SymbolKind = SymbolKind(0x1144);
/// The kind of a register-relative def-range (`S_DEFRANGE_REGISTER_REL`).
pub const S_DEFRANGE_REGISTER_REL: SymbolKind = SymbolKind(0x1145);
/// The kind of a module-local procedure start with an ID index (`S_LPROC32_ID`).
pub const S_LPROC32_ID: SymbolKind = SymbolKind(0x1146);
/// The kind of a global procedure start with an ID index (`S_GPROC32_ID`).
pub const S_GPROC32_ID: SymbolKind = SymbolKind(0x1147);
/// The kind of a build information reference (`S_BUILDINFO`).
pub const S_BUILDINFO: SymbolKind = SymbolKind(0x114c);
/// The kind of an inline site start (`S_INLINESITE`).
pub const S_INLINESITE: SymbolKind = SymbolKind(0x114d);
/// The kind of the end of an inline site scope (`S_INLINESITE_END`).
pub const S_INLINESITE_END: SymbolKind = SymbolKind(0x114e);
/// The kind of the end of a procedure scope with an ID index (`S_PROC_ID_END`).
pub const S_PROC_ID_END: SymbolKind = SymbolKind(0x114f);
/// The kind of a file-static variable (`S_FILESTATIC`).
pub const S_FILESTATIC: SymbolKind = SymbolKind(0x1153);
/// The kind of a jump table record (`S_ARMSWITCHTABLE`).
pub const S_ARMSWITCHTABLE: SymbolKind = SymbolKind(0x1159);
/// The kind of a list of callees (`S_CALLEES`).
pub const S_CALLEES: SymbolKind = SymbolKind(0x115a);
/// The kind of a heap allocation call site (`S_HEAPALLOCSITE`).
pub const S_HEAPALLOCSITE: SymbolKind = SymbolKind(0x115e);
/// The kind of a frame register record (`S_FRAMEREG`).
pub const S_FRAMEREG: SymbolKind = SymbolKind(0x1166);
/// The kind of a FASTLINK reference (`S_FASTLINK`).
pub const S_FASTLINK: SymbolKind = SymbolKind(0x1167);
/// The kind of a list of inlinees (`S_INLINEES`).
pub const S_INLINEES: SymbolKind = SymbolKind(0x1168);
/// The kind of an indirect register-relative def-range
/// (`S_DEFRANGE_REGISTER_REL_INDIR`).
pub const S_DEFRANGE_REGISTER_REL_INDIR: SymbolKind = SymbolKind(0x1177);
/// The kind of a register-relative compiler temporary for Edit and Continue
/// (`S_REGREL32_ENCTMP`).
pub const S_REGREL32_ENCTMP: SymbolKind = SymbolKind(0x1179);

/// The error context for a payload that is too short for its fields.
const SHORT_PAYLOAD: &str = "symbol record payload is shorter than its fields";

impl SectionOffset {
    /// Read a `u32` offset and then a `u16` section number.
    fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let offset = fields.u32()?;
        let section = fields.u16()?;
        Ok(Self { section, offset })
    }

    /// Write the `u32` offset and then the `u16` section number.
    fn write(self, out: &mut Output<'_>) -> Result<()> {
        out.u32(self.offset)?;
        out.u16(self.section)
    }
}

/// The parent and end links of a record that starts a scope.
///
/// Both values are offsets in the module stream. They include the 4-byte
/// signature at the start of the stream.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ScopeLinks {
    /// The offset of the record of the enclosing scope, or 0 at the top level.
    pub parent: ModuleSymbolOffset,
    /// The offset of the record that ends this scope.
    pub end: ModuleSymbolOffset,
}

/// A decoded symbol record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Symbol<'a> {
    /// The original record.
    record: Record<'a>,
    /// The typed fields of the kind.
    data: SymbolData<'a>,
    /// The number of zero bytes after the fields.
    trailing_zeros: usize,
}

/// The typed fields of a symbol record.
///
/// Kinds that share a layout share a variant. Use [`Symbol::kind`] to tell
/// them apart, for example [`S_GDATA32`] and [`S_LDATA32`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum SymbolData<'a> {
    /// [`S_OBJNAME`].
    ObjectName(ObjectName<'a>),
    /// [`S_COMPILE2`].
    Compile2(Compile2<'a>),
    /// [`S_COMPILE3`].
    Compile3(Compile3<'a>),
    /// [`S_BUILDINFO`]: the ID index of an `LF_BUILDINFO` record in the IPI stream.
    BuildInfo(IdIndex),
    /// [`S_SOURCELINK`]: an ID index in the IPI stream.
    SourceLink(IdIndex),
    /// [`S_ENVBLOCK`].
    EnvironmentBlock(EnvironmentBlock<'a>),
    /// [`S_UNAMESPACE`]: the namespace name.
    UsingNamespace(&'a [u8]),
    /// [`S_GPROC32`], [`S_LPROC32`], [`S_LPROC32_DPC`], their `_ID` kinds
    /// and the `EX` kinds, for example [`S_GPROC32EX`].
    Procedure(Procedure<'a>),
    /// [`S_GPROCMIPS`], [`S_LPROCMIPS`] and their `_ID` kinds.
    MipsProcedure(MipsProcedure<'a>),
    /// [`S_GPROCIA64`], [`S_LPROCIA64`] and their `_ID` kinds.
    Ia64Procedure(Ia64Procedure<'a>),
    /// [`S_FRAMEPROC`].
    FrameProcedure(FrameProcedure),
    /// [`S_BLOCK32`].
    Block(Block<'a>),
    /// [`S_WITH32`].
    With(With<'a>),
    /// [`S_SEPCODE`].
    SeparatedCode(SeparatedCode),
    /// [`S_LABEL32`].
    Label(Label<'a>),
    /// [`S_THUNK32`].
    Thunk(Thunk<'a>),
    /// [`S_END`] and [`S_PROC_ID_END`]. These kinds have no fields.
    ScopeEnd,
    /// [`S_INLINESITE`] and [`S_INLINESITE2`].
    InlineSite(InlineSite<'a>),
    /// [`S_INLINESITE_END`]. This kind has no fields.
    InlineSiteEnd,
    /// [`S_INLINEES`]: the ID indices of the functions inlined into the procedure.
    Inlinees(IdList<'a>),
    /// [`S_CALLEES`]: the functions that the procedure calls.
    Callees(FunctionList<'a>),
    /// [`S_CALLERS`]: the functions that call the procedure.
    Callers(FunctionList<'a>),
    /// [`S_CALLSITEINFO`].
    CallSiteInfo(CallSiteInfo),
    /// [`S_ARMSWITCHTABLE`].
    JumpTable(JumpTable),
    /// [`S_POGODATA`].
    ProfileData(ProfileData),
    /// [`S_HOTPATCHFUNC`].
    HotPatchFunction(HotPatchFunction<'a>),
    /// [`S_ASSOCIATION`].
    Association(Association),
    /// [`S_HYBRIDRANGE`].
    HybridRange(HybridRange),
    /// [`S_ANNOTATION`].
    AnnotationStrings(AnnotationStrings<'a>),
    /// [`S_LOCAL`].
    Local(Local<'a>),
    /// [`S_DEFRANGE`] and [`S_DEFRANGE_SUBFIELD`].
    DefRangeProgram(DefRangeProgram<'a>),
    /// [`S_DEFRANGE_FRAMEPOINTER_REL`].
    DefRangeFramePointerRelative(DefRangeFramePointerRelative<'a>),
    /// [`S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE`]: the offset from the frame
    /// pointer. The location is valid in the full scope of the variable.
    DefRangeFramePointerRelativeFullScope(i32),
    /// [`S_DEFRANGE_REGISTER`].
    DefRangeRegister(DefRangeRegister<'a>),
    /// [`S_DEFRANGE_REGISTER_REL`].
    DefRangeRegisterRelative(DefRangeRegisterRelative<'a>),
    /// [`S_DEFRANGE_REGISTER_REL_INDIR`].
    DefRangeRegisterRelativeIndirect(DefRangeRegisterRelativeIndirect<'a>),
    /// [`S_DEFRANGE_SUBFIELD_REGISTER`].
    DefRangeSubfieldRegister(DefRangeSubfieldRegister<'a>),
    /// [`S_DEFRANGE_HLSL`] and [`S_DEFRANGE_DPC_PTR_TAG`].
    DefRangeHlsl(DefRangeHlsl<'a>),
    /// [`S_DEFRANGE_CONSTVAL_ON_ENTRY`].
    DefRangeConstantOnEntry(EntryValues<'a>),
    /// [`S_DEFRANGE_GLOBALSYM_ON_ENTRY`]: the address of the global symbol.
    DefRangeGlobalOnEntry(SectionOffset),
    /// [`S_REGREL32`] and [`S_REGREL32_ENCTMP`].
    RegisterRelative(RegisterRelative<'a>),
    /// [`S_REGREL32_INDIR`] and [`S_REGREL32_INDIR_ENCTMP`].
    RegisterRelativeIndirect(RegisterRelativeIndirect<'a>),
    /// [`S_BPREL32`] and [`S_BPREL32_ENCTMP`].
    BasePointerRelative(BasePointerRelative<'a>),
    /// [`S_BPREL32_INDIR`] and [`S_BPREL32_INDIR_ENCTMP`].
    BasePointerRelativeIndirect(BasePointerRelativeIndirect<'a>),
    /// [`S_REGISTER`].
    Register(RegisterVariable<'a>),
    /// [`S_MANYREG`], [`S_MANYREG2`], [`S_MANMANYREG`], [`S_MANMANYREG2`] and
    /// [`S_ATTR_MANYREG`].
    ManyRegisters(ManyRegisters<'a>),
    /// [`S_MANFRAMEREL`] and [`S_ATTR_FRAMEREL`].
    AttributedFrameRelative(AttributedFrameRelative<'a>),
    /// [`S_MANREGISTER`] and [`S_ATTR_REGISTER`].
    AttributedRegister(AttributedRegister<'a>),
    /// [`S_MANREGREL`] and [`S_ATTR_REGREL`].
    AttributedRegisterRelative(AttributedRegisterRelative<'a>),
    /// [`S_LOCALSLOT`] and [`S_PARAMSLOT`].
    Slot(Slot<'a>),
    /// [`S_FILESTATIC`].
    FileStatic(FileStatic<'a>),
    /// [`S_STATICLOCAL`].
    StaticLocal(StaticLocal),
    /// [`S_GTHREAD32`] and [`S_LTHREAD32`].
    ThreadData(Data<'a>),
    /// [`S_FRAMECOOKIE`].
    FrameCookie(FrameCookie),
    /// [`S_FRAMEREG`]: a register as a `CV_HREG_e` number for the CPU of the
    /// module.
    ///
    /// No reference gives the layout. The MSVC ARM64 fixture of #12 writes
    /// a 16-bit register and two zero bytes after `S_FRAMEPROC`, in
    /// procedures with exception handling and in their funclets (#13).
    FrameRegister(u16),
    /// [`S_HEAPALLOCSITE`].
    HeapAllocationSite(HeapAllocationSite),
    /// [`S_TRAMPOLINE`].
    Trampoline(Trampoline),
    /// [`S_FASTLINK`] and [`S_REF_MINIPDB`].
    FastLink(FastLink<'a>),
    /// [`S_SECTION`].
    Section(Section<'a>),
    /// [`S_COFFGROUP`].
    CoffGroup(CoffGroup<'a>),
    /// [`S_EXPORT`].
    Export(Export<'a>),
    /// [`S_DISCARDED`].
    Discarded(Discarded<'a>),
    /// [`S_MOD_TYPEREF`].
    ModuleTypeReference(ModuleTypeReference),
    /// [`S_PDBMAP`].
    PdbMap(PdbMap<'a>),
    /// [`S_UDT`] and [`S_COBOLUDT`].
    UserDefinedType(UserDefinedType<'a>),
    /// [`S_GDATA32`] and [`S_LDATA32`].
    Data(Data<'a>),
    /// [`S_GMANDATA`] and [`S_LMANDATA`]. The type field holds a metadata
    /// token.
    ManagedData(Data<'a>),
    /// [`S_GDATA_HLSL`], [`S_LDATA_HLSL`], [`S_GDATA_HLSL32`] and
    /// [`S_LDATA_HLSL32`].
    HlslData(HlslData<'a>),
    /// [`S_GDATA_HLSL32_EX`] and [`S_LDATA_HLSL32_EX`].
    HlslDataEx(HlslDataEx<'a>),
    /// [`S_LOCAL_DPC_GROUPSHARED`].
    DpcGroupSharedLocal(DpcGroupSharedLocal<'a>),
    /// [`S_DPC_SYM_TAG_MAP`].
    DpcTagMap(DpcTagMap<'a>),
    /// [`S_VFTABLE32`].
    VirtualFunctionTable(VirtualFunctionTable),
    /// [`S_CONSTANT`].
    Constant(Constant<'a>),
    /// [`S_PUB32`].
    Public(Public<'a>),
    /// [`S_PROCREF`], [`S_LPROCREF`] and [`S_TOKENREF`]. An `S_TOKENREF`
    /// record names a managed procedure.
    ProcedureReference(ProcedureReference<'a>),
    /// [`S_DATAREF`] and [`S_ANNOTATIONREF`]: a reference to a data or
    /// annotation record in a module, with the layout of
    /// [`SymbolData::ProcedureReference`].
    SymbolReference(ProcedureReference<'a>),
    /// [`S_GMANPROC`] and [`S_LMANPROC`].
    ManagedProcedure(ManagedProcedure<'a>),
    /// [`S_MANSLOT`].
    ManagedSlot(ManagedSlot<'a>),
    /// [`S_MANCONSTANT`].
    ManagedConstant(ManagedConstant<'a>),
    /// [`S_MANTYPREF`]: a type index.
    ManagedTypeReference(TypeIndex),
    /// [`S_OEM`].
    Oem(Oem<'a>),
    /// [`S_SKIP`]: filler bytes without fields.
    ///
    /// Microsoft tools use the record to reserve space for incremental
    /// builds. LLD replaces a record that it cannot remap with a zero-filled
    /// `S_SKIP` record of the same size. The value is the whole payload.
    Skip(&'a [u8]),
    /// A kind that this crate does not decode. The value is the raw payload
    /// after the kind field, with its padding.
    Unsupported(&'a [u8]),
}

impl<'a> Symbol<'a> {
    /// Decode `record` as a symbol.
    ///
    /// The bytes after the fields must be zero. Most producers write fewer
    /// than four of them to align the next record. The Windows SDK HLSL
    /// compiler `fxc` writes `S_LOCAL` records of a fixed length, so more
    /// zero bytes follow the name (#30). [`Symbol::encode`] keeps them.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the payload of a decoded kind
    /// is too short for its fields, has a name without a NUL terminator, has
    /// padding that is not zero, or breaks a rule of its kind. Returns
    /// [`ErrorKind::UnsupportedVersion`] for a numeric leaf that is not
    /// known. The error does not have a file offset.
    pub fn decode(record: Record<'a>) -> Result<Self> {
        let mut fields = Fields::new(record.data, ErrorKind::InvalidFormat, SHORT_PAYLOAD);
        let data = decode_data(SymbolKind(record.kind), &mut fields)?;
        let trailing_zeros = match data {
            SymbolData::Unsupported(_) | SymbolData::Skip(_) => 0,
            _ if fields.rest().iter().any(|&byte| byte != 0) => {
                return Err(crate::Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("CodeView symbol record has invalid padding"));
            }
            _ => fields.rest().len(),
        };
        Ok(Self {
            record,
            data,
            trailing_zeros,
        })
    }

    /// Return the number of zero bytes after the fields of the record.
    ///
    /// The value is usually below 4. [`SymbolData::Unsupported`] and
    /// [`SymbolData::Skip`] keep these bytes in their value and give 0.
    #[must_use]
    pub const fn trailing_zeros(&self) -> usize {
        self.trailing_zeros
    }

    /// Return the original record.
    #[must_use]
    pub const fn record(&self) -> Record<'a> {
        self.record
    }

    /// Return the record kind, for example [`S_GPROC32`].
    #[must_use]
    pub const fn kind(&self) -> SymbolKind {
        SymbolKind(self.record.kind)
    }

    /// Return the typed fields.
    #[must_use]
    pub const fn data(&self) -> &SymbolData<'a> {
        &self.data
    }
}

/// Decode the fields of `kind`. Do not check the padding.
fn decode_data<'a>(kind: SymbolKind, fields: &mut Fields<'a>) -> Result<SymbolData<'a>> {
    if let Some(data) = decode_variable(kind, fields)? {
        return Ok(data);
    }
    Ok(match kind {
        S_OBJNAME => SymbolData::ObjectName(ObjectName::read(fields)?),
        S_COMPILE2 => SymbolData::Compile2(Compile2::read(fields)?),
        S_COMPILE3 => SymbolData::Compile3(Compile3::read(fields)?),
        S_BUILDINFO => SymbolData::BuildInfo(fields.id_index()?),
        S_SOURCELINK => SymbolData::SourceLink(fields.id_index()?),
        S_ENVBLOCK => SymbolData::EnvironmentBlock(EnvironmentBlock::read(fields)?),
        S_UNAMESPACE => SymbolData::UsingNamespace(fields.name()?),
        S_GPROC32 | S_LPROC32 | S_LPROC32_DPC => {
            SymbolData::Procedure(Procedure::read(fields, false, false)?)
        }
        S_GPROC32_ID | S_LPROC32_ID | S_LPROC32_DPC_ID => {
            SymbolData::Procedure(Procedure::read(fields, true, false)?)
        }
        S_GPROC32EX | S_LPROC32EX => SymbolData::Procedure(Procedure::read(fields, false, true)?),
        S_GPROC32EX_ID | S_LPROC32EX_ID => {
            SymbolData::Procedure(Procedure::read(fields, true, true)?)
        }
        S_GPROCMIPS | S_LPROCMIPS => SymbolData::MipsProcedure(MipsProcedure::read(fields, false)?),
        S_GPROCMIPS_ID | S_LPROCMIPS_ID => {
            SymbolData::MipsProcedure(MipsProcedure::read(fields, true)?)
        }
        S_GPROCIA64 | S_LPROCIA64 => SymbolData::Ia64Procedure(Ia64Procedure::read(fields, false)?),
        S_GPROCIA64_ID | S_LPROCIA64_ID => {
            SymbolData::Ia64Procedure(Ia64Procedure::read(fields, true)?)
        }
        S_FRAMEPROC => SymbolData::FrameProcedure(FrameProcedure::read(fields)?),
        S_BLOCK32 => SymbolData::Block(Block::read(fields)?),
        S_WITH32 => SymbolData::With(With::read(fields)?),
        S_SEPCODE => SymbolData::SeparatedCode(SeparatedCode::read(fields)?),
        S_LABEL32 => SymbolData::Label(Label::read(fields)?),
        S_THUNK32 => SymbolData::Thunk(Thunk::read(fields)?),
        S_END | S_PROC_ID_END => SymbolData::ScopeEnd,
        S_INLINESITE => SymbolData::InlineSite(InlineSite::read(fields, false)?),
        S_INLINESITE2 => SymbolData::InlineSite(InlineSite::read(fields, true)?),
        S_INLINESITE_END => SymbolData::InlineSiteEnd,
        S_INLINEES => SymbolData::Inlinees(procedure::read_id_list(fields)?),
        S_CALLEES => SymbolData::Callees(FunctionList::read(fields)?),
        S_CALLERS => SymbolData::Callers(FunctionList::read(fields)?),
        S_CALLSITEINFO => SymbolData::CallSiteInfo(CallSiteInfo::read(fields)?),
        S_ARMSWITCHTABLE => SymbolData::JumpTable(JumpTable::read(fields)?),
        S_POGODATA => SymbolData::ProfileData(ProfileData::read(fields)?),
        S_HOTPATCHFUNC => SymbolData::HotPatchFunction(HotPatchFunction::read(fields)?),
        S_ASSOCIATION => SymbolData::Association(Association::read(fields)?),
        S_HYBRIDRANGE => SymbolData::HybridRange(HybridRange::read(fields)?),
        S_ANNOTATION => SymbolData::AnnotationStrings(AnnotationStrings::read(fields)?),
        S_FRAMECOOKIE => SymbolData::FrameCookie(FrameCookie::read(fields)?),
        S_FRAMEREG => SymbolData::FrameRegister(fields.u16()?),
        S_HEAPALLOCSITE => SymbolData::HeapAllocationSite(HeapAllocationSite::read(fields)?),
        S_TRAMPOLINE => SymbolData::Trampoline(Trampoline::read(fields)?),
        S_FASTLINK => SymbolData::FastLink(FastLink::read(fields, false)?),
        S_REF_MINIPDB => SymbolData::FastLink(FastLink::read(fields, true)?),
        S_SECTION => SymbolData::Section(Section::read(fields)?),
        S_COFFGROUP => SymbolData::CoffGroup(CoffGroup::read(fields)?),
        S_EXPORT => SymbolData::Export(Export::read(fields)?),
        S_DISCARDED => SymbolData::Discarded(Discarded::read(fields)?),
        S_MOD_TYPEREF => SymbolData::ModuleTypeReference(ModuleTypeReference::read(fields)?),
        S_PDBMAP => SymbolData::PdbMap(PdbMap::read(fields)?),
        S_VFTABLE32 => SymbolData::VirtualFunctionTable(VirtualFunctionTable::read(fields)?),
        S_PUB32 => SymbolData::Public(Public::read(fields)?),
        S_PROCREF | S_LPROCREF | S_TOKENREF => {
            SymbolData::ProcedureReference(ProcedureReference::read(fields)?)
        }
        S_DATAREF | S_ANNOTATIONREF => {
            SymbolData::SymbolReference(ProcedureReference::read(fields)?)
        }
        S_GMANPROC | S_LMANPROC => SymbolData::ManagedProcedure(ManagedProcedure::read(fields)?),
        S_MANTYPREF => SymbolData::ManagedTypeReference(fields.type_index()?),
        S_OEM => SymbolData::Oem(Oem::read(fields)?),
        S_SKIP => SymbolData::Skip(fields.rest()),
        _ => SymbolData::Unsupported(fields.rest()),
    })
}

/// Decode the fields of a variable, data or def-range kind.
///
/// Return `None` for the other kinds.
fn decode_variable<'a>(
    kind: SymbolKind,
    fields: &mut Fields<'a>,
) -> Result<Option<SymbolData<'a>>> {
    Ok(Some(match kind {
        S_LOCAL => SymbolData::Local(Local::read(fields)?),
        S_DEFRANGE => SymbolData::DefRangeProgram(DefRangeProgram::read(fields, false)?),
        S_DEFRANGE_SUBFIELD => SymbolData::DefRangeProgram(DefRangeProgram::read(fields, true)?),
        S_DEFRANGE_FRAMEPOINTER_REL => {
            SymbolData::DefRangeFramePointerRelative(DefRangeFramePointerRelative::read(fields)?)
        }
        S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE => {
            SymbolData::DefRangeFramePointerRelativeFullScope(fields.i32()?)
        }
        S_DEFRANGE_REGISTER => SymbolData::DefRangeRegister(DefRangeRegister::read(fields)?),
        S_DEFRANGE_REGISTER_REL => {
            SymbolData::DefRangeRegisterRelative(DefRangeRegisterRelative::read(fields)?)
        }
        S_DEFRANGE_REGISTER_REL_INDIR => SymbolData::DefRangeRegisterRelativeIndirect(
            DefRangeRegisterRelativeIndirect::read(fields)?,
        ),
        S_DEFRANGE_SUBFIELD_REGISTER => {
            SymbolData::DefRangeSubfieldRegister(DefRangeSubfieldRegister::read(fields)?)
        }
        S_DEFRANGE_HLSL | S_DEFRANGE_DPC_PTR_TAG => {
            SymbolData::DefRangeHlsl(DefRangeHlsl::read(fields)?)
        }
        S_DEFRANGE_CONSTVAL_ON_ENTRY => {
            SymbolData::DefRangeConstantOnEntry(EntryValues::read(fields)?)
        }
        S_DEFRANGE_GLOBALSYM_ON_ENTRY => {
            SymbolData::DefRangeGlobalOnEntry(read_global_on_entry(fields)?)
        }
        S_REGREL32 | S_REGREL32_ENCTMP => {
            SymbolData::RegisterRelative(RegisterRelative::read(fields)?)
        }
        S_REGREL32_INDIR | S_REGREL32_INDIR_ENCTMP => {
            SymbolData::RegisterRelativeIndirect(RegisterRelativeIndirect::read(fields)?)
        }
        S_BPREL32 | S_BPREL32_ENCTMP => {
            SymbolData::BasePointerRelative(BasePointerRelative::read(fields)?)
        }
        S_BPREL32_INDIR | S_BPREL32_INDIR_ENCTMP => {
            SymbolData::BasePointerRelativeIndirect(BasePointerRelativeIndirect::read(fields)?)
        }
        S_REGISTER => SymbolData::Register(RegisterVariable::read(fields)?),
        S_MANYREG => SymbolData::ManyRegisters(ManyRegisters::read(fields, false, false)?),
        S_MANYREG2 => SymbolData::ManyRegisters(ManyRegisters::read(fields, false, true)?),
        S_MANMANYREG => SymbolData::ManyRegisters(ManyRegisters::read(fields, true, false)?),
        S_MANMANYREG2 | S_ATTR_MANYREG => {
            SymbolData::ManyRegisters(ManyRegisters::read(fields, true, true)?)
        }
        S_MANFRAMEREL | S_ATTR_FRAMEREL => {
            SymbolData::AttributedFrameRelative(AttributedFrameRelative::read(fields)?)
        }
        S_MANREGISTER | S_ATTR_REGISTER => {
            SymbolData::AttributedRegister(AttributedRegister::read(fields)?)
        }
        S_MANREGREL | S_ATTR_REGREL => {
            SymbolData::AttributedRegisterRelative(AttributedRegisterRelative::read(fields)?)
        }
        S_LOCALSLOT | S_PARAMSLOT => SymbolData::Slot(Slot::read(fields)?),
        S_FILESTATIC => SymbolData::FileStatic(FileStatic::read(fields)?),
        S_STATICLOCAL => SymbolData::StaticLocal(StaticLocal::read(fields)?),
        S_GTHREAD32 | S_LTHREAD32 => SymbolData::ThreadData(Data::read(fields)?),
        S_UDT | S_COBOLUDT => SymbolData::UserDefinedType(UserDefinedType::read(fields)?),
        S_GDATA32 | S_LDATA32 => SymbolData::Data(Data::read(fields)?),
        S_GMANDATA | S_LMANDATA => SymbolData::ManagedData(Data::read(fields)?),
        S_GDATA_HLSL | S_LDATA_HLSL => SymbolData::HlslData(HlslData::read(fields, false)?),
        S_GDATA_HLSL32 | S_LDATA_HLSL32 => SymbolData::HlslData(HlslData::read(fields, true)?),
        S_GDATA_HLSL32_EX | S_LDATA_HLSL32_EX => SymbolData::HlslDataEx(HlslDataEx::read(fields)?),
        S_LOCAL_DPC_GROUPSHARED => {
            SymbolData::DpcGroupSharedLocal(DpcGroupSharedLocal::read(fields)?)
        }
        S_DPC_SYM_TAG_MAP => SymbolData::DpcTagMap(DpcTagMap::read(fields)?),
        S_CONSTANT => SymbolData::Constant(Constant::read(fields)?),
        S_MANSLOT => SymbolData::ManagedSlot(ManagedSlot::read(fields)?),
        S_MANCONSTANT => SymbolData::ManagedConstant(ManagedConstant::read(fields)?),
        _ => return Ok(None),
    }))
}
