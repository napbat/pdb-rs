//! The scope tree step of `verify-native-edit`.
//!
//! For each native PDB, the step decodes each module through `PdbEditor::module_mut`. It
//! counts the modules with a scope tree, the modules without a symbol region and the opaque
//! modules. An opaque module is a problem. Then it plans without a change. The plan encodes
//! each decoded module stream again, so an empty `PlanReport::changed` list shows that each
//! module stream encodes again to its source bytes.

use std::fs;

use pdb_rs::ErrorKind;
use pdb_rs::native::codeview::index::ModuleIndex;
use pdb_rs::native::editor::ModuleSymbol;
use pdb_rs::native::{EditOptions, IdentityPolicy, Pdb};

use super::super::native::NativeFixture;
use crate::jobs::Jobs;

/// The module counts of one PDB or of all PDBs.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Counts {
    /// The number of modules.
    pub modules: usize,
    /// The number of modules whose symbols form a scope tree.
    pub trees: usize,
    /// The number of modules without a symbol region.
    pub absent: usize,
    /// The number of modules whose symbols stay opaque.
    pub opaque: usize,
    /// The number of scopes in all trees.
    pub scopes: usize,
}

impl Counts {
    /// Add the counts of `other`.
    const fn add(&mut self, other: Self) {
        self.modules += other.modules;
        self.trees += other.trees;
        self.absent += other.absent;
        self.opaque += other.opaque;
        self.scopes += other.scopes;
    }
}

/// The result of one PDB.
#[derive(Debug, Default)]
struct TreeResult {
    /// The module counts.
    counts: Counts,
    /// The streams that the plan without a change gives as changed.
    changed: Vec<u32>,
    /// The problems.
    problems: Vec<String>,
}

/// Return the number of scopes in `nodes`, at all depths.
fn scope_count(nodes: &[ModuleSymbol]) -> usize {
    nodes
        .iter()
        .map(|node| match node {
            ModuleSymbol::Record(_) => 0,
            ModuleSymbol::Scope(scope) => 1 + scope_count(&scope.children),
        })
        .sum()
}

/// Decode each module of the PDB `source` and plan without a change.
fn check(source: &[u8], result: &mut TreeResult) -> pdb_rs::Result<()> {
    let pdb = Pdb::parse(source)?;
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    let counts = &mut result.counts;
    for index in 0..editor.module_count()? {
        let module = ModuleIndex(u32::try_from(index).unwrap_or(u32::MAX));
        let contents = editor.module_mut(module)?;
        counts.modules += 1;
        match contents.symbols() {
            Ok(nodes) if !contents.has_stream() || contents.record().symbol_bytes == 0 => {
                if !nodes.is_empty() {
                    result
                        .problems
                        .push(format!("module {index} has symbols but no symbol region"));
                }
                counts.absent += 1;
            }
            Ok(nodes) => {
                counts.trees += 1;
                counts.scopes += scope_count(nodes);
            }
            Err(error) if error.kind() == ErrorKind::UnsupportedEdit => {
                counts.opaque += 1;
                result
                    .problems
                    .push(format!("module {index} stays opaque: {error}"));
            }
            Err(error) => return Err(error),
        }
    }
    let plan = editor.plan(EditOptions::default())?;
    result.changed = plan.report().changed().map(|id| id.0).collect();
    if !result.changed.is_empty() {
        result.problems.push(format!(
            "the plan without a change gives the changed streams {:?}",
            result.changed
        ));
    }
    Ok(())
}

/// Run the step on one PDB.
fn check_fixture(fixture: &NativeFixture) -> TreeResult {
    let mut result = TreeResult::default();
    match fs::read(&fixture.pdb) {
        Ok(source) => {
            if let Err(error) = check(&source, &mut result) {
                result
                    .problems
                    .push(format!("cannot decode or plan the modules: {error}"));
            }
        }
        Err(error) => result
            .problems
            .push(format!("cannot read {}: {error}", fixture.pdb.display())),
    }
    result
}

/// Print the counts of one PDB or of all PDBs after `prefix`.
fn print_counts(prefix: &str, counts: Counts) {
    println!(
        "{prefix}modules {}, scope trees {}, no symbol region {}, opaque {}, scopes {}",
        counts.modules, counts.trees, counts.absent, counts.opaque, counts.scopes
    );
}

/// Run the step on the PDBs of `list` and print one line for each PDB and a total line.
/// Return the totals and the number of problems.
pub(super) fn run(list: &[NativeFixture], jobs: &Jobs) -> (Counts, usize) {
    let results = jobs.map(list, check_fixture);
    let mut total = Counts::default();
    let mut problems = 0;
    for (fixture, result) in list.iter().zip(&results) {
        let status = if result.problems.is_empty() {
            "pass"
        } else {
            "FAIL"
        };
        print_counts(
            &format!("{} scope trees: {status}, ", fixture.label()),
            result.counts,
        );
        for problem in &result.problems {
            println!("  problem: {problem}");
        }
        total.add(result.counts);
        problems += result.problems.len();
    }
    print_counts(
        &format!(
            "scope trees: native PDBs {}, problems {problems}, ",
            list.len()
        ),
        total,
    );
    (total, problems)
}
