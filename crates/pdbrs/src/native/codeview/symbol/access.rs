//! Accessors for the fields that many symbol kinds share.

use super::super::index::{IdIndex, TypeIndex};
use super::{
    ProcedureReference, ProcedureType, S_MANFRAMEREL, S_MANMANYREG, S_MANMANYREG2, S_MANREGISTER,
    S_MANREGREL, ScopeLinks, SectionOffset, Symbol, SymbolData,
};
use crate::Bytes;

impl<'a> Symbol<'a> {
    /// Return the name bytes, if the kind has a name.
    ///
    /// The bytes do not include the NUL terminator. `S_OBJNAME` gives the
    /// object file path, `S_UNAMESPACE` gives the namespace and `S_EXPORT`
    /// gives the exported name. Compile records have a version string, not a
    /// name, so they give `None`. `S_WITH32` has an expression, not a name.
    #[must_use]
    pub fn name(&self) -> Option<&Bytes<'a>> {
        let name = match &self.data {
            SymbolData::ObjectName(value) => &value.name,
            SymbolData::UsingNamespace(name) => name,
            SymbolData::Procedure(value) => &value.name,
            SymbolData::MipsProcedure(value) => &value.name,
            SymbolData::Ia64Procedure(value) => &value.name,
            SymbolData::Block(value) => &value.name,
            SymbolData::Label(value) => &value.name,
            SymbolData::Thunk(value) => &value.name,
            SymbolData::HotPatchFunction(value) => &value.name,
            SymbolData::Local(value) => &value.name,
            SymbolData::RegisterRelative(value) => &value.name,
            SymbolData::RegisterRelativeIndirect(value) => &value.name,
            SymbolData::BasePointerRelative(value) => &value.name,
            SymbolData::BasePointerRelativeIndirect(value) => &value.name,
            SymbolData::Register(value) => &value.name,
            SymbolData::ManyRegisters(value) => &value.name,
            SymbolData::AttributedFrameRelative(value) => &value.name,
            SymbolData::AttributedRegister(value) => &value.name,
            SymbolData::AttributedRegisterRelative(value) => &value.name,
            SymbolData::Slot(value) => &value.name,
            SymbolData::FileStatic(value) => &value.name,
            SymbolData::FastLink(value) => &value.name,
            SymbolData::Section(value) => &value.name,
            SymbolData::CoffGroup(value) => &value.name,
            SymbolData::Export(value) => &value.name,
            SymbolData::UserDefinedType(value) => &value.name,
            SymbolData::Data(value)
            | SymbolData::ThreadData(value)
            | SymbolData::ManagedData(value) => &value.name,
            SymbolData::HlslData(value) => &value.name,
            SymbolData::HlslDataEx(value) => &value.name,
            SymbolData::DpcGroupSharedLocal(value) => &value.name,
            SymbolData::Constant(value) => &value.name,
            SymbolData::Public(value) => &value.name,
            SymbolData::ProcedureReference(value) | SymbolData::SymbolReference(value) => {
                &value.name
            }
            SymbolData::ManagedProcedure(value) => &value.name,
            SymbolData::ManagedSlot(value) => &value.name,
            SymbolData::ManagedConstant(value) => &value.name,
            _ => return None,
        };
        Some(name)
    }

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
    /// blocks, separated code, thunks and inline sites start a scope.
    #[must_use]
    pub const fn scope_links(&self) -> Option<ScopeLinks> {
        self.data.scope_links()
    }
}

impl SymbolData<'_> {
    /// Return the parent and end links, if the fields start a scope.
    ///
    /// See [`Symbol::scope_links`].
    #[must_use]
    pub const fn scope_links(&self) -> Option<ScopeLinks> {
        let (parent, end) = match self {
            SymbolData::Procedure(value) => (value.parent, value.end),
            SymbolData::MipsProcedure(value) => (value.header.parent, value.header.end),
            SymbolData::Ia64Procedure(value) => (value.header.parent, value.header.end),
            SymbolData::Block(value) => (value.parent, value.end),
            SymbolData::With(value) => (value.parent, value.end),
            SymbolData::SeparatedCode(value) => (value.parent, value.end),
            SymbolData::Thunk(value) => (value.parent, value.end),
            SymbolData::InlineSite(value) => (value.parent, value.end),
            SymbolData::ManagedProcedure(value) => (value.parent, value.end),
            _ => return None,
        };
        Some(ScopeLinks { parent, end })
    }
}
