//! The remap cases of `verify-native-edit`.
//!
//! After the task section, the command runs `PdbEditor::remap_source_paths` on each
//! native PDB. The function replaces the build root of the fixtures with `/_/` (see
//! [`BuildRoot`]). The command writes `target/verify-native-edit/<fixture>/<PDB file stem>/
//! remap-source-paths.pdb` and checks it:
//!
//! 1. The byte search for the root over all streams finds the root only in the fields that
//!    the remap does not change (see [`search`]).
//! 2. The consumer checks of [`super::checks`]: `pdbtool check` and `compare` (the report
//!    lists exactly the streams that `compare` shows as different), `llvm-pdbutil` and
//!    `DIA2Dump`, the new `/src/files/` names and `pdbstr -r -s:srcsrv`.
//! 3. `DIA2Dump -l` of the output is `DIA2Dump -l` of the source with the function applied to
//!    each file name: the lines of each procedure have the new file names and the same line
//!    numbers.
//! 4. A function that keeps every path gives a plan without a changed stream.

mod search;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use pdb_rs::native::auxiliary::SourceServer;
use pdb_rs::native::injected_source::SOURCE_FILE_STREAM_PREFIX;
use pdb_rs::native::{EditOptions, IdentityPolicy, Pdb};

use super::super::native::NativeFixture;
use super::super::native::consumers::dia2dump;
use super::super::source_paths::BuildRoot;
use super::Tools;
use super::checks;
use super::edits::{self, Expectation, Outcome, lossy};
use crate::jobs::Jobs;

/// The name of the case. The output file is `<name>.pdb`.
const CASE: &str = "remap-source-paths";

/// The result of the case on one PDB.
#[derive(Debug, Default)]
struct CaseResult {
    /// `true` if the command wrote an output.
    applied: bool,
    /// The number of paths that got a new value.
    paths: usize,
    /// The streams that the plan changed, removed or added.
    changed: Vec<u32>,
    /// The occurrences of the root in each excluded class.
    excluded: BTreeMap<&'static str, usize>,
    /// The number of procedures in the `DIA2Dump -l` output.
    procedures: usize,
    /// The explained classes of the consumer checks.
    explained: Vec<&'static str>,
    /// The problems without an explanation.
    problems: Vec<String>,
}

impl CaseResult {
    /// Return a result with one problem.
    fn failed(problem: String) -> Self {
        Self {
            problems: vec![problem],
            ..Self::default()
        }
    }
}

/// Return the `srcsrv` text of `pdb` with the new source file paths, or `None` if the PDB has
/// no `srcsrv` stream.
fn expected_source_server(root: &BuildRoot, pdb: &Pdb<&[u8]>) -> Result<Option<String>, String> {
    let read = |error: pdb_rs::Error| format!("cannot read srcsrv: {error}");
    let Some(stream) = pdb.info().named_stream(b"srcsrv") else {
        return Ok(None);
    };
    let bytes = pdb.stream_bytes(stream).map_err(read)?;
    SourceServer::parse(&bytes, stream).map_err(read)?;
    let mut text = String::new();
    // Rebuild the text line by line: the source file lines get the new first field.
    let mut in_files = false;
    for line in String::from_utf8_lossy(&bytes).split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.to_ascii_lowercase().starts_with("srcsrv:") {
            in_files = trimmed.to_ascii_lowercase().contains("source files");
            text.push_str(line);
            continue;
        }
        if in_files && !trimmed.trim().is_empty() {
            let (path, rest) = trimmed
                .split_once('*')
                .map_or((trimmed, None), |(path, rest)| (path, Some(rest)));
            let new = root
                .remap(path.as_bytes())
                .map_or_else(|| path.to_owned(), |new| lossy(&new));
            text.push_str(&new);
            if let Some(rest) = rest {
                text.push('*');
                text.push_str(rest);
            }
            text.push_str(&line[trimmed.len()..]);
            continue;
        }
        text.push_str(line);
    }
    Ok(Some(text))
}

/// Set the expected `/src/files/` names and `srcsrv` text of the remap of `pdb`.
fn expectation(root: &BuildRoot, pdb: &Pdb<&[u8]>) -> Result<Expectation, String> {
    let mut expect = Expectation::default();
    for named in pdb.info().named_streams() {
        let Some(name) = named.name.strip_prefix(SOURCE_FILE_STREAM_PREFIX) else {
            continue;
        };
        if let Some(new) = root.remap(name) {
            let mut stream = SOURCE_FILE_STREAM_PREFIX.to_vec();
            stream.extend_from_slice(&new);
            expect.named_present.push(lossy(&stream));
            expect.named_absent.push(lossy(&named.name));
        }
    }
    expect.source_server = expected_source_server(root, pdb)?;
    Ok(expect)
}

/// Apply the remap to the PDB `source` and plan the output.
fn apply(root: &BuildRoot, source: &[u8]) -> Result<(edits::Edited, usize), String> {
    let pdb = Pdb::parse(source).map_err(|error| format!("cannot read the PDB: {error}"))?;
    let mut keep = pdb.edit(IdentityPolicy::Preserve);
    keep.remap_source_paths(|_| None).map_err(|error| {
        format!("the task with a function that keeps every path fails: {error}")
    })?;
    let plan = keep
        .plan(EditOptions::default())
        .map_err(|error| format!("the plan of a function that keeps every path fails: {error}"))?;
    let kept: Vec<u32> = plan.report().changed().map(|id| id.0).collect();
    if !kept.is_empty() {
        return Err(format!(
            "a function that keeps every path changes the streams {kept:?}"
        ));
    }
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    let paths = editor
        .remap_source_paths(|path| root.remap(path))
        .map_err(|error| format!("the task fails: {error}"))?;
    let expect = expectation(root, &pdb)?;
    match edits::plan(&editor, EditOptions::default(), expect, Vec::new())? {
        Outcome::Edited(edited) => Ok((*edited, paths)),
        Outcome::Rejected(error) => Err(format!("the plan rejects the remap: {error}")),
    }
}

/// Return the `DIA2Dump -l` text of the source with the function applied to each file name.
/// A file name is the text after the last tab of a line, with its checksum suffix.
fn remap_lines(root: &BuildRoot, text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        match line.rsplit_once('\t') {
            Some((head, tail)) => match root.remap(tail.as_bytes()) {
                Some(new) => {
                    out.push_str(head);
                    out.push('\t');
                    out.push_str(&lossy(&new));
                }
                None => out.push_str(line),
            },
            None => out.push_str(line),
        }
    }
    out
}

/// Compare `DIA2Dump -l` of the output with the source. Return the number of procedures.
fn compare_lines(
    tools: &Tools,
    root: &BuildRoot,
    source: &Path,
    output: &Path,
    problems: &mut Vec<String>,
) -> Result<usize, String> {
    let before = dia2dump(&tools.dia2dump, ["-l"], source)?;
    let after = dia2dump(&tools.dia2dump, ["-l"], output)?;
    if before.code != after.code {
        problems.push(format!(
            "DIA2Dump -l exits {:?}, on the source {:?}",
            after.code, before.code
        ));
    }
    let expected = remap_lines(root, &before.text);
    if expected != after.text {
        let first = expected
            .lines()
            .zip(after.text.lines())
            .find(|(left, right)| left != right)
            .map_or_else(String::new, |(left, right)| {
                format!(": expected `{left}`, found `{right}`")
            });
        problems.push(format!(
            "DIA2Dump -l differs from the remapped source{first}"
        ));
    }
    Ok(after
        .text
        .lines()
        .filter(|line| line.trim_start().starts_with("line ") && line.contains('\t'))
        .count())
}

/// Run the case on one PDB and check the output.
fn check_fixture(
    tools: &Tools,
    root: &BuildRoot,
    fixture: &NativeFixture,
    work: &Path,
) -> CaseResult {
    let source = match fs::read(&fixture.pdb) {
        Ok(bytes) => bytes,
        Err(error) => {
            return CaseResult::failed(format!("cannot read {}: {error}", fixture.pdb.display()));
        }
    };
    let baseline = match checks::baseline(tools, &fixture.pdb) {
        Ok(baseline) => baseline,
        Err(error) => return CaseResult::failed(error),
    };
    let directory = fixture.work_directory(work);
    let path = directory.join(format!("{CASE}.pdb"));
    let _ = fs::remove_file(&path);
    if let Err(error) = fs::create_dir_all(&directory) {
        return CaseResult::failed(format!("cannot make {}: {error}", directory.display()));
    }
    let (edited, paths) = match apply(root, &source) {
        Ok(applied) => applied,
        Err(error) => return CaseResult::failed(error),
    };
    if let Err(error) = fs::write(&path, &edited.bytes) {
        return CaseResult::failed(format!("cannot write {}: {error}", path.display()));
    }
    let mut result = CaseResult {
        applied: true,
        paths,
        changed: edited.changed.clone(),
        ..CaseResult::default()
    };
    match search::search(root, &edited.bytes) {
        Ok(found) => {
            result.excluded = found.classes;
            result.problems.extend(found.problems);
        }
        Err(error) => result.problems.push(error),
    }
    match checks::check(tools, &fixture.pdb, baseline, &path, &edited) {
        Ok(findings) => {
            result.explained = findings.explained;
            result.problems.extend(findings.problems);
        }
        Err(error) => result.problems.push(error),
    }
    match compare_lines(tools, root, &fixture.pdb, &path, &mut result.problems) {
        Ok(count) => result.procedures = count,
        Err(error) => result.problems.push(error),
    }
    result
}

/// Run the remap cases on the PDBs of `list`, print the results and a summary line, and
/// return the number of problems.
pub(super) fn run(
    tools: &Tools,
    list: &[NativeFixture],
    fixtures: &Path,
    work: &Path,
    jobs: &Jobs,
) -> usize {
    println!("remap cases:");
    let root = match BuildRoot::from_manifest(fixtures) {
        Ok(root) => root,
        Err(error) => {
            println!("problem: {error}");
            return 1;
        }
    };
    println!("build root: {} -> /_/", root.text());
    let results = jobs.map(list, |fixture| check_fixture(tools, &root, fixture, work));
    let (mut applied, mut paths, mut problems) = (0, 0, 0);
    let mut excluded = BTreeMap::<&str, (usize, usize)>::new();
    let mut classes = BTreeMap::<&str, usize>::new();
    for (fixture, result) in list.iter().zip(&results) {
        let status = if result.problems.is_empty() {
            "pass"
        } else {
            "FAIL"
        };
        let hits: Vec<String> = result
            .excluded
            .iter()
            .map(|(class, count)| format!("{count} {class}"))
            .collect();
        println!(
            "{} {CASE}: {status}, {} paths, {} procedure line blocks, changed streams {:?}; root left: {}",
            fixture.label(),
            result.paths,
            result.procedures,
            result.changed,
            if hits.is_empty() {
                "none".to_owned()
            } else {
                hits.join(", ")
            }
        );
        for problem in &result.problems {
            println!("  problem: {problem}");
        }
        applied += usize::from(result.applied);
        paths += result.paths;
        problems += result.problems.len();
        for (class, count) in &result.excluded {
            let entry = excluded.entry(class).or_default();
            entry.0 += count;
            entry.1 += 1;
        }
        for class in &result.explained {
            *classes.entry(class).or_default() += 1;
        }
    }
    for (class, count) in &classes {
        println!("explained ({CASE}): {class}: {count} PDBs");
    }
    for (class, (count, pdbs)) in &excluded {
        println!("root left {class}: {count} times in {pdbs} PDBs");
    }
    println!(
        "remap section: native PDBs: {}, applied: {applied}, paths: {paths}, problems without an explanation: {problems}",
        list.len()
    );
    problems
}
