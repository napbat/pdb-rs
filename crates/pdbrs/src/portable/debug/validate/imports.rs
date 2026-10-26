//! Rules of the `ImportScope` table: parent chains and imports blobs.

use alloc::vec::Vec;

use super::methods::reference;
use super::{Rule, ValidationReport, external_exists};
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::debug::ImportScope;
use crate::portable::debug::rows::row_error;
use crate::portable::metadata::TableId;
use crate::{ErrorKind, Result};

/// The state of an import scope in the cycle search.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Visit {
    /// The chain of the scope is not checked yet.
    New,
    /// The scope is on the chain that the search follows now.
    OnChain,
    /// The chain of the scope ends at a root.
    Done,
}

/// Check the parent chains and the imports of each import scope.
pub(super) fn import_scopes<S: ReadAt>(pdb: &Pdb<S>, report: &mut ValidationReport) -> Result<()> {
    let mut parents = Vec::new();
    let mut imports = 0;
    for scope in pdb.import_scopes() {
        let scope = scope?;
        parents.push(scope.parent().get());
        imports += check_imports(pdb, &scope)?;
    }
    check_cycles(&parents)?;
    report.checked(
        Rule::ImportScopeParents,
        u64::try_from(parents.len()).unwrap_or(u64::MAX),
    );
    report.checked(Rule::Imports, imports);
    Ok(())
}

/// Return an error if a parent chain has a cycle. `parents[n]` is the parent of row `n + 1`.
///
/// The row reader already checked that each parent is at most the row count.
fn check_cycles(parents: &[u32]) -> Result<()> {
    let mut visits = alloc::vec![Visit::New; parents.len()];
    let mut chain = Vec::new();
    for start in 0..parents.len() {
        let mut current = start;
        while visits[current] == Visit::New {
            visits[current] = Visit::OnChain;
            chain.push(current);
            match parents[current] {
                0 => break,
                parent => current = parent as usize - 1,
            }
        }
        if visits[current] == Visit::OnChain && parents[current] != 0 {
            let row = u32::try_from(current + 1).unwrap_or(u32::MAX);
            return Err(row_error(
                ErrorKind::InvalidFormat,
                "import scope parent chain has a cycle",
                TableId::IMPORT_SCOPE,
                row,
                Some("Parent"),
            ));
        }
        for &row in &chain {
            visits[row] = Visit::Done;
        }
        chain.clear();
    }
    Ok(())
}

/// Decode the imports of a scope, check their references and return their number.
fn check_imports<S: ReadAt>(pdb: &Pdb<S>, scope: &ImportScope) -> Result<u64> {
    let assemblies = pdb.pdb_stream().external_row_count(TableId::ASSEMBLY_REF);
    let mut items = 0;
    for import in scope.imports() {
        let import = import.map_err(|error| scope.blob_error(error))?;
        if import
            .assembly()
            .is_some_and(|assembly| assembly.get() > assemblies)
        {
            return Err(scope.blob_error(reference("import AssemblyRef row does not exist")));
        }
        if import
            .target_type()
            .is_some_and(|token| !external_exists(pdb, token))
        {
            return Err(scope.blob_error(reference("import type row does not exist")));
        }
        for text in [import.alias(), import.namespace()].into_iter().flatten() {
            pdb.blob(text).map_err(|error| scope.blob_error(error))?;
        }
        items += 1;
    }
    Ok(items)
}
