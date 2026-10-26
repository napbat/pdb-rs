//! Validation of the ordering and reference rules of the debug tables.
//!
//! [`Pdb::validate_debug`] reads every row and blob of the debug tables and checks the rules
//! of the specification that need no assembly. It stops at the first violation. On success it
//! returns a [`ValidationReport`] with each rule and the number of items that it checked. Rules
//! that need the IL bodies of the assembly are in the report as not checked.

mod imports;
mod methods;
mod order;
mod scopes;

use alloc::vec::Vec;

use crate::Result;
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::metadata::{MetadataToken, TableId};

/// The options of [`Pdb::validate_debug`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ValidateOptions {
    /// The row count of the `MethodDef` table of the assembly.
    ///
    /// If it is `None`, the validation uses the external count of the `#Pdb` stream. If the
    /// `#Pdb` stream does not reference `MethodDef` either, the row count rule of
    /// `MethodDebugInformation` is not checked.
    pub method_def_count: Option<u32>,
}

/// A rule that [`Pdb::validate_debug`] checks.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Rule {
    /// Each document name blob decodes, its parts are in the `#Blob` heap and no two
    /// documents have the same name.
    DocumentNames,
    /// The hash of each document with a defined hash algorithm has the size of that algorithm.
    DocumentHashes,
    /// The `MethodDebugInformation` table is empty or has one row for each `MethodDef` row.
    MethodDebugInformationCount,
    /// Each sequence point blob decodes: the IL offsets increase, the lines and columns are in
    /// range, the documents and the `LocalSignature` row exist, and a method without sequence
    /// points has a nil `Document` column.
    SequencePoints,
    /// The `LocalScope` table is sorted by `Method`, then `StartOffset`, then `Length` in
    /// descending order, and each scope has a non-nil method.
    LocalScopeOrder,
    /// Each scope has a length above 0 and an end below `0x80000000`. The first scope of each
    /// method starts at 0 and contains the other scopes of the method. Two scopes of a method
    /// are nested or do not overlap.
    LocalScopeRanges,
    /// The `VariableList` runs are contiguous and give each `LocalVariable` row to one scope.
    VariableRuns,
    /// The `ConstantList` runs are contiguous and give each `LocalConstant` row to one scope.
    ConstantRuns,
    /// No two variables of a scope have the same slot index or the same name.
    LocalVariables,
    /// Each constant signature decodes, its type rows exist and no two constants of a scope
    /// have the same name.
    LocalConstants,
    /// The parent chains of the import scopes have no cycle.
    ImportScopeParents,
    /// Each imports blob decodes, and its assemblies, types, aliases and namespaces exist.
    Imports,
    /// The `StateMachineMethod` table is sorted by `MoveNextMethod` with no duplicate, has no
    /// duplicate `KickoffMethod` and has no nil method.
    StateMachineMethods,
    /// The `CustomDebugInformation` table is sorted by `Parent` and has no nil parent.
    CustomDebugInformationParents,
    /// IL offsets of sequence points and scopes are at instruction starts.
    InstructionBoundaries,
    /// The first scope of each method spans the full IL body.
    MethodBodySizes,
}

/// The result of one rule.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Outcome {
    /// The rule was checked on `items` items without a violation.
    Checked {
        /// The number of checked items, for example rows or sequence points.
        items: u64,
    },
    /// The rule was not checked.
    NotChecked {
        /// The missing input.
        reason: &'static str,
    },
}

/// One rule and its result.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RuleReport {
    /// The rule.
    pub rule: Rule,
    /// The result.
    pub outcome: Outcome,
}

/// The report of a successful [`Pdb::validate_debug`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ValidationReport {
    rules: Vec<RuleReport>,
}

impl ValidationReport {
    /// Return each rule with its result, in check order.
    #[must_use]
    pub fn rules(&self) -> &[RuleReport] {
        &self.rules
    }

    /// Return the result of a rule, if the report has it.
    #[must_use]
    pub fn outcome(&self, rule: Rule) -> Option<Outcome> {
        self.rules
            .iter()
            .find(|report| report.rule == rule)
            .map(|report| report.outcome)
    }

    /// Add a checked rule.
    fn checked(&mut self, rule: Rule, items: u64) {
        self.rules.push(RuleReport {
            rule,
            outcome: Outcome::Checked { items },
        });
    }

    /// Add a rule that was not checked.
    fn not_checked(&mut self, rule: Rule, reason: &'static str) {
        self.rules.push(RuleReport {
            rule,
            outcome: Outcome::NotChecked { reason },
        });
    }
}

/// The reason for the rules that need method bodies.
const NEEDS_BODIES: &str = "needs the IL method bodies of the assembly";

impl<S: ReadAt> Pdb<S> {
    /// Check the ordering and reference rules of the debug tables.
    ///
    /// The validation reads every row and blob of the debug tables. References to external
    /// tables (for example `TypeRef` rows in constant signatures) are checked against the row
    /// counts of the `#Pdb` stream.
    ///
    /// # Errors
    ///
    /// Returns the first violation, with its location:
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) for a broken rule or a
    ///   blob that does not decode.
    /// - [`ErrorKind::InvalidReference`](crate::ErrorKind::InvalidReference) for a reference
    ///   to a row or heap entry that does not exist.
    /// - The errors of the row, heap and blob accessors.
    pub fn validate_debug(&self, options: ValidateOptions) -> Result<ValidationReport> {
        let mut report = ValidationReport::default();
        methods::documents(self, &mut report)?;
        methods::method_count(self, options, &mut report)?;
        methods::sequence_points(self, &mut report)?;
        scopes::scopes(self, &mut report)?;
        scopes::variables(self, &mut report)?;
        scopes::constants(self, &mut report)?;
        imports::import_scopes(self, &mut report)?;
        order::state_machine_methods(self, &mut report)?;
        order::custom_debug_information(self, &mut report)?;
        report.not_checked(Rule::InstructionBoundaries, NEEDS_BODIES);
        report.not_checked(Rule::MethodBodySizes, NEEDS_BODIES);
        Ok(report)
    }
}

/// Return `true` if the row of an external token exists: it is not nil and at most the row
/// count of its table in the `#Pdb` stream.
fn external_exists<S: ReadAt>(pdb: &Pdb<S>, token: MetadataToken) -> bool {
    TableId::new(token.table()).is_some_and(|table| {
        token.row() != 0 && token.row() <= pdb.pdb_stream().external_row_count(table)
    })
}

/// Return a count as a `u64`.
fn count(items: usize) -> u64 {
    u64::try_from(items).unwrap_or(u64::MAX)
}
