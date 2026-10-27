//! Read the tables of a source Portable PDB into the collections of an editor.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::Methods;
use super::collections::{
    CustomDebugInformations, Documents, ImportScopes, LocalScopes, SourceRun,
};
use crate::PortableLocation;
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::debug::StateMachineMethod;
use crate::portable::metadata::{LocalScopeId, MetadataStream, MethodDefId, TableId};
use crate::{Error, ErrorKind, Result};

/// Read the documents in row order.
pub(super) fn documents<S: ReadAt>(pdb: &Pdb<S>) -> Result<Documents> {
    let records = pdb
        .documents()
        .map(|document| document.map(|(_, document)| document))
        .collect::<Result<Vec<_>>>()?;
    Ok(Documents::from_records(records))
}

/// Read the methods with sequence points.
pub(super) fn methods<S: ReadAt>(pdb: &Pdb<S>) -> Result<Methods> {
    let mut records = BTreeMap::new();
    for information in pdb.method_debug_informations() {
        let (id, information) = information?;
        if !information.sequence_points.is_empty() {
            records.insert(MethodDefId(id.0), information);
        }
    }
    Ok(Methods {
        records,
        table: pdb.tables().row_count(TableId::METHOD_DEBUG_INFORMATION) > 0,
    })
}

/// Read the local scopes with their variables and constants, and record the source runs.
///
/// # Errors
///
/// The errors of [`Pdb::local_scope`], and [`ErrorKind::InvalidFormat`] if a local variable
/// or constant is not in the run of a local scope.
pub(super) fn local_scopes<S: ReadAt>(pdb: &Pdb<S>) -> Result<LocalScopes> {
    let tables = pdb.tables();
    let mut scopes = LocalScopes::default();
    for scope in pdb.local_scopes() {
        let (id, scope) = scope?;
        let runs = pdb.scope_runs(id)?;
        if id == LocalScopeId(1) {
            // Keep the list value of an empty table: Roslyn writes 1, the F# compiler writes 0.
            let list = |table, value: u32| {
                if tables.row_count(table) == 0 {
                    value
                } else {
                    1
                }
            };
            scopes.empty_lists = [
                list(TableId::LOCAL_VARIABLE, runs.row.variable_list.0),
                list(TableId::LOCAL_CONSTANT, runs.row.constant_list.0),
            ];
        }
        let run = |range: core::ops::Range<u32>| SourceRun {
            start: range.start,
            count: range.end - range.start,
        };
        scopes
            .source
            .push([run(runs.variables), run(runs.constants)]);
        scopes.push(scope)?;
    }
    let count = |constants: bool| -> usize {
        let scopes = scopes.records.iter();
        if constants {
            scopes.map(|scope| scope.constants.len()).sum()
        } else {
            scopes.map(|scope| scope.variables.len()).sum()
        }
    };
    let complete = [
        (TableId::LOCAL_VARIABLE, count(false)),
        (TableId::LOCAL_CONSTANT, count(true)),
    ];
    for (table, count) in complete {
        if u32::try_from(count).ok() != Some(tables.row_count(table)) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("a row is not in the run of a local scope")
                .with_portable(
                    PortableLocation::new()
                        .with_stream(MetadataStream::Tables)
                        .with_table(table),
                ));
        }
    }
    Ok(scopes)
}

/// Read the import scopes in row order.
pub(super) fn import_scopes<S: ReadAt>(pdb: &Pdb<S>) -> Result<ImportScopes> {
    let records = pdb
        .import_scopes()
        .map(|scope| scope.map(|(_, scope)| scope))
        .collect::<Result<Vec<_>>>()?;
    Ok(ImportScopes::from_records(records))
}

/// Read the state machine methods in row order.
pub(super) fn state_machine_methods<S: ReadAt>(pdb: &Pdb<S>) -> Result<Vec<StateMachineMethod>> {
    pdb.state_machine_methods()
        .map(|method| method.map(|(_, method)| method))
        .collect()
}

/// Read the custom debug information records in row order.
pub(super) fn custom_debug_information<S: ReadAt>(pdb: &Pdb<S>) -> Result<CustomDebugInformations> {
    let records = pdb
        .custom_debug_informations()
        .map(|information| information.map(|(_, information)| Some(information)))
        .collect::<Result<Vec<_>>>()?;
    Ok(CustomDebugInformations { records })
}
