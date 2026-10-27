//! The task cases of `verify-native-edit`.
//!
//! After the section `change and removal cases:`, the command runs each editing task of
//! `pdbrs::native::PdbEditor`
//! on each native PDB and writes `target/verify-native-edit/<fixture>/<PDB file stem>/
//! task-<case>.pdb`. Each output gets the consumer checks of [`super::checks`]: `pdbtool check`
//! and `compare` (the report lists exactly the streams that `compare` shows as different),
//! `llvm-pdbutil` and `DIA2Dump`, `pdbstr -r` for `srcsrv` and `sourcelink`, and the name
//! lookups of the renamed symbols. The rename case also reads the output with `pdbrs`: each
//! renamed procedure reference names a procedure with the same new name.
//!
//! A case that does not apply to a PDB gives a note and no output.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use pdbrs::native::auxiliary::NamedStreamKind;
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::symbol::{
    ModuleSymbolTarget, S_LPROCREF, S_PROCREF, Symbol, SymbolData,
};
use pdbrs::native::symbols::{HashTable, indexed_name};
use pdbrs::native::{EditOptions, IdentityPolicy, Pdb, PdbEditor};

use super::super::native::NativeFixture;
use super::Tools;
use super::checks;
use super::edits::{
    self, Expectation, Outcome, SOURCE_LINK, SOURCE_SERVER, first_public, first_rename_global,
    first_unique, indexes, lossy,
};
use crate::jobs::Jobs;

/// The suffix that the rename task appends to a name.
const TASK_SUFFIX: &str = "_pdbrs_task";

/// One task case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Task {
    /// `set_source_link`.
    SetSourceLink,
    /// `set_source_server`.
    SetSourceServer,
    /// `remove_named_stream` of the first named stream that the editor can remove.
    RemoveNamedStream,
    /// `rename_symbols` of a public, a global and a procedure name.
    RenameSymbols,
}

impl Task {
    /// All cases in the order of the command.
    const ALL: [Self; 4] = [
        Self::SetSourceLink,
        Self::SetSourceServer,
        Self::RemoveNamedStream,
        Self::RenameSymbols,
    ];

    /// The name of the case in the output. The output file is `task-<name>.pdb`.
    const fn name(self) -> &'static str {
        match self {
            Self::SetSourceLink => "set-source-link",
            Self::SetSourceServer => "set-source-server",
            Self::RemoveNamedStream => "remove-named-stream",
            Self::RenameSymbols => "rename-symbols",
        }
    }
}

/// The result of one case of one PDB.
#[derive(Debug)]
struct CaseResult {
    /// The case.
    task: Task,
    /// `true` if the case applies to the PDB and the command wrote an output.
    applied: bool,
    /// The streams that the plan changed, removed or added.
    changed: Vec<u32>,
    /// The notes of the case.
    notes: Vec<String>,
    /// The explained classes.
    explained: Vec<&'static str>,
    /// The problems without an explanation.
    problems: Vec<String>,
}

impl CaseResult {
    /// Return a result with one problem.
    fn failed(task: Task, problem: String) -> Self {
        Self {
            task,
            applied: false,
            changed: Vec::new(),
            notes: Vec::new(),
            explained: Vec::new(),
            problems: vec![problem],
        }
    }
}

/// The change of one case.
enum Applied {
    /// The case does not apply. The text tells why.
    NotApplied(String),
    /// The edited PDB, and the new names of the renamed procedure references.
    Edited(Box<edits::Edited>, Vec<String>),
}

/// Return the first named stream that the copy-on-write editor can remove, in the order of
/// the named-stream map.
fn removable(pdb: &Pdb<&[u8]>) -> Option<String> {
    pdb.info()
        .named_streams()
        .find(|named| {
            !matches!(
                NamedStreamKind::classify(&named.name),
                NamedStreamKind::Names
                    | NamedStreamKind::TmCache
                    | NamedStreamKind::UdtSourceLineUndone
                    | NamedStreamKind::SourceHeaderBlock
                    | NamedStreamKind::SourceFile { .. }
            )
        })
        .map(|named| lossy(&named.name))
}

/// Return the names of the records of `table`.
fn table_names<'r>(records: &'r [u8], table: HashTable<'_>) -> pdbrs::Result<BTreeSet<&'r [u8]>> {
    table
        .records()
        .map(|record| indexed_name(records, record.symbol))
        .collect()
}

/// Rename the first unique public name, the first unique global data, constant or UDT name
/// and the first unique procedure reference name. Return the new names of the procedure
/// references, or a note.
fn rename(
    pdb: &Pdb<&[u8]>,
    editor: &mut PdbEditor<'_, &[u8]>,
    expect: &mut Expectation,
) -> pdbrs::Result<Result<Vec<String>, String>> {
    let indexes = indexes(pdb)?;
    let procedure = match indexes.globals {
        Some(globals) => first_unique(indexes.records, globals.hash_table(), |kind| {
            kind == S_PROCREF || kind == S_LPROCREF
        })?,
        None => None,
    };
    let mut names = BTreeSet::new();
    for (name, _) in [
        first_public(&indexes)?,
        first_rename_global(&indexes)?,
        procedure.clone(),
    ]
    .into_iter()
    .flatten()
    {
        names.insert(name);
    }
    if names.is_empty() {
        return Ok(Err("no public, global or procedure name to rename".into()));
    }
    let suffix = TASK_SUFFIX.as_bytes();
    let renamed =
        editor.rename_symbols(|name| names.contains(name).then(|| [name, suffix].concat()))?;
    if renamed == 0 {
        return Ok(Err("rename_symbols renamed no record".into()));
    }
    let publics = match indexes.publics {
        Some(publics) => table_names(indexes.records, publics.hash_table())?,
        None => BTreeSet::new(),
    };
    let globals = match indexes.globals {
        Some(globals) => table_names(indexes.records, globals.hash_table())?,
        None => BTreeSet::new(),
    };
    let mut references = Vec::new();
    for name in names {
        let (old, new) = (lossy(name), lossy(&[name, suffix].concat()));
        if publics.contains(name) {
            expect.publics_absent.push(old.clone());
            expect.publics.push(new.clone());
        }
        if globals.contains(name) {
            expect.globals_renamed.push(old.clone());
            expect.globals.push(new.clone());
        }
        if procedure
            .as_ref()
            .is_some_and(|(procedure, _)| *procedure == name)
        {
            expect.procedures_renamed.push((old, new.clone()));
            references.push(new);
        }
    }
    Ok(Ok(references))
}

/// Apply `task` to the PDB `source`.
fn apply(task: Task, source: &[u8]) -> Result<Applied, String> {
    let pdb = Pdb::parse(source).map_err(|error| format!("cannot read the PDB: {error}"))?;
    let policy = if task == Task::RenameSymbols {
        IdentityPolicy::IncrementAge
    } else {
        IdentityPolicy::Preserve
    };
    let mut editor = pdb.edit(policy);
    let mut expect = Expectation::default();
    let prepared: pdbrs::Result<Result<Vec<String>, String>> = match task {
        Task::SetSourceLink => editor
            .set_source_link(SOURCE_LINK.as_bytes().to_vec())
            .map(|()| {
                expect.named_present.push("sourcelink".into());
                expect.source_link = Some(SOURCE_LINK.to_owned());
                Ok(Vec::new())
            }),
        Task::SetSourceServer => editor
            .set_source_server(SOURCE_SERVER.as_bytes().to_vec())
            .map(|()| {
                expect.named_present.push("srcsrv".into());
                expect.source_server = Some(SOURCE_SERVER.to_owned());
                Ok(Vec::new())
            }),
        Task::RemoveNamedStream => match removable(&pdb) {
            Some(name) => editor.remove_named_stream(name.as_bytes()).map(|()| {
                expect.named_absent.push(name);
                Ok(Vec::new())
            }),
            None => Ok(Err(
                "the PDB has no named stream that the editor can remove".into(),
            )),
        },
        Task::RenameSymbols => rename(&pdb, &mut editor, &mut expect),
    };
    let references = match prepared.map_err(|error| format!("the task fails: {error}"))? {
        Ok(references) => references,
        Err(note) => return Ok(Applied::NotApplied(note)),
    };
    match edits::plan(&editor, EditOptions::default(), expect, Vec::new())? {
        Outcome::Edited(edited) => Ok(Applied::Edited(edited, references)),
        Outcome::Rejected(error) => Err(format!("the plan rejects the edit: {error}")),
    }
}

/// Check with `pdbrs` that each procedure reference with a name of `names` names a procedure
/// with the same name, and that each name has a reference.
fn confirm_references(bytes: &[u8], names: &[String]) -> Result<(), String> {
    let pdb =
        Pdb::parse(bytes).map_err(|error| format!("pdbrs cannot read the output: {error}"))?;
    let read = |error: pdbrs::Error| format!("pdbrs cannot read the output: {error}");
    let records = pdb.symbol_records().map_err(read)?.unwrap_or_default();
    let mut found = BTreeSet::new();
    for record in Records::new(records) {
        let symbol = Symbol::decode(record.map_err(read)?).map_err(read)?;
        let SymbolData::ProcedureReference(reference) = symbol.data() else {
            continue;
        };
        let name = lossy(&reference.name);
        if !names.contains(&name) {
            continue;
        }
        let ModuleSymbolTarget::Position {
            module: Some(module),
            offset,
        } = reference.target
        else {
            return Err(format!("the reference {name} names no module"));
        };
        let stream = pdb
            .module(module)
            .map_err(read)?
            .ok_or_else(|| format!("the module of the reference {name} has no stream"))?;
        let base = u32::try_from(stream.symbol_offset()).map_err(|error| error.to_string())?;
        let target = Records::new(stream.symbol_data())
            .filter_map(Result::ok)
            .find(|record| record.offset + base == offset.0)
            .ok_or_else(|| format!("the reference {name} names no record"))?;
        let target = Symbol::decode(target).map_err(read)?;
        if target.name().map(|name| lossy(name)) != Some(name.clone()) {
            return Err(format!(
                "the reference {name} names a record with the name {:?}",
                target.name().map(|name| lossy(name))
            ));
        }
        found.insert(name);
    }
    match names.iter().find(|name| !found.contains(*name)) {
        Some(name) => Err(format!("pdbrs finds no procedure reference {name}")),
        None => Ok(()),
    }
}

/// Run one case on one PDB and check the output.
fn check_case(
    tools: &Tools,
    fixture: &NativeFixture,
    baseline: checks::Baseline,
    source: &[u8],
    directory: &Path,
    task: Task,
) -> CaseResult {
    let path = directory.join(format!("task-{}.pdb", task.name()));
    let _ = fs::remove_file(&path);
    let (edited, references) = match apply(task, source) {
        Ok(Applied::Edited(edited, references)) => (edited, references),
        Ok(Applied::NotApplied(note)) => {
            return CaseResult {
                task,
                applied: false,
                changed: Vec::new(),
                notes: vec![note],
                explained: Vec::new(),
                problems: Vec::new(),
            };
        }
        Err(error) => return CaseResult::failed(task, error),
    };
    if let Err(error) = fs::write(&path, &edited.bytes) {
        return CaseResult::failed(task, format!("cannot write {}: {error}", path.display()));
    }
    let mut findings = match checks::check(tools, &fixture.pdb, baseline, &path, &edited) {
        Ok(findings) => findings,
        Err(error) => return CaseResult::failed(task, error),
    };
    if let Err(problem) = confirm_references(&edited.bytes, &references) {
        findings.problems.push(problem);
    }
    CaseResult {
        task,
        applied: true,
        changed: edited.changed,
        notes: edited.notes,
        explained: findings.explained,
        problems: findings.problems,
    }
}

/// Run all cases on one PDB.
fn check_fixture(tools: &Tools, fixture: &NativeFixture, work: &Path) -> Vec<CaseResult> {
    let first = Task::ALL[0];
    let source = match fs::read(&fixture.pdb) {
        Ok(bytes) => bytes,
        Err(error) => {
            let problem = format!("cannot read {}: {error}", fixture.pdb.display());
            return vec![CaseResult::failed(first, problem)];
        }
    };
    let baseline = match checks::baseline(tools, &fixture.pdb) {
        Ok(baseline) => baseline,
        Err(error) => return vec![CaseResult::failed(first, error)],
    };
    let directory = fixture.work_directory(work);
    if let Err(error) = fs::create_dir_all(&directory) {
        let problem = format!("cannot make {}: {error}", directory.display());
        return vec![CaseResult::failed(first, problem)];
    }
    Task::ALL
        .iter()
        .map(|&task| check_case(tools, fixture, baseline, &source, &directory, task))
        .collect()
}

/// Run the task cases on the PDBs of `list`, print the results and a summary line, and
/// return the number of problems.
pub(super) fn run(tools: &Tools, list: &[NativeFixture], work: &Path, jobs: &Jobs) -> usize {
    println!("task cases:");
    let results = jobs.map(list, |fixture| check_fixture(tools, fixture, work));
    let (mut cases, mut applied, mut notes, mut problems) = (0, 0, 0, 0);
    let mut classes = BTreeMap::<(&str, &str), usize>::new();
    let mut tasks = BTreeMap::<Task, (usize, usize)>::new();
    for (fixture, results) in list.iter().zip(&results) {
        for result in results {
            let status = if result.problems.is_empty() {
                "pass"
            } else {
                "FAIL"
            };
            println!(
                "{} {}: {status}, changed streams {:?}",
                fixture.label(),
                result.task.name(),
                result.changed
            );
            for note in &result.notes {
                println!("  note: {note}");
            }
            for problem in &result.problems {
                println!("  problem: {problem}");
            }
            cases += 1;
            applied += usize::from(result.applied);
            notes += result.notes.len();
            problems += result.problems.len();
            let entry = tasks.entry(result.task).or_default();
            entry.0 += usize::from(result.applied);
            entry.1 += result.notes.len();
            for class in &result.explained {
                *classes.entry((result.task.name(), class)).or_default() += 1;
            }
        }
    }
    for ((task, class), count) in &classes {
        println!("explained ({task}): {class}: {count} PDBs");
    }
    for (task, (applied, notes)) in &tasks {
        println!(
            "task {}: applied on {applied} PDBs, notes {notes}",
            task.name()
        );
    }
    println!(
        "task section: native PDBs: {}, cases: {cases}, applied: {applied}, notes: {notes}, problems without an explanation: {problems}",
        list.len()
    );
    problems
}
