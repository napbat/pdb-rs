//! The Portable PDB editor: one model for builds and edits, and its plan.

use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::collections::{CustomDebugInformations, Documents, ImportScopes, LocalScopes};
use super::resolve::{Base, Model, Resolved, resolve};
use super::{Methods, decode};
use crate::PortableLocation;
use crate::io::ReadAt;
use crate::portable::custom::CustomDebugValue;
use crate::portable::debug::{MethodDebugInformation, StateMachineMethod};
use crate::portable::metadata::{MetadataStream, MethodDefId, TableId};
use crate::portable::write::{ExternalRowCounts, PDB_VERSION};
use crate::portable::{IdentityPolicy, Pdb, PdbWritePlan};
use crate::{Error, ErrorKind, Result};

/// The options of [`PdbEditor::plan`].
///
/// The type has no options yet. A later option is a `with_*` function.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct EditOptions {}

/// The editor of a Portable PDB: one model of records for a new PDB and for an edit.
///
/// [`PdbEditor::new`] starts without a source. [`Pdb::edit`] starts from a source: the
/// editor borrows the source and decodes a table on the first `*_mut` call of that table.
/// Both take the identity policy of the output (#42 rule 6). The plan computes a
/// content-derived ID from the output bytes.
///
/// The records are the records of the reader with `'static` (see
/// [`debug`](crate::portable::debug)). Documents and import scopes keep the order of
/// addition, so their IDs are the row IDs of the output. Local scopes and custom debug
/// information get their rows in the plan, so they have handles.
///
/// # Plan
///
/// [`PdbEditor::plan`] resolves the handles and makes a [`PdbWritePlan`]. It reads a table
/// that no `*_mut` call decoded from the source, so the output does not depend on the calls.
///
/// 1. It starts the heaps with the source heaps, or with empty heaps without a source. It
///    splits each document name into parts as Roslyn does and encodes the names and the
///    imports. The other blobs keep the bytes of the record.
/// 2. It sorts the local scopes by method, start offset and length (longest first), and gives
///    the variables and the constants of each scope one run, in their order.
/// 3. It sorts the state machine methods by `MoveNext` method and the custom debug information
///    by the coded parent. Records with the same key keep the order of addition.
/// 4. It gives each handle its row ID and makes the raw rows.
///    [`PdbParts::plan`](crate::portable::write::PdbParts::plan) then checks all rows and
///    computes the index widths from the final heap sizes and row counts.
/// 5. It compares each output stream with the source stream for the report.
///
/// # Guarantees of an edit
///
/// - The heaps of the output start with the bytes of the source heaps. Each source heap entry
///   keeps its offset, and a value that the source has gets its source offset.
/// - Documents and import scopes keep their row IDs. New documents and import scopes come
///   after them.
/// - Without a change, each record keeps its row. For a Portable PDB that
///   `System.Reflection.Metadata` wrote, the output then has the same bytes as the source.
/// - Custom debug information of an unknown kind keeps its bytes. If a change gives a source
///   local scope, local variable or local constant a new row ID, such a value can contain the
///   old row ID. [`PdbEditor::plan`] then fails before output. Remove the unknown values or
///   add the new records after the source records of the same method to keep the rows.
///
/// The editor does not keep the source unchanged in other ways: for example, a document name
/// or an imports blob that the editor encodes again can have other bytes if the source
/// producer used another encoding. The plan then adds the new blob, and the source blob stays
/// in the heap.
pub struct PdbEditor<'p, S = &'static [u8]> {
    source: Option<&'p Pdb<S>>,
    policy: IdentityPolicy,
    version: Vec<u8>,
    entry_point: Option<MethodDefId>,
    external: ExternalRowCounts,
    documents: Option<Documents>,
    methods: Option<Methods>,
    local_scopes: Option<LocalScopes>,
    import_scopes: Option<ImportScopes>,
    state_machine_methods: Option<Vec<StateMachineMethod>>,
    custom: Option<CustomDebugInformations>,
}

impl<S> Clone for PdbEditor<'_, S> {
    fn clone(&self) -> Self {
        Self {
            source: self.source,
            policy: self.policy,
            version: self.version.clone(),
            entry_point: self.entry_point,
            external: self.external,
            documents: self.documents.clone(),
            methods: self.methods.clone(),
            local_scopes: self.local_scopes.clone(),
            import_scopes: self.import_scopes.clone(),
            state_machine_methods: self.state_machine_methods.clone(),
            custom: self.custom.clone(),
        }
    }
}

impl<S> core::fmt::Debug for PdbEditor<'_, S> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PdbEditor")
            .field("source", &self.source.is_some())
            .field("policy", &self.policy)
            .field("entry_point", &self.entry_point)
            .field("external", &self.external)
            .finish_non_exhaustive()
    }
}

impl PdbEditor<'static> {
    /// Make an editor without a source and without records, for an assembly with the
    /// external row counts `external`. The plan sets the PDB ID with `policy`.
    #[must_use]
    pub fn new(external: ExternalRowCounts, policy: IdentityPolicy) -> Self {
        Self {
            source: None,
            policy,
            version: PDB_VERSION.to_vec(),
            entry_point: None,
            external,
            documents: Some(Documents::default()),
            methods: Some(Methods::default()),
            local_scopes: Some(LocalScopes::default()),
            import_scopes: Some(ImportScopes::default()),
            state_machine_methods: Some(Vec::new()),
            custom: Some(CustomDebugInformations::default()),
        }
    }
}

impl<S: ReadAt> Pdb<S> {
    /// Make an editor of this Portable PDB. The plan sets the PDB ID with `policy`.
    ///
    /// The editor borrows the PDB and reads nothing now. See [`PdbEditor`] for the guarantees
    /// of an edit.
    #[must_use]
    pub fn edit(&self, policy: IdentityPolicy) -> PdbEditor<'_, S> {
        PdbEditor {
            source: Some(self),
            policy,
            version: self.metadata().version_string().to_vec(),
            entry_point: self.pdb_stream().entry_point(),
            external: ExternalRowCounts::from_pdb_stream(self.pdb_stream()),
            documents: None,
            methods: None,
            local_scopes: None,
            import_scopes: None,
            state_machine_methods: None,
            custom: None,
        }
    }
}

/// Return the table of `slot`, and decode it from `source` first if `slot` is empty. On an
/// error, `slot` does not change.
fn decoded<'s, S: ReadAt, T: Default>(
    slot: &'s mut Option<T>,
    source: Option<&Pdb<S>>,
    decode: fn(&Pdb<S>) -> Result<T>,
) -> Result<&'s mut T> {
    if slot.is_none() {
        *slot = Some(source.map_or_else(|| Ok(T::default()), decode)?);
    }
    Ok(slot.get_or_insert_with(T::default))
}

/// Return the table of `slot`, or the table that `decode` reads from `source` if `slot` is
/// empty.
fn view<'s, S: ReadAt, T: Clone + Default>(
    slot: Option<&'s T>,
    source: Option<&Pdb<S>>,
    decode: fn(&Pdb<S>) -> Result<T>,
) -> Result<Cow<'s, T>> {
    Ok(match (slot, source) {
        (Some(table), _) => Cow::Borrowed(table),
        (None, Some(pdb)) => Cow::Owned(decode(pdb)?),
        (None, None) => Cow::Owned(T::default()),
    })
}

impl<S: ReadAt> PdbEditor<'_, S> {
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

    /// Return the documents for a change. Decode them from the source on the first call.
    ///
    /// # Errors
    ///
    /// The errors of [`Pdb::documents`]. On an error, the editor does not change.
    pub fn documents_mut(&mut self) -> Result<&mut Documents> {
        decoded(&mut self.documents, self.source, decode::documents)
    }

    /// Return the methods with sequence points for a change, by `MethodDef` row. Decode them
    /// from the source on the first call.
    ///
    /// The plan writes a `MethodDebugInformation` row for each external `MethodDef` row if a
    /// method has sequence points or the source has the table.
    ///
    /// # Errors
    ///
    /// The errors of [`Pdb::method_debug_informations`]. On an error, the editor does not
    /// change.
    pub fn methods_mut(
        &mut self,
    ) -> Result<&mut BTreeMap<MethodDefId, MethodDebugInformation<'static>>> {
        decoded(&mut self.methods, self.source, decode::methods).map(|methods| &mut methods.records)
    }

    /// Return the local scopes with their variables and constants for a change. Decode them
    /// from the source on the first call.
    ///
    /// # Errors
    ///
    /// - The errors of [`Pdb::local_scopes`].
    /// - [`ErrorKind::InvalidFormat`] if a local variable or constant is not in the run of a
    ///   local scope.
    ///
    /// On an error, the editor does not change.
    pub fn local_scopes_mut(&mut self) -> Result<&mut LocalScopes> {
        decoded(&mut self.local_scopes, self.source, decode::local_scopes)
    }

    /// Return the import scopes for a change. Decode them from the source on the first call.
    ///
    /// # Errors
    ///
    /// The errors of [`Pdb::import_scopes`]. On an error, the editor does not change.
    pub fn import_scopes_mut(&mut self) -> Result<&mut ImportScopes> {
        decoded(&mut self.import_scopes, self.source, decode::import_scopes)
    }

    /// Return the state machine methods for a change. Decode them from the source on the
    /// first call. The plan sorts them by `MoveNext` method.
    ///
    /// # Errors
    ///
    /// The errors of [`Pdb::state_machine_methods`]. On an error, the editor does not change.
    pub fn state_machine_methods_mut(&mut self) -> Result<&mut Vec<StateMachineMethod>> {
        decoded(
            &mut self.state_machine_methods,
            self.source,
            decode::state_machine_methods,
        )
    }

    /// Return the custom debug information for a change. Decode it from the source on the
    /// first call.
    ///
    /// # Errors
    ///
    /// The errors of [`Pdb::custom_debug_informations`]. On an error, the editor does not
    /// change.
    pub fn custom_debug_information_mut(&mut self) -> Result<&mut CustomDebugInformations> {
        decoded(
            &mut self.custom,
            self.source,
            decode::custom_debug_information,
        )
    }

    /// Decode every table that no `*_mut` call decoded.
    ///
    /// # Errors
    ///
    /// The errors of the `*_mut` functions. On an error, the tables that decoded before the
    /// error stay decoded.
    pub fn decode_all(&mut self) -> Result<()> {
        self.documents_mut()?;
        self.methods_mut()?;
        self.local_scopes_mut()?;
        self.import_scopes_mut()?;
        self.state_machine_methods_mut()?;
        self.custom_debug_information_mut()?;
        Ok(())
    }

    /// Resolve the handles, check the records, freeze the layout and compare the output with
    /// the source.
    ///
    /// See [`PdbEditor`] for the steps. The plan does not change the editor.
    ///
    /// # Errors
    ///
    /// - The errors of the source reads, if a table is not decoded yet.
    /// - [`ErrorKind::InvalidReference`] for a handle or an ID without a record, a sequence
    ///   point document that the editor does not have, or a custom debug information parent
    ///   that is not a row of a `HasCustomDebugInformation` table.
    /// - [`ErrorKind::InvalidReference`] if the edit gives a source local scope, local variable
    ///   or local constant a new row ID and a custom debug information value of an unknown
    ///   kind (or a value that does not decode) exists.
    /// - [`ErrorKind::InvalidFormat`] if a sequence point blob does not decode, or a name or an
    ///   import does not encode.
    /// - The errors of [`PdbParts::plan`](crate::portable::write::PdbParts::plan).
    ///
    /// The errors have a portable location.
    pub fn plan(&self, options: EditOptions) -> Result<PdbWritePlan> {
        let EditOptions {} = options;
        let source = self.source;
        let documents = view(self.documents.as_ref(), source, decode::documents)?;
        let methods = view(self.methods.as_ref(), source, decode::methods)?;
        let scopes = view(self.local_scopes.as_ref(), source, decode::local_scopes)?;
        let import_scopes = view(self.import_scopes.as_ref(), source, decode::import_scopes)?;
        let state_machines = view(
            self.state_machine_methods.as_ref(),
            source,
            decode::state_machine_methods,
        )?;
        let custom = view(
            self.custom.as_ref(),
            source,
            decode::custom_debug_information,
        )?;
        let model = Model {
            version: &self.version,
            entry_point: self.entry_point,
            external: self.external,
            documents: &documents,
            methods: &methods,
            scopes: &scopes,
            import_scopes: &import_scopes,
            state_machines: &state_machines,
            custom: &custom,
        };
        let base = source.map(Base::read).transpose()?;
        let resolved = resolve(&model, base)?;
        if source.is_some() {
            check_opaque(&scopes, &custom, &resolved)?;
        }
        let mut plan = resolved.parts.plan(self.policy)?;
        if let Some(pdb) = source {
            plan.compare(pdb)?;
        }
        Ok(plan)
    }
}

/// Return `true` if a source local scope, local variable or local constant gets a new row ID.
fn moved(scopes: &LocalScopes, resolved: &Resolved) -> bool {
    (0..)
        .zip(&scopes.source)
        .zip(&scopes.records)
        .any(|((index, runs), scope): ((u32, _), _)| {
            let position = usize::try_from(index).unwrap_or(usize::MAX);
            if resolved.scope_rows.get(position) != Some(&(index + 1)) {
                return true;
            }
            let lengths = [scope.variables.len(), scope.constants.len()];
            runs.iter()
                .zip(lengths)
                .enumerate()
                .any(|(list, (run, len))| {
                    run.count > 0
                        && (u32::try_from(len).is_ok_and(|len| len < run.count)
                            || resolved.list_rows[position][list] != run.start)
                })
        })
        || scopes.records.len() < scopes.source.len()
}

/// Fail if a source row moves and a custom debug information value of an unknown kind
/// exists.
fn check_opaque(
    scopes: &LocalScopes,
    custom: &CustomDebugInformations,
    resolved: &Resolved,
) -> Result<()> {
    if !moved(scopes, resolved) {
        return Ok(());
    }
    for (handle, information) in custom.iter() {
        let opaque = !matches!(
            CustomDebugValue::parse(information.kind, &information.value),
            Ok(value) if !matches!(value, CustomDebugValue::Unknown { .. })
        );
        if opaque {
            let row = u32::try_from(handle.index() + 1).unwrap_or(u32::MAX);
            return Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context(
                    "the edit moves source local scopes, variables or constants, and an unknown custom debug information value can refer to their rows",
                )
                .with_portable(
                    PortableLocation::new()
                        .with_stream(MetadataStream::Tables)
                        .with_table(TableId::CUSTOM_DEBUG_INFORMATION)
                        .with_row(row)
                        .with_column("Value"),
                ));
        }
    }
    Ok(())
}
