//! Validation of the references and the scope structure of symbol records.
//!
//! [`validate_module_symbols`] checks one module symbol stream and
//! [`validate_shared_symbols`] checks the shared symbol record stream. Both
//! functions collect each broken rule as a [`Violation`] in a
//! [`ValidationReport`]. They do not stop at the first violation.
//!
//! A [`ValidationContext`] gives the data from other streams: the ends of
//! the TPI and IPI streams, the kinds of the IPI records, the module count,
//! the records of the module streams and the section lengths. A check that
//! needs data that the context does not have is not done.
//!
//! The module checks use [`ModuleSymbols`] for the parent and end links, the
//! end kinds and the depth limit. [`Rule`] lists the other checks. The rules
//! come from `cvinfo.h`, from LLVM `SymbolRecord.h` and `CodeViewDebug.cpp`
//! (revision `b9f2235f`), and from the producer fixtures of #12. The
//! [`Rule`] documentation names each rule that producers break and how the
//! check accepts their output.
//!
//! [`ModuleSymbols`]: super::ModuleSymbols

mod module;
mod register;
mod rule;

use alloc::vec::Vec;
use core::fmt;

use super::super::Records;
use super::super::id_record::{LF_BUILDINFO, LF_FUNC_ID, LF_MFUNC_ID};
use super::super::index::{FIRST_RECORD_INDEX, IdIndex, TypeIndex};
use super::{
    S_ANNOTATION, S_ANNOTATIONREF, S_BUILDINFO, S_DATAREF, S_GDATA32, S_GMANDATA, S_GMANPROC,
    S_GPROC32, S_GPROC32_ID, S_GPROC32EX, S_GPROC32EX_ID, S_GPROCIA64, S_GPROCIA64_ID, S_GPROCMIPS,
    S_GPROCMIPS_ID, S_GTHREAD32, S_LDATA32, S_LMANDATA, S_LMANPROC, S_LPROC32, S_LPROC32_DPC,
    S_LPROC32_DPC_ID, S_LPROC32_ID, S_LPROC32EX, S_LPROC32EX_ID, S_LPROCIA64, S_LPROCIA64_ID,
    S_LPROCMIPS, S_LPROCMIPS_ID, S_LPROCREF, S_LTHREAD32, S_PROCREF, S_SOURCELINK, S_TOKENREF,
    Symbol, SymbolData,
};
use crate::native::msf::StreamId;
use crate::{Error, ErrorKind, Result};

pub use rule::{Rule, Violation};

/// The nesting limit of [`ValidationContext::new`].
pub const DEFAULT_MAX_DEPTH: usize = 256;

/// The global procedure kinds of all architectures.
const GLOBAL_PROCEDURES: &[u16] = &[
    S_GPROC32,
    S_GPROC32_ID,
    S_GPROC32EX,
    S_GPROC32EX_ID,
    S_GPROCMIPS,
    S_GPROCIA64,
    S_GPROCMIPS_ID,
    S_GPROCIA64_ID,
];

/// The module-local procedure kinds of all architectures.
const LOCAL_PROCEDURES: &[u16] = &[
    S_LPROC32,
    S_LPROC32_ID,
    S_LPROC32EX,
    S_LPROC32EX_ID,
    S_LPROCMIPS,
    S_LPROCIA64,
    S_LPROCMIPS_ID,
    S_LPROCIA64_ID,
    S_LPROC32_DPC,
    S_LPROC32_DPC_ID,
];

/// The procedure kinds with an ID index. `S_PROC_ID_END` ends their scope.
const ID_PROCEDURES: &[u16] = &[
    S_GPROC32_ID,
    S_LPROC32_ID,
    S_GPROC32EX_ID,
    S_LPROC32EX_ID,
    S_LPROCMIPS_ID,
    S_GPROCMIPS_ID,
    S_LPROCIA64_ID,
    S_GPROCIA64_ID,
    S_LPROC32_DPC_ID,
];

/// The managed procedure kinds.
const MANAGED_PROCEDURES: &[u16] = &[S_GMANPROC, S_LMANPROC];

/// The record kinds that `S_PROCREF` can identify.
///
/// The global procedure kinds come from LLVM. The .NET native PDB writer
/// and the MSVC `/clr` linker also write `S_PROCREF` for `S_GMANPROC`
/// records (.NET and MSVC `/clr` fixtures of #12).
const GLOBAL_REFERENCE_TARGETS: &[u16] = &[
    S_GPROC32,
    S_GPROC32_ID,
    S_GPROC32EX,
    S_GPROC32EX_ID,
    S_GPROCMIPS,
    S_GPROCIA64,
    S_GPROCMIPS_ID,
    S_GPROCIA64_ID,
    S_GMANPROC,
];

/// The record kinds that `S_LPROCREF` can identify: the module-local
/// procedure kinds and `S_LMANPROC`, as for [`GLOBAL_REFERENCE_TARGETS`].
const LOCAL_REFERENCE_TARGETS: &[u16] = &[
    S_LPROC32,
    S_LPROC32_ID,
    S_LPROC32EX,
    S_LPROC32EX_ID,
    S_LPROCMIPS,
    S_LPROCIA64,
    S_LPROCMIPS_ID,
    S_LPROCIA64_ID,
    S_LPROC32_DPC,
    S_LPROC32_DPC_ID,
    S_LMANPROC,
];

/// The data kinds that `S_DATAREF` can identify.
const DATA: &[u16] = &[
    S_LDATA32,
    S_GDATA32,
    S_LTHREAD32,
    S_GTHREAD32,
    S_LMANDATA,
    S_GMANDATA,
];

/// The IPI kinds of a function ID.
const FUNCTION_IDS: &[u16] = &[LF_FUNC_ID, LF_MFUNC_ID];

/// What a module stream has at an offset.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ModuleRecord {
    /// A record of this kind starts at the offset.
    Kind(u16),
    /// No record starts at the offset.
    Missing,
    /// The caller does not know the records of the module, for example
    /// because the module has no symbol stream that the caller read.
    Unknown,
}

/// The data from other streams that the checks use.
///
/// [`ValidationContext::new`] gives the TPI and IPI ends. Add the optional
/// data with the `with_` functions. A check that needs data that is not
/// present is not done.
#[derive(Clone, Copy)]
pub struct ValidationContext<'c> {
    /// The index after the last TPI record.
    type_index_end: TypeIndex,
    /// The index after the last IPI record.
    id_index_end: IdIndex,
    /// The kind of the IPI record of an ID index.
    id_kind: Option<&'c dyn Fn(IdIndex) -> Option<u16>>,
    /// The number of modules in the DBI stream.
    module_count: Option<usize>,
    /// The record at a module index and a module stream offset.
    module_record: Option<&'c dyn Fn(u16, u32) -> ModuleRecord>,
    /// The virtual size of each section, by section number minus 1.
    section_lengths: Option<&'c [u32]>,
    /// The largest number of open scopes.
    max_depth: usize,
}

impl fmt::Debug for ValidationContext<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidationContext")
            .field("type_index_end", &self.type_index_end)
            .field("id_index_end", &self.id_index_end)
            .field("id_kind", &self.id_kind.is_some())
            .field("module_count", &self.module_count)
            .field("module_record", &self.module_record.is_some())
            .field("section_lengths", &self.section_lengths)
            .field("max_depth", &self.max_depth)
            .finish()
    }
}

impl<'c> ValidationContext<'c> {
    /// Create a context with the index ends of the TPI and IPI streams.
    ///
    /// Use the `index_end` fields of the TPI and IPI headers. The nesting
    /// limit is [`DEFAULT_MAX_DEPTH`].
    #[must_use]
    pub const fn new(type_index_end: TypeIndex, id_index_end: IdIndex) -> Self {
        Self {
            type_index_end,
            id_index_end,
            id_kind: None,
            module_count: None,
            module_record: None,
            section_lengths: None,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }

    /// Add a function that gives the record kind of an ID index, for
    /// example `LF_FUNC_ID`, or `None` if the IPI stream has no such record.
    #[must_use]
    pub const fn with_id_kinds(mut self, id_kind: &'c dyn Fn(IdIndex) -> Option<u16>) -> Self {
        self.id_kind = Some(id_kind);
        self
    }

    /// Add the number of modules in the DBI stream.
    #[must_use]
    pub const fn with_module_count(mut self, count: usize) -> Self {
        self.module_count = Some(count);
        self
    }

    /// Add a function that gives the record at a module index and a module
    /// stream offset. The module index is the module number minus 1. The
    /// offset includes the 4-byte signature.
    #[must_use]
    pub const fn with_module_records(
        mut self,
        module_record: &'c dyn Fn(u16, u32) -> ModuleRecord,
    ) -> Self {
        self.module_record = Some(module_record);
        self
    }

    /// Add the virtual size of each image section, in section header order.
    #[must_use]
    pub const fn with_section_lengths(mut self, lengths: &'c [u32]) -> Self {
        self.section_lengths = Some(lengths);
        self
    }

    /// Set the largest number of open scopes in a module stream.
    #[must_use]
    pub const fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }
}

/// The result of a validation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ValidationReport {
    /// The stream that the report is for.
    pub stream: StreamId,
    /// The number of records that the validation decoded and checked.
    pub records: usize,
    /// The broken rules in stream order. After a [`Rule::Walk`] violation,
    /// the records after the failed record are not checked.
    pub violations: Vec<Violation>,
}

impl ValidationReport {
    /// Create an empty report.
    const fn new(stream: StreamId) -> Self {
        Self {
            stream,
            records: 0,
            violations: Vec::new(),
        }
    }

    /// Return `true` if no rule is broken.
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.violations.is_empty()
    }

    /// Add a violation.
    fn push(&mut self, violation: Violation) -> Result<()> {
        self.violations.try_reserve(1).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("cannot allocate the symbol validation report")
        })?;
        self.violations.push(violation);
        Ok(())
    }

    /// Add a violation of `rule` at the record at `offset`.
    fn violate(&mut self, rule: Rule, offset: u32, kind: u16) -> Result<()> {
        self.push(Violation::new(rule, self.stream, offset, kind))
    }
}

/// Check the records of one module symbol stream.
///
/// `symbols` holds the records without the signature, and `base` is the
/// module stream offset of the first record: 4 if the signature is present.
/// `stream` identifies the module stream in the violations.
///
/// The function walks the records with [`ModuleSymbols`]. If the walk stops
/// at an error, the report gets a [`Rule::Walk`] violation and the checks
/// stop. The [`Rule`] variants list the other checks.
///
/// # Errors
///
/// - [`ErrorKind::UnsupportedVersion`] if a record starts a scope kind that
///   [`ModuleSymbols`] does not decode. The scopes after it cannot be checked.
/// - [`ErrorKind::LimitExceeded`] if an allocation fails.
///
/// [`ModuleSymbols`]: super::ModuleSymbols
pub fn validate_module_symbols(
    symbols: &[u8],
    stream: StreamId,
    base: u32,
    context: &ValidationContext<'_>,
) -> Result<ValidationReport> {
    module::validate(symbols, stream, base, context)
}

/// Check the records of the shared symbol record stream.
///
/// The function checks the type and ID references of each record and the
/// module references of `S_PROCREF`, `S_LPROCREF` and `S_TOKENREF`. A
/// record that does not decode gives a [`Rule::Walk`] violation, and the
/// checks stop.
///
/// # Errors
///
/// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
pub fn validate_shared_symbols(
    symbols: &[u8],
    stream: StreamId,
    context: &ValidationContext<'_>,
) -> Result<ValidationReport> {
    let mut report = ValidationReport::new(stream);
    let mut records = Records::new(symbols);
    loop {
        // A slice length always fits in `u64` on supported targets.
        let position = u64::try_from(records.position()).unwrap_or(u64::MAX);
        let Some(record) = records.next() else {
            break;
        };
        let symbol = match record.and_then(|record| {
            Symbol::decode(record)
                .map_err(|error| error.with_stream(stream.0, u64::from(record.offset)))
        }) {
            Ok(symbol) => symbol,
            Err(error) => {
                let error = match error.stream_location() {
                    Some(_) => error,
                    None => error.with_stream(stream.0, position),
                };
                report.push(Violation::walk(&error, stream))?;
                break;
            }
        };
        report.records += 1;
        check_references(&symbol, symbol.record().offset, context, &mut report)?;
    }
    Ok(report)
}

/// Check the type, ID and module references of a symbol at `offset`.
fn check_references(
    symbol: &Symbol<'_>,
    offset: u32,
    context: &ValidationContext<'_>,
    report: &mut ValidationReport,
) -> Result<()> {
    let kind = symbol.kind();
    if let Some(index) = symbol.type_index() {
        if index.is_primitive() {
            if index.primitive().is_err() {
                report.violate(Rule::PrimitiveType, offset, kind)?;
            }
        } else if index >= context.type_index_end {
            report.violate(Rule::TypeIndexRange, offset, kind)?;
        }
    }
    if let Some(id) = symbol.id_index() {
        // No reference gives the IPI kind of an `S_SOURCELINK` record.
        let expected = match kind {
            S_BUILDINFO => Some(&[LF_BUILDINFO][..]),
            S_SOURCELINK => None,
            _ => Some(FUNCTION_IDS),
        };
        check_id(id, expected, offset, kind, context, report)?;
    }
    let functions = match symbol.data() {
        SymbolData::Inlinees(list) => Some(*list),
        SymbolData::Callees(list) | SymbolData::Callers(list) => Some(list.functions()),
        _ => None,
    };
    for id in functions.into_iter().flatten() {
        check_id(id, Some(FUNCTION_IDS), offset, kind, context, report)?;
    }
    if let Some(reference) = symbol.reference() {
        check_module_reference(
            kind,
            reference.module,
            reference.offset,
            offset,
            context,
            report,
        )?;
    }
    Ok(())
}

/// Check that `id` is in the IPI stream and has one of the `expected` kinds.
fn check_id(
    id: IdIndex,
    expected: Option<&[u16]>,
    offset: u32,
    kind: u16,
    context: &ValidationContext<'_>,
    report: &mut ValidationReport,
) -> Result<()> {
    if id.0 < FIRST_RECORD_INDEX || id >= context.id_index_end {
        return report.violate(Rule::IdIndexRange, offset, kind);
    }
    if let Some(id_kind) = context.id_kind {
        match id_kind(id) {
            None => report.violate(Rule::IdIndexRange, offset, kind)?,
            Some(found) if expected.is_some_and(|expected| !expected.contains(&found)) => {
                report.violate(Rule::IdKind, offset, kind)?;
            }
            Some(_) => {}
        }
    }
    Ok(())
}

/// Return the record kinds that a module reference of `kind` can identify.
fn reference_targets(kind: u16) -> &'static [u16] {
    match kind {
        S_PROCREF => GLOBAL_REFERENCE_TARGETS,
        S_LPROCREF => LOCAL_REFERENCE_TARGETS,
        S_TOKENREF => MANAGED_PROCEDURES,
        S_DATAREF => DATA,
        S_ANNOTATIONREF => &[S_ANNOTATION],
        _ => &[],
    }
}

/// Check the module number and the target record of a module reference.
fn check_module_reference(
    kind: u16,
    module: u16,
    target: u32,
    offset: u32,
    context: &ValidationContext<'_>,
    report: &mut ValidationReport,
) -> Result<()> {
    let Some(index) = module.checked_sub(1) else {
        return report.violate(Rule::ModuleIndex, offset, kind);
    };
    if context
        .module_count
        .is_some_and(|count| usize::from(index) >= count)
    {
        return report.violate(Rule::ModuleIndex, offset, kind);
    }
    let Some(module_record) = context.module_record else {
        return Ok(());
    };
    match module_record(index, target) {
        ModuleRecord::Kind(found) if !reference_targets(kind).contains(&found) => {
            report.violate(Rule::ModuleRecordKind, offset, kind)
        }
        ModuleRecord::Missing => report.violate(Rule::ModuleRecordMissing, offset, kind),
        ModuleRecord::Kind(_) | ModuleRecord::Unknown => Ok(()),
    }
}
