//! The `verify-remap` command (OMAP output and clean output).
//!
//! For each native PDB of the fixture manifest (see [`super::native::native_fixtures`]), the
//! command makes the range sets of [`ranges`] from the procedures of the source
//! (`SymbolLocator::procedures`). For each set, it remaps the PDB with
//! `PdbEditor::remap_addresses` in OMAP mode and writes the output to
//! `target/verify-remap/<fixture>/<PDB file stem>/<set>.pdb`. Then it checks the output:
//!
//! 1. The plan report: the changed streams are the Info stream (`IdentityPolicy::IncrementAge`),
//!    the DBI stream and each OMAP output stream with new bytes, and the plan removes no
//!    stream. A section header stream with the source bytes stays a copied stream.
//! 2. The pdbrs reader: the output has the address tables that `ImageRemap::address_tables`
//!    gives.
//! 3. Determinism: the same ranges, added in reverse order, give the same bytes.
//! 4. `pdbtool check` finds no error.
//! 5. `DIA2Dump` (see [`dia`]): each procedure, public symbol, data symbol and line has the new
//!    RVA that the ranges give for its source RVA.
//!
//! A procedure is removable if no reference record of the global index (`S_PROCREF`,
//! `S_LPROCREF`) names it and it has no separated code chunk (`S_SEPCODE`). The removal set
//! removes these procedures.
//!
//! After the OMAP part, the command checks the clean output of each set (see [`clean`]). It
//! compares the clean output with the OMAP output of the same set.

mod clean;
mod dia;
mod ranges;
mod unwind;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use pdbrs::native::address::{AddressTables, Rva};
use pdbrs::native::codeview::symbol::{ModuleSymbolTarget, SymbolData};
use pdbrs::native::dbi::DebugStream;
use pdbrs::native::info::Feature;
use pdbrs::native::locate::SymbolLocator;
use pdbrs::native::msf::StreamId;
use pdbrs::native::section::SectionHeaderStream;
use pdbrs::native::{EditOptions, IdentityPolicy, ImageRemap, Pdb, RemapMode};

use super::native::consumers::{Consumers, pdbtool_check};
use super::native::{NativeFixture, native_fixtures};
use crate::jobs::Jobs;
use dia::{Baseline, Counts};
use ranges::{Procedure, RangeSet, SetKind};

/// The debug streams of OMAP output.
const OMAP_STREAMS: [DebugStream; 4] = [
    DebugStream::OmapToSource,
    DebugStream::OmapFromSource,
    DebugStream::SectionHeader,
    DebugStream::OriginalSectionHeader,
];

/// The paths of the consumer tools.
#[derive(Debug)]
struct Tools {
    /// `pdbtool.exe`.
    pdbtool: PathBuf,
    /// `DIA2Dump.exe`.
    dia2dump: PathBuf,
    /// `llvm-pdbutil.exe`.
    pdbutil: PathBuf,
}

/// The result of one range set on one PDB.
#[derive(Debug, Default)]
struct SetResult {
    /// The number of ranges.
    ranges: usize,
    /// The number of removed procedures.
    removed: usize,
    /// The streams that the plan changed.
    changed: Vec<u32>,
    /// The `DIA2Dump` counts.
    counts: Counts,
    /// The problems.
    problems: Vec<String>,
}

/// The results of one PDB.
#[derive(Debug, Default)]
struct FixtureResult {
    /// The number of selected procedures.
    procedures: usize,
    /// The number of removable procedures.
    removable: usize,
    /// The results of each set, or one problem for the PDB.
    sets: Vec<(SetKind, SetResult)>,
    /// The notes of the `DIA2Dump` check.
    notes: Vec<String>,
    /// A problem of the PDB before the sets.
    problem: Option<String>,
    /// The results of the clean check of each set.
    clean: Vec<(SetKind, clean::SetResult)>,
}

/// The source PDB, read once for all sets.
struct Source {
    /// The source bytes.
    bytes: Vec<u8>,
    /// The address tables.
    tables: AddressTables,
    /// The selected procedures.
    procedures: Vec<Procedure>,
    /// `true` for a PDB without type merging (feature `NOTM`).
    lazy: bool,
    /// The facts of the clean check.
    facts: clean::Facts,
}

/// Read the source PDB and select its procedures.
fn source(path: &Path) -> Result<Source, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let pdb = Pdb::parse(bytes.as_slice()).map_err(|error| format!("pdbrs: {error}"))?;
    let tables = pdb
        .address_tables()
        .map_err(|error| format!("pdbrs: {error}"))?
        .ok_or("the PDB has no section header stream")?;
    let locator = SymbolLocator::new(&pdb).map_err(|error| format!("locator: {error}"))?;
    let mut targets = BTreeSet::new();
    if let Some(globals) = pdb.globals().map_err(|error| format!("globals: {error}"))? {
        for symbol in globals.symbols() {
            let symbol = symbol.map_err(|error| format!("globals: {error}"))?;
            if let SymbolData::ProcedureReference(reference) = symbol.data()
                && let ModuleSymbolTarget::Position { module, offset } = reference.target
            {
                targets.insert((module, offset));
            }
        }
    }
    let mut candidates = Vec::new();
    for procedure in locator.procedures() {
        let procedure = procedure.map_err(|error| format!("procedures: {error}"))?;
        let Some(start) = procedure.start else {
            continue;
        };
        let chunks = locator
            .code_chunks(&procedure)
            .map_err(|error| format!("code chunks: {error}"))?;
        let named = targets.contains(&(Some(procedure.module), procedure.offset));
        candidates.push(Procedure {
            start: start.0,
            length: procedure.length,
            removable: !named && chunks.len() <= 1,
        });
    }
    let sections: Vec<_> = tables.sections.iter().collect();
    let procedures = ranges::select(&sections, candidates);
    let lazy = pdb
        .info()
        .features()
        .any(|feature| feature == Feature::NO_TYPE_MERGE);
    let facts = clean::Facts::read(&pdb, &locator, &tables)?;
    Ok(Source {
        bytes,
        tables,
        procedures,
        lazy,
        facts,
    })
}

/// Make the `ImageRemap` of a range set. Add the ranges in reverse order if `reverse` is
/// `true`.
fn image_remap(set: &RangeSet, reverse: bool) -> Result<ImageRemap, String> {
    let sections = SectionHeaderStream::from_records(&set.sections)
        .map_err(|error| format!("section headers: {error}"))?;
    let mut ranges = set.ranges.clone();
    if reverse {
        ranges.reverse();
    }
    ranges
        .iter()
        .try_fold(ImageRemap::new(sections), |remap, range| {
            remap.with_range(Rva(range.old), range.length, Rva(range.new))
        })
        .map_err(|error| format!("range: {error}"))
}

/// The output of one range set: the bytes and the streams that the plan changed and removed.
struct Output {
    /// The output bytes.
    bytes: Vec<u8>,
    /// The output streams that the plan changed.
    changed: BTreeSet<u32>,
    /// The source streams that the plan removed.
    dropped: Vec<u32>,
}

/// Remap the source with `remap` in OMAP mode.
fn remap_output(source: &Source, remap: &ImageRemap) -> Result<Output, String> {
    let error = |error: pdbrs::Error| format!("pdbrs: {error}");
    let pdb = Pdb::parse(source.bytes.as_slice()).map_err(error)?;
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .remap_addresses(remap, RemapMode::Omap)
        .map_err(error)?;
    let plan = editor.plan(EditOptions::default()).map_err(error)?;
    let changed = plan.report().changed().map(|id| id.0).collect();
    let dropped = plan.report().dropped().map(|id| id.0).collect();
    let bytes = plan.to_vec().map_err(error)?;
    Ok(Output {
        bytes,
        changed,
        dropped,
    })
}

/// Return the bytes of the debug stream `kind` of `pdb`, or `None` without such a stream.
fn debug_bytes(pdb: &Pdb<&[u8]>, kind: DebugStream) -> Option<Vec<u8>> {
    let id = pdb.debug_info().ok()??.debug_stream(kind)?;
    pdb.stream_bytes(id).ok()
}

/// Check the plan report and the address tables of an output. Return the problems.
///
/// The report must list the Info stream, the DBI stream and each OMAP output stream that has
/// no source stream or other bytes than its source stream.
fn check_output(source: &Source, remap: &ImageRemap, output: &Output) -> Vec<String> {
    let mut problems = Vec::new();
    let (pdb, input) = match (
        Pdb::parse(output.bytes.as_slice()),
        Pdb::parse(source.bytes.as_slice()),
    ) {
        (Ok(pdb), Ok(input)) => (pdb, input),
        (Err(error), _) | (_, Err(error)) => {
            return vec![format!("pdbrs cannot read the output: {error}")];
        }
    };
    let slots: Option<Vec<StreamId>> = pdb.debug_info().ok().flatten().and_then(|dbi| {
        OMAP_STREAMS
            .iter()
            .map(|&kind| dbi.debug_stream(kind))
            .collect()
    });
    let Some(slots) = slots else {
        return vec!["the output DBI stream does not name the 4 OMAP output streams".to_owned()];
    };
    let mut expected = BTreeSet::from([1, 3]);
    for (kind, id) in OMAP_STREAMS.into_iter().zip(&slots) {
        let before = debug_bytes(&input, kind);
        if before.is_none() || before != debug_bytes(&pdb, kind) {
            expected.insert(id.0);
        }
    }
    if output.changed != expected {
        problems.push(format!(
            "the report lists the changed streams {:?}, not {expected:?}",
            output.changed
        ));
    }
    if !output.dropped.is_empty() {
        problems.push(format!(
            "the report lists removed streams {:?}",
            output.dropped
        ));
    }
    match (pdb.address_tables(), remap.address_tables(&source.tables)) {
        (Ok(Some(read)), Ok(made)) if read == made => {}
        (Ok(_), Ok(_)) => {
            problems.push("the output address tables differ from the remap tables".to_owned());
        }
        (Err(error), _) | (_, Err(error)) => problems.push(format!("address tables: {error}")),
    }
    problems
}

/// The OMAP output of one set for the clean check: the bytes and the `DIA2Dump` outputs.
type OmapResult = Option<(Vec<u8>, Option<dia::OmapDumps>)>;

/// Remap one PDB with one range set and check the output. Return the result and the
/// written output.
fn check_set(
    tools: &Tools,
    source: &Source,
    baseline: &Baseline,
    set: &RangeSet,
    path: &Path,
) -> (SetResult, OmapResult) {
    let mut result = SetResult {
        ranges: set.ranges.len(),
        removed: set.removed(&source.procedures),
        ..SetResult::default()
    };
    let output = image_remap(set, false).and_then(|remap| {
        let output = remap_output(source, &remap)?;
        result.problems = check_output(source, &remap, &output);
        let again = remap_output(source, &image_remap(set, true)?)?;
        if again.bytes != output.bytes {
            result
                .problems
                .push("the ranges in reverse order give other output bytes".to_owned());
        }
        Ok(output)
    });
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            result.problems.push(error);
            return (result, None);
        }
    };
    result.changed = output.changed.iter().copied().collect();
    let _ = fs::remove_file(path);
    if let Err(error) = fs::write(path, &output.bytes) {
        result
            .problems
            .push(format!("cannot write {}: {error}", path.display()));
        return (result, None);
    }
    match pdbtool_check(&tools.pdbtool, path) {
        Ok(0) => {}
        Ok(errors) => result
            .problems
            .push(format!("pdbtool check: {errors} PDBs with errors")),
        Err(error) => result.problems.push(error),
    }
    let dumps = match dia::check(&tools.dia2dump, baseline, set, path) {
        Ok((counts, problems, dumps)) => {
            result.counts = counts;
            result.problems.extend(problems);
            Some(dumps)
        }
        Err(error) => {
            result.problems.push(error);
            None
        }
    };
    (result, Some((output.bytes, dumps)))
}

/// Remap one PDB with each range set and check the outputs.
fn check_fixture(tools: &Tools, fixture: &NativeFixture, work: &Path) -> FixtureResult {
    let failed = |problem: String| FixtureResult {
        problem: Some(problem),
        ..FixtureResult::default()
    };
    let source = match source(&fixture.pdb) {
        Ok(source) => source,
        Err(error) => return failed(error),
    };
    let baseline = match dia::baseline(&tools.dia2dump, &fixture.pdb, source.lazy) {
        Ok(baseline) => baseline,
        Err(error) => return failed(error),
    };
    let clean_baseline = match clean::baseline(tools, &fixture.pdb) {
        Ok(baseline) => baseline,
        Err(error) => return failed(error),
    };
    let directory = fixture.work_directory(work);
    if let Err(error) = fs::create_dir_all(&directory) {
        return failed(format!("cannot make {}: {error}", directory.display()));
    }
    let label = fixture.label();
    let sections: Vec<_> = source.tables.sections.iter().collect();
    let mut sets = Vec::new();
    let mut clean = Vec::new();
    for kind in SetKind::ALL {
        let path = directory.join(format!("{}.pdb", kind.name()));
        let clean_path = directory.join(format!("{}.clean.pdb", kind.name()));
        let started = Instant::now();
        let (result, clean_result) = match ranges::range_set(kind, &sections, &source.procedures) {
            Ok(set) => {
                let (result, omap) = check_set(tools, &source, &baseline, &set, &path);
                let omap_time = started.elapsed();
                let omap = omap.as_ref().map(|(bytes, dumps)| clean::OmapOutput {
                    bytes,
                    path: &path,
                    dumps: dumps.as_ref(),
                });
                let clean_result =
                    clean::check_set(tools, &source, &clean_baseline, &set, omap, &clean_path);
                eprintln!(
                    "verify-remap: {label}: {}: OMAP {:.1} s, clean {:.1} s",
                    kind.name(),
                    omap_time.as_secs_f64(),
                    started.elapsed().saturating_sub(omap_time).as_secs_f64()
                );
                (result, clean_result)
            }
            Err(error) => (
                SetResult {
                    problems: vec![error],
                    ..SetResult::default()
                },
                clean::SetResult::default(),
            ),
        };
        sets.push((kind, result));
        clean.push((kind, clean_result));
    }
    FixtureResult {
        procedures: source.procedures.len(),
        removable: source
            .procedures
            .iter()
            .filter(|procedure| procedure.removable)
            .count(),
        sets,
        notes: baseline.notes,
        problem: None,
        clean,
    }
}

/// Print the results of one PDB. Return the number of problems.
fn print_results(fixture: &NativeFixture, result: &FixtureResult) -> usize {
    if let Some(problem) = &result.problem {
        println!("{}: FAIL, {problem}", fixture.label());
        return 1;
    }
    println!(
        "{}: procedures {}, removable {}",
        fixture.label(),
        result.procedures,
        result.removable
    );
    for note in &result.notes {
        println!("  note: {note}");
    }
    let mut problems = 0;
    for (kind, set) in &result.sets {
        let status = if set.problems.is_empty() {
            "pass"
        } else {
            "FAIL"
        };
        println!(
            "  {}: {status}, ranges {}, removed procedures {}, changed streams {:?}, {}",
            kind.name(),
            set.ranges,
            set.removed,
            set.changed,
            set.counts
        );
        for problem in &set.problems {
            println!("    problem: {problem}");
        }
        problems += set.problems.len();
    }
    problems
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
    })
}

/// Run the command on the fixtures in `fixtures`. Return `true` if all checks pass.
///
/// # Errors
///
/// Returns an error if the manifest cannot be read, a tool is missing or the work directory
/// cannot be made.
pub fn verify_remap(workspace: &Path, fixtures: &Path, jobs: &Jobs) -> Result<bool, String> {
    let work = workspace.join("target").join("verify-remap");
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
    let mut totals = Counts::default();
    for (fixture, result) in list.iter().zip(&results) {
        problems += print_results(fixture, result);
        for (_, set) in &result.sets {
            totals.add(&set.counts);
        }
    }
    if totals.explained > 0 {
        println!("explained: {}: {} lines", dia::EXPLAINED, totals.explained);
    }
    let outputs: usize = results.iter().map(|result| result.sets.len()).sum();
    println!(
        "native PDBs: {}, outputs: {outputs}, {totals}, problems: {problems}",
        list.len()
    );
    println!("{}", clean::HEADER);
    let mut clean_totals = clean::Totals::default();
    for (fixture, result) in list.iter().zip(&results) {
        if result.problem.is_some() {
            println!("{}: skipped, the OMAP part failed", fixture.label());
        } else {
            clean_totals.print_fixture(&fixture.label(), &result.clean);
        }
    }
    clean_totals.print_summary();
    if problems + clean_totals.problems == 0 {
        println!("result: pass");
    } else {
        println!(
            "result: FAIL, problems: OMAP {problems}, clean {}",
            clean_totals.problems
        );
    }
    Ok(problems + clean_totals.problems == 0)
}
