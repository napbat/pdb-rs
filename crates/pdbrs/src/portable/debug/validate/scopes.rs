//! Rules of the `LocalScope`, `LocalVariable` and `LocalConstant` tables.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use core::cmp::Reverse;

use super::methods::reference;
use super::{Rule, ValidationReport, external_exists};
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::debug::rows::row_error;
use crate::portable::debug::{ConstantValue, LocalConstant, LocalScope};
use crate::portable::metadata::{MethodDefId, TableId};
use crate::{ErrorKind, Result};

/// The end of the IL range of a scope (exclusive).
const MAX_SCOPE_END: u32 = 0x8000_0000;

/// Make an [`ErrorKind::InvalidFormat`] error for a `LocalScope` row.
fn scope_error(scope: &LocalScope, context: &'static str, column: &'static str) -> crate::Error {
    row_error(
        ErrorKind::InvalidFormat,
        context,
        TableId::LOCAL_SCOPE,
        scope.id().get(),
        Some(column),
    )
}

/// The open scopes of the current method.
struct Nesting {
    method: MethodDefId,
    /// The end of the first scope of the method.
    root_end: u32,
    /// The ends of the scopes that contain the current offset, from outer to inner.
    open: Vec<u32>,
}

/// Check the order, the methods and the IL ranges of the scopes.
pub(super) fn scopes<S: ReadAt>(pdb: &Pdb<S>, report: &mut ValidationReport) -> Result<()> {
    let mut previous = None;
    let mut nesting: Option<Nesting> = None;
    let mut items = 0;
    for scope in pdb.local_scopes() {
        let scope = scope?;
        if scope.method().is_nil() {
            return Err(scope_error(&scope, "scope method is nil", "Method"));
        }
        let key = (
            scope.method(),
            scope.start_offset(),
            Reverse(scope.length()),
        );
        if previous.is_some_and(|previous| key < previous) {
            return Err(scope_error(
                &scope,
                "LocalScope table is not sorted by Method, StartOffset and descending Length",
                "Method",
            ));
        }
        previous = Some(key);
        let start = scope.start_offset().get();
        let end = scope
            .end_offset()
            .filter(|&end| scope.length() > 0 && end < MAX_SCOPE_END)
            .ok_or_else(|| {
                scope_error(&scope, "scope is empty or ends after 0x7FFFFFFF", "Length")
            })?;
        match &mut nesting {
            Some(open) if open.method == scope.method() => {
                if end > open.root_end {
                    return Err(scope_error(
                        &scope,
                        "scope is outside the first scope of its method",
                        "StartOffset",
                    ));
                }
                while open.open.last().is_some_and(|&last| last <= start) {
                    open.open.pop();
                }
                if open.open.last().is_some_and(|&last| end > last) {
                    return Err(scope_error(
                        &scope,
                        "scope overlaps another scope of its method without nesting",
                        "Length",
                    ));
                }
                open.open.push(end);
            }
            _ => {
                if start != 0 {
                    return Err(scope_error(
                        &scope,
                        "first scope of a method does not start at IL offset 0",
                        "StartOffset",
                    ));
                }
                let open = alloc::vec![end];
                nesting = Some(Nesting {
                    method: scope.method(),
                    root_end: end,
                    open,
                });
            }
        }
        items += 1;
    }
    report.checked(Rule::LocalScopeOrder, items);
    report.checked(Rule::LocalScopeRanges, items);
    Ok(())
}

/// Check that the runs of a list column start at row 1 and give each row to one scope.
///
/// [`Pdb::local_scope`] checks that the runs do not decrease and that the last run ends after
/// the last row. If the table has no rows, the runs are empty.
fn check_runs<S: ReadAt>(pdb: &Pdb<S>, table: TableId, column: &'static str) -> Result<()> {
    let rows = pdb.tables().row_count(table);
    if rows == 0 {
        return Ok(());
    }
    let Some(first) = pdb.local_scopes().next() else {
        return Err(row_error(
            ErrorKind::InvalidFormat,
            "rows have no owner scope because the LocalScope table is empty",
            table,
            1,
            None,
        ));
    };
    let first = first?;
    let start = if table == TableId::LOCAL_VARIABLE {
        first.row().variable_list.get()
    } else {
        first.row().constant_list.get()
    };
    if start == 1 {
        Ok(())
    } else {
        Err(scope_error(
            &first,
            "rows before the run of the first scope have no owner scope",
            column,
        ))
    }
}

/// Check the variable runs and the variables of each scope.
pub(super) fn variables<S: ReadAt>(pdb: &Pdb<S>, report: &mut ValidationReport) -> Result<()> {
    check_runs(pdb, TableId::LOCAL_VARIABLE, "VariableList")?;
    let mut items = 0;
    for scope in pdb.local_scopes() {
        let scope = scope?;
        let mut indexes = BTreeSet::new();
        let mut names = BTreeSet::new();
        for id in scope.variables() {
            let variable = pdb.local_variable(id)?;
            let duplicate = |context| {
                row_error(
                    ErrorKind::InvalidFormat,
                    context,
                    TableId::LOCAL_VARIABLE,
                    id.get(),
                    None,
                )
            };
            if !indexes.insert(variable.index()) {
                return Err(duplicate(
                    "two variables of a scope have the same slot index",
                ));
            }
            if !names.insert(variable.name().to_vec()) {
                return Err(duplicate("two variables of a scope have the same name"));
            }
            items += 1;
        }
    }
    report.checked(Rule::VariableRuns, items);
    report.checked(Rule::LocalVariables, items);
    Ok(())
}

/// Check the constant runs and the constants of each scope.
pub(super) fn constants<S: ReadAt>(pdb: &Pdb<S>, report: &mut ValidationReport) -> Result<()> {
    check_runs(pdb, TableId::LOCAL_CONSTANT, "ConstantList")?;
    let mut items = 0;
    for scope in pdb.local_scopes() {
        let scope = scope?;
        let mut names = BTreeSet::new();
        for id in scope.constants() {
            let constant = pdb.local_constant(id)?;
            check_signature(pdb, &constant)?;
            if !names.insert(constant.name().to_vec()) {
                return Err(row_error(
                    ErrorKind::InvalidFormat,
                    "two constants of a scope have the same name",
                    TableId::LOCAL_CONSTANT,
                    id.get(),
                    Some("Name"),
                ));
            }
            items += 1;
        }
    }
    report.checked(Rule::ConstantRuns, items);
    report.checked(Rule::LocalConstants, items);
    Ok(())
}

/// Decode a constant signature and check that its type rows exist.
fn check_signature<S: ReadAt>(pdb: &Pdb<S>, constant: &LocalConstant) -> Result<()> {
    let signature = constant.signature()?;
    let value_type = match signature.value() {
        ConstantValue::Enum { enum_type, .. } => Some(enum_type),
        ConstantValue::General { constant_type, .. } => Some(constant_type),
        ConstantValue::Primitive(_) | ConstantValue::Object => None,
    };
    let types = signature
        .modifiers()
        .map(|modifier| modifier.modifier)
        .chain(value_type);
    for token in types {
        if !external_exists(pdb, token) {
            return Err(constant.blob_error(reference("constant type row does not exist")));
        }
    }
    Ok(())
}
