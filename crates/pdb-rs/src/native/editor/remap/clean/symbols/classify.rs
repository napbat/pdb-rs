//! The address fields of each kind of symbol record.
//!
//! [`shape`] sorts each [`SymbolData`] variant into one address form. The match names each
//! variant, so that a new variant needs a decision here.

use crate::native::address::SectionOffset;
use crate::native::codeview::symbol::{
    AddressGaps, AddressRange, SeparatedCode, SymbolData, Trampoline,
};

/// The code range of a procedure record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Code {
    /// The start address.
    pub(super) address: SectionOffset,
    /// The length of the code in bytes.
    pub(super) length: u32,
    /// The offset of the end of the prologue.
    pub(super) debug_start: u32,
    /// The offset of the start of the epilogue.
    pub(super) debug_end: u32,
}

/// The address form of a symbol record.
#[derive(Clone, Copy, Debug)]
pub(super) enum Shape {
    /// The record holds no address.
    None,
    /// The kind has no decoder, so the record can hold an address that the rewrite cannot
    /// find.
    Opaque,
    /// The record holds one or more addresses without a length ([`points`]).
    Points,
    /// The record holds a code range without a split form.
    Extent {
        /// The start address.
        start: SectionOffset,
        /// The length in bytes.
        length: u32,
    },
    /// A procedure.
    Procedure(Code),
    /// An `S_SEPCODE` record.
    SeparatedCode(SeparatedCode),
    /// A def-range with a range and gaps ([`def_range`]).
    DefRange(AddressRange),
    /// An inline site: binary annotations relative to its procedure.
    InlineSite,
    /// An `S_TRAMPOLINE` record: the thunk range and the target point.
    Trampoline(Trampoline),
    /// An `S_SECTION` record: the RVA of the section.
    Section(u32),
}

/// Return the address form of `data`.
#[expect(
    clippy::match_same_arms,
    reason = "each variant has its own arm, so that a new variant needs a decision"
)]
pub(super) fn shape(data: &SymbolData<'_>) -> Shape {
    match data {
        SymbolData::Procedure(value) => Shape::Procedure(Code {
            address: value.address,
            length: value.code_length,
            debug_start: value.debug_start,
            debug_end: value.debug_end,
        }),
        SymbolData::MipsProcedure(value) => Shape::Procedure(Code {
            address: value.address,
            length: value.header.code_length,
            debug_start: value.header.debug_start,
            debug_end: value.header.debug_end,
        }),
        SymbolData::Ia64Procedure(value) => Shape::Procedure(Code {
            address: value.address,
            length: value.header.code_length,
            debug_start: value.header.debug_start,
            debug_end: value.header.debug_end,
        }),
        SymbolData::ManagedProcedure(value) => Shape::Procedure(Code {
            address: value.address,
            length: value.code_length,
            debug_start: value.debug_start,
            debug_end: value.debug_end,
        }),
        SymbolData::Block(value) => Shape::Extent {
            start: value.address,
            length: value.code_length,
        },
        SymbolData::With(value) => Shape::Extent {
            start: value.address,
            length: value.code_length,
        },
        SymbolData::Thunk(value) => Shape::Extent {
            start: value.address,
            length: u32::from(value.length),
        },
        // A call site has no length. It maps as a point and has no split.
        SymbolData::CallSiteInfo(value) => Shape::Extent {
            start: value.address,
            length: 0,
        },
        SymbolData::HeapAllocationSite(value) => Shape::Extent {
            start: value.address,
            length: u32::from(value.instruction_length),
        },
        SymbolData::HybridRange(value) => Shape::Extent {
            start: value.address,
            length: value.length,
        },
        SymbolData::SeparatedCode(value) => Shape::SeparatedCode(*value),
        SymbolData::Trampoline(value) => Shape::Trampoline(*value),
        SymbolData::Section(value) => Shape::Section(value.rva),
        SymbolData::InlineSite(_) => Shape::InlineSite,
        SymbolData::DefRangeProgram(value) => Shape::DefRange(value.range),
        SymbolData::DefRangeFramePointerRelative(value) => Shape::DefRange(value.range),
        SymbolData::DefRangeRegister(value) => Shape::DefRange(value.range),
        SymbolData::DefRangeRegisterRelative(value) => Shape::DefRange(value.range),
        SymbolData::DefRangeRegisterRelativeIndirect(value) => Shape::DefRange(value.range),
        SymbolData::DefRangeSubfieldRegister(value) => Shape::DefRange(value.range),
        SymbolData::DefRangeHlsl(value) => Shape::DefRange(value.range),
        SymbolData::Label(_)
        | SymbolData::Data(_)
        | SymbolData::ThreadData(_)
        | SymbolData::ManagedData(_)
        | SymbolData::Public(_)
        | SymbolData::CoffGroup(_)
        | SymbolData::AnnotationStrings(_)
        | SymbolData::Association(_)
        | SymbolData::VirtualFunctionTable(_)
        | SymbolData::StaticLocal(_)
        | SymbolData::JumpTable(_)
        | SymbolData::FrameProcedure(_)
        | SymbolData::AttributedFrameRelative(_)
        | SymbolData::AttributedRegister(_)
        | SymbolData::AttributedRegisterRelative(_)
        | SymbolData::ManagedSlot(_)
        | SymbolData::DefRangeGlobalOnEntry(_) => Shape::Points,
        // The attributes of the kinds without attributes are absent.
        SymbolData::ManyRegisters(value) => match value.attributes {
            Some(_) => Shape::Points,
            None => Shape::None,
        },
        SymbolData::Unsupported(_) => Shape::Opaque,
        // The embedded records of `S_DISCARDED` describe code that the linker removed. No
        // consumer reads their addresses.
        SymbolData::Discarded(_) => Shape::None,
        // `S_FASTLINK` names a COFF section of an object file, not an image address.
        SymbolData::FastLink(_) => Shape::None,
        SymbolData::ObjectName(_)
        | SymbolData::Compile2(_)
        | SymbolData::Compile3(_)
        | SymbolData::BuildInfo(_)
        | SymbolData::SourceLink(_)
        | SymbolData::EnvironmentBlock(_)
        | SymbolData::UsingNamespace(_)
        | SymbolData::ScopeEnd
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
        | SymbolData::Export(_)
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
        | SymbolData::Skip(_) => Shape::None,
    }
}

/// Return references to the point fields of a [`Shape::Points`] record. `name_field` in
/// `codeview/symbol/access.rs` uses the same form for the shared and the mutable access.
macro_rules! point_fields {
    ($data:expr, $($mutability:tt)?) => {
        match $data {
            SymbolData::Label(value) => [Some(&$($mutability)? value.address), None, None],
            SymbolData::Data(value)
            | SymbolData::ThreadData(value)
            | SymbolData::ManagedData(value) => [Some(&$($mutability)? value.address), None, None],
            SymbolData::Public(value) => [Some(&$($mutability)? value.address), None, None],
            SymbolData::CoffGroup(value) => [Some(&$($mutability)? value.address), None, None],
            SymbolData::AnnotationStrings(value) => {
                [Some(&$($mutability)? value.address), None, None]
            }
            SymbolData::Association(value) => [Some(&$($mutability)? value.address), None, None],
            SymbolData::VirtualFunctionTable(value) => {
                [Some(&$($mutability)? value.address), None, None]
            }
            SymbolData::StaticLocal(value) => [
                Some(&$($mutability)? value.address),
                Some(&$($mutability)? value.parent_address),
                None,
            ],
            SymbolData::JumpTable(value) => [
                Some(&$($mutability)? value.base),
                Some(&$($mutability)? value.branch),
                Some(&$($mutability)? value.table),
            ],
            SymbolData::FrameProcedure(value) => {
                [Some(&$($mutability)? value.exception_handler), None, None]
            }
            SymbolData::AttributedFrameRelative(value) => {
                [Some(&$($mutability)? value.attributes.address), None, None]
            }
            SymbolData::AttributedRegister(value) => {
                [Some(&$($mutability)? value.attributes.address), None, None]
            }
            SymbolData::AttributedRegisterRelative(value) => {
                [Some(&$($mutability)? value.attributes.address), None, None]
            }
            SymbolData::ManyRegisters(value) => match &$($mutability)? value.attributes {
                Some(attributes) => [Some(&$($mutability)? attributes.address), None, None],
                None => [None, None, None],
            },
            SymbolData::ManagedSlot(value) => [Some(&$($mutability)? value.address), None, None],
            SymbolData::DefRangeGlobalOnEntry(value) => [Some(value), None, None],
            _ => [None, None, None],
        }
    };
}

/// Return the point fields of a [`Shape::Points`] record, in a fixed order.
pub(super) fn points(data: &SymbolData<'_>) -> [Option<SectionOffset>; 3] {
    point_fields!(data,).map(Option::<&SectionOffset>::copied)
}

/// Write `values` to the point fields of a [`Shape::Points`] record, in the order of
/// [`points`].
pub(super) fn set_points(data: &mut SymbolData<'_>, values: [Option<SectionOffset>; 3]) {
    for (field, value) in point_fields!(data, mut).into_iter().zip(values) {
        if let (Some(field), Some(value)) = (field, value) {
            *field = value;
        }
    }
}

/// Write the code range of a [`Shape::Procedure`] record.
pub(super) fn set_code(data: &mut SymbolData<'_>, code: Code) {
    match data {
        SymbolData::Procedure(value) => {
            value.address = code.address;
            value.code_length = code.length;
            value.debug_start = code.debug_start;
            value.debug_end = code.debug_end;
        }
        SymbolData::MipsProcedure(value) => {
            value.address = code.address;
            value.header.code_length = code.length;
            value.header.debug_start = code.debug_start;
            value.header.debug_end = code.debug_end;
        }
        SymbolData::Ia64Procedure(value) => {
            value.address = code.address;
            value.header.code_length = code.length;
            value.header.debug_start = code.debug_start;
            value.header.debug_end = code.debug_end;
        }
        SymbolData::ManagedProcedure(value) => {
            value.address = code.address;
            value.code_length = code.length;
            value.debug_start = code.debug_start;
            value.debug_end = code.debug_end;
        }
        _ => {}
    }
}

/// Write the start of the range of an [`Shape::Extent`] record.
pub(super) fn set_extent_start(data: &mut SymbolData<'_>, start: SectionOffset) {
    match data {
        SymbolData::Block(value) => value.address = start,
        SymbolData::With(value) => value.address = start,
        SymbolData::Thunk(value) => value.address = start,
        SymbolData::CallSiteInfo(value) => value.address = start,
        SymbolData::HeapAllocationSite(value) => value.address = start,
        SymbolData::HybridRange(value) => value.address = start,
        _ => {}
    }
}

/// Return references to the range and the gaps of a [`Shape::DefRange`] record.
macro_rules! def_range_fields {
    ($data:expr, $($mutability:tt)?) => {
        match $data {
            SymbolData::DefRangeProgram(value) => {
                Some((&$($mutability)? value.range, &$($mutability)? value.gaps))
            }
            SymbolData::DefRangeFramePointerRelative(value) => {
                Some((&$($mutability)? value.range, &$($mutability)? value.gaps))
            }
            SymbolData::DefRangeRegister(value) => {
                Some((&$($mutability)? value.range, &$($mutability)? value.gaps))
            }
            SymbolData::DefRangeRegisterRelative(value) => {
                Some((&$($mutability)? value.range, &$($mutability)? value.gaps))
            }
            SymbolData::DefRangeRegisterRelativeIndirect(value) => {
                Some((&$($mutability)? value.range, &$($mutability)? value.gaps))
            }
            SymbolData::DefRangeSubfieldRegister(value) => {
                Some((&$($mutability)? value.range, &$($mutability)? value.gaps))
            }
            SymbolData::DefRangeHlsl(value) => {
                Some((&$($mutability)? value.range, &$($mutability)? value.gaps))
            }
            _ => None,
        }
    };
}

/// Return the gaps of a [`Shape::DefRange`] record.
pub(super) fn gaps<'d>(data: &'d SymbolData<'static>) -> Option<&'d AddressGaps<'static>> {
    def_range_fields!(data,).map(|(_, gaps)| gaps)
}

/// Write the range and the gaps of a [`Shape::DefRange`] record.
pub(super) fn set_def_range(
    data: &mut SymbolData<'static>,
    range: AddressRange,
    gaps: Option<AddressGaps<'static>>,
) {
    if let Some((field, old)) = def_range_fields!(data, mut) {
        *field = range;
        if let Some(gaps) = gaps {
            *old = gaps;
        }
    }
}
