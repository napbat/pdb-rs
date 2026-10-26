//! The checks of the parts before layout.
//!
//! The checks find every reference that does not identify data and every order rule of the
//! Portable PDB specification that the raw rows break. They do not decode blobs.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use super::parts::PdbParts;
use crate::PortableLocation;
use crate::portable::metadata::{
    BlobOffset, CodedIndex, GuidIndex, MAX_ROW, MetadataStream, MetadataToken, StringOffset,
    TableId,
};
use crate::{Error, ErrorKind, Result};

/// The largest metadata version string, without its terminator.
const MAX_VERSION_LEN: usize = 254;

/// The row counts that the checks use, indexed by table number: local counts of the debug
/// tables and external counts of the type-system tables.
pub(super) struct Counts([u32; 64]);

impl Counts {
    /// Return the row count of `table`.
    pub(super) const fn get(&self, table: TableId) -> u32 {
        self.0[table.index()]
    }

    /// Return the counts, indexed by table number.
    pub(super) const fn as_array(&self) -> &[u32; 64] {
        &self.0
    }
}

/// Make an error at a cell of a debug table.
fn cell(
    kind: ErrorKind,
    table: TableId,
    row: usize,
    column: &'static str,
    context: &'static str,
) -> Error {
    let row = u32::try_from(row + 1).unwrap_or(u32::MAX);
    Error::new(kind, None).with_context(context).with_portable(
        PortableLocation::new()
            .with_stream(MetadataStream::Tables)
            .with_table(table)
            .with_row(row)
            .with_column(column),
    )
}

/// Make an error in the `#Pdb` stream.
fn pdb_error(kind: ErrorKind, context: &'static str) -> Error {
    Error::new(kind, None)
        .with_context(context)
        .with_portable(MetadataStream::Pdb.location())
}

/// Check the header fields and the row counts. Return the row counts.
pub(super) fn counts(parts: &PdbParts) -> Result<Counts> {
    if parts.version.len() > MAX_VERSION_LEN || parts.version.contains(&0) {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context(
                "metadata version string is longer than 254 bytes or contains a zero byte",
            )
            .with_portable(MetadataStream::Root.location()));
    }
    let mut counts = *parts.external.as_array();
    for table in (0_u8..64).filter_map(TableId::new) {
        let count = counts[table.index()];
        if count > 0 && !table.is_type_system() {
            return Err(pdb_error(
                ErrorKind::InvalidFormat,
                "an external row count is not for a type-system table",
            )
            .with_portable(PortableLocation::new().with_table(table)));
        }
        if count > MAX_ROW {
            return Err(pdb_error(
                ErrorKind::LimitExceeded,
                "external row count does not fit in a metadata token",
            )
            .with_portable(PortableLocation::new().with_table(table)));
        }
    }
    for table in TableId::DEBUG_TABLES {
        let count = u32::try_from(parts.tables.len(table))
            .ok()
            .filter(|&count| count <= MAX_ROW)
            .ok_or_else(|| {
                Error::new(ErrorKind::LimitExceeded, None)
                    .with_context("row count does not fit in a metadata token")
                    .with_portable(
                        PortableLocation::new()
                            .with_stream(MetadataStream::Tables)
                            .with_table(table),
                    )
            })?;
        counts[table.index()] = count;
    }
    let counts = Counts(counts);
    if let Some(method) = parts.entry_point
        && (method.is_nil() || method.get() > counts.get(TableId::METHOD_DEF))
    {
        return Err(pdb_error(
            ErrorKind::InvalidReference,
            "entry point is not an external MethodDef row",
        ));
    }
    let methods = counts.get(TableId::METHOD_DEF);
    let debug_rows = counts.get(TableId::METHOD_DEBUG_INFORMATION);
    if debug_rows != 0 && debug_rows != methods {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("MethodDebugInformation needs no rows or one row for each MethodDef row")
            .with_portable(
                PortableLocation::new()
                    .with_stream(MetadataStream::Tables)
                    .with_table(TableId::METHOD_DEBUG_INFORMATION),
            ));
    }
    Ok(counts)
}

/// The reference checks of one table.
struct Rows<'a> {
    parts: &'a PdbParts,
    counts: &'a Counts,
    table: TableId,
}

impl Rows<'_> {
    /// Check a `#Strings` offset.
    fn string(&self, row: usize, column: &'static str, offset: StringOffset) -> Result<()> {
        if self.parts.strings.contains(offset) {
            return Ok(());
        }
        Err(self.reference(row, column, "string offset is after the #Strings heap"))
    }

    /// Check a `#Blob` offset.
    fn blob(&self, row: usize, column: &'static str, offset: BlobOffset) -> Result<()> {
        if self.parts.blobs.contains(offset) {
            return Ok(());
        }
        Err(self.reference(row, column, "blob offset is not the start of a #Blob entry"))
    }

    /// Check a `#GUID` index.
    fn guid(&self, row: usize, column: &'static str, index: GuidIndex) -> Result<()> {
        if index.get() <= self.parts.guids.count() {
            return Ok(());
        }
        Err(self.reference(row, column, "GUID index is after the #GUID heap"))
    }

    /// Check a reference to a row of `target`. A nil reference is valid if `nil` is `true`.
    fn row(
        &self,
        row: usize,
        column: &'static str,
        target: TableId,
        value: u32,
        nil: bool,
    ) -> Result<()> {
        if (nil || value != 0) && value <= self.counts.get(target) {
            return Ok(());
        }
        Err(self.reference(
            row,
            column,
            "row reference is nil or after the end of its table",
        ))
    }

    /// Check a list start: 1 to the row count of `target` plus 1, or nil (0) if `target` has
    /// no rows.
    ///
    /// `System.Reflection.Metadata` writes nil if the producer gives no list, for example
    /// the F# compiler for `ConstantList`. Readers reject nil if the table has rows.
    fn list(&self, row: usize, column: &'static str, target: TableId, value: u32) -> Result<()> {
        let count = self.counts.get(target);
        if value == 0 && count != 0 {
            return Err(self.reference(row, column, "list is nil, but its table has rows"));
        }
        if u64::from(value) <= u64::from(count) + 1 {
            return Ok(());
        }
        Err(self.reference(row, column, "list start is after the end of its table"))
    }

    /// Make an [`ErrorKind::InvalidReference`] error at a cell.
    fn reference(&self, row: usize, column: &'static str, context: &'static str) -> Error {
        cell(
            ErrorKind::InvalidReference,
            self.table,
            row,
            column,
            context,
        )
    }

    /// Make an [`ErrorKind::InvalidFormat`] error for a row in the wrong order.
    fn order(&self, row: usize, column: &'static str, context: &'static str) -> Error {
        cell(ErrorKind::InvalidFormat, self.table, row, column, context)
    }
}

/// Check every row of the parts.
pub(super) fn rows(parts: &PdbParts, counts: &Counts) -> Result<()> {
    let table = |table| Rows {
        parts,
        counts,
        table,
    };
    documents(&table(TableId::DOCUMENT))?;
    let check = table(TableId::METHOD_DEBUG_INFORMATION);
    for (row, info) in parts.tables.method_debug_information.iter().enumerate() {
        check.row(
            row,
            "Document",
            TableId::DOCUMENT,
            info.document.get(),
            true,
        )?;
        check.blob(row, "SequencePoints", info.sequence_points)?;
    }
    local_scopes(&table(TableId::LOCAL_SCOPE))?;
    let check = table(TableId::LOCAL_VARIABLE);
    for (row, variable) in parts.tables.local_variables.iter().enumerate() {
        check.string(row, "Name", variable.name)?;
    }
    let check = table(TableId::LOCAL_CONSTANT);
    for (row, constant) in parts.tables.local_constants.iter().enumerate() {
        check.string(row, "Name", constant.name)?;
        check.blob(row, "Signature", constant.signature)?;
    }
    import_scopes(&table(TableId::IMPORT_SCOPE))?;
    state_machine_methods(&table(TableId::STATE_MACHINE_METHOD))?;
    custom_debug_information(&table(TableId::CUSTOM_DEBUG_INFORMATION))
}

/// Check the `Document` rows. The name must not be nil.
fn documents(check: &Rows<'_>) -> Result<()> {
    for (row, document) in check.parts.tables.documents.iter().enumerate() {
        if document.name.get() == 0 {
            return Err(check.reference(row, "Name", "document name is nil"));
        }
        check.blob(row, "Name", document.name)?;
        check.guid(row, "HashAlgorithm", document.hash_algorithm)?;
        check.blob(row, "Hash", document.hash)?;
        check.guid(row, "Language", document.language)?;
    }
    Ok(())
}

/// Check the `LocalScope` rows, their order and their variable and constant runs.
fn local_scopes(check: &Rows<'_>) -> Result<()> {
    let scopes = &check.parts.tables.local_scopes;
    for (row, scope) in scopes.iter().enumerate() {
        check.row(
            row,
            "Method",
            TableId::METHOD_DEF,
            scope.method.get(),
            false,
        )?;
        let import = scope.import_scope.get();
        check.row(row, "ImportScope", TableId::IMPORT_SCOPE, import, true)?;
        let variables = scope.variable_list.get();
        check.list(row, "VariableList", TableId::LOCAL_VARIABLE, variables)?;
        let constants = scope.constant_list.get();
        check.list(row, "ConstantList", TableId::LOCAL_CONSTANT, constants)?;
        let Some(previous) = row.checked_sub(1).map(|index| scopes[index]) else {
            continue;
        };
        let key = |scope: &crate::portable::metadata::LocalScopeRow| {
            (
                scope.method,
                scope.start_offset,
                core::cmp::Reverse(scope.length),
            )
        };
        if key(scope) < key(&previous) {
            return Err(check.order(
                row,
                "Method",
                "LocalScope is not sorted by method, start offset and length (longest first)",
            ));
        }
        if variables < previous.variable_list.get() {
            return Err(check.order(row, "VariableList", "VariableList decreases"));
        }
        if constants < previous.constant_list.get() {
            return Err(check.order(row, "ConstantList", "ConstantList decreases"));
        }
    }
    Ok(())
}

/// Check the `ImportScope` rows and that no parent chain has a cycle.
fn import_scopes(check: &Rows<'_>) -> Result<()> {
    let scopes = &check.parts.tables.import_scopes;
    for (row, scope) in scopes.iter().enumerate() {
        check.row(
            row,
            "Parent",
            TableId::IMPORT_SCOPE,
            scope.parent.get(),
            true,
        )?;
        check.blob(row, "Imports", scope.imports)?;
    }
    // 0: not visited, 1: on the current chain, 2: the chain ends at nil.
    let mut state: Vec<u8> = Vec::new();
    state.try_reserve_exact(scopes.len()).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("import scope check allocation failed")
    })?;
    state.resize(scopes.len(), 0);
    let mut chain = Vec::new();
    for start in 0..scopes.len() {
        let mut current = start;
        while state[current] == 0 {
            state[current] = 1;
            chain.push(current);
            // The parent is checked above, so it is nil or a row index plus 1.
            match usize::try_from(scopes[current].parent.get())
                .ok()
                .and_then(|parent| parent.checked_sub(1))
            {
                Some(parent) => current = parent,
                None => break,
            }
        }
        if state[current] == 1 && scopes[current].parent.get() != 0 {
            return Err(check.reference(current, "Parent", "import scope parents form a cycle"));
        }
        for &row in &chain {
            state[row] = 2;
        }
        chain.clear();
    }
    Ok(())
}

/// Check the `StateMachineMethod` rows: sorted by a unique `MoveNext` method and unique
/// kickoff methods.
fn state_machine_methods(check: &Rows<'_>) -> Result<()> {
    let methods = &check.parts.tables.state_machine_methods;
    let mut kickoffs = BTreeSet::new();
    for (row, method) in methods.iter().enumerate() {
        let move_next = method.move_next_method.get();
        check.row(row, "MoveNextMethod", TableId::METHOD_DEF, move_next, false)?;
        let kickoff = method.kickoff_method.get();
        check.row(row, "KickoffMethod", TableId::METHOD_DEF, kickoff, false)?;
        if row > 0 && methods[row - 1].move_next_method.get() >= move_next {
            return Err(check.order(
                row,
                "MoveNextMethod",
                "StateMachineMethod is not sorted by a unique MoveNextMethod",
            ));
        }
        if !kickoffs.insert(kickoff) {
            return Err(check.order(row, "KickoffMethod", "KickoffMethod is not unique"));
        }
    }
    Ok(())
}

/// Check the `CustomDebugInformation` rows and their order by the coded parent.
fn custom_debug_information(check: &Rows<'_>) -> Result<()> {
    let rows = &check.parts.tables.custom_debug_information;
    let mut previous = 0;
    for (row, information) in rows.iter().enumerate() {
        let parent = coded_parent(check, row, information.parent)?;
        if parent < previous {
            return Err(check.order(
                row,
                "Parent",
                "CustomDebugInformation is not sorted by Parent",
            ));
        }
        previous = parent;
        check.guid(row, "Kind", information.kind)?;
        check.blob(row, "Value", information.value)?;
    }
    Ok(())
}

/// Check a `HasCustomDebugInformation` parent and return its coded value.
fn coded_parent(check: &Rows<'_>, row: usize, parent: MetadataToken) -> Result<u32> {
    let table = TableId::new(parent.table());
    let valid =
        table.is_some_and(|table| parent.row() != 0 && parent.row() <= check.counts.get(table));
    let coded = CodedIndex::HasCustomDebugInformation.encode(parent);
    match coded {
        Some(coded) if valid => Ok(coded),
        _ => Err(check.reference(
            row,
            "Parent",
            "parent is not a row of a HasCustomDebugInformation table",
        )),
    }
}
