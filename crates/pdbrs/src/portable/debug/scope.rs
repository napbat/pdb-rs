//! The `LocalScope`, `LocalVariable` and `StateMachineMethod` tables: records and handles.
//!
//! The `VariableList` and `ConstantList` columns of a scope give the first row of a run. The
//! run ends before the first row of the run of the next scope, or after the last row of the
//! table for the last scope. A [`LocalScope`] record holds the variables and the constants of
//! its runs.

use alloc::vec::Vec;
use core::ops::Range;

use super::LocalConstant;
use super::rows::{TypedRows, each_row, row_error};
use crate::bytes::{Bytes, into_owned};
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::metadata::{
    IlOffset, ImportScopeId, LocalConstantId, LocalScopeId, LocalScopeRow, LocalVariableId,
    LocalVariableRow, MethodDefId, StateMachineMethodId, StateMachineMethodRow, TableId,
};
use crate::{ErrorKind, Result};

/// The `DebuggerHidden` flag of `LocalVariable.Attributes`.
const DEBUGGER_HIDDEN: u16 = 0x0001;

/// A handle of a local scope: the index of the scope in the order of addition.
///
/// The plan of a [`PdbEditor`](crate::portable::PdbEditor) gives the scope its row ID. An
/// editor of a source holds the source scopes in row order, so the reader gives the scope of
/// source row `r` the handle with index `r - 1`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LocalScopeHandle(u32);

impl LocalScopeHandle {
    /// Make the handle of the scope at the zero-based index `index`.
    pub(crate) const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Return the zero-based index of the scope.
    pub(crate) fn index(self) -> usize {
        // A `u32` always fits in `usize` on the supported targets.
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

/// A handle of a local variable: its scope and its position in the variables of the scope.
///
/// A change of the order of [`LocalScope::variables`] changes the variable that the handle
/// names.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LocalVariableHandle {
    /// The scope.
    pub scope: LocalScopeHandle,
    /// The zero-based position in [`LocalScope::variables`].
    pub index: u32,
}

/// A handle of a local constant: its scope and its position in the constants of the scope.
///
/// A change of the order of [`LocalScope::constants`] changes the constant that the handle
/// names.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LocalConstantHandle {
    /// The scope.
    pub scope: LocalScopeHandle,
    /// The zero-based position in [`LocalScope::constants`].
    pub index: u32,
}

/// A local scope of a method with its local variables and local constants.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct LocalScope<'a> {
    /// The method.
    pub method: MethodDefId,
    /// The import scope, or nil.
    pub import_scope: ImportScopeId,
    /// The IL offset of the scope start.
    pub start_offset: IlOffset,
    /// The scope length in IL bytes.
    pub length: u32,
    /// The local variables, in row order.
    pub variables: Vec<LocalVariable<'a>>,
    /// The local constants, in row order.
    pub constants: Vec<LocalConstant<'a>>,
}

into_owned!(struct LocalScope { variables, constants; method, import_scope, start_offset, length });

impl LocalScope<'_> {
    /// Return the IL offset after the scope end, or `None` if it does not fit in 32 bits.
    #[must_use]
    pub const fn end_offset(&self) -> Option<IlOffset> {
        match self.start_offset.0.checked_add(self.length) {
            Some(end) => Some(IlOffset(end)),
            None => None,
        }
    }
}

/// A local variable of a local scope.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct LocalVariable<'a> {
    /// The `LocalVariableAttributes` flags.
    pub attributes: u16,
    /// The slot index in the local signature of the method.
    pub index: u16,
    /// The name. The bytes are UTF-8 in valid metadata.
    pub name: Bytes<'a>,
}

into_owned!(struct LocalVariable { name; attributes, index });

impl LocalVariable<'_> {
    /// Return `true` if the debugger does not show the variable (`DebuggerHidden`).
    #[must_use]
    pub const fn is_debugger_hidden(&self) -> bool {
        self.attributes & DEBUGGER_HIDDEN != 0
    }
}

/// A state machine method: the `MoveNext` method and its kickoff method.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct StateMachineMethod {
    /// The `MoveNext` method of the state machine.
    pub move_next_method: MethodDefId,
    /// The kickoff method: the async or iterator method that starts the state machine.
    pub kickoff_method: MethodDefId,
}

/// A `LocalScope` row with its variable and constant runs.
#[derive(Clone, Debug)]
pub(crate) struct ScopeRuns {
    /// The raw row.
    pub(crate) row: LocalScopeRow,
    /// The `LocalVariable` row numbers of the scope.
    pub(crate) variables: Range<u32>,
    /// The `LocalConstant` row numbers of the scope.
    pub(crate) constants: Range<u32>,
}

impl<S: ReadAt> Pdb<S> {
    /// Read a `LocalScope` row and compute its variable and constant runs.
    ///
    /// A list column of 0 (nil) is an empty run if its table has no rows. The F# compiler
    /// writes nil `ConstantList` values; `System.Reflection.Metadata` also reads nil as an
    /// empty run.
    pub(crate) fn scope_runs(&self, id: LocalScopeId) -> Result<ScopeRuns> {
        let row: LocalScopeRow = self.row(id)?;
        let tables = self.tables();
        let count = tables.row_count(TableId::LOCAL_SCOPE);
        let next: Option<LocalScopeRow> = if id.0 < count {
            Some(self.row(LocalScopeId(id.0 + 1))?)
        } else {
            None
        };
        let run = |table, start: u32, next: Option<u32>, column| {
            let rows = tables.row_count(table);
            let error = |context| {
                row_error(
                    ErrorKind::InvalidFormat,
                    context,
                    TableId::LOCAL_SCOPE,
                    id.0,
                    Some(column),
                )
            };
            if rows == 0 {
                return Ok(1..1);
            }
            if start == 0 {
                return Err(error("list is nil, but its table has rows"));
            }
            let end = next.unwrap_or(rows + 1);
            if start <= end {
                Ok(start..end)
            } else {
                Err(error("list starts after the list of the next scope"))
            }
        };
        Ok(ScopeRuns {
            row,
            variables: run(
                TableId::LOCAL_VARIABLE,
                row.variable_list.0,
                next.map(|next| next.variable_list.0),
                "VariableList",
            )?,
            constants: run(
                TableId::LOCAL_CONSTANT,
                row.constant_list.0,
                next.map(|next| next.constant_list.0),
                "ConstantList",
            )?,
        })
    }

    /// Read a local scope with the variables and the constants of its runs.
    ///
    /// # Errors
    ///
    /// - The errors of [`Pdb::row`] for the row and for the next row.
    /// - [`ErrorKind::InvalidFormat`] if a run starts after the run of the next scope, or a
    ///   list column is nil and its table has rows.
    /// - The errors of [`Pdb::local_variable`] and [`Pdb::local_constant`].
    pub fn local_scope(&self, id: LocalScopeId) -> Result<LocalScope<'static>> {
        let runs = self.scope_runs(id)?;
        let variables = runs
            .variables
            .map(|row| self.local_variable(LocalVariableId(row)))
            .collect::<Result<Vec<_>>>()?;
        let constants = runs
            .constants
            .map(|row| self.local_constant(LocalConstantId(row)))
            .collect::<Result<Vec<_>>>()?;
        Ok(LocalScope {
            method: runs.row.method,
            import_scope: runs.row.import_scope,
            start_offset: runs.row.start_offset,
            length: runs.row.length,
            variables,
            constants,
        })
    }

    /// Read all local scopes in row order, each with its row ID.
    #[must_use]
    pub fn local_scopes(&self) -> TypedRows<'_, S, (LocalScopeId, LocalScope<'static>)> {
        each_row(self, TableId::LOCAL_SCOPE, |pdb, row| {
            let id = LocalScopeId(row);
            pdb.local_scope(id).map(|scope| (id, scope))
        })
    }

    /// Return the scope whose run holds the row `row` of `table` (`LocalVariable` or
    /// `LocalConstant`), and the position of the row in the run.
    ///
    /// The search assumes runs that do not decrease, as [`Pdb::local_scope`] checks them.
    pub(crate) fn local_owner(&self, table: TableId, row: u32) -> Result<(LocalScopeHandle, u32)> {
        let list = |scope: &LocalScopeRow| {
            if table == TableId::LOCAL_VARIABLE {
                scope.variable_list.0
            } else {
                scope.constant_list.0
            }
        };
        // Find the last scope whose run starts at or before `row`.
        let (mut low, mut high) = (1, self.tables().row_count(TableId::LOCAL_SCOPE) + 1);
        while low < high {
            let middle = low + (high - low) / 2;
            let scope: LocalScopeRow = self.row(LocalScopeId(middle))?;
            if list(&scope) <= row {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let owner = low - 1;
        if owner == 0 {
            return Err(row_error(
                ErrorKind::InvalidFormat,
                "the row is not in the run of a local scope",
                table,
                row,
                None,
            ));
        }
        let start = list(&self.row(LocalScopeId(owner))?);
        Ok((LocalScopeHandle(owner - 1), row - start))
    }

    /// Read a local variable with its name.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::row`] and [`Pdb::string`].
    pub fn local_variable(&self, id: LocalVariableId) -> Result<LocalVariable<'static>> {
        let row: LocalVariableRow = self.row(id)?;
        Ok(LocalVariable {
            attributes: row.attributes,
            index: row.index,
            name: self.string(row.name)?.into(),
        })
    }

    /// Read all local variables in row order, each with its row ID.
    #[must_use]
    pub fn local_variables(&self) -> TypedRows<'_, S, (LocalVariableId, LocalVariable<'static>)> {
        each_row(self, TableId::LOCAL_VARIABLE, |pdb, row| {
            let id = LocalVariableId(row);
            pdb.local_variable(id).map(|variable| (id, variable))
        })
    }

    /// Read a state machine method.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::row`].
    pub fn state_machine_method(&self, id: StateMachineMethodId) -> Result<StateMachineMethod> {
        let row: StateMachineMethodRow = self.row(id)?;
        Ok(StateMachineMethod {
            move_next_method: row.move_next_method,
            kickoff_method: row.kickoff_method,
        })
    }

    /// Read all state machine methods in row order, each with its row ID.
    #[must_use]
    pub fn state_machine_methods(
        &self,
    ) -> TypedRows<'_, S, (StateMachineMethodId, StateMachineMethod)> {
        each_row(self, TableId::STATE_MACHINE_METHOD, |pdb, row| {
            let id = StateMachineMethodId(row);
            pdb.state_machine_method(id).map(|method| (id, method))
        })
    }
}
