//! The address fields of the records of the symbol record stream.

use crate::native::address::SectionOffset;
use crate::native::codeview::symbol::SymbolData;

/// The value of an unused entry of [`Points`].
const UNUSED: SectionOffset = SectionOffset {
    section: 0,
    offset: 0,
};

/// The address fields of one record.
#[derive(Clone, Copy, Debug)]
pub(super) enum Fields {
    /// The record holds no address.
    None,
    /// The record holds one to three points.
    Points(Points),
    /// The record is an `S_SECTION` record: the field is the PDB RVA of the section.
    Section(u32),
    /// The record has a code range or starts a scope. The field is its start address.
    /// Only a module stream can hold such a record with an address.
    Range(SectionOffset),
    /// The record has no decoder.
    Unsupported,
}

/// The points of one record, in field order.
#[derive(Clone, Copy, Debug)]
pub(super) struct Points {
    /// The values. Only the first `count` values are points.
    values: [SectionOffset; 3],
    /// The number of points.
    count: usize,
}

impl Points {
    /// Return the points of a record with one point.
    const fn one(value: SectionOffset) -> Self {
        Self {
            values: [value, UNUSED, UNUSED],
            count: 1,
        }
    }

    /// Return the points of a record with two points.
    const fn two(first: SectionOffset, second: SectionOffset) -> Self {
        Self {
            values: [first, second, UNUSED],
            count: 2,
        }
    }

    /// Return the points of a record with three points.
    const fn three(values: [SectionOffset; 3]) -> Self {
        Self { values, count: 3 }
    }

    /// Return the points for a change.
    pub(super) fn values_mut(&mut self) -> &mut [SectionOffset] {
        &mut self.values[..self.count]
    }
}

/// Return the address fields of `data`.
pub(super) fn fields(data: &SymbolData<'_>) -> Fields {
    match data {
        SymbolData::Public(public) => Fields::Points(Points::one(public.address)),
        SymbolData::Data(data) | SymbolData::ThreadData(data) | SymbolData::ManagedData(data) => {
            Fields::Points(Points::one(data.address))
        }
        SymbolData::AnnotationStrings(annotation) => {
            Fields::Points(Points::one(annotation.address))
        }
        SymbolData::Label(label) => Fields::Points(Points::one(label.address)),
        SymbolData::CoffGroup(group) => Fields::Points(Points::one(group.address)),
        SymbolData::VirtualFunctionTable(table) => Fields::Points(Points::one(table.address)),
        SymbolData::DefRangeGlobalOnEntry(address) => Fields::Points(Points::one(*address)),
        SymbolData::Association(association) => Fields::Points(Points::one(association.address)),
        SymbolData::ManagedSlot(slot) => Fields::Points(Points::one(slot.address)),
        SymbolData::StaticLocal(local) => {
            Fields::Points(Points::two(local.address, local.parent_address))
        }
        SymbolData::JumpTable(table) => {
            Fields::Points(Points::three([table.base, table.branch, table.table]))
        }
        SymbolData::ManyRegisters(registers) => {
            registers.attributes.map_or(Fields::None, |attributes| {
                Fields::Points(Points::one(attributes.address))
            })
        }
        SymbolData::AttributedFrameRelative(local) => {
            Fields::Points(Points::one(local.attributes.address))
        }
        SymbolData::AttributedRegister(local) => {
            Fields::Points(Points::one(local.attributes.address))
        }
        SymbolData::AttributedRegisterRelative(local) => {
            Fields::Points(Points::one(local.attributes.address))
        }
        SymbolData::Section(section) => Fields::Section(section.rva),
        SymbolData::Procedure(procedure) => Fields::Range(procedure.address),
        SymbolData::MipsProcedure(procedure) => Fields::Range(procedure.address),
        SymbolData::Ia64Procedure(procedure) => Fields::Range(procedure.address),
        SymbolData::ManagedProcedure(procedure) => Fields::Range(procedure.address),
        SymbolData::Block(block) => Fields::Range(block.address),
        SymbolData::With(with) => Fields::Range(with.address),
        SymbolData::SeparatedCode(code) => Fields::Range(code.address),
        SymbolData::Thunk(thunk) => Fields::Range(thunk.address),
        SymbolData::Trampoline(trampoline) => Fields::Range(trampoline.thunk),
        SymbolData::CallSiteInfo(site) => Fields::Range(site.address),
        SymbolData::HeapAllocationSite(site) => Fields::Range(site.address),
        SymbolData::HybridRange(range) => Fields::Range(range.address),
        SymbolData::FrameProcedure(frame) => Fields::Range(frame.exception_handler),
        SymbolData::DefRangeProgram(range) => Fields::Range(range.range.start),
        SymbolData::DefRangeFramePointerRelative(range) => Fields::Range(range.range.start),
        SymbolData::DefRangeRegister(range) => Fields::Range(range.range.start),
        SymbolData::DefRangeRegisterRelative(range) => Fields::Range(range.range.start),
        SymbolData::DefRangeRegisterRelativeIndirect(range) => Fields::Range(range.range.start),
        SymbolData::DefRangeSubfieldRegister(range) => Fields::Range(range.range.start),
        SymbolData::DefRangeHlsl(range) => Fields::Range(range.range.start),
        SymbolData::Unsupported(_) => Fields::Unsupported,
        SymbolData::ObjectName(_)
        | SymbolData::Compile2(_)
        | SymbolData::Compile3(_)
        | SymbolData::BuildInfo(_)
        | SymbolData::SourceLink(_)
        | SymbolData::EnvironmentBlock(_)
        | SymbolData::UsingNamespace(_)
        | SymbolData::ScopeEnd
        | SymbolData::InlineSite(_)
        | SymbolData::InlineSiteEnd
        | SymbolData::Inlinees(_)
        | SymbolData::Callees(_)
        | SymbolData::Callers(_)
        | SymbolData::ProfileData(_)
        | SymbolData::HotPatchFunction(_)
        | SymbolData::Local(_)
        | SymbolData::DefRangeFramePointerRelativeFullScope(_)
        | SymbolData::DefRangeConstantOnEntry(_)
        | SymbolData::RegisterRelative(_)
        | SymbolData::RegisterRelativeIndirect(_)
        | SymbolData::BasePointerRelative(_)
        | SymbolData::BasePointerRelativeIndirect(_)
        | SymbolData::Register(_)
        | SymbolData::Slot(_)
        | SymbolData::FileStatic(_)
        | SymbolData::FrameCookie(_)
        | SymbolData::FrameRegister(_)
        | SymbolData::FastLink(_)
        | SymbolData::Export(_)
        | SymbolData::Discarded(_)
        | SymbolData::ModuleTypeReference(_)
        | SymbolData::PdbMap(_)
        | SymbolData::UserDefinedType(_)
        | SymbolData::HlslData(_)
        | SymbolData::HlslDataEx(_)
        | SymbolData::DpcGroupSharedLocal(_)
        | SymbolData::DpcTagMap(_)
        | SymbolData::Constant(_)
        | SymbolData::ProcedureReference(_)
        | SymbolData::SymbolReference(_)
        | SymbolData::ManagedConstant(_)
        | SymbolData::ManagedTypeReference(_)
        | SymbolData::Oem(_)
        | SymbolData::Skip(_) => Fields::None,
    }
}

/// Write `points` into the fields of `data` that [`fields`] reads.
pub(super) fn set_points(data: &mut SymbolData<'_>, points: &Points) {
    let [first, second, third] = points.values;
    match data {
        SymbolData::Public(public) => public.address = first,
        SymbolData::Data(data) | SymbolData::ThreadData(data) | SymbolData::ManagedData(data) => {
            data.address = first;
        }
        SymbolData::AnnotationStrings(annotation) => annotation.address = first,
        SymbolData::Label(label) => label.address = first,
        SymbolData::CoffGroup(group) => group.address = first,
        SymbolData::VirtualFunctionTable(table) => table.address = first,
        SymbolData::DefRangeGlobalOnEntry(address) => *address = first,
        SymbolData::Association(association) => association.address = first,
        SymbolData::ManagedSlot(slot) => slot.address = first,
        SymbolData::StaticLocal(local) => {
            local.address = first;
            local.parent_address = second;
        }
        SymbolData::JumpTable(table) => {
            table.base = first;
            table.branch = second;
            table.table = third;
        }
        SymbolData::ManyRegisters(registers) => {
            if let Some(attributes) = &mut registers.attributes {
                attributes.address = first;
            }
        }
        SymbolData::AttributedFrameRelative(local) => local.attributes.address = first,
        SymbolData::AttributedRegister(local) => local.attributes.address = first,
        SymbolData::AttributedRegisterRelative(local) => local.attributes.address = first,
        _ => {}
    }
}
