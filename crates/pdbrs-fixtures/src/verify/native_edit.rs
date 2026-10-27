//! The `verify-native-edit` command.
//!
//! The command applies each edit of [`edits::Edit`] with `pdbrs::native::PdbEditor` to each
//! native PDB of the fixture manifest (see [`super::native::native_fixtures`]). It writes each
//! edited PDB to `target/verify-native-edit/<fixture>/<PDB file stem>/<edit>.pdb` and checks it
//! with the independent consumers (see [`checks`]). For the strip edit of the
//! `msvc-x64-stripped` fixture, it also compares the result with the stripped PDB that
//! `link /PDBSTRIPPED` wrote (see [`stripped`]).
//!
//! After the summary line of these edits, the command prints the section
//! `change and removal cases:`. The section has these parts:
//!
//! 1. The change and removal cases on each PDB (see [`cases`]), with the same checks and a
//!    read of the output with `pdbrs`.
//! 2. The scope tree step: each module decodes into a scope tree, and a plan without a change
//!    changes no stream (see [`scope_trees`]).
//! 3. The read count of a named stream change on `msvc-x64-large` (see [`read_count`]).
//! 4. A summary line of the section.
//!
//! Then the command prints the section `task cases:`: each editing task on each PDB,
//! with the same checks and a summary line (see [`tasks`]).
//!
//! The section `remap cases:` remaps the source paths of each PDB and checks the output
//! (see [`remap`]).
//!
//! The last section `type removal cases:` removes the unreferenced type records and UDT
//! source line records of each PDB, with the same checks, a comparison of the mapped
//! `llvm-pdbutil` dump and of the `DIA2Dump` output, and a summary line (see [`removal`]).
//!
//! The command prints each explained class of differences. It fails for each problem without
//! an explanation, for each rejected edit and for each missing tool.

mod cases;
mod checks;
mod edits;
mod read_count;
mod remap;
mod removal;
mod scope_trees;
mod stripped;
mod tasks;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::native::consumers::Consumers;
use super::native::{NativeFixture, native_fixtures};
use crate::jobs::Jobs;
use edits::{Edit, Outcome};

/// The fixture with the `link /PDBSTRIPPED` output.
const STRIPPED_FIXTURE: &str = "msvc-x64-stripped";
/// The file name of the full PDB of [`STRIPPED_FIXTURE`].
const FULL_PDB: &str = "demo.pdb";
/// The file name of the stripped PDB of [`STRIPPED_FIXTURE`].
const STRIPPED_PDB: &str = "demo.stripped.pdb";

/// The paths of the consumer tools.
#[derive(Debug)]
pub(crate) struct Tools {
    /// `pdbtool.exe`.
    pub pdbtool: PathBuf,
    /// `DIA2Dump.exe`.
    pub dia2dump: PathBuf,
    /// `llvm-pdbutil.exe`.
    pub pdbutil: PathBuf,
    /// `pdbstr.exe` of the Windows SDK debuggers.
    pub pdbstr: PathBuf,
}

/// Find `pdbstr.exe` below the `ProgramFiles(x86)` directory.
fn pdbstr() -> Result<PathBuf, String> {
    let path = std::env::var_os("ProgramFiles(x86)")
        .map_or_else(|| PathBuf::from(r"C:\Program Files (x86)"), PathBuf::from)
        .join(r"Windows Kits\10\Debuggers\x64\srcsrv\pdbstr.exe");
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("{} does not exist", path.display()))
    }
}

/// The result of one edit of one PDB.
#[derive(Debug)]
struct EditResult {
    /// The edit.
    edit: Edit,
    /// The streams that the plan changed, removed or added.
    changed: Vec<u32>,
    /// The notes of the edit.
    notes: Vec<String>,
    /// The explained classes.
    explained: Vec<&'static str>,
    /// The problems without an explanation.
    problems: Vec<String>,
}

/// Apply all edits to one PDB and check the results.
fn check_fixture(tools: &Tools, fixture: &NativeFixture, work: &Path) -> Vec<EditResult> {
    let failed = |edit, problem: String| EditResult {
        edit,
        changed: Vec::new(),
        notes: Vec::new(),
        explained: Vec::new(),
        problems: vec![problem],
    };
    let source = match fs::read(&fixture.pdb) {
        Ok(bytes) => bytes,
        Err(error) => {
            let problem = format!("cannot read {}: {error}", fixture.pdb.display());
            return vec![failed(Edit::Named, problem)];
        }
    };
    let baseline = match checks::baseline(tools, &fixture.pdb) {
        Ok(baseline) => baseline,
        Err(error) => return vec![failed(Edit::Named, error)],
    };
    let directory = fixture.work_directory(work);
    if let Err(error) = fs::create_dir_all(&directory) {
        let problem = format!("cannot make {}: {error}", directory.display());
        return vec![failed(Edit::Named, problem)];
    }
    let mut results = Vec::new();
    for edit in Edit::ALL {
        let path = directory.join(format!("{}.pdb", edit.name()));
        let _ = fs::remove_file(&path);
        let edited = match edits::apply(edit, &source) {
            Ok(Outcome::Edited(edited)) => edited,
            Ok(Outcome::Rejected(error)) => {
                results.push(failed(edit, format!("the plan rejects the edit: {error}")));
                continue;
            }
            Err(error) => {
                results.push(failed(edit, error));
                continue;
            }
        };
        if let Err(error) = fs::write(&path, &edited.bytes) {
            results.push(failed(
                edit,
                format!("cannot write {}: {error}", path.display()),
            ));
            continue;
        }
        match checks::check(tools, &fixture.pdb, baseline, &path, &edited) {
            Ok(findings) => results.push(EditResult {
                edit,
                changed: edited.changed,
                notes: edited.notes,
                explained: findings.explained,
                problems: findings.problems,
            }),
            Err(error) => results.push(failed(edit, error)),
        }
    }
    results
}

/// Find the tools, or return the reasons why they are missing.
fn tools(workspace: &Path, fixtures: &Path) -> Result<Tools, String> {
    let consumers = Consumers::locate(workspace, fixtures);
    let missing = |name: &str, reason: &str| format!("tool {name} is missing: {reason}");
    Ok(Tools {
        pdbtool: consumers
            .pdbtool
            .map_err(|reason| missing("pdbtool", &reason))?,
        dia2dump: consumers
            .dia2dump
            .map_err(|reason| missing("DIA2Dump", &reason))?,
        pdbutil: consumers
            .pdbutil
            .map_err(|reason| missing("llvm-pdbutil", &reason))?,
        pdbstr: pdbstr().map_err(|reason| missing("pdbstr", &reason))?,
    })
}

/// Print the results of one PDB. Return the number of problems.
fn print_results(fixture: &NativeFixture, results: &[EditResult]) -> usize {
    let mut problems = 0;
    for result in results {
        let status = if result.problems.is_empty() {
            "pass"
        } else {
            "FAIL"
        };
        println!(
            "{} {}: {status}, changed streams {:?}",
            fixture.label(),
            result.edit.name(),
            result.changed
        );
        for note in &result.notes {
            println!("  note: {note}");
        }
        for problem in &result.problems {
            println!("  problem: {problem}");
        }
        problems += result.problems.len();
    }
    problems
}

/// Compare the strip of `msvc-x64-stripped/demo.pdb` with `demo.stripped.pdb`. Return the
/// number of unexplained differences.
fn compare_stripped(tools: &Tools, list: &[NativeFixture], work: &Path) -> usize {
    let find = |name: &str| {
        list.iter()
            .find(|fixture| fixture.fixture == STRIPPED_FIXTURE && fixture.file_name() == name)
    };
    let (Some(full), Some(linked)) = (find(FULL_PDB), find(STRIPPED_PDB)) else {
        println!("{STRIPPED_FIXTURE}: FAIL, the manifest has no {FULL_PDB} and {STRIPPED_PDB}");
        return 1;
    };
    let ours = full
        .work_directory(work)
        .join(format!("{}.pdb", Edit::Strip.name()));
    match stripped::compare(tools, &ours, &linked.pdb) {
        Ok(comparison) => {
            println!(
                "strip of {} against link /PDBSTRIPPED {}: same sections {:?}",
                full.label(),
                linked.label(),
                comparison.same
            );
            for explained in &comparison.explained {
                println!("  explained: {explained}");
            }
            for difference in &comparison.unexplained {
                println!("  problem: {difference}");
            }
            comparison.unexplained.len()
        }
        Err(error) => {
            println!("strip comparison: FAIL, {error}");
            1
        }
    }
}

/// Run the command on the fixtures in `fixtures`. Return `true` if all checks pass.
///
/// # Errors
///
/// Returns an error if the manifest cannot be read, a tool is missing or the work directory
/// cannot be made.
pub fn verify_native_edit(workspace: &Path, fixtures: &Path, jobs: &Jobs) -> Result<bool, String> {
    let work = workspace.join("target").join("verify-native-edit");
    fs::create_dir_all(&work)
        .map_err(|error| format!("cannot make {}: {error}", work.display()))?;
    let (list, skipped) = native_fixtures(fixtures)?;
    if list.is_empty() {
        return Err(format!(
            "{} has no native PDB outputs",
            fixtures.join("manifest.json").display()
        ));
    }
    let tools = tools(workspace, fixtures)?;
    for entry in &skipped {
        println!("{}: skipped, {}", entry.fixture, entry.reason);
    }
    let results = jobs.map(&list, |fixture| check_fixture(&tools, fixture, &work));
    let mut problems = 0;
    let mut classes = BTreeMap::<(&str, &str), usize>::new();
    let mut edits = 0;
    for (fixture, results) in list.iter().zip(&results) {
        problems += print_results(fixture, results);
        for result in results {
            edits += 1;
            for class in &result.explained {
                *classes.entry((result.edit.name(), class)).or_default() += 1;
            }
        }
    }
    problems += compare_stripped(&tools, &list, &work);
    for ((edit, class), count) in &classes {
        println!("explained ({edit}): {class}: {count} PDBs");
    }
    println!(
        "native PDBs: {}, edits: {edits}, problems without an explanation: {problems}",
        list.len()
    );
    let change_removal_problems = change_removal_section(&tools, &list, &work, jobs);
    let task_problems = tasks::run(&tools, &list, &work, jobs);
    let remap_problems = remap::run(&tools, &list, fixtures, &work, jobs);
    let removal_problems = removal::run_section(&tools, &list, &work, jobs);
    Ok(problems == 0
        && change_removal_problems == 0
        && task_problems == 0
        && remap_problems == 0
        && removal_problems == 0)
}

/// Run the section `change and removal cases:`: the change and removal cases, the scope tree
/// step and the read count. Print a summary line and return the number of problems.
fn change_removal_section(
    tools: &Tools,
    list: &[NativeFixture],
    work: &Path,
    jobs: &Jobs,
) -> usize {
    let totals = cases::run(tools, list, work, jobs);
    let (trees, tree_problems) = scope_trees::run(list, jobs);
    let read_problems = read_count::run(list);
    let problems = totals.problems + tree_problems + read_problems;
    println!(
        "change and removal section: native PDBs: {}, cases: {}, applied: {}, notes: {}, modules: {}, scope trees: {}, problems without an explanation: {problems}",
        list.len(),
        totals.cases,
        totals.applied,
        totals.notes,
        trees.modules,
        trees.trees
    );
    problems
}
