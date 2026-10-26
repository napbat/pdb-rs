//! Semantic construction and editing of Portable PDBs.
//!
//! [`PdbBuilder`] and [`PdbEditor`] document the plan and the edits. The handles and the
//! records are in [`crate::portable::build`].

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::PortableLocation;
use crate::portable::metadata::{
    DocumentId, ImportScopeId, MetadataStream, MethodDefId, StateMachineMethodRow, TableId,
};
use crate::portable::write::{ExternalRowCounts, PDB_VERSION, PdbParts};
use crate::portable::{IdentityPolicy, PdbWritePlan};
use crate::{Error, ErrorKind, Result};

mod editor;
mod model;
mod resolve;

pub use editor::PdbEditor;
pub use model::{
    CustomDebugHandle, CustomDebugInfo, CustomDebugParent, DocumentInfo, ImportInfo,
    ImportScopeInfo, LocalConstantHandle, LocalConstantInfo, LocalScopeHandle, LocalScopeInfo,
    LocalVariableHandle, LocalVariableInfo, MethodInfo,
};

/// Make an [`ErrorKind::InvalidReference`] error for a handle or an ID of a table.
fn missing(table: TableId) -> Error {
    Error::new(ErrorKind::InvalidReference, None)
        .with_context("the builder has no record for the handle")
        .with_portable(
            PortableLocation::new()
                .with_stream(MetadataStream::Tables)
                .with_table(table),
        )
}

/// Return the next handle number for a list of `len` records.
fn next(len: usize, table: TableId) -> Result<u32> {
    u32::try_from(len)
        .ok()
        .filter(|&len| len < crate::portable::metadata::MAX_ROW)
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

/// The records of a Portable PDB with typed handles.
///
/// The builder holds the records of a Portable PDB as owned values: documents, the sequence
/// points of each method, local scopes with their variables and constants, import scopes,
/// state machine methods and custom debug information. Records refer to each other through
/// typed handles, not row IDs:
///
/// - Documents and import scopes keep the order of addition, so their handles are the final
///   [`DocumentId`] and [`ImportScopeId`].
/// - Local scopes, local variables, local constants and custom debug information get their
///   rows in the plan, so they have the opaque handles [`LocalScopeHandle`],
///   [`LocalVariableHandle`], [`LocalConstantHandle`] and [`CustomDebugHandle`].
///
/// [`PdbBuilder::plan`] resolves the handles and makes a [`PdbWritePlan`]:
///
/// 1. It adds the names, the blobs and the GUIDs to new heaps. It splits each document name
///    into parts as Roslyn does and encodes the name, the imports, the sequence points and the
///    local constant signatures with the codecs of [`debug`](crate::portable::debug).
/// 2. It sorts the local scopes by method, start offset and length (longest first), and gives
///    the variables and the constants of each scope one run, in the order of addition.
/// 3. It sorts the state machine methods by `MoveNext` method and the custom debug information
///    by the coded parent. Records with the same key keep the order of addition.
/// 4. It gives each handle its row ID and makes the raw rows. [`PdbParts::plan`] then checks
///    all rows and computes the index widths from the final heap sizes and row counts.
///
/// [`Pdb::edit`](crate::portable::Pdb::edit) makes a [`PdbEditor`]: a builder with the records
/// of an existing Portable PDB. See [`PdbEditor`] for the guarantees of an edit.
#[derive(Clone, Debug)]
pub struct PdbBuilder {
    version: Vec<u8>,
    entry_point: Option<MethodDefId>,
    external: ExternalRowCounts,
    documents: Vec<DocumentInfo>,
    methods: BTreeMap<u32, MethodInfo>,
    /// `true` if the `MethodDebugInformation` table has a row for each method, also if no
    /// method has sequence points.
    method_table: bool,
    scopes: Vec<LocalScopeInfo>,
    variables: Vec<(LocalScopeHandle, LocalVariableInfo)>,
    constants: Vec<(LocalScopeHandle, LocalConstantInfo)>,
    import_scopes: Vec<ImportScopeInfo>,
    state_machines: Vec<StateMachineMethodRow>,
    custom: Vec<Option<CustomDebugInfo>>,
    /// The `VariableList` and `ConstantList` value of each scope if the table has no rows:
    /// 1 (Roslyn) or 0 (the F# compiler).
    empty_lists: [u32; 2],
}

impl PdbBuilder {
    /// Make a builder without records for an assembly with the external row counts
    /// `external`.
    #[must_use]
    pub fn new(external: ExternalRowCounts) -> Self {
        Self {
            version: PDB_VERSION.to_vec(),
            entry_point: None,
            external,
            documents: Vec::new(),
            methods: BTreeMap::new(),
            method_table: false,
            scopes: Vec::new(),
            variables: Vec::new(),
            constants: Vec::new(),
            import_scopes: Vec::new(),
            state_machines: Vec::new(),
            custom: Vec::new(),
            empty_lists: [1, 1],
        }
    }

    /// Return the external row counts.
    #[must_use]
    pub const fn external(&self) -> &ExternalRowCounts {
        &self.external
    }

    /// Return the external row counts for a change.
    pub const fn external_mut(&mut self) -> &mut ExternalRowCounts {
        &mut self.external
    }

    /// Return the entry point method.
    #[must_use]
    pub const fn entry_point(&self) -> Option<MethodDefId> {
        self.entry_point
    }

    /// Set the entry point method.
    pub const fn set_entry_point(&mut self, method: Option<MethodDefId>) {
        self.entry_point = method;
    }

    /// Add a document and return its ID.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::LimitExceeded`] if the table is full.
    pub fn add_document(&mut self, document: DocumentInfo) -> Result<DocumentId> {
        let id = next(self.documents.len(), TableId::DOCUMENT)? + 1;
        self.documents.push(document);
        Ok(DocumentId(id))
    }

    /// Return a document.
    #[must_use]
    pub fn document(&self, id: DocumentId) -> Option<&DocumentInfo> {
        self.documents.get(index(id.0)?)
    }

    /// Return a document for a change.
    pub fn document_mut(&mut self, id: DocumentId) -> Option<&mut DocumentInfo> {
        self.documents.get_mut(index(id.0)?)
    }

    /// Return the number of documents.
    #[must_use]
    pub fn document_count(&self) -> usize {
        self.documents.len()
    }

    /// Set the sequence points of a method. Return the earlier value.
    ///
    /// The plan writes a `MethodDebugInformation` row for each external `MethodDef` row if a
    /// method has sequence points.
    pub fn set_method(&mut self, method: MethodDefId, info: MethodInfo) -> Option<MethodInfo> {
        self.methods.insert(method.0, info)
    }

    /// Remove the sequence points of a method and return them.
    pub fn remove_method(&mut self, method: MethodDefId) -> Option<MethodInfo> {
        self.methods.remove(&method.0)
    }

    /// Return the sequence points of a method.
    #[must_use]
    pub fn method(&self, method: MethodDefId) -> Option<&MethodInfo> {
        self.methods.get(&method.0)
    }

    /// Return the sequence points of a method for a change.
    pub fn method_mut(&mut self, method: MethodDefId) -> Option<&mut MethodInfo> {
        self.methods.get_mut(&method.0)
    }

    /// Return the methods with sequence points, in `MethodDef` order.
    #[must_use]
    pub fn methods(&self) -> Methods<'_> {
        Methods(self.methods.iter())
    }

    /// Return the methods with sequence points for a change, in `MethodDef` order.
    #[must_use]
    pub fn methods_mut(&mut self) -> MethodsMut<'_> {
        MethodsMut(self.methods.iter_mut())
    }

    /// Add a local scope and return its handle.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::LimitExceeded`] if the table is full.
    pub fn add_local_scope(&mut self, scope: LocalScopeInfo) -> Result<LocalScopeHandle> {
        let handle = LocalScopeHandle(next(self.scopes.len(), TableId::LOCAL_SCOPE)?);
        self.scopes.push(scope);
        Ok(handle)
    }

    /// Return a local scope.
    #[must_use]
    pub fn local_scope(&self, handle: LocalScopeHandle) -> Option<&LocalScopeInfo> {
        self.scopes.get(handle.index())
    }

    /// Return a local scope for a change.
    pub fn local_scope_mut(&mut self, handle: LocalScopeHandle) -> Option<&mut LocalScopeInfo> {
        self.scopes.get_mut(handle.index())
    }

    /// Return each local scope with its handle, in the order of addition.
    #[must_use]
    pub fn local_scopes(&self) -> LocalScopes<'_> {
        LocalScopes((0..).zip(&self.scopes))
    }

    /// Add a local variable to the end of the variables of `scope`. Return its handle.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the builder has no scope `scope`.
    /// - [`ErrorKind::LimitExceeded`] if the table is full.
    pub fn add_local_variable(
        &mut self,
        scope: LocalScopeHandle,
        variable: LocalVariableInfo,
    ) -> Result<LocalVariableHandle> {
        if scope.index() >= self.scopes.len() {
            return Err(missing(TableId::LOCAL_SCOPE));
        }
        let handle = LocalVariableHandle(next(self.variables.len(), TableId::LOCAL_VARIABLE)?);
        self.variables.push((scope, variable));
        Ok(handle)
    }

    /// Return a local variable and its scope.
    #[must_use]
    pub fn local_variable(
        &self,
        handle: LocalVariableHandle,
    ) -> Option<(LocalScopeHandle, &LocalVariableInfo)> {
        let (scope, variable) = self.variables.get(handle.index())?;
        Some((*scope, variable))
    }

    /// Return a local variable for a change.
    pub fn local_variable_mut(
        &mut self,
        handle: LocalVariableHandle,
    ) -> Option<&mut LocalVariableInfo> {
        Some(&mut self.variables.get_mut(handle.index())?.1)
    }

    /// Add a local constant to the end of the constants of `scope`. Return its handle.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the builder has no scope `scope`.
    /// - [`ErrorKind::LimitExceeded`] if the table is full.
    pub fn add_local_constant(
        &mut self,
        scope: LocalScopeHandle,
        constant: LocalConstantInfo,
    ) -> Result<LocalConstantHandle> {
        if scope.index() >= self.scopes.len() {
            return Err(missing(TableId::LOCAL_SCOPE));
        }
        let handle = LocalConstantHandle(next(self.constants.len(), TableId::LOCAL_CONSTANT)?);
        self.constants.push((scope, constant));
        Ok(handle)
    }

    /// Return a local constant and its scope.
    #[must_use]
    pub fn local_constant(
        &self,
        handle: LocalConstantHandle,
    ) -> Option<(LocalScopeHandle, &LocalConstantInfo)> {
        let (scope, constant) = self.constants.get(handle.index())?;
        Some((*scope, constant))
    }

    /// Return a local constant for a change.
    pub fn local_constant_mut(
        &mut self,
        handle: LocalConstantHandle,
    ) -> Option<&mut LocalConstantInfo> {
        Some(&mut self.constants.get_mut(handle.index())?.1)
    }

    /// Add an import scope and return its ID.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::LimitExceeded`] if the table is full.
    pub fn add_import_scope(&mut self, scope: ImportScopeInfo) -> Result<ImportScopeId> {
        let id = next(self.import_scopes.len(), TableId::IMPORT_SCOPE)? + 1;
        self.import_scopes.push(scope);
        Ok(ImportScopeId(id))
    }

    /// Return an import scope.
    #[must_use]
    pub fn import_scope(&self, id: ImportScopeId) -> Option<&ImportScopeInfo> {
        self.import_scopes.get(index(id.0)?)
    }

    /// Return an import scope for a change.
    pub fn import_scope_mut(&mut self, id: ImportScopeId) -> Option<&mut ImportScopeInfo> {
        self.import_scopes.get_mut(index(id.0)?)
    }

    /// Add a state machine method: the `MoveNext` method and the kickoff method.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::LimitExceeded`] if the table is full.
    pub fn add_state_machine_method(
        &mut self,
        move_next: MethodDefId,
        kickoff: MethodDefId,
    ) -> Result<()> {
        next(self.state_machines.len(), TableId::STATE_MACHINE_METHOD)?;
        self.state_machines.push(StateMachineMethodRow {
            move_next_method: move_next,
            kickoff_method: kickoff,
        });
        Ok(())
    }

    /// Add a custom debug information record and return its handle.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::LimitExceeded`] if the table is full.
    pub fn add_custom_debug_information(
        &mut self,
        information: CustomDebugInfo,
    ) -> Result<CustomDebugHandle> {
        let table = TableId::CUSTOM_DEBUG_INFORMATION;
        let handle = CustomDebugHandle(next(self.custom.len(), table)?);
        self.custom.push(Some(information));
        Ok(handle)
    }

    /// Return a custom debug information record.
    #[must_use]
    pub fn custom_debug_information(&self, handle: CustomDebugHandle) -> Option<&CustomDebugInfo> {
        self.custom.get(handle.index())?.as_ref()
    }

    /// Return a custom debug information record for a change.
    pub fn custom_debug_information_mut(
        &mut self,
        handle: CustomDebugHandle,
    ) -> Option<&mut CustomDebugInfo> {
        self.custom.get_mut(handle.index())?.as_mut()
    }

    /// Remove a custom debug information record and return it.
    pub fn remove_custom_debug_information(
        &mut self,
        handle: CustomDebugHandle,
    ) -> Option<CustomDebugInfo> {
        self.custom.get_mut(handle.index())?.take()
    }

    /// Return each custom debug information record with its handle, in the order of addition.
    #[must_use]
    pub fn custom_debug_informations(&self) -> CustomDebugInformations<'_> {
        CustomDebugInformations((0..).zip(&self.custom))
    }

    /// Resolve the handles and make the raw parts with new heaps.
    ///
    /// # Errors
    ///
    /// The errors of [`PdbBuilder::plan`] that occur before [`PdbParts::plan`].
    pub fn to_parts(&self) -> Result<PdbParts> {
        Ok(resolve::resolve(self, None)?.parts)
    }

    /// Resolve the handles, check the records and freeze the layout.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] for a handle or an ID without a record, a sequence
    ///   point document that the builder does not have, or a custom debug information parent
    ///   that is not a row of a `HasCustomDebugInformation` table.
    /// - [`ErrorKind::InvalidFormat`] if a sequence point list, a name or an import does not
    ///   encode (see [`SequencePoints::encode_parts`](crate::portable::debug::SequencePoints::encode_parts)).
    /// - The errors of [`PdbParts::plan`].
    ///
    /// The errors have a portable location.
    pub fn plan(&self, id: IdentityPolicy) -> Result<PdbWritePlan> {
        self.to_parts()?.plan(id)
    }
}

/// An iterator over the methods of a [`PdbBuilder`] with sequence points, in `MethodDef` order.
#[derive(Clone, Debug)]
pub struct Methods<'a>(alloc::collections::btree_map::Iter<'a, u32, MethodInfo>);

impl<'a> Iterator for Methods<'a> {
    type Item = (MethodDefId, &'a MethodInfo);

    fn next(&mut self) -> Option<Self::Item> {
        self.0
            .next()
            .map(|(&method, info)| (MethodDefId(method), info))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl ExactSizeIterator for Methods<'_> {}

impl core::iter::FusedIterator for Methods<'_> {}

/// An iterator over the methods of a [`PdbBuilder`] for a change, in `MethodDef` order.
#[derive(Debug)]
pub struct MethodsMut<'a>(alloc::collections::btree_map::IterMut<'a, u32, MethodInfo>);

impl<'a> Iterator for MethodsMut<'a> {
    type Item = (MethodDefId, &'a mut MethodInfo);

    fn next(&mut self) -> Option<Self::Item> {
        self.0
            .next()
            .map(|(&method, info)| (MethodDefId(method), info))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl ExactSizeIterator for MethodsMut<'_> {}

impl core::iter::FusedIterator for MethodsMut<'_> {}

/// An iterator over the local scopes of a [`PdbBuilder`] with their handles.
#[derive(Clone, Debug)]
pub struct LocalScopes<'a>(
    core::iter::Zip<core::ops::RangeFrom<u32>, core::slice::Iter<'a, LocalScopeInfo>>,
);

impl<'a> Iterator for LocalScopes<'a> {
    type Item = (LocalScopeHandle, &'a LocalScopeInfo);

    fn next(&mut self) -> Option<Self::Item> {
        self.0
            .next()
            .map(|(index, scope)| (LocalScopeHandle(index), scope))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl ExactSizeIterator for LocalScopes<'_> {}

impl core::iter::FusedIterator for LocalScopes<'_> {}

/// An iterator over the custom debug information records of a [`PdbBuilder`] with their
/// handles.
#[derive(Clone, Debug)]
pub struct CustomDebugInformations<'a>(
    core::iter::Zip<core::ops::RangeFrom<u32>, core::slice::Iter<'a, Option<CustomDebugInfo>>>,
);

impl<'a> Iterator for CustomDebugInformations<'a> {
    type Item = (CustomDebugHandle, &'a CustomDebugInfo);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.by_ref().find_map(|(index, information)| {
            Some((CustomDebugHandle(index), information.as_ref()?))
        })
    }
}

impl core::iter::FusedIterator for CustomDebugInformations<'_> {}
