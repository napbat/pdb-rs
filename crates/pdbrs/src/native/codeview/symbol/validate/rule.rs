//! The validation rules and the violations that they report.

use crate::native::codeview::SymbolKind;
use crate::native::msf::StreamId;
use crate::{Error, ErrorKind};

/// A rule that a symbol record can break.
///
/// [`Rule::description`] gives a short text for each rule. The text is also
/// the error context of [`Violation::error`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Rule {
    /// The walk of the module stream stopped. A record does not decode, or
    /// a parent link, an end link or an end kind breaks a check of
    /// [`ModuleSymbols`](super::super::ModuleSymbols). The violation keeps
    /// the error kind and the context of that error.
    Walk,
    /// An `S_PROC_ID_END` record ends a scope that is not a procedure with an
    /// ID index.
    ProcedureIdEndKind,
    /// An `S_END` record ends a procedure with an ID index. Such a procedure
    /// ends with `S_PROC_ID_END`.
    ProcedureIdEndMissing,
    /// A block or an inline site is at the top level. These scopes are
    /// always in a procedure.
    ScopeWithoutParent,
    /// The `next` link of a procedure or a thunk is not 0 and does not
    /// identify a later procedure or thunk.
    NextLink,
    /// The code range of a block or other scope is not inside the code range
    /// of its enclosing scope.
    ScopeRange,
    /// A code range does not fit in 32 bits.
    CodeRangeOverflow,
    /// A code range names a section that the image does not have, or it
    /// goes past the end of its section.
    SectionRange,
    /// A type index is at or after the end of the TPI stream.
    TypeIndexRange,
    /// A type index below `0x1000` is not a known primitive type.
    PrimitiveType,
    /// An ID index is outside the records of the IPI stream.
    IdIndexRange,
    /// An ID index refers to an IPI record of the wrong kind. For example,
    /// an `S_BUILDINFO` record refers to an `LF_FUNC_ID` record.
    IdKind,
    /// A module reference has module number 0 or a module number after the
    /// last module.
    ModuleIndex,
    /// A module reference offset is not the start of a record in the module.
    ModuleRecordMissing,
    /// A module reference offset identifies a record of the wrong kind.
    ModuleRecordKind,
    /// A def-range record does not follow an `S_LOCAL` or `S_FILESTATIC`
    /// record, or another def-range record of the same variable. An `S_SKIP`
    /// record between them is accepted.
    DefRangeWithoutLocal,
    /// A def-range has a range length of 0, and its innermost scope with a
    /// code range is not empty.
    DefRangeEmpty,
    /// A def-range range is not inside the code range of its procedure.
    ///
    /// The check does not use the code range of a block. LLVM writes
    /// def-ranges that start before the block of the variable (Clang and
    /// Rust release fixtures), because it takes the block range from
    /// the instructions and the def-range from the variable locations.
    DefRangeOutsideProcedure,
    /// A def-range gap has a length of 0.
    GapEmpty,
    /// A def-range gap goes past the end of its range.
    GapOutsideRange,
    /// The def-range gaps are not in order of their start, or two gaps
    /// overlap.
    GapOrder,
    /// An `S_LOCAL` record is at the top level.
    LocalOutsideScope,
    /// A parameter local is not directly in a procedure or an inline site.
    ParameterScope,
    /// An `S_LOCAL` record sets a reserved flag bit (bits 11 to 15).
    LocalFlagsReserved,
    /// A local that is marked as optimized out has a def-range.
    OptimizedOutWithRange,
    /// A register number is not known for the CPU of the module.
    RegisterUnknown,
    /// A def-range in an inline site does not overlap or touch the code of
    /// that inline site or of an enclosing inline site.
    ///
    /// The binary annotations of the inline sites give their code ranges,
    /// also in the separated code chunks of the procedure. The gaps of the
    /// def-range are not part of its range. A part touches a code range if
    /// it starts at the end of the range or ends at its start.
    ///
    /// The check does not require that the def-range is inside the code of
    /// its inline site. MSVC, Clang, rustc and fxc write def-ranges that
    /// continue after the inlined code, for example for a value that the
    /// inlined function returns (producer fixtures). MSVC PGO writes
    /// def-ranges of a local of an inner inline site in the separated code
    /// of an outer inline site. The check skips a def-range of length 0 and
    /// a def-range with the "may have no user name" bit (`CV_RANGEATTR`):
    /// the name of such a variable can be out of scope in the range.
    DefRangeOutsideInlineSite,
    /// The binary annotations of an inline site do not give valid code
    /// ranges: a code offset overflows 32 bits, or a range starts before the
    /// range in front of it. The def-ranges of the site are not checked.
    InlineSiteAnnotations,
    /// The parent offset of a subfield def-range is not less than the size of
    /// the type of its local.
    ///
    /// The rule applies to `S_DEFRANGE_SUBFIELD`,
    /// `S_DEFRANGE_SUBFIELD_REGISTER` and to `S_DEFRANGE_REGISTER_REL` with
    /// the spilled-member bit. It needs
    /// [`ValidationContext::with_type_sizes`](super::ValidationContext::with_type_sizes).
    SubfieldOutsideType,
}

impl Rule {
    /// Return a short description of the rule.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Walk => "the module symbol walk stopped",
            Self::ProcedureIdEndKind => "S_PROC_ID_END ends a scope that is not an ID procedure",
            Self::ProcedureIdEndMissing => "S_END ends an ID procedure",
            Self::ScopeWithoutParent => "block or inline site is at the top level",
            Self::NextLink => "procedure next link does not identify a later procedure",
            Self::ScopeRange => "scope code range is outside its enclosing scope",
            Self::CodeRangeOverflow => "code range does not fit in 32 bits",
            Self::SectionRange => "code range is outside its section",
            Self::TypeIndexRange => "type index is outside the TPI stream",
            Self::PrimitiveType => "primitive type index has an unknown kind or mode",
            Self::IdIndexRange => "ID index is outside the IPI stream",
            Self::IdKind => "ID index refers to an IPI record of the wrong kind",
            Self::ModuleIndex => "module reference names a missing module",
            Self::ModuleRecordMissing => "module reference offset is not a record start",
            Self::ModuleRecordKind => "module reference identifies a record of the wrong kind",
            Self::DefRangeWithoutLocal => "def-range does not follow a local",
            Self::DefRangeEmpty => "def-range has an empty range",
            Self::DefRangeOutsideProcedure => "def-range is outside its procedure",
            Self::GapEmpty => "def-range gap is empty",
            Self::GapOutsideRange => "def-range gap goes past the end of its range",
            Self::GapOrder => "def-range gaps are out of order or overlap",
            Self::LocalOutsideScope => "local is at the top level",
            Self::ParameterScope => "parameter local is not directly in a procedure or inline site",
            Self::LocalFlagsReserved => "local sets a reserved flag bit",
            Self::OptimizedOutWithRange => "optimized-out local has a def-range",
            Self::RegisterUnknown => "register is not known for the module CPU",
            Self::DefRangeOutsideInlineSite => {
                "def-range does not touch the code of its inline sites"
            }
            Self::InlineSiteAnnotations => "inline site annotations give no valid code ranges",
            Self::SubfieldOutsideType => "subfield def-range offset is outside its local type",
        }
    }

    /// Return the error kind of a violation of the rule.
    ///
    /// A wrong link or index gives [`ErrorKind::InvalidReference`]. Other
    /// rules give [`ErrorKind::InvalidFormat`].
    #[must_use]
    pub const fn error_kind(self) -> ErrorKind {
        match self {
            Self::NextLink
            | Self::ScopeRange
            | Self::SectionRange
            | Self::TypeIndexRange
            | Self::IdIndexRange
            | Self::IdKind
            | Self::ModuleIndex
            | Self::ModuleRecordMissing
            | Self::ModuleRecordKind
            | Self::DefRangeOutsideProcedure
            | Self::DefRangeOutsideInlineSite
            | Self::SubfieldOutsideType => ErrorKind::InvalidReference,
            _ => ErrorKind::InvalidFormat,
        }
    }
}

/// One broken rule at one record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Violation {
    /// The broken rule.
    pub rule: Rule,
    /// The error kind. For [`Rule::Walk`], this is the kind of the walk
    /// error. For other rules, it is [`Rule::error_kind`].
    pub error_kind: ErrorKind,
    /// The error context. For [`Rule::Walk`], this is the context of the
    /// walk error. For other rules, it is [`Rule::description`].
    pub context: &'static str,
    /// The stream that holds the record.
    pub stream: StreamId,
    /// The stream offset of the record. For a module stream, the offset
    /// includes the 4-byte signature.
    pub offset: u64,
    /// The kind of the record, or `None` if the walk stopped before the
    /// record was decoded.
    pub record_kind: Option<SymbolKind>,
}

impl Violation {
    /// Create a violation of `rule` at a record.
    pub(super) fn new(rule: Rule, stream: StreamId, offset: u32, record_kind: SymbolKind) -> Self {
        Self {
            rule,
            error_kind: rule.error_kind(),
            context: rule.description(),
            stream,
            offset: u64::from(offset),
            record_kind: Some(record_kind),
        }
    }

    /// Create a [`Rule::Walk`] violation from the error that stopped a walk.
    pub(super) fn walk(error: &Error, stream: StreamId) -> Self {
        Self {
            rule: Rule::Walk,
            error_kind: error.kind(),
            context: error.context().unwrap_or(Rule::Walk.description()),
            stream,
            offset: error.stream_location().map_or(0, |(_, offset)| offset),
            record_kind: None,
        }
    }

    /// Return the violation as an [`Error`] with the stream location.
    #[must_use]
    pub fn error(&self) -> Error {
        Error::new(self.error_kind, None)
            .with_context(self.context)
            .with_stream(self.stream, self.offset)
    }
}
