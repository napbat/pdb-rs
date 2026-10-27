//! The record removal cases of `verify-native-edit`.
//!
//! After the task section, the command removes records from each native PDB in two cases:
//!
//! - `remove-types` removes each TPI record that is not a UDT or a modifier of a UDT and that
//!   nothing refers to, with `TypeList::remove`.
//! - `remove-ids` removes each `LF_UDT_SRC_LINE` and `LF_UDT_MOD_SRC_LINE` record of the IPI
//!   stream that nothing refers to, with `IdList::remove`.
//!
//! [`references`] selects the records. Each case plans with `EditOptions::default()` and
//! writes `target/verify-native-edit/<fixture>/<PDB file stem>/<case>.pdb`.
//!
//! A plan error `UnsupportedEdit` is a refusal, not a problem. Each output gets these checks:
//!
//! 1. The checks of [`super::checks::check`]: `pdbtool check`, `pdbtool compare` against the
//!    report, and the exit codes of the full dumps.
//! 2. `llvm-pdbutil dump --types --ids --symbols --globals` of the output must be equal to the
//!    dump of the source after the mapping of [`dump::map_dump`]. The indices of the mapping
//!    come from `PdbWritePlan::type_index` or `PdbWritePlan::id_index`.
//! 3. The masked `DIA2Dump -all` output must be equal to the output for the source: DIA prints
//!    no record index, and the cases remove no record that DIA lists.

mod dump;
mod references;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use pdbrs::ErrorKind;
use pdbrs::native::codeview::index::{IdIndex, TypeIndex};
use pdbrs::native::{EditOptions, IdentityPolicy, Pdb, PdbWritePlan};

use super::super::native::NativeFixture;
use super::super::native::consumers::{Run, dia2dump, run};
use super::Tools;
use super::checks::{self, Baseline};
use super::edits::{self, Edited, Expectation, Outcome};
use crate::jobs::Jobs;
use dump::Space;

/// The arguments of the dump comparison, before the PDB.
const DUMP_ARGUMENTS: [&str; 5] = ["dump", "--types", "--ids", "--symbols", "--globals"];

/// The explained class of a dump that fails on the output as on the source.
const DUMP_FAILS: &str =
    "llvm-pdbutil dump --types --ids --symbols --globals fails as on the source";

/// The explained class of a `DIA2Dump -all` run that fails on the output as on the source.
const DIA_FAILS: &str = "DIA2Dump -all fails as on the source";

/// One removal case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    /// `remove-types`.
    Types,
    /// `remove-ids`.
    Ids,
}

impl Case {
    /// All cases in the order of the command.
    const ALL: [Self; 2] = [Self::Types, Self::Ids];

    /// The name of the case in the output and the file stem of the output files.
    const fn name(self) -> &'static str {
        match self {
            Self::Types => "remove-types",
            Self::Ids => "remove-ids",
        }
    }

    /// The index space that the case compacts.
    const fn space(self) -> Space {
        match self {
            Self::Types => Space::Types,
            Self::Ids => Space::Ids,
        }
    }
}

/// The state of a case on one PDB.
#[derive(Debug)]
enum Status {
    /// The plan accepted the removal and the command wrote the output.
    Applied,
    /// The plan refused the removal. The text is the error.
    Refused(String),
    /// The case did not change the PDB.
    NotApplied,
}

/// The result of a case on one PDB.
#[derive(Debug)]
struct CaseResult {
    /// The case.
    case: Case,
    /// The state.
    status: Status,
    /// The number of removed records.
    removed: usize,
    /// The streams that the plan changed, removed or added.
    changed: Vec<u32>,
    /// The notes.
    notes: Vec<String>,
    /// The explained classes.
    explained: Vec<&'static str>,
    /// The problems without an explanation.
    problems: Vec<String>,
}

impl CaseResult {
    /// Return a result without a change.
    const fn new(case: Case, status: Status) -> Self {
        Self {
            case,
            status,
            removed: 0,
            changed: Vec::new(),
            notes: Vec::new(),
            explained: Vec::new(),
            problems: Vec::new(),
        }
    }

    /// Return a result with one problem.
    fn failed(case: Case, problem: String) -> Self {
        let mut result = Self::new(case, Status::NotApplied);
        result.problems.push(problem);
        result
    }
}

/// The output of the plan: the edited PDB and the output index of each source index of the
/// compacted space from `0x1000` (`None` for a removed record).
struct Removal {
    /// The edited PDB.
    edited: Box<Edited>,
    /// The output indices.
    indices: Vec<Option<u32>>,
}

/// The inputs of the cases on one PDB that the cases share.
struct Source<'s> {
    /// The fixture.
    fixture: &'s NativeFixture,
    /// The bytes of the PDB.
    bytes: &'s [u8],
    /// The dump of [`DUMP_ARGUMENTS`].
    dump: Run,
    /// The work directory of the PDB.
    directory: &'s Path,
    /// The results of the full dumps, after the first applied case.
    baseline: Option<Baseline>,
    /// The masked `DIA2Dump -all` run, after the first applied case.
    dia: Option<Run>,
}

/// Run `llvm-pdbutil` with [`DUMP_ARGUMENTS`] on `pdb`.
fn pdbutil_dump(tools: &Tools, pdb: &Path) -> Result<Run, String> {
    let mut arguments: Vec<&OsStr> = DUMP_ARGUMENTS.iter().map(OsStr::new).collect();
    arguments.push(pdb.as_os_str());
    run(&tools.pdbutil, arguments, None)
}

/// Return the output index of each source index of the space of `case` from `0x1000` to
/// `end`.
///
/// Check also that the plan refuses each removed index with `InvalidReference`.
fn output_indices(
    plan: &PdbWritePlan<'_, &[u8]>,
    case: Case,
    removed: &BTreeSet<u32>,
    end: u32,
) -> Result<Vec<Option<u32>>, String> {
    (0x1000..end)
        .map(|index| {
            let output = match case {
                Case::Types => plan.type_index(TypeIndex(index)).map(|output| output.0),
                Case::Ids => plan.id_index(IdIndex(index)).map(|output| output.0),
            };
            match (output, removed.contains(&index)) {
                (Ok(output), false) => Ok(Some(output)),
                (Err(error), true) if error.kind() == ErrorKind::InvalidReference => Ok(None),
                (output, _) => Err(format!(
                    "the plan gives {output:?} for the {} index {index:#06X}",
                    case.name()
                )),
            }
        })
        .collect()
}

/// Remove `candidates` of the space of `case` from `source` and plan. Return
/// `Ok(Err(error))` for a plan error.
fn remove(
    source: &[u8],
    case: Case,
    candidates: &[u32],
) -> Result<Result<Removal, pdbrs::Error>, String> {
    let pdb = Pdb::parse(source).map_err(|error| format!("cannot read the PDB: {error}"))?;
    let stream = match case {
        Case::Types => pdb.types(),
        Case::Ids => pdb.ids(),
    };
    let end = stream
        .map_err(|error| format!("cannot read the record stream: {error}"))?
        .map_or(0x1000, |records| records.header().index_end);
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let model = |error: pdbrs::Error| format!("cannot read the records: {error}");
    for &index in candidates {
        let removed = match case {
            Case::Types => editor.types_mut().map_err(model)?.remove(TypeIndex(index)),
            Case::Ids => editor.ids_mut().map_err(model)?.remove(IdIndex(index)),
        };
        removed.map_err(|error| format!("the removal of {index:#06X} fails: {error}"))?;
    }
    let removed: BTreeSet<u32> = candidates.iter().copied().collect();
    let (outcome, indices) = edits::plan_with(
        &editor,
        EditOptions::default(),
        Expectation::default(),
        Vec::new(),
        |plan| output_indices(plan, case, &removed, end),
    )?;
    Ok(match (outcome, indices) {
        (Outcome::Edited(edited), Some(indices)) => Ok(Removal { edited, indices }),
        (Outcome::Rejected(error), _) => Err(error),
        (Outcome::Edited(_), None) => return Err("the plan gives no output indices".into()),
    })
}

/// Compare the dump of the output at `path` with the mapped dump of the source.
fn compare_dumps(
    tools: &Tools,
    source: &Source<'_>,
    path: &Path,
    case: Case,
    candidates: &[u32],
    indices: &[Option<u32>],
    result: &mut CaseResult,
) -> Result<(), String> {
    let output = pdbutil_dump(tools, path)?;
    if output.code != source.dump.code {
        result.problems.push(format!(
            "llvm-pdbutil dump exit {:?}, source {:?}: {}",
            output.code,
            source.dump.code,
            crate::step::last_lines(&output.text, 2)
        ));
        return Ok(());
    }
    if output.code != Some(0) {
        result.explained.push(DUMP_FAILS);
        return Ok(());
    }
    let removed: BTreeSet<u32> = candidates.iter().copied().collect();
    let map = |index: u32| {
        let position = usize::try_from(index.checked_sub(0x1000)?).ok()?;
        indices.get(position).copied().flatten()
    };
    let removal = dump::Removal {
        space: case.space(),
        removed: &removed,
        map: &map,
    };
    let mapped = match dump::map_dump(&source.dump.text, &removal) {
        Ok(mapped) => mapped,
        Err(error) => {
            result
                .problems
                .push(format!("cannot map the source dump: {error}"));
            return Ok(());
        }
    };
    if let Some(difference) = dump::first_difference(&mapped, &output.text) {
        for (suffix, text) in [
            ("source", mapped.as_str()),
            ("output", output.text.as_str()),
        ] {
            let file = source
                .directory
                .join(format!("{}.{suffix}.txt", case.name()));
            fs::write(&file, text)
                .map_err(|error| format!("cannot write {}: {error}", file.display()))?;
        }
        result.problems.push(format!(
            "llvm-pdbutil dump differs from the mapped source dump at {difference}"
        ));
    }
    Ok(())
}

/// Compare the masked `DIA2Dump -all` outputs of the source and the output.
fn compare_dia(
    tools: &Tools,
    source: &mut Source<'_>,
    path: &Path,
    result: &mut CaseResult,
) -> Result<(), String> {
    let before = match source.dia.take() {
        Some(run) => run,
        None => dia2dump(&tools.dia2dump, ["-all"], &source.fixture.pdb)?,
    };
    let after = dia2dump(&tools.dia2dump, ["-all"], path)?;
    if before.code != after.code {
        result.problems.push(format!(
            "DIA2Dump -all exit {:?}, source {:?}",
            after.code, before.code
        ));
    } else if before.code != Some(0) {
        result.explained.push(DIA_FAILS);
    } else if let Some(difference) = dump::first_difference(&before.text, &after.text) {
        result.problems.push(format!(
            "DIA2Dump -all differs from the source at {difference}"
        ));
    }
    source.dia = Some(before);
    Ok(())
}

/// Run `case` on one PDB with the records `candidates`. Return an error for a problem that
/// stops the case.
fn run_case(
    tools: &Tools,
    source: &mut Source<'_>,
    case: Case,
    candidates: &[u32],
) -> Result<CaseResult, String> {
    let name = case.name();
    for suffix in ["pdb", "source.txt", "output.txt"] {
        let _ = fs::remove_file(source.directory.join(format!("{name}.{suffix}")));
    }
    if candidates.is_empty() {
        let mut result = CaseResult::new(case, Status::NotApplied);
        let note = match case {
            Case::Types => "no unreferenced type record",
            Case::Ids => "no unreferenced UDT source line record",
        };
        result.notes.push(note.into());
        return Ok(result);
    }
    let removal = match remove(source.bytes, case, candidates)? {
        Ok(removal) => removal,
        Err(error) if error.kind() == ErrorKind::UnsupportedEdit => {
            return Ok(CaseResult::new(case, Status::Refused(error.to_string())));
        }
        Err(error) => return Err(format!("the plan rejects the removal: {error}")),
    };
    let path = source.directory.join(format!("{name}.pdb"));
    fs::write(&path, &removal.edited.bytes)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    let baseline = match source.baseline {
        Some(baseline) => baseline,
        None => checks::baseline(tools, &source.fixture.pdb)?,
    };
    source.baseline = Some(baseline);
    let findings = checks::check(tools, &source.fixture.pdb, baseline, &path, &removal.edited)?;
    let mut result = CaseResult::new(case, Status::Applied);
    result.removed = candidates.len();
    result.changed.clone_from(&removal.edited.changed);
    result.notes.clone_from(&removal.edited.notes);
    result.explained = findings.explained;
    result.problems = findings.problems;
    compare_dumps(
        tools,
        source,
        &path,
        case,
        candidates,
        &removal.indices,
        &mut result,
    )?;
    compare_dia(tools, source, &path, &mut result)?;
    Ok(result)
}

/// Run both cases on one PDB.
fn check_fixture(tools: &Tools, fixture: &NativeFixture, work: &Path) -> Vec<CaseResult> {
    let directory = fixture.work_directory(work);
    let prepared = fs::read(&fixture.pdb)
        .map_err(|error| format!("cannot read {}: {error}", fixture.pdb.display()))
        .and_then(|bytes| {
            fs::create_dir_all(&directory)
                .map_err(|error| format!("cannot make {}: {error}", directory.display()))?;
            let dump = pdbutil_dump(tools, &fixture.pdb)?;
            let pdb = Pdb::parse(bytes.as_slice())
                .map_err(|error| format!("cannot read the PDB: {error}"))?;
            let candidates = references::candidates(&pdb, &dump.text)?;
            Ok((bytes, dump, candidates))
        });
    let (bytes, dump, candidates) = match prepared {
        Ok(prepared) => prepared,
        Err(problem) => return vec![CaseResult::failed(Case::Types, problem)],
    };
    let mut source = Source {
        fixture,
        bytes: &bytes,
        dump,
        directory: &directory,
        baseline: None,
        dia: None,
    };
    Case::ALL
        .iter()
        .map(|&case| {
            let records = match case {
                Case::Types => &candidates.types,
                Case::Ids => &candidates.ids,
            };
            run_case(tools, &mut source, case, records)
                .unwrap_or_else(|problem| CaseResult::failed(case, problem))
        })
        .collect()
}

/// Print the result of one case on one PDB.
fn print_result(fixture: &NativeFixture, result: &CaseResult) {
    let label = fixture.label();
    let name = result.case.name();
    match &result.status {
        Status::Refused(error) => println!("{label} {name}: refused ({error})"),
        Status::Applied => println!(
            "{label} {name}: {}, removed {} records, changed streams {:?}",
            if result.problems.is_empty() {
                "pass"
            } else {
                "FAIL"
            },
            result.removed,
            result.changed
        ),
        Status::NotApplied if result.problems.is_empty() => println!("{label} {name}: no change"),
        Status::NotApplied => println!("{label} {name}: FAIL"),
    }
    for note in &result.notes {
        println!("  note: {note}");
    }
    for problem in &result.problems {
        println!("  problem: {problem}");
    }
}

/// The counts of the summary line.
#[derive(Debug, Default)]
struct Totals {
    /// The cases.
    cases: usize,
    /// The applied cases.
    applied: usize,
    /// The refused cases.
    refused: usize,
    /// The notes.
    notes: usize,
    /// The removed TPI records.
    types: usize,
    /// The removed IPI records.
    ids: usize,
    /// The problems without an explanation.
    problems: usize,
}

/// Run the removal cases on the PDBs of `list`, print the results and a summary line, and
/// return the number of problems.
pub(super) fn run_section(
    tools: &Tools,
    list: &[NativeFixture],
    work: &Path,
    jobs: &Jobs,
) -> usize {
    println!("type removal cases:");
    let results = jobs.map(list, |fixture| check_fixture(tools, fixture, work));
    let mut totals = Totals::default();
    let mut classes = BTreeMap::<(&str, &str), usize>::new();
    for (fixture, results) in list.iter().zip(&results) {
        for result in results {
            print_result(fixture, result);
            totals.cases += 1;
            match result.status {
                Status::Applied => totals.applied += 1,
                Status::Refused(_) => totals.refused += 1,
                Status::NotApplied => {}
            }
            match result.case {
                Case::Types => totals.types += result.removed,
                Case::Ids => totals.ids += result.removed,
            }
            totals.notes += result.notes.len();
            totals.problems += result.problems.len();
            for class in &result.explained {
                *classes.entry((result.case.name(), class)).or_default() += 1;
            }
        }
    }
    for ((case, class), count) in &classes {
        println!("explained ({case}): {class}: {count} PDBs");
    }
    println!(
        "type removal section: native PDBs: {}, cases: {}, applied: {}, refused: {}, notes: {}, removed type records: {}, removed ID records: {}, problems without an explanation: {}",
        list.len(),
        totals.cases,
        totals.applied,
        totals.refused,
        totals.notes,
        totals.types,
        totals.ids,
        totals.problems
    );
    totals.problems
}
