//! The record collections of a [`PdbEditor`](crate::portable::PdbEditor).

use alloc::vec::Vec;

use crate::PortableLocation;
use crate::portable::custom::{CustomDebugHandle, CustomDebugInformation};
use crate::portable::debug::{Document, ImportScope, LocalScope, LocalScopeHandle};
use crate::portable::metadata::{DocumentId, ImportScopeId, MAX_ROW, MetadataStream, TableId};
use crate::{Error, ErrorKind, Result};

/// Return the zero-based index of the next record of a list of `len` records.
///
/// # Errors
///
/// [`ErrorKind::LimitExceeded`] if the table is full.
fn next(len: usize, table: TableId) -> Result<u32> {
    u32::try_from(len)
        .ok()
        .filter(|&len| len < MAX_ROW)
        .ok_or_else(|| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("row count does not fit in a metadata token")
                .with_portable(
                    PortableLocation::new()
                        .with_stream(MetadataStream::Tables)
                        .with_table(table),
                )
        })
}

/// Return the zero-based index of a 1-based row ID.
fn index(row: u32) -> Option<usize> {
    usize::try_from(row).ok()?.checked_sub(1)
}

/// Return the 1-based row ID of a zero-based index. A list holds at most
/// [`MAX_ROW`] records.
fn row(index: usize) -> u32 {
    u32::try_from(index + 1).unwrap_or(u32::MAX)
}

/// Define a collection of records whose IDs keep the order of addition.
macro_rules! id_table {
    ($(#[$doc:meta])* $name:ident, $record:ident, $id:ident, $table:expr) => {
        $(#[$doc])*
        #[derive(Clone, Debug, Default, Eq, PartialEq)]
        pub struct $name {
            records: Vec<$record<'static>>,
        }

        impl $name {
            /// Make a collection with the records in row order.
            pub(super) const fn from_records(records: Vec<$record<'static>>) -> Self {
                Self { records }
            }

            /// Add a record after the other records and return its ID.
            ///
            /// # Errors
            ///
            /// [`ErrorKind::LimitExceeded`] if the table is full.
            pub fn push(&mut self, record: $record<'static>) -> Result<$id> {
                let id = $id(next(self.records.len(), $table)? + 1);
                self.records.push(record);
                Ok(id)
            }

            /// Return a record, or `None` if the collection has no record with the ID.
            #[must_use]
            pub fn get(&self, id: $id) -> Option<&$record<'static>> {
                self.records.get(index(id.0)?)
            }

            /// Return a record for a change.
            pub fn get_mut(&mut self, id: $id) -> Option<&mut $record<'static>> {
                self.records.get_mut(index(id.0)?)
            }

            /// Return each record with its ID, in ID order.
            pub fn iter(&self) -> impl ExactSizeIterator<Item = ($id, &$record<'static>)> {
                self.records
                    .iter()
                    .enumerate()
                    .map(|(index, record)| ($id(row(index)), record))
            }

            /// Return each record with its ID for a change, in ID order.
            pub fn iter_mut(
                &mut self,
            ) -> impl ExactSizeIterator<Item = ($id, &mut $record<'static>)> {
                self.records
                    .iter_mut()
                    .enumerate()
                    .map(|(index, record)| ($id(row(index)), record))
            }

            /// Return the number of records.
            #[must_use]
            pub fn len(&self) -> usize {
                self.records.len()
            }

            /// Return `true` if the collection has no records.
            #[must_use]
            pub fn is_empty(&self) -> bool {
                self.records.is_empty()
            }
        }
    };
}

id_table!(
    /// The documents of an editor. A document ID is its row ID in the output.
    Documents,
    Document,
    DocumentId,
    TableId::DOCUMENT
);

id_table!(
    /// The import scopes of an editor. An import scope ID is its row ID in the output.
    ImportScopes,
    ImportScope,
    ImportScopeId,
    TableId::IMPORT_SCOPE
);

/// The run of the variables or the constants of a source scope.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct SourceRun {
    /// The first source row.
    pub(super) start: u32,
    /// The number of rows.
    pub(super) count: u32,
}

/// The value of the `VariableList` or `ConstantList` column of each local scope if the
/// `LocalVariable` or `LocalConstant` table has no rows. Both values give empty runs.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum EmptyListForm {
    /// The column holds 1, the first row, as Roslyn and `System.Reflection.Metadata` write it.
    #[default]
    First,
    /// The column holds 0 (nil), as the F# compiler writes it.
    Nil,
}

impl EmptyListForm {
    /// Return the form of a list column value of the first scope of an empty table.
    pub(super) const fn from_value(value: u32) -> Self {
        if value == 0 { Self::Nil } else { Self::First }
    }

    /// Return the column value of the form.
    pub(super) const fn value(self) -> u32 {
        match self {
            Self::First => 1,
            Self::Nil => 0,
        }
    }
}

/// The local scopes of an editor, with their variables and constants.
///
/// The plan sorts the scopes by method, start offset and length (longest first), and gives
/// each scope its row ID. A [`LocalScopeHandle`] names a scope before the plan.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LocalScopes {
    pub(super) records: Vec<LocalScope<'static>>,
    /// The variable run and the constant run of each source scope, in source row order.
    pub(super) source: Vec<[SourceRun; 2]>,
    /// The form of the `VariableList` and the `ConstantList` column for an empty table.
    pub(super) empty_lists: [EmptyListForm; 2],
}

impl LocalScopes {
    /// Return the form of the `VariableList` column if the `LocalVariable` table has no rows.
    /// An editor of a source takes it from the source.
    #[must_use]
    pub const fn variable_list_form(&self) -> EmptyListForm {
        self.empty_lists[0]
    }

    /// Set the form of the `VariableList` column for an empty `LocalVariable` table.
    pub const fn set_variable_list_form(&mut self, form: EmptyListForm) {
        self.empty_lists[0] = form;
    }

    /// Return the form of the `ConstantList` column if the `LocalConstant` table has no rows.
    /// An editor of a source takes it from the source.
    #[must_use]
    pub const fn constant_list_form(&self) -> EmptyListForm {
        self.empty_lists[1]
    }

    /// Set the form of the `ConstantList` column for an empty `LocalConstant` table.
    pub const fn set_constant_list_form(&mut self, form: EmptyListForm) {
        self.empty_lists[1] = form;
    }

    /// Add a scope after the other scopes and return its handle.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::LimitExceeded`] if the table is full.
    pub fn push(&mut self, scope: LocalScope<'static>) -> Result<LocalScopeHandle> {
        let handle = LocalScopeHandle::new(next(self.records.len(), TableId::LOCAL_SCOPE)?);
        self.records.push(scope);
        Ok(handle)
    }

    /// Return a scope, or `None` if the collection has no scope with the handle.
    #[must_use]
    pub fn get(&self, handle: LocalScopeHandle) -> Option<&LocalScope<'static>> {
        self.records.get(handle.index())
    }

    /// Return a scope for a change.
    pub fn get_mut(&mut self, handle: LocalScopeHandle) -> Option<&mut LocalScope<'static>> {
        self.records.get_mut(handle.index())
    }

    /// Return each scope with its handle, in the order of addition.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (LocalScopeHandle, &LocalScope<'static>)> {
        self.records
            .iter()
            .enumerate()
            .map(|(index, scope)| (LocalScopeHandle::new(row(index) - 1), scope))
    }

    /// Return each scope with its handle for a change, in the order of addition.
    pub fn iter_mut(
        &mut self,
    ) -> impl ExactSizeIterator<Item = (LocalScopeHandle, &mut LocalScope<'static>)> {
        self.records
            .iter_mut()
            .enumerate()
            .map(|(index, scope)| (LocalScopeHandle::new(row(index) - 1), scope))
    }

    /// Return the number of scopes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Return `true` if the collection has no scopes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// The custom debug information records of an editor.
///
/// The plan sorts the records by the coded parent; records with the same parent keep the
/// order of addition. A [`CustomDebugHandle`] names a record before the plan.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CustomDebugInformations {
    pub(super) records: Vec<Option<CustomDebugInformation<'static>>>,
}

impl CustomDebugInformations {
    /// Add a record and return its handle.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::LimitExceeded`] if the table is full.
    pub fn push(
        &mut self,
        information: CustomDebugInformation<'static>,
    ) -> Result<CustomDebugHandle> {
        let table = TableId::CUSTOM_DEBUG_INFORMATION;
        let handle = CustomDebugHandle::new(next(self.records.len(), table)?);
        self.records.push(Some(information));
        Ok(handle)
    }

    /// Return a record, or `None` if the collection has no record with the handle.
    #[must_use]
    pub fn get(&self, handle: CustomDebugHandle) -> Option<&CustomDebugInformation<'static>> {
        self.records.get(handle.index())?.as_ref()
    }

    /// Return a record for a change.
    pub fn get_mut(
        &mut self,
        handle: CustomDebugHandle,
    ) -> Option<&mut CustomDebugInformation<'static>> {
        self.records.get_mut(handle.index())?.as_mut()
    }

    /// Remove a record and return it. The other handles stay valid.
    pub fn remove(&mut self, handle: CustomDebugHandle) -> Option<CustomDebugInformation<'static>> {
        self.records.get_mut(handle.index())?.take()
    }

    /// Return each record with its handle, in the order of addition.
    pub fn iter(
        &self,
    ) -> impl Iterator<Item = (CustomDebugHandle, &CustomDebugInformation<'static>)> {
        (0..).zip(&self.records).filter_map(|(index, information)| {
            Some((CustomDebugHandle::new(index), information.as_ref()?))
        })
    }

    /// Return each record with its handle for a change, in the order of addition.
    pub fn iter_mut(
        &mut self,
    ) -> impl Iterator<Item = (CustomDebugHandle, &mut CustomDebugInformation<'static>)> {
        (0..)
            .zip(&mut self.records)
            .filter_map(|(index, information)| {
                Some((CustomDebugHandle::new(index), information.as_mut()?))
            })
    }

    /// Return the number of records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.iter().flatten().count()
    }

    /// Return `true` if the collection has no records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.iter().all(Option::is_none)
    }
}
