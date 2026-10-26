//! The `LocalScope`, `LocalVariable` and `StateMachineMethod` tables: typed rows.
//!
//! The `VariableList` and `ConstantList` columns of a scope give the first row of a run. The
//! run ends before the first row of the run of the next scope, or after the last row of the
//! table for the last scope.

use alloc::vec::Vec;
use core::ops::Range;

use super::rows::{each_row, row_error};
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::metadata::{
    IlOffset, ImportScopeId, LocalConstantId, LocalScopeId, LocalScopeRow, LocalVariableId,
    LocalVariableRow, MethodDefId, StateMachineMethodId, StateMachineMethodRow, TableId,
};
use crate::{ErrorKind, Result};

/// The `DebuggerHidden` flag of `LocalVariable.Attributes`.
const DEBUGGER_HIDDEN: u16 = 0x0001;

/// A typed row of the `LocalScope` table with its variable and constant runs.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LocalScope {
    id: LocalScopeId,
    row: LocalScopeRow,
    variables: Range<u32>,
    constants: Range<u32>,
}

impl LocalScope {
    /// Return the row ID.
    #[must_use]
    pub const fn id(&self) -> LocalScopeId {
        self.id
    }

    /// Return the raw row.
    #[must_use]
    pub const fn row(&self) -> &LocalScopeRow {
        &self.row
    }

    /// Return the method of the scope.
    #[must_use]
    pub const fn method(&self) -> MethodDefId {
        self.row.method
    }

    /// Return the import scope.
    #[must_use]
    pub const fn import_scope(&self) -> ImportScopeId {
        self.row.import_scope
    }

    /// Return the IL offset of the scope start.
    #[must_use]
    pub const fn start_offset(&self) -> IlOffset {
        self.row.start_offset
    }

    /// Return the scope length in IL bytes.
    #[must_use]
    pub const fn length(&self) -> u32 {
        self.row.length
    }

    /// Return the IL offset after the scope end, or `None` if it does not fit in 32 bits.
    #[must_use]
    pub const fn end_offset(&self) -> Option<IlOffset> {
        match self.row.start_offset.0.checked_add(self.row.length) {
            Some(end) => Some(IlOffset(end)),
            None => None,
        }
    }

    /// Return the local variables of the scope in row order.
    pub fn variables(&self) -> impl ExactSizeIterator<Item = LocalVariableId> + use<> {
        self.variables.clone().map(LocalVariableId)
    }

    /// Return the local constants of the scope in row order.
    pub fn constants(&self) -> impl ExactSizeIterator<Item = LocalConstantId> + use<> {
        self.constants.clone().map(LocalConstantId)
    }
}

/// A typed row of the `LocalVariable` table with its name.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct LocalVariable {
    id: LocalVariableId,
    row: LocalVariableRow,
    name: Vec<u8>,
}

impl LocalVariable {
    /// Return the row ID.
    #[must_use]
    pub const fn id(&self) -> LocalVariableId {
        self.id
    }

    /// Return the raw row.
    #[must_use]
    pub const fn row(&self) -> &LocalVariableRow {
        &self.row
    }

    /// Return the `LocalVariableAttributes` flags.
    #[must_use]
    pub const fn attributes(&self) -> u16 {
        self.row.attributes
    }

    /// Return `true` if the debugger does not show the variable (`DebuggerHidden`).
    #[must_use]
    pub const fn is_debugger_hidden(&self) -> bool {
        self.row.attributes & DEBUGGER_HIDDEN != 0
    }

    /// Return the slot index in the local signature of the method.
    #[must_use]
    pub const fn index(&self) -> u16 {
        self.row.index
    }

    /// Return the name. The bytes are UTF-8 in valid metadata.
    #[must_use]
    pub fn name(&self) -> &[u8] {
        &self.name
    }
}

/// A typed row of the `StateMachineMethod` table.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StateMachineMethod {
    id: StateMachineMethodId,
    row: StateMachineMethodRow,
}

impl StateMachineMethod {
    /// Return the row ID.
    #[must_use]
    pub const fn id(&self) -> StateMachineMethodId {
        self.id
    }

    /// Return the raw row.
    #[must_use]
    pub const fn row(&self) -> &StateMachineMethodRow {
        &self.row
    }

    /// Return the `MoveNext` method of the state machine.
    #[must_use]
    pub const fn move_next_method(&self) -> MethodDefId {
        self.row.move_next_method
    }

    /// Return the kickoff method: the async or iterator method that starts the state machine.
    #[must_use]
    pub const fn kickoff_method(&self) -> MethodDefId {
        self.row.kickoff_method
    }
}

impl<S: ReadAt> Pdb<S> {
    /// Read a `LocalScope` row and compute its variable and constant runs.
    ///
    /// A list column of 0 (nil) is an empty run if its table has no rows. The F# compiler
    /// writes nil `ConstantList` values; `System.Reflection.Metadata` also reads nil as an
    /// empty run.
    ///
    /// # Errors
    ///
    /// - The errors of [`Pdb::row`] for the row and for the next row.
    /// - [`ErrorKind::InvalidFormat`] if a run starts after the run of the next scope, or a
    ///   list column is nil and its table has rows.
    pub fn local_scope(&self, id: LocalScopeId) -> Result<LocalScope> {
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
        Ok(LocalScope {
            id,
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

    /// Read all `LocalScope` rows in row order.
    pub fn local_scopes(&self) -> impl ExactSizeIterator<Item = Result<LocalScope>> + '_ {
        each_row(self, TableId::LOCAL_SCOPE, |pdb, row| {
            pdb.local_scope(LocalScopeId(row))
        })
    }

    /// Read a `LocalVariable` row with its name.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::row`] and [`Pdb::string`].
    pub fn local_variable(&self, id: LocalVariableId) -> Result<LocalVariable> {
        let row: LocalVariableRow = self.row(id)?;
        Ok(LocalVariable {
            id,
            row,
            name: self.string(row.name)?,
        })
    }

    /// Read all `LocalVariable` rows in row order.
    pub fn local_variables(&self) -> impl ExactSizeIterator<Item = Result<LocalVariable>> + '_ {
        each_row(self, TableId::LOCAL_VARIABLE, |pdb, row| {
            pdb.local_variable(LocalVariableId(row))
        })
    }

    /// Read a `StateMachineMethod` row.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::row`].
    pub fn state_machine_method(&self, id: StateMachineMethodId) -> Result<StateMachineMethod> {
        Ok(StateMachineMethod {
            id,
            row: self.row(id)?,
        })
    }

    /// Read all `StateMachineMethod` rows in row order.
    pub fn state_machine_methods(
        &self,
    ) -> impl ExactSizeIterator<Item = Result<StateMachineMethod>> + '_ {
        each_row(self, TableId::STATE_MACHINE_METHOD, |pdb, row| {
            pdb.state_machine_method(StateMachineMethodId(row))
        })
    }
}
