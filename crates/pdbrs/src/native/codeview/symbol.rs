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

use super::index::{IdIndex, TypeIndex};
use super::{Fields, Record};
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
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub struct $name(pub $raw);

        impl $name {
            $($(#[$value_meta])* pub const $value_name: Self = Self($value);)*
        }
    };
}

mod annotation;
mod compile;
mod global;
mod procedure;
mod scope;
mod variable;

pub use annotation::{Annotation, Annotations, BinaryAnnotations};
pub use compile::{
    Compile2, Compile3, CompileFlags, EnvironmentBlock, ObjectName, SourceLanguage, StringList,
    Strings,
};
pub use global::{CoffGroup, ProcedureReference, Public, PublicFlags, Section};
pub use procedure::{
    Block, CallSiteInfo, FramePointer, FrameProcedure, FrameProcedureFlags, IdList, IdListIter,
    InlineSite, JumpTable, JumpTableEntry, Label, Procedure, ProcedureFlags, ProcedureType, Thunk,
    ThunkOrdinal,
};
pub use scope::{ModuleSymbols, ScopedSymbol};
pub use variable::{
    AddressGap, AddressGapIter, AddressGaps, AddressRange, Constant, Data,
    DefRangeFramePointerRelative, DefRangeRegister, DefRangeRegisterRelative,
    DefRangeRegisterRelativeIndirect, Local, LocalFlags, RegisterRelative, UserDefinedType,
};

/// The kind of the end of a procedure, block or thunk scope (`S_END`).
pub const S_END: u16 = 0x0006;
/// The kind of a frame procedure record (`S_FRAMEPROC`).
pub const S_FRAMEPROC: u16 = 0x1012;
/// The kind of an object file name (`S_OBJNAME`).
pub const S_OBJNAME: u16 = 0x1101;
/// The kind of a thunk start (`S_THUNK32`).
pub const S_THUNK32: u16 = 0x1102;
/// The kind of a block start (`S_BLOCK32`).
pub const S_BLOCK32: u16 = 0x1103;
/// The kind of a code label (`S_LABEL32`).
pub const S_LABEL32: u16 = 0x1105;
/// The kind of a constant (`S_CONSTANT`).
pub const S_CONSTANT: u16 = 0x1107;
/// The kind of a user-defined type name (`S_UDT`).
pub const S_UDT: u16 = 0x1108;
/// The kind of a module-local data symbol (`S_LDATA32`).
pub const S_LDATA32: u16 = 0x110c;
/// The kind of a global data symbol (`S_GDATA32`).
pub const S_GDATA32: u16 = 0x110d;
/// The kind of a public symbol (`S_PUB32`).
pub const S_PUB32: u16 = 0x110e;
/// The kind of a module-local procedure start (`S_LPROC32`).
pub const S_LPROC32: u16 = 0x110f;
/// The kind of a global procedure start (`S_GPROC32`).
pub const S_GPROC32: u16 = 0x1110;
/// The kind of a register-relative variable (`S_REGREL32`).
pub const S_REGREL32: u16 = 0x1111;
/// The kind of a compile record, version 2 (`S_COMPILE2`).
pub const S_COMPILE2: u16 = 0x1116;
/// The kind of a using-namespace record (`S_UNAMESPACE`).
pub const S_UNAMESPACE: u16 = 0x1124;
/// The kind of a reference to a global procedure (`S_PROCREF`).
pub const S_PROCREF: u16 = 0x1125;
/// The kind of a reference to a module-local procedure (`S_LPROCREF`).
pub const S_LPROCREF: u16 = 0x1127;
/// The kind of an image section record (`S_SECTION`).
pub const S_SECTION: u16 = 0x1136;
/// The kind of a COFF group record (`S_COFFGROUP`).
pub const S_COFFGROUP: u16 = 0x1137;
/// The kind of a call site record (`S_CALLSITEINFO`).
pub const S_CALLSITEINFO: u16 = 0x1139;
/// The kind of a compile record, version 3 (`S_COMPILE3`).
pub const S_COMPILE3: u16 = 0x113c;
/// The kind of a build environment block (`S_ENVBLOCK`).
pub const S_ENVBLOCK: u16 = 0x113d;
/// The kind of a local variable (`S_LOCAL`).
pub const S_LOCAL: u16 = 0x113e;
/// The kind of a register def-range (`S_DEFRANGE_REGISTER`).
pub const S_DEFRANGE_REGISTER: u16 = 0x1141;
/// The kind of a frame-pointer-relative def-range (`S_DEFRANGE_FRAMEPOINTER_REL`).
pub const S_DEFRANGE_FRAMEPOINTER_REL: u16 = 0x1142;
/// The kind of a full-scope frame-pointer-relative def-range
/// (`S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE`).
pub const S_DEFRANGE_FRAMEPOINTER_REL_FULL_SCOPE: u16 = 0x1144;
/// The kind of a register-relative def-range (`S_DEFRANGE_REGISTER_REL`).
pub const S_DEFRANGE_REGISTER_REL: u16 = 0x1145;
/// The kind of a module-local procedure start with an ID index (`S_LPROC32_ID`).
pub const S_LPROC32_ID: u16 = 0x1146;
/// The kind of a global procedure start with an ID index (`S_GPROC32_ID`).
pub const S_GPROC32_ID: u16 = 0x1147;
/// The kind of a build information reference (`S_BUILDINFO`).
pub const S_BUILDINFO: u16 = 0x114c;
/// The kind of an inline site start (`S_INLINESITE`).
pub const S_INLINESITE: u16 = 0x114d;
/// The kind of the end of an inline site scope (`S_INLINESITE_END`).
pub const S_INLINESITE_END: u16 = 0x114e;
/// The kind of the end of a procedure scope with an ID index (`S_PROC_ID_END`).
pub const S_PROC_ID_END: u16 = 0x114f;
/// The kind of a jump table record (`S_ARMSWITCHTABLE`).
pub const S_ARMSWITCHTABLE: u16 = 0x1159;
/// The kind of a list of callees (`S_CALLEES`).
pub const S_CALLEES: u16 = 0x115a;
/// The kind of a list of inlinees (`S_INLINEES`).
pub const S_INLINEES: u16 = 0x1168;
/// The kind of an indirect register-relative def-range
/// (`S_DEFRANGE_REGISTER_REL_INDIR`).
pub const S_DEFRANGE_REGISTER_REL_INDIR: u16 = 0x1177;

/// The error context for a payload that is too short for its fields.
const SHORT_PAYLOAD: &str = "symbol record payload is shorter than its fields";

/// A section number and an offset in that section.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct SectionOffset {
    /// The section number. Section numbers start at 1. The value 0 can
    /// identify an absolute or discarded symbol.
    pub section: u16,
    /// The offset in the section.
    pub offset: u32,
}

impl SectionOffset {
    /// Read a `u32` offset and then a `u16` section number.
    fn read(fields: &mut Fields<'_>) -> Result<Self> {
        let offset = fields.u32()?;
        let section = fields.u16()?;
        Ok(Self { section, offset })
    }
}

/// The parent and end links of a record that starts a scope.
///
/// Both values are offsets in the module stream. They include the 4-byte
/// signature at the start of the stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ScopeLinks {
    /// The offset of the record of the enclosing scope, or 0 at the top level.
    pub parent: u32,
    /// The offset of the record that ends this scope.
    pub end: u32,
}

/// A decoded symbol record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Symbol<'a> {
    /// The original record.
    record: Record<'a>,
    /// The typed fields of the kind.
    data: SymbolData<'a>,
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
    /// [`S_ENVBLOCK`].
    EnvironmentBlock(EnvironmentBlock<'a>),
    /// [`S_UNAMESPACE`]: the namespace name.
    UsingNamespace(&'a [u8]),
    /// [`S_GPROC32`], [`S_LPROC32`], [`S_GPROC32_ID`] and [`S_LPROC32_ID`].
    Procedure(Procedure<'a>),
    /// [`S_FRAMEPROC`].
    FrameProcedure(FrameProcedure),
    /// [`S_BLOCK32`].
    Block(Block<'a>),
    /// [`S_LABEL32`].
    Label(Label<'a>),
    /// [`S_THUNK32`].
    Thunk(Thunk<'a>),
    /// [`S_END`] and [`S_PROC_ID_END`]. These kinds have no fields.
    ScopeEnd,
    /// [`S_INLINESITE`].
    InlineSite(InlineSite<'a>),
    /// [`S_INLINESITE_END`]. This kind has no fields.
    InlineSiteEnd,
    /// [`S_INLINEES`]: the ID indices of the functions inlined into the procedure.
    Inlinees(IdList<'a>),
    /// [`S_CALLEES`]: the ID indices of the functions that the procedure calls.
    Callees(IdList<'a>),
    /// [`S_CALLSITEINFO`].
    CallSiteInfo(CallSiteInfo),
    /// [`S_ARMSWITCHTABLE`].
    JumpTable(JumpTable),
    /// [`S_LOCAL`].
    Local(Local<'a>),
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
    /// [`S_REGREL32`].
    RegisterRelative(RegisterRelative<'a>),
    /// [`S_SECTION`].
    Section(Section<'a>),
    /// [`S_COFFGROUP`].
    CoffGroup(CoffGroup<'a>),
    /// [`S_UDT`].
    UserDefinedType(UserDefinedType<'a>),
    /// [`S_GDATA32`] and [`S_LDATA32`].
    Data(Data<'a>),
    /// [`S_CONSTANT`].
    Constant(Constant<'a>),
    /// [`S_PUB32`].
    Public(Public<'a>),
    /// [`S_PROCREF`] and [`S_LPROCREF`].
    ProcedureReference(ProcedureReference<'a>),
    /// A kind that this crate does not decode. The value is the raw payload
    /// after the kind field, with its padding.
    Unsupported(&'a [u8]),
}

impl<'a> Symbol<'a> {
    /// Decode `record` as a symbol.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the payload of a decoded kind
    /// is too short for its fields, has a name without a NUL terminator, has
    /// padding that is not zero, or breaks a rule of its kind. Returns
    /// [`ErrorKind::UnsupportedVersion`] for a constant value that is not an
    /// integer. The error does not have a file offset.
    pub fn decode(record: Record<'a>) -> Result<Self> {
        let mut fields = Fields::new(record.data, ErrorKind::InvalidFormat, SHORT_PAYLOAD);
        let data = decode_data(record.kind, &mut fields)?;
        if !matches!(data, SymbolData::Unsupported(_)) {
            fields.finish_zero_padding()?;
        }
        Ok(Self { record, data })
    }

    /// Return the original record.
    #[must_use]
    pub const fn record(&self) -> Record<'a> {
        self.record
    }

    /// Return the record kind, for example [`S_GPROC32`].
    #[must_use]
    pub const fn kind(&self) -> u16 {
        self.record.kind
    }

    /// Return the typed fields.
    #[must_use]
    pub const fn data(&self) -> &SymbolData<'a> {
        &self.data
    }

    /// Return the name bytes, if the kind has a name.
    ///
    /// The bytes do not include the NUL terminator. [`S_OBJNAME`] gives the
    /// object file path and [`S_UNAMESPACE`] gives the namespace. Compile
    /// records have a version string, not a name, so they give `None`.
    #[must_use]
    pub const fn name(&self) -> Option<&'a [u8]> {
        match self.data {
            SymbolData::ObjectName(value) => Some(value.name),
            SymbolData::UsingNamespace(name) => Some(name),
            SymbolData::Procedure(value) => Some(value.name),
            SymbolData::Block(value) => Some(value.name),
            SymbolData::Label(value) => Some(value.name),
            SymbolData::Thunk(value) => Some(value.name),
            SymbolData::Local(value) => Some(value.name),
            SymbolData::RegisterRelative(value) => Some(value.name),
            SymbolData::Section(value) => Some(value.name),
            SymbolData::CoffGroup(value) => Some(value.name),
            SymbolData::UserDefinedType(value) => Some(value.name),
            SymbolData::Data(value) => Some(value.name),
            SymbolData::Constant(value) => Some(value.name),
            SymbolData::Public(value) => Some(value.name),
            SymbolData::ProcedureReference(value) => Some(value.name),
            _ => None,
        }
    }

    /// Return the start address, if the kind has one address.
    ///
    /// Procedures, blocks, labels, thunks, data, publics, COFF groups and
    /// call sites have an address. [`S_ARMSWITCHTABLE`] has three
    /// addresses, and def-ranges have address ranges. Read these values from
    /// [`Symbol::data`].
    #[must_use]
    pub const fn address(&self) -> Option<SectionOffset> {
        match self.data {
            SymbolData::Procedure(value) => Some(value.address),
            SymbolData::Block(value) => Some(value.address),
            SymbolData::Label(value) => Some(value.address),
            SymbolData::Thunk(value) => Some(value.address),
            SymbolData::Data(value) => Some(value.address),
            SymbolData::Public(value) => Some(value.address),
            SymbolData::CoffGroup(value) => Some(value.address),
            SymbolData::CallSiteInfo(value) => Some(value.address),
            _ => None,
        }
    }

    /// Return the index in the TPI stream, if the kind has one.
    ///
    /// Procedures without the `_ID` suffix, locals, register-relative
    /// variables, user-defined types, data, constants and call sites have a
    /// type index.
    #[must_use]
    pub const fn type_index(&self) -> Option<TypeIndex> {
        match self.data {
            SymbolData::Procedure(Procedure {
                signature: ProcedureType::Type(index),
                ..
            }) => Some(index),
            SymbolData::Local(value) => Some(value.type_index),
            SymbolData::RegisterRelative(value) => Some(value.type_index),
            SymbolData::UserDefinedType(value) => Some(value.type_index),
            SymbolData::Data(value) => Some(value.type_index),
            SymbolData::Constant(value) => Some(value.type_index),
            SymbolData::CallSiteInfo(value) => Some(value.type_index),
            _ => None,
        }
    }

    /// Return the index in the IPI stream, if the kind has one.
    ///
    /// [`S_GPROC32_ID`] and [`S_LPROC32_ID`] give the function ID,
    /// [`S_INLINESITE`] gives the inlinee and [`S_BUILDINFO`] gives the
    /// build information record.
    #[must_use]
    pub const fn id_index(&self) -> Option<IdIndex> {
        match self.data {
            SymbolData::Procedure(Procedure {
                signature: ProcedureType::Id(index),
                ..
            })
            | SymbolData::BuildInfo(index) => Some(index),
            SymbolData::InlineSite(value) => Some(value.inlinee),
            _ => None,
        }
    }

    /// Return the module and offset of the referenced procedure.
    ///
    /// Only [`S_PROCREF`] and [`S_LPROCREF`] give a value.
    #[must_use]
    pub const fn procedure_reference(&self) -> Option<ProcedureReference<'a>> {
        match self.data {
            SymbolData::ProcedureReference(value) => Some(value),
            _ => None,
        }
    }

    /// Return the parent and end links, if the record starts a scope.
    ///
    /// Procedures, blocks, thunks and inline sites start a scope.
    #[must_use]
    pub const fn scope(&self) -> Option<ScopeLinks> {
        let (parent, end) = match self.data {
            SymbolData::Procedure(value) => (value.parent, value.end),
            SymbolData::Block(value) => (value.parent, value.end),
            SymbolData::Thunk(value) => (value.parent, value.end),
            SymbolData::InlineSite(value) => (value.parent, value.end),
            _ => return None,
        };
        Some(ScopeLinks { parent, end })
    }
}

/// Decode the fields of `kind`. Do not check the padding.
fn decode_data<'a>(kind: u16, fields: &mut Fields<'a>) -> Result<SymbolData<'a>> {
    Ok(match kind {
        S_OBJNAME => SymbolData::ObjectName(ObjectName::read(fields)?),
        S_COMPILE2 => SymbolData::Compile2(Compile2::read(fields)?),
        S_COMPILE3 => SymbolData::Compile3(Compile3::read(fields)?),
        S_BUILDINFO => SymbolData::BuildInfo(fields.id_index()?),
        S_ENVBLOCK => SymbolData::EnvironmentBlock(EnvironmentBlock::read(fields)?),
        S_UNAMESPACE => SymbolData::UsingNamespace(fields.name()?),
        S_GPROC32 | S_LPROC32 => SymbolData::Procedure(Procedure::read(fields, false)?),
        S_GPROC32_ID | S_LPROC32_ID => SymbolData::Procedure(Procedure::read(fields, true)?),
        S_FRAMEPROC => SymbolData::FrameProcedure(FrameProcedure::read(fields)?),
        S_BLOCK32 => SymbolData::Block(Block::read(fields)?),
        S_LABEL32 => SymbolData::Label(Label::read(fields)?),
        S_THUNK32 => SymbolData::Thunk(Thunk::read(fields)?),
        S_END | S_PROC_ID_END => SymbolData::ScopeEnd,
        S_INLINESITE => SymbolData::InlineSite(InlineSite::read(fields)?),
        S_INLINESITE_END => SymbolData::InlineSiteEnd,
        S_INLINEES => SymbolData::Inlinees(IdList::read(fields)?),
        S_CALLEES => SymbolData::Callees(IdList::read(fields)?),
        S_CALLSITEINFO => SymbolData::CallSiteInfo(CallSiteInfo::read(fields)?),
        S_ARMSWITCHTABLE => SymbolData::JumpTable(JumpTable::read(fields)?),
        S_LOCAL => SymbolData::Local(Local::read(fields)?),
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
        S_REGREL32 => SymbolData::RegisterRelative(RegisterRelative::read(fields)?),
        S_SECTION => SymbolData::Section(Section::read(fields)?),
        S_COFFGROUP => SymbolData::CoffGroup(CoffGroup::read(fields)?),
        S_UDT => SymbolData::UserDefinedType(UserDefinedType::read(fields)?),
        S_GDATA32 | S_LDATA32 => SymbolData::Data(Data::read(fields)?),
        S_CONSTANT => SymbolData::Constant(Constant::read(fields)?),
        S_PUB32 => SymbolData::Public(Public::read(fields)?),
        S_PROCREF | S_LPROCREF => SymbolData::ProcedureReference(ProcedureReference::read(fields)?),
        _ => SymbolData::Unsupported(fields.rest()),
    })
}
