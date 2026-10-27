//! Accessors for the fields that many symbol kinds share.

use super::super::Fields;
use super::super::index::{IdIndex, TypeIndex};
use super::{
    ProcedureReference, ProcedureType, S_MANFRAMEREL, S_MANMANYREG, S_MANMANYREG2, S_MANREGISTER,
    S_MANREGREL, SHORT_PAYLOAD, ScopeLinks, SectionOffset, Symbol, SymbolData,
};
use crate::{Bytes, ErrorKind};

impl<'a> Symbol<'a> {
    /// Return the name bytes, if the kind has a name.
    ///
    /// See [`SymbolData::name`].
    #[must_use]
    pub fn name(&self) -> Option<&Bytes<'a>> {
        self.data.name()
    }
}

/// Return a reference to the name field of a symbol data value, or `None`.
/// `name` and `name_mut` share this list of kinds.
macro_rules! name_field {
    ($data:expr, $($mutability:tt)?) => {
        Some(match $data {
            SymbolData::ObjectName(value) => &$($mutability)? value.name,
            SymbolData::UsingNamespace(name) => name,
            SymbolData::Procedure(value) => &$($mutability)? value.name,
            SymbolData::MipsProcedure(value) => &$($mutability)? value.name,
            SymbolData::Ia64Procedure(value) => &$($mutability)? value.name,
            SymbolData::Block(value) => &$($mutability)? value.name,
            SymbolData::Label(value) => &$($mutability)? value.name,
            SymbolData::Thunk(value) => &$($mutability)? value.name,
            SymbolData::HotPatchFunction(value) => &$($mutability)? value.name,
            SymbolData::Local(value) => &$($mutability)? value.name,
            SymbolData::RegisterRelative(value) => &$($mutability)? value.name,
            SymbolData::RegisterRelativeIndirect(value) => &$($mutability)? value.name,
            SymbolData::BasePointerRelative(value) => &$($mutability)? value.name,
            SymbolData::BasePointerRelativeIndirect(value) => &$($mutability)? value.name,
            SymbolData::Register(value) => &$($mutability)? value.name,
            SymbolData::ManyRegisters(value) => &$($mutability)? value.name,
            SymbolData::AttributedFrameRelative(value) => &$($mutability)? value.name,
            SymbolData::AttributedRegister(value) => &$($mutability)? value.name,
            SymbolData::AttributedRegisterRelative(value) => &$($mutability)? value.name,
            SymbolData::Slot(value) => &$($mutability)? value.name,
            SymbolData::FileStatic(value) => &$($mutability)? value.name,
            SymbolData::FastLink(value) => &$($mutability)? value.name,
            SymbolData::Section(value) => &$($mutability)? value.name,
            SymbolData::CoffGroup(value) => &$($mutability)? value.name,
            SymbolData::Export(value) => &$($mutability)? value.name,
            SymbolData::UserDefinedType(value) => &$($mutability)? value.name,
            SymbolData::Data(value)
            | SymbolData::ThreadData(value)
            | SymbolData::ManagedData(value) => &$($mutability)? value.name,
            SymbolData::HlslData(value) => &$($mutability)? value.name,
            SymbolData::HlslDataEx(value) => &$($mutability)? value.name,
            SymbolData::DpcGroupSharedLocal(value) => &$($mutability)? value.name,
            SymbolData::Constant(value) => &$($mutability)? value.name,
            SymbolData::Public(value) => &$($mutability)? value.name,
            SymbolData::ProcedureReference(value) | SymbolData::SymbolReference(value) => {
                &$($mutability)? value.name
            }
            SymbolData::ManagedProcedure(value) => &$($mutability)? value.name,
            SymbolData::ManagedSlot(value) => &$($mutability)? value.name,
            SymbolData::ManagedConstant(value) => &$($mutability)? value.name,
            _ => return None,
        })
    };
}

impl<'a> SymbolData<'a> {
    /// Return the name bytes, if the kind has a name.
    ///
    /// The bytes do not include the NUL terminator. `S_OBJNAME` gives the
    /// object file path, `S_UNAMESPACE` gives the namespace and `S_EXPORT`
    /// gives the exported name. Compile records have a version string, not a
    /// name, so they give `None`. `S_WITH32` has an expression, not a name.
    #[must_use]
    pub fn name(&self) -> Option<&Bytes<'a>> {
        name_field!(self,)
    }

    /// Return the name bytes for a change, if the kind has a name.
    ///
    /// The kinds are the kinds of [`SymbolData::name`]. The encoder writes
    /// the name with a NUL terminator, so the name must not contain a NUL
    /// byte.
    pub fn name_mut(&mut self) -> Option<&mut Bytes<'a>> {
        name_field!(self, mut)
    }
}

impl<'a> Symbol<'a> {
    /// Return the start address, if the kind has one address.
    ///
    /// Procedures, blocks, `with` blocks, separated code, labels, thunks,
    /// data, thread data, publics, COFF groups, call sites, heap allocation
    /// sites, static locals, annotations, associations, hybrid ranges and
    /// virtual function tables have an address. `S_TRAMPOLINE` gives the
    /// address of the thunk. `S_ARMSWITCHTABLE` has three addresses, and
    /// def-ranges have address ranges. Read these values from
    /// [`Symbol::data`].
    #[must_use]
    pub const fn address(&self) -> Option<SectionOffset> {
        Some(match &self.data {
            SymbolData::Procedure(value) => value.address,
            SymbolData::MipsProcedure(value) => value.address,
            SymbolData::Ia64Procedure(value) => value.address,
            SymbolData::Block(value) => value.address,
            SymbolData::With(value) => value.address,
            SymbolData::SeparatedCode(value) => value.address,
            SymbolData::Label(value) => value.address,
            SymbolData::Thunk(value) => value.address,
            SymbolData::HeapAllocationSite(value) => value.address,
            SymbolData::Trampoline(value) => value.thunk,
            SymbolData::Data(value)
            | SymbolData::ThreadData(value)
            | SymbolData::ManagedData(value) => value.address,
            SymbolData::Public(value) => value.address,
            SymbolData::CoffGroup(value) => value.address,
            SymbolData::CallSiteInfo(value) => value.address,
            SymbolData::ManagedProcedure(value) => value.address,
            SymbolData::StaticLocal(value) => value.address,
            SymbolData::AnnotationStrings(value) => value.address,
            SymbolData::Association(value) => value.address,
            SymbolData::HybridRange(value) => value.address,
            SymbolData::VirtualFunctionTable(value) => value.address,
            _ => return None,
        })
    }

    /// Return the index in the TPI stream, if the kind has one.
    ///
    /// Procedures without the `_ID` suffix, variables, user-defined types,
    /// data, constants, call sites and heap allocation sites have a type
    /// index. The managed kinds `S_MANFRAMEREL`, `S_MANREGISTER`,
    /// `S_MANREGREL`, `S_MANMANYREG`, `S_MANMANYREG2` and `S_GMANDATA` store
    /// a metadata token, and slots store a type index or a token, so this
    /// function gives `None` for them. The type index of `S_FASTLINK`
    /// refers to the object file, so this function gives `None` for it.
    #[must_use]
    pub const fn type_index(&self) -> Option<TypeIndex> {
        let kind = self.kind();
        let managed = matches!(
            kind,
            S_MANFRAMEREL | S_MANREGISTER | S_MANREGREL | S_MANMANYREG | S_MANMANYREG2
        );
        let index = match &self.data {
            SymbolData::Procedure(value) => value.signature,
            SymbolData::MipsProcedure(value) => value.signature,
            SymbolData::Ia64Procedure(value) => value.signature,
            SymbolData::Local(value) => ProcedureType::Type(value.type_index),
            SymbolData::RegisterRelative(value) => ProcedureType::Type(value.type_index),
            SymbolData::RegisterRelativeIndirect(value) => ProcedureType::Type(value.type_index),
            SymbolData::BasePointerRelative(value) => ProcedureType::Type(value.type_index),
            SymbolData::BasePointerRelativeIndirect(value) => ProcedureType::Type(value.type_index),
            SymbolData::Register(value) => ProcedureType::Type(value.type_index),
            SymbolData::ManyRegisters(value) if !managed => ProcedureType::Type(value.type_index),
            SymbolData::AttributedFrameRelative(value) if !managed => {
                ProcedureType::Type(value.type_index)
            }
            SymbolData::AttributedRegister(value) if !managed => {
                ProcedureType::Type(value.type_index)
            }
            SymbolData::AttributedRegisterRelative(value) if !managed => {
                ProcedureType::Type(value.type_index)
            }
            SymbolData::FileStatic(value) => ProcedureType::Type(value.type_index),
            SymbolData::HeapAllocationSite(value) => ProcedureType::Type(value.type_index),
            SymbolData::UserDefinedType(value) => ProcedureType::Type(value.type_index),
            SymbolData::Data(value) | SymbolData::ThreadData(value) => {
                ProcedureType::Type(value.type_index)
            }
            SymbolData::HlslData(value) => ProcedureType::Type(value.type_index),
            SymbolData::HlslDataEx(value) => ProcedureType::Type(value.type_index),
            SymbolData::DpcGroupSharedLocal(value) => ProcedureType::Type(value.type_index),
            SymbolData::Constant(value) => ProcedureType::Type(value.type_index),
            SymbolData::CallSiteInfo(value) => ProcedureType::Type(value.type_index),
            SymbolData::Oem(value) => ProcedureType::Type(value.type_index),
            SymbolData::ManagedTypeReference(index) => ProcedureType::Type(*index),
            _ => return None,
        };
        match index {
            ProcedureType::Type(index) => Some(index),
            ProcedureType::Id(_) => None,
        }
    }

    /// Return the index in the IPI stream, if the kind has one.
    ///
    /// The `_ID` procedure kinds give the function ID, inline sites give the
    /// inlinee, `S_HOTPATCHFUNC` gives the function, and `S_BUILDINFO` and
    /// `S_SOURCELINK` give their ID records.
    #[must_use]
    pub const fn id_index(&self) -> Option<IdIndex> {
        let signature = match &self.data {
            SymbolData::Procedure(value) => value.signature,
            SymbolData::MipsProcedure(value) => value.signature,
            SymbolData::Ia64Procedure(value) => value.signature,
            SymbolData::BuildInfo(index) | SymbolData::SourceLink(index) => {
                ProcedureType::Id(*index)
            }
            SymbolData::InlineSite(value) => ProcedureType::Id(value.inlinee),
            SymbolData::HotPatchFunction(value) => ProcedureType::Id(value.function),
            _ => return None,
        };
        match signature {
            ProcedureType::Id(index) => Some(index),
            ProcedureType::Type(_) => None,
        }
    }

    /// Return the module and offset of the referenced procedure.
    ///
    /// Only `S_PROCREF`, `S_LPROCREF` and `S_TOKENREF` give a value. Use
    /// [`Symbol::reference`] for all reference kinds.
    #[must_use]
    pub const fn procedure_reference(&self) -> Option<&ProcedureReference<'a>> {
        match &self.data {
            SymbolData::ProcedureReference(value) => Some(value),
            _ => None,
        }
    }

    /// Return the module and offset of the referenced record.
    ///
    /// `S_PROCREF`, `S_LPROCREF`, `S_TOKENREF`, `S_DATAREF` and
    /// `S_ANNOTATIONREF` give a value.
    #[must_use]
    pub const fn reference(&self) -> Option<&ProcedureReference<'a>> {
        match &self.data {
            SymbolData::ProcedureReference(value) | SymbolData::SymbolReference(value) => {
                Some(value)
            }
            _ => None,
        }
    }

    /// Return the parent and end links, if the record starts a scope.
    ///
    /// Procedures of all architectures, managed procedures, blocks, `with`
    /// blocks, separated code, thunks and inline sites start a scope. The
    /// links are the first two `u32` fields of the source record. The
    /// typed fields do not hold them, because they are positions in the
    /// module stream.
    #[must_use]
    pub fn links(&self) -> Option<ScopeLinks> {
        if !self.data.starts_scope() {
            return None;
        }
        // The decoder read the two links, so the payload holds them.
        let mut fields = Fields::new(&self.payload, ErrorKind::InvalidFormat, SHORT_PAYLOAD);
        ScopeLinks::read(&mut fields).ok()
    }
}

impl SymbolData<'_> {
    /// Return `true` if the fields belong to a record that starts a scope.
    ///
    /// See [`Symbol::links`].
    pub(crate) const fn starts_scope(&self) -> bool {
        matches!(
            self,
            SymbolData::Procedure(_)
                | SymbolData::MipsProcedure(_)
                | SymbolData::Ia64Procedure(_)
                | SymbolData::Block(_)
                | SymbolData::With(_)
                | SymbolData::SeparatedCode(_)
                | SymbolData::Thunk(_)
                | SymbolData::InlineSite(_)
                | SymbolData::ManagedProcedure(_)
        )
    }
}
