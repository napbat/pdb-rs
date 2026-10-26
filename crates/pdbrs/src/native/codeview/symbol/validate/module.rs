//! The checks of one module symbol stream.
//!
//! [`ModuleSymbols`] checks the parent and end links. This module keeps a
//! copy of the open scopes with their code ranges, and checks the records in
//! each scope against them. The `inline_site` module gives the code ranges of
//! inline sites.

mod inline_site;

use alloc::vec::Vec;

use super::super::{
    AddressGaps, AddressRange, Annotation, BinaryAnnotations, ChunkRange, LocalFlags,
    ModuleSymbols, S_BLOCK32, S_END, S_INLINESITE, S_INLINESITE2, S_PROC_ID_END, S_SEPCODE,
    S_THUNK32, ScopedSymbol, Symbol, SymbolData, separated_chunks,
};
use super::{
    GLOBAL_PROCEDURES, ID_PROCEDURES, LOCAL_PROCEDURES, MANAGED_PROCEDURES, Rule,
    ValidationContext, ValidationReport, Violation, check_references, register,
};
use crate::native::address::SectionOffset;
use crate::native::codeview::index::TypeIndex;
use crate::native::msf::StreamId;
use crate::{Error, ErrorKind, Result};

/// The bits of `CV_LVARFLAGS` that `cvinfo.h` reserves (`unused`).
const RESERVED_LOCAL_FLAGS: u16 = 0xf800;

/// The scope kinds that are always in a procedure.
const NESTED_SCOPES: &[u16] = &[S_BLOCK32, S_INLINESITE, S_INLINESITE2];

/// The scope kinds that can hold a parameter local directly, other than the
/// procedure kinds.
///
/// `cvinfo.h` does not say where separated code puts its parameters, so the
/// check accepts them in `S_SEPCODE` scopes.
const PARAMETER_SCOPES: &[u16] = &[S_INLINESITE, S_INLINESITE2, S_SEPCODE];

/// A code range in one section. The end is exclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CodeRange {
    /// The section number.
    section: u16,
    /// The offset of the first byte.
    start: u32,
    /// The offset after the last byte.
    end: u32,
}

impl CodeRange {
    /// Create a range, or return `None` if the end does not fit in 32 bits.
    fn new(address: SectionOffset, length: u32) -> Option<Self> {
        Some(Self {
            section: address.section,
            start: address.offset,
            end: address.offset.checked_add(length)?,
        })
    }

    /// Return `true` if `other` is inside this range.
    const fn contains(&self, other: &Self) -> bool {
        self.section == other.section && self.start <= other.start && other.end <= self.end
    }
}

/// The code ranges of an inline site.
#[derive(Clone, Copy, Debug)]
enum SiteRanges {
    /// The ranges are `Checker::site_ranges[start..end]`.
    Valid {
        /// The position of the first range.
        start: usize,
        /// The position after the last range.
        end: usize,
    },
    /// The annotations give no valid ranges.
    Unknown,
}

/// A def-range check against the code ranges of an inline site that needs
/// a separated code chunk of the procedure. [`Checker::finish`] runs it,
/// because MSVC writes `S_SEPCODE` after the end of the procedure.
struct DeferredSite {
    /// The code range of the procedure (chunk 0).
    procedure: CodeRange,
    /// The code ranges of the inline site.
    ranges: Vec<ChunkRange>,
    /// The parts of the def-range outside its gaps.
    parts: Vec<CodeRange>,
    /// The offset of the def-range record.
    offset: u32,
    /// The kind of the def-range record.
    kind: u16,
}

/// An open scope with its code range.
#[derive(Clone, Copy, Debug)]
struct Frame {
    /// The record kind of the scope start.
    kind: u16,
    /// The code range, if the kind has one and the range is valid.
    range: Option<CodeRange>,
    /// `true` if the scope covers no code: a code range of length 0, or an
    /// inline site whose code length annotations are all 0.
    empty: bool,
    /// The code ranges of an inline site; `None` for other scopes.
    site: Option<SiteRanges>,
}

/// The location fields of a def-range record.
struct DefRange<'a> {
    /// The code range, or `None` for a location that is valid in the full
    /// scope of the variable.
    range: Option<AddressRange>,
    /// The gaps in the range.
    gaps: Option<AddressGaps<'a>>,
    /// `true` if the range attributes set the "may have no user name" bit
    /// (`CV_RANGEATTR::maybe`). The location is then valid also where the
    /// name of the variable is not in scope.
    may_have_no_name: bool,
}

/// The state of the checks of one module stream.
struct Checker<'r, 'c> {
    /// The data from other streams.
    context: &'r ValidationContext<'c>,
    /// The collected violations.
    report: ValidationReport,
    /// The open scopes, innermost last.
    stack: Vec<Frame>,
    /// The offset and kind of each scope start, in stream order.
    starts: Vec<(u32, u16)>,
    /// The offset, kind and `next` link of each procedure and thunk with a
    /// link that is not 0.
    nexts: Vec<(u32, u16, u32)>,
    /// The procedure start, code range, offset and kind of each def-range
    /// outside its procedure. [`Checker::finish`] checks them against the
    /// separated code of the procedure.
    outside: Vec<(SectionOffset, CodeRange, u32, u16)>,
    /// The parent address and the code range of each `S_SEPCODE` scope.
    separated: Vec<(SectionOffset, CodeRange)>,
    /// The CPU of the first compile record.
    cpu: Option<u16>,
    /// The flags of the local that the next def-range record belongs to.
    local: Option<LocalFlags>,
    /// The type of that local.
    local_type: Option<TypeIndex>,
    /// The code ranges of the open inline sites, in stack order.
    site_ranges: Vec<ChunkRange>,
    /// The def-range checks that need separated code chunks.
    deferred: Vec<DeferredSite>,
}

/// Check a module symbol stream. See [`super::validate_module_symbols`].
pub(super) fn validate(
    symbols: &[u8],
    stream: StreamId,
    base: u32,
    context: &ValidationContext<'_>,
) -> Result<ValidationReport> {
    let mut checker = Checker {
        context,
        report: ValidationReport::new(stream),
        stack: Vec::new(),
        starts: Vec::new(),
        nexts: Vec::new(),
        outside: Vec::new(),
        separated: Vec::new(),
        cpu: None,
        local: None,
        local_type: None,
        site_ranges: Vec::new(),
        deferred: Vec::new(),
    };
    for scoped in ModuleSymbols::new(symbols, stream, base, context.max_depth) {
        match scoped {
            Ok(scoped) => checker.record(&scoped)?,
            Err(error) if error.kind() == ErrorKind::UnsupportedVersion => return Err(error),
            Err(error) => {
                checker.report.push(Violation::walk(&error, stream))?;
                break;
            }
        }
    }
    checker.finish()
}

/// Return an error for a failed allocation.
fn allocation_error() -> Error {
    Error::new(ErrorKind::LimitExceeded, None)
        .with_context("cannot allocate the symbol validation state")
}

/// Push `value` with a checked allocation.
fn push<T>(values: &mut Vec<T>, value: T) -> Result<()> {
    values.try_reserve(1).map_err(|_| allocation_error())?;
    values.push(value);
    Ok(())
}

/// Return the start address and the code length of a scope start.
fn scope_code(symbol: &Symbol<'_>) -> Option<(SectionOffset, u32)> {
    match symbol.data() {
        SymbolData::Procedure(value) => Some((value.address, value.code_length)),
        SymbolData::MipsProcedure(value) => Some((value.address, value.header.code_length)),
        SymbolData::Ia64Procedure(value) => Some((value.address, value.header.code_length)),
        SymbolData::ManagedProcedure(value) => Some((value.address, value.code_length)),
        SymbolData::Block(value) => Some((value.address, value.code_length)),
        SymbolData::With(value) => Some((value.address, value.code_length)),
        SymbolData::SeparatedCode(value) => Some((value.address, value.code_length)),
        SymbolData::Thunk(value) => Some((value.address, u32::from(value.length))),
        _ => None,
    }
}

/// Return the `next` link of a procedure or a thunk.
fn next_link(symbol: &Symbol<'_>) -> Option<u32> {
    match symbol.data() {
        SymbolData::Procedure(value) => Some(value.next),
        SymbolData::MipsProcedure(value) => Some(value.header.next),
        SymbolData::Ia64Procedure(value) => Some(value.header.next),
        SymbolData::ManagedProcedure(value) => Some(value.next),
        SymbolData::Thunk(value) => Some(value.next),
        _ => None,
    }
}

/// Return the register number of a record, if it has one.
///
/// HLSL def-ranges have HLSL register types, not `CV_HREG_e` numbers.
fn register_number(symbol: &Symbol<'_>) -> Option<u16> {
    match symbol.data() {
        SymbolData::DefRangeRegister(value) => Some(value.register),
        SymbolData::DefRangeSubfieldRegister(value) => Some(value.register),
        SymbolData::DefRangeRegisterRelative(value) => Some(value.register),
        SymbolData::DefRangeRegisterRelativeIndirect(value) => Some(value.register),
        SymbolData::RegisterRelative(value) => Some(value.register),
        SymbolData::RegisterRelativeIndirect(value) => Some(value.register),
        SymbolData::Register(value) => Some(value.register),
        SymbolData::AttributedRegister(value) => Some(value.register),
        SymbolData::AttributedRegisterRelative(value) => Some(value.register),
        SymbolData::FrameCookie(value) => Some(value.register),
        SymbolData::FrameRegister(register) => Some(*register),
        _ => None,
    }
}

/// Return the location fields of a def-range record.
///
/// The `_ON_ENTRY` kinds give a value at the procedure entry, so they have
/// no code range.
fn def_range<'a>(symbol: &Symbol<'a>) -> Option<DefRange<'a>> {
    let (range, gaps) = match *symbol.data() {
        SymbolData::DefRangeFramePointerRelative(value) => (value.range, value.gaps),
        SymbolData::DefRangeRegister(value) => (value.range, value.gaps),
        SymbolData::DefRangeSubfieldRegister(value) => (value.range, value.gaps),
        SymbolData::DefRangeRegisterRelative(value) => (value.range, value.gaps),
        SymbolData::DefRangeRegisterRelativeIndirect(value) => (value.range, value.gaps),
        SymbolData::DefRangeProgram(value) => (value.range, value.gaps),
        SymbolData::DefRangeHlsl(value) => (value.range, value.gaps),
        SymbolData::DefRangeFramePointerRelativeFullScope(_)
        | SymbolData::DefRangeConstantOnEntry(_)
        | SymbolData::DefRangeGlobalOnEntry(_) => {
            return Some(DefRange {
                range: None,
                gaps: None,
                may_have_no_name: false,
            });
        }
        _ => return None,
    };
    let may_have_no_name = match symbol.data() {
        SymbolData::DefRangeRegister(value) => value.may_have_no_name(),
        SymbolData::DefRangeSubfieldRegister(value) => value.may_have_no_name(),
        _ => false,
    };
    Some(DefRange {
        range: Some(range),
        gaps: Some(gaps),
        may_have_no_name,
    })
}

/// Return the offset in the parent variable of a subfield def-range.
///
/// `S_DEFRANGE_REGISTER_REL` gives an offset only if its spilled-member bit
/// (bit 0 of the flags) is set. The offset is in bits 4 to 15.
fn subfield_offset(symbol: &Symbol<'_>) -> Option<u32> {
    match symbol.data() {
        SymbolData::DefRangeProgram(value) => value.parent_offset,
        SymbolData::DefRangeSubfieldRegister(value) => Some(value.parent_offset),
        SymbolData::DefRangeRegisterRelative(value) if value.flags & 1 != 0 => {
            Some(u32::from(value.flags >> 4))
        }
        _ => None,
    }
}

/// Return the code range of chunk `number` of `procedure`.
///
/// Chunk 0 is the procedure. Chunk `n` is the `n`-th `S_SEPCODE` scope in
/// `separated` whose parent address is the start of the procedure.
fn chunk(
    separated: &[(SectionOffset, CodeRange)],
    procedure: CodeRange,
    number: u32,
) -> Option<CodeRange> {
    let Some(index) = usize::try_from(number).ok()?.checked_sub(1) else {
        return Some(procedure);
    };
    let start = SectionOffset {
        section: procedure.section,
        offset: procedure.start,
    };
    separated_chunks(separated, start).nth(index).copied()
}

/// Return `true` if an inline site has code length annotations and all of
/// them are 0.
fn inline_site_is_empty(annotations: BinaryAnnotations<'_>) -> bool {
    let mut lengths = annotations
        .iter()
        .filter_map(|annotation| match annotation {
            Annotation::ChangeCodeLength(length)
            | Annotation::ChangeCodeLengthAndCodeOffset {
                code_length: length,
                ..
            } => Some(length),
            _ => None,
        });
    lengths.next() == Some(0) && lengths.all(|length| length == 0)
}

/// Return `true` if a record of `kind` is a procedure or a thunk start.
fn is_procedure_or_thunk(kind: u16) -> bool {
    kind == S_THUNK32
        || GLOBAL_PROCEDURES.contains(&kind)
        || LOCAL_PROCEDURES.contains(&kind)
        || MANAGED_PROCEDURES.contains(&kind)
}

/// Return `true` if a parameter local can be directly in a scope of `kind`.
fn holds_parameters(kind: u16) -> bool {
    PARAMETER_SCOPES.contains(&kind)
        || GLOBAL_PROCEDURES.contains(&kind)
        || LOCAL_PROCEDURES.contains(&kind)
        || MANAGED_PROCEDURES.contains(&kind)
}

impl Checker<'_, '_> {
    /// Add a violation of `rule` at the record at `offset`.
    fn violate(&mut self, rule: Rule, offset: u32, kind: u16) -> Result<()> {
        self.report.violate(rule, offset, kind)
    }

    /// Check one record and update the state.
    fn record(&mut self, scoped: &ScopedSymbol<'_>) -> Result<()> {
        let symbol = &scoped.symbol;
        let (offset, kind) = (scoped.offset, symbol.kind());
        self.report.records += 1;
        check_references(symbol, offset, self.context, &mut self.report)?;
        match symbol.data() {
            SymbolData::Compile2(value) => self.cpu = self.cpu.or(Some(value.machine)),
            SymbolData::Compile3(value) => self.cpu = self.cpu.or(Some(value.machine)),
            _ => {}
        }
        if let (Some(cpu), Some(number)) = (self.cpu, register_number(symbol))
            && register::known(cpu, number) == Some(false)
        {
            self.violate(Rule::RegisterUnknown, offset, kind)?;
        }
        match symbol.data() {
            SymbolData::Local(local) => {
                self.local_record(local.flags, offset, kind)?;
                self.local_type = Some(local.type_index);
            }
            // The def-ranges after an `S_FILESTATIC` record give the location
            // of an enregistered static (`FILESTATICSYM` in `cvinfo.h`).
            SymbolData::FileStatic(value) => {
                self.local = Some(value.flags);
                self.local_type = Some(value.type_index);
            }
            SymbolData::DpcGroupSharedLocal(value) => {
                self.local = Some(value.flags);
                self.local_type = Some(value.type_index);
            }
            // LLD replaces a record that it cannot remap with `S_SKIP`, also
            // between a local and its def-ranges (Rust LLD fixture of #12).
            SymbolData::Skip(_) => {}
            _ => {
                if let Some(range) = def_range(symbol) {
                    self.def_range(&range, offset, kind)?;
                    self.subfield(symbol, offset, kind)?;
                } else {
                    self.local = None;
                    self.local_type = None;
                }
            }
        }
        if symbol.scope().is_some() {
            self.open(symbol, offset)?;
        } else if matches!(
            symbol.data(),
            SymbolData::ScopeEnd | SymbolData::InlineSiteEnd
        ) {
            self.close(offset, kind)?;
        }
        Ok(())
    }

    /// Check an `S_LOCAL` record and start its def-range sequence.
    fn local_record(&mut self, flags: LocalFlags, offset: u32, kind: u16) -> Result<()> {
        self.local = Some(flags);
        if flags.0 & RESERVED_LOCAL_FLAGS != 0 {
            self.violate(Rule::LocalFlagsReserved, offset, kind)?;
        }
        let Some(scope) = self.stack.last() else {
            return self.violate(Rule::LocalOutsideScope, offset, kind);
        };
        if flags.contains(LocalFlags::PARAMETER) && !holds_parameters(scope.kind) {
            self.violate(Rule::ParameterScope, offset, kind)?;
        }
        Ok(())
    }

    /// Check a def-range record against its local and its scopes.
    fn def_range(&mut self, def_range: &DefRange<'_>, offset: u32, kind: u16) -> Result<()> {
        match self.local {
            None => self.violate(Rule::DefRangeWithoutLocal, offset, kind)?,
            Some(flags) if flags.contains(LocalFlags::OPTIMIZED_OUT) => {
                self.violate(Rule::OptimizedOutWithRange, offset, kind)?;
            }
            Some(_) => {}
        }
        let Some(range) = def_range.range else {
            return Ok(());
        };
        let procedure = self.stack.iter().find_map(|frame| frame.range);
        // LLVM writes a zero-length range for a variable in a block or an
        // inline site that covers no code (Rust fixtures of #12). Such a
        // range is not a defect.
        if range.length == 0 && !self.stack.last().is_some_and(|frame| frame.empty) {
            self.violate(Rule::DefRangeEmpty, offset, kind)?;
        }
        match CodeRange::new(range.start, u32::from(range.length)) {
            None => self.violate(Rule::CodeRangeOverflow, offset, kind)?,
            Some(code) => {
                self.check_section(&code, offset, kind)?;
                // A variable in separated code lives in the range of its
                // `S_SEPCODE` scope, outside the procedure (MSVC PGO fixture
                // of #27).
                let separated = self.stack.iter().any(|frame| {
                    frame.kind == S_SEPCODE
                        && frame.range.is_some_and(|outer| outer.contains(&code))
                });
                if let Some(outer) = procedure.filter(|outer| !outer.contains(&code) && !separated)
                {
                    // MSVC also writes the `S_SEPCODE` record after the end of
                    // its procedure, and the procedure has def-ranges in the
                    // separated code. `finish` checks these def-ranges.
                    let start = SectionOffset {
                        section: outer.section,
                        offset: outer.start,
                    };
                    push(&mut self.outside, (start, code, offset, kind))?;
                }
            }
        }
        if let Some(gaps) = def_range.gaps {
            self.gaps(gaps, range.length, offset, kind)?;
        }
        if def_range.may_have_no_name {
            return Ok(());
        }
        self.inline_site(range, def_range.gaps, offset, kind)
    }

    /// Check a def-range against the code ranges of its innermost inline
    /// site.
    fn inline_site(
        &mut self,
        range: AddressRange,
        gaps: Option<AddressGaps<'_>>,
        offset: u32,
        kind: u16,
    ) -> Result<()> {
        let site = self.stack.iter().rev().find_map(|frame| frame.site);
        let (Some(SiteRanges::Valid { end, .. }), Some(procedure)) =
            (site, self.stack.iter().find_map(|frame| frame.range))
        else {
            return Ok(());
        };
        let start = self
            .stack
            .iter()
            .find_map(|frame| match frame.site {
                Some(SiteRanges::Valid { start, .. }) => Some(start),
                _ => None,
            })
            .unwrap_or(0);
        let mut parts = Vec::new();
        inline_site::parts(range, gaps, &mut parts)?;
        if parts.is_empty() {
            return Ok(());
        }
        let ranges = &self.site_ranges[start..end];
        let mut absolute = Vec::new();
        if inline_site::absolute(
            ranges,
            |number| chunk(&self.separated, procedure, number),
            &mut absolute,
        )? {
            if !inline_site::touches(&absolute, &parts) {
                self.violate(Rule::DefRangeOutsideInlineSite, offset, kind)?;
            }
        } else if ranges.iter().any(|range| range.chunk != 0) {
            let mut copy = Vec::new();
            copy.try_reserve_exact(ranges.len())
                .map_err(|_| allocation_error())?;
            copy.extend_from_slice(ranges);
            let deferred = DeferredSite {
                procedure,
                ranges: copy,
                parts,
                offset,
                kind,
            };
            push(&mut self.deferred, deferred)?;
        }
        Ok(())
    }

    /// Check the parent offset of a subfield def-range against the size of
    /// the type of its local.
    fn subfield(&mut self, symbol: &Symbol<'_>, offset: u32, kind: u16) -> Result<()> {
        let (Some(parent_offset), Some(local_type), Some(type_size)) = (
            subfield_offset(symbol),
            self.local_type,
            self.context.type_size,
        ) else {
            return Ok(());
        };
        if type_size(local_type).is_some_and(|size| u64::from(parent_offset) >= size) {
            self.violate(Rule::SubfieldOutsideType, offset, kind)?;
        }
        Ok(())
    }

    /// Check the gaps of a def-range with a range of `length` bytes.
    fn gaps(&mut self, gaps: AddressGaps<'_>, length: u16, offset: u32, kind: u16) -> Result<()> {
        let mut previous_end = 0_u32;
        for gap in gaps {
            let start = u32::from(gap.start);
            let end = start + u32::from(gap.length);
            if gap.length == 0 {
                self.violate(Rule::GapEmpty, offset, kind)?;
            }
            if end > u32::from(length) {
                self.violate(Rule::GapOutsideRange, offset, kind)?;
            }
            if start < previous_end {
                self.violate(Rule::GapOrder, offset, kind)?;
            }
            previous_end = previous_end.max(end);
        }
        Ok(())
    }

    /// Check that a code range is inside its image section.
    ///
    /// Section 0 names no image section. Symbols of discarded or absolute
    /// code use it, so the check accepts it.
    fn check_section(&mut self, code: &CodeRange, offset: u32, kind: u16) -> Result<()> {
        let Some(lengths) = self.context.section_lengths else {
            return Ok(());
        };
        let Some(index) = usize::from(code.section).checked_sub(1) else {
            return Ok(());
        };
        if lengths.get(index).is_none_or(|&length| code.end > length) {
            self.violate(Rule::SectionRange, offset, kind)?;
        }
        Ok(())
    }

    /// Check a scope start and push its frame.
    fn open(&mut self, symbol: &Symbol<'_>, offset: u32) -> Result<()> {
        let kind = symbol.kind();
        if self.stack.is_empty() && NESTED_SCOPES.contains(&kind) {
            self.violate(Rule::ScopeWithoutParent, offset, kind)?;
        }
        let mut range = None;
        if let Some((address, length)) = scope_code(symbol) {
            range = CodeRange::new(address, length);
            match range {
                None => self.violate(Rule::CodeRangeOverflow, offset, kind)?,
                Some(code) => {
                    self.check_section(&code, offset, kind)?;
                    let parent = self.stack.iter().rev().find_map(|frame| frame.range);
                    if parent.is_some_and(|parent| !parent.contains(&code)) {
                        self.violate(Rule::ScopeRange, offset, kind)?;
                    }
                }
            }
        }
        if let Some(next) = next_link(symbol).filter(|&next| next != 0) {
            push(&mut self.nexts, (offset, kind, next))?;
        }
        if let (SymbolData::SeparatedCode(separated), Some(code)) = (symbol.data(), range) {
            push(&mut self.separated, (separated.parent_address, code))?;
        }
        push(&mut self.starts, (offset, kind))?;
        let mut site = None;
        let empty = match symbol.data() {
            SymbolData::InlineSite(value) => {
                site = Some(self.site_ranges(value.annotations, offset, kind)?);
                inline_site_is_empty(value.annotations)
            }
            _ => range.is_some_and(|code| code.start == code.end),
        };
        push(
            &mut self.stack,
            Frame {
                kind,
                range,
                empty,
                site,
            },
        )
    }

    /// Compute the code ranges of an inline site from its annotations.
    fn site_ranges(
        &mut self,
        annotations: BinaryAnnotations<'_>,
        offset: u32,
        kind: u16,
    ) -> Result<SiteRanges> {
        let start = self.site_ranges.len();
        if inline_site::site_ranges(annotations, &mut self.site_ranges)? {
            let end = self.site_ranges.len();
            Ok(SiteRanges::Valid { start, end })
        } else {
            self.violate(Rule::InlineSiteAnnotations, offset, kind)?;
            Ok(SiteRanges::Unknown)
        }
    }

    /// Check an end record and pop the frame of its scope.
    fn close(&mut self, offset: u32, kind: u16) -> Result<()> {
        // `ModuleSymbols` makes sure that an open scope exists.
        let Some(frame) = self.stack.pop() else {
            return Ok(());
        };
        if let Some(SiteRanges::Valid { start, .. }) = frame.site {
            self.site_ranges.truncate(start);
        }
        let id_procedure = ID_PROCEDURES.contains(&frame.kind);
        if kind == S_PROC_ID_END && !id_procedure {
            self.violate(Rule::ProcedureIdEndKind, offset, kind)?;
        } else if kind == S_END && id_procedure {
            self.violate(Rule::ProcedureIdEndMissing, offset, kind)?;
        }
        Ok(())
    }

    /// Check the `next` links and return the report.
    fn finish(mut self) -> Result<ValidationReport> {
        let outside = core::mem::take(&mut self.outside);
        for (procedure, code, offset, kind) in outside {
            let separated = self
                .separated
                .iter()
                .any(|&(parent, range)| parent == procedure && range.contains(&code));
            if !separated {
                self.violate(Rule::DefRangeOutsideProcedure, offset, kind)?;
            }
        }
        let deferred = core::mem::take(&mut self.deferred);
        let mut absolute = Vec::new();
        for site in deferred {
            // A site that names a chunk that the stream does not have is not
            // checked.
            let known = inline_site::absolute(
                &site.ranges,
                |number| chunk(&self.separated, site.procedure, number),
                &mut absolute,
            )?;
            if known && !inline_site::touches(&absolute, &site.parts) {
                self.violate(Rule::DefRangeOutsideInlineSite, site.offset, site.kind)?;
            }
        }
        let nexts = core::mem::take(&mut self.nexts);
        for (offset, kind, next) in nexts {
            let target = self
                .starts
                .binary_search_by_key(&next, |&(start, _)| start)
                .ok()
                .map(|index| self.starts[index].1);
            if next <= offset || !target.is_some_and(is_procedure_or_thunk) {
                self.violate(Rule::NextLink, offset, kind)?;
            }
        }
        self.report
            .violations
            .sort_by_key(|violation| violation.offset);
        Ok(self.report)
    }
}
