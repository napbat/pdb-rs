//! The change and removal cases of `verify-native-edit`.
//!
//! The command runs each [`Case`] on each native PDB after the edits of [`super::edits`]. Each
//! case changes or removes one record with `IdentityPolicy::IncrementAge` and writes
//! `target/verify-native-edit/<fixture>/<PDB file stem>/<case>.pdb`. Then it runs the
//! consumer checks of [`super::checks`] and reads the output with `pdbrs` (see [`confirm`]).
//!
//! A case that does not apply to a PDB gives a note and no output.

mod change;
mod confirm;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use pdbrs::native::{EditOptions, IdentityPolicy, Pdb};

use super::super::native::NativeFixture;
use super::Tools;
use super::checks;
use super::edits::{self, Outcome};
use crate::jobs::Jobs;
use change::Prepared;

/// One change and removal case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Case {
    /// Add 1 to the address offset of a public symbol.
    PublicAddress,
    /// Change the type of a global data symbol.
    GlobalType,
    /// Change the type of a module symbol.
    LocalType,
    /// Add 1 to a line number.
    LineNumber,
    /// Add 1 to the offset of an `LF_MEMBER` member.
    MemberOffset,
    /// Remove a public symbol.
    RemovePublic,
    /// Remove a global symbol.
    RemoveGlobal,
    /// Remove a module symbol.
    RemoveModuleSymbol,
}

impl Case {
    /// All cases in the order of the command.
    const ALL: [Self; 8] = [
        Self::PublicAddress,
        Self::GlobalType,
        Self::LocalType,
        Self::LineNumber,
        Self::MemberOffset,
        Self::RemovePublic,
        Self::RemoveGlobal,
        Self::RemoveModuleSymbol,
    ];

    /// The name of the case in the output and in file names.
    const fn name(self) -> &'static str {
        match self {
            Self::PublicAddress => "public-address",
            Self::GlobalType => "global-type",
            Self::LocalType => "local-type",
            Self::LineNumber => "line-number",
            Self::MemberOffset => "member-offset",
            Self::RemovePublic => "remove-public",
            Self::RemoveGlobal => "remove-global",
            Self::RemoveModuleSymbol => "remove-module-symbol",
        }
    }
}

/// The result of one case of one PDB.
#[derive(Debug)]
struct CaseResult {
    /// The case.
    case: Case,
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
    fn failed(case: Case, problem: String) -> Self {
        Self {
            case,
            applied: false,
            changed: Vec::new(),
            notes: Vec::new(),
            explained: Vec::new(),
            problems: vec![problem],
        }
    }
}

/// The change of one case: an output, or a note.
enum Applied {
    /// The case does not apply. The text tells why.
    NotApplied(String),
    /// The edited PDB and the place of the change.
    Edited(Box<edits::Edited>, change::Target),
}

/// Apply `case` to the PDB `source`.
fn apply(case: Case, source: &[u8]) -> Result<Applied, String> {
    let pdb = Pdb::parse(source).map_err(|error| format!("cannot read the PDB: {error}"))?;
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let prepared = change::prepare(case, &pdb, &mut editor)
        .map_err(|error| format!("cannot prepare the edit: {error}"))?;
    let (target, expect) = match prepared {
        Prepared::NotApplied(note) => return Ok(Applied::NotApplied(note)),
        Prepared::Changed(changed) => *changed,
    };
    match edits::plan(&editor, EditOptions::default(), expect, Vec::new())? {
        Outcome::Edited(edited) => Ok(Applied::Edited(edited, target)),
        Outcome::Rejected(error) => Err(format!("the plan rejects the edit: {error}")),
    }
}

/// Run one case on one PDB and check the output.
fn check_case(
    tools: &Tools,
    fixture: &NativeFixture,
    baseline: checks::Baseline,
    source: &[u8],
    directory: &Path,
    case: Case,
) -> CaseResult {
    let path = directory.join(format!("{}.pdb", case.name()));
    let _ = fs::remove_file(&path);
    let (edited, target) = match apply(case, source) {
        Ok(Applied::Edited(edited, target)) => (edited, target),
        Ok(Applied::NotApplied(note)) => {
            return CaseResult {
                case,
                applied: false,
                changed: Vec::new(),
                notes: vec![note],
                explained: Vec::new(),
                problems: Vec::new(),
            };
        }
        Err(error) => return CaseResult::failed(case, error),
    };
    if let Err(error) = fs::write(&path, &edited.bytes) {
        return CaseResult::failed(case, format!("cannot write {}: {error}", path.display()));
    }
    let mut findings = match checks::check(tools, &fixture.pdb, baseline, &path, &edited) {
        Ok(findings) => findings,
        Err(error) => return CaseResult::failed(case, error),
    };
    if let Err(problem) = confirm::confirm(&edited.bytes, &target) {
        findings.problems.push(problem);
    }
    CaseResult {
        case,
        applied: true,
        changed: edited.changed,
        notes: edited.notes,
        explained: findings.explained,
        problems: findings.problems,
    }
}

/// Run all cases on one PDB.
fn check_fixture(tools: &Tools, fixture: &NativeFixture, work: &Path) -> Vec<CaseResult> {
    let first = Case::ALL[0];
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
    Case::ALL
        .iter()
        .map(|&case| check_case(tools, fixture, baseline, &source, &directory, case))
        .collect()
}

/// The totals of the change and removal cases.
#[derive(Debug, Default)]
pub(super) struct Totals {
    /// The number of cases that ran.
    pub cases: usize,
    /// The number of cases that wrote an output.
    pub applied: usize,
    /// The number of notes.
    pub notes: usize,
    /// The number of problems without an explanation.
    pub problems: usize,
}

/// Print the results of one PDB and add them to `totals`.
fn print_results(fixture: &NativeFixture, results: &[CaseResult], totals: &mut Totals) {
    for result in results {
        let status = if result.problems.is_empty() {
            "pass"
        } else {
            "FAIL"
        };
        println!(
            "{} {}: {status}, changed streams {:?}",
            fixture.label(),
            result.case.name(),
            result.changed
        );
        for note in &result.notes {
            println!("  note: {note}");
        }
        for problem in &result.problems {
            println!("  problem: {problem}");
        }
        totals.cases += 1;
        totals.applied += usize::from(result.applied);
        totals.notes += result.notes.len();
        totals.problems += result.problems.len();
    }
}

/// Run the change and removal cases on the PDBs of `list`, print the results and return the
/// totals.
pub(super) fn run(tools: &Tools, list: &[NativeFixture], work: &Path, jobs: &Jobs) -> Totals {
    println!("change and removal cases:");
    let results = jobs.map(list, |fixture| check_fixture(tools, fixture, work));
    let mut totals = Totals::default();
    let mut classes = BTreeMap::<(&str, &str), usize>::new();
    let mut cases = BTreeMap::<Case, (usize, usize)>::new();
    for (fixture, results) in list.iter().zip(&results) {
        print_results(fixture, results, &mut totals);
        for result in results {
            let entry = cases.entry(result.case).or_default();
            entry.0 += usize::from(result.applied);
            entry.1 += result.notes.len();
            for class in &result.explained {
                *classes.entry((result.case.name(), class)).or_default() += 1;
            }
        }
    }
    for ((case, class), count) in &classes {
        println!("explained ({case}): {class}: {count} PDBs");
    }
    for (case, (applied, notes)) in &cases {
        println!(
            "case {}: applied on {applied} PDBs, notes {notes}",
            case.name()
        );
    }
    totals
}
