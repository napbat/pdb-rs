//! The `verify-native-build` command.
//!
//! The command rebuilds each native PDB of the fixture manifest with the pdbrs builder and
//! compares the original and the rebuilt PDB through the consumer tools:
//!
//! 1. `Pdb::contents`, `PdbContents::to_builder` with `IdentityPolicy::Preserve` and
//!    `PdbBuilder::plan` with `PdbContents::build_options` give the rebuilt PDB. A second plan
//!    must give the same bytes. The rebuilt PDB keeps the file name of the original.
//! 2. `pdbtool check` of the rebuilt PDB must give 0 errors. `pdbtool compare` gives the
//!    stream counts.
//! 3. The normalized `llvm-pdbutil dump --all` outputs (see [`normalize_pdbutil`]) and the
//!    masked `DIA2Dump -all` outputs must have only explained differences (see
//!    [`diff`](super::diff)).
//! 4. `DIA2Dump -sym <rva>`, `-lines <rva>` and `-type <name>` must give the same masked output
//!    for each address and type name of the sample (see [`sample`](super::sample)).
//! 5. `llvm-symbolizer` must give the same frames for each address of the sample with each PDB
//!    (see [`symbolize`]).
//!
//! The command writes the rebuilt PDBs, the dumps and the unexplained differences to
//! `target/verify-native-build/<fixture>/<PDB file stem>/`. A missing tool makes its checks
//! fail; the other checks still run.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use pdbrs::native::IdentityPolicy;
use pdbrs::native::Pdb;

use super::consumers::{
    Consumers, Run, StreamDifferences, dia2dump, normalize_pdbutil, pdbtool_check, pdbtool_compare,
    pdbutil_dump,
};
use super::diff::{Comparison, Dump, Exits};
use super::sample::{Sample, separated_addresses};
use super::symbolize::{self, Frames};
use super::{NativeFixture, native_fixtures};
use crate::jobs::Jobs;
use crate::verify::SHOWN_DIFFERENCES;

/// Rebuild each native PDB fixture and compare it with the original through the consumer
/// tools. Print one summary line for each PDB.
///
/// Return `false` if a rebuild fails, a check cannot run or a check finds an unexplained
/// difference.
pub fn verify_native_build(workspace: &Path, fixtures: &Path, jobs: &Jobs) -> Result<bool, String> {
    let work = workspace.join("target").join("verify-native-build");
    fs::create_dir_all(&work)
        .map_err(|error| format!("cannot make {}: {error}", work.display()))?;
    let (list, skipped) = native_fixtures(fixtures)?;
    if list.is_empty() {
        return Err(format!(
            "{} has no native PDB outputs",
            fixtures.join("manifest.json").display()
        ));
    }
    let consumers = Consumers::locate(workspace, fixtures);
    for (name, reason) in consumers.missing() {
        println!("tool {name} is missing, its checks fail: {reason}");
    }
    for entry in &skipped {
        println!("{}: skipped: {}", entry.fixture, entry.reason);
    }
    let reports = jobs.map(&list, |fixture| {
        check(fixture, &consumers, &fixture.work_directory(&work), jobs)
    });
    let mut passed = 0;
    for (fixture, report) in list.iter().zip(&reports) {
        let pass = report.passed();
        println!(
            "{}: {}; {}",
            fixture.label(),
            if pass { "pass" } else { "FAIL" },
            report.summary()
        );
        for line in report.details() {
            println!("  {line}");
        }
        passed += usize::from(pass);
    }
    println!(
        "{passed} of {} native PDBs pass, {} fixtures skipped; the results are in {}",
        list.len(),
        skipped.len(),
        work.display()
    );
    Ok(passed == list.len())
}

/// The result of the rebuild.
struct Rebuilt {
    /// The path of the rebuilt PDB.
    path: PathBuf,
    /// `true` if two plans give the same bytes.
    deterministic: bool,
    /// The stream count of the original.
    streams_before: u32,
    /// The stream count of the rebuilt PDB.
    streams_after: u32,
    /// The source streams that the rebuild drops.
    unclaimed: Vec<u32>,
}

/// The result of the two dumps of one tool.
struct DumpCheck {
    /// The exit codes of the two runs.
    exits: Exits,
    /// The classified differences.
    comparison: Comparison,
}

impl DumpCheck {
    /// Return `true` if all differences are explained.
    fn passed(&self) -> bool {
        self.exits.explained() && self.comparison.unexplained.is_empty()
    }

    /// Describe the result.
    fn describe(&self) -> String {
        format!("{}{}", self.comparison.describe(), self.exits.describe())
    }
}

/// The mismatch counts of the DIA queries.
#[derive(Default)]
struct Queries {
    /// The `-sym` outputs that differ.
    sym: usize,
    /// The `-lines` outputs that differ.
    lines: usize,
    /// The `-type` outputs that differ.
    types: usize,
}

/// The symbolizer result.
struct Symbolized {
    /// The addresses whose frames differ.
    mismatches: usize,
    /// The number of addresses.
    total: usize,
    /// The addresses without a function name with the original PDB.
    unresolved: usize,
}

/// The results of all checks of one PDB.
struct Report {
    rebuilt: Result<Rebuilt, String>,
    pdbtool: Result<u32, String>,
    compare: Result<StreamDifferences, String>,
    pdbutil: Result<DumpCheck, String>,
    dia_all: Result<DumpCheck, String>,
    sample: Result<Sample, String>,
    queries: Result<Queries, String>,
    /// `None` if the fixture has no image.
    symbolizer: Option<Result<Symbolized, String>>,
}

/// The text of a missing check result.
fn not_run(reason: &str) -> String {
    format!("not run: {reason}")
}

/// The result of a check that needs the rebuilt PDB, after a failed rebuild.
fn not_built<T>() -> Result<T, String> {
    Err(not_run("no rebuilt PDB"))
}

impl Report {
    /// A report for a failed rebuild.
    fn failed(error: String) -> Self {
        Self {
            rebuilt: Err(error),
            pdbtool: not_built(),
            compare: not_built(),
            pdbutil: not_built(),
            dia_all: not_built(),
            sample: not_built(),
            queries: not_built(),
            symbolizer: None,
        }
    }

    /// Return `true` if every check passes.
    fn passed(&self) -> bool {
        let rebuilt = self
            .rebuilt
            .as_ref()
            .is_ok_and(|rebuilt| rebuilt.deterministic);
        let queries = self
            .queries
            .as_ref()
            .is_ok_and(|queries| queries.sym + queries.lines + queries.types == 0);
        let symbolizer = self
            .symbolizer
            .as_ref()
            .is_none_or(|result| result.as_ref().is_ok_and(|result| result.mismatches == 0));
        rebuilt
            && self.pdbtool == Ok(0)
            && self.compare.is_ok()
            && self.pdbutil.as_ref().is_ok_and(DumpCheck::passed)
            && self.dia_all.as_ref().is_ok_and(DumpCheck::passed)
            && self.sample.is_ok()
            && queries
            && symbolizer
    }

    /// The summary line of the checks, without the label and the result.
    fn summary(&self) -> String {
        let mut parts = Vec::new();
        parts.push(match &self.rebuilt {
            Ok(rebuilt) => format!(
                "rebuild {}, streams {} -> {}{}",
                if rebuilt.deterministic {
                    "deterministic"
                } else {
                    "NOT DETERMINISTIC"
                },
                rebuilt.streams_before,
                rebuilt.streams_after,
                match &self.compare {
                    Ok(differences) => format!(" ({} differ)", differences.streams.len()),
                    Err(error) => format!(" (pdbtool compare {error})"),
                }
            ),
            Err(error) => format!("rebuild FAILED: {error}"),
        });
        if self.rebuilt.is_err() {
            return parts.join("; ");
        }
        parts.push(match &self.pdbtool {
            Ok(errors) => format!("pdbtool check {errors} errors"),
            Err(error) => format!("pdbtool check {error}"),
        });
        parts.push(format!("llvm-pdbutil {}", describe(&self.pdbutil)));
        parts.push(format!("DIA -all {}", describe(&self.dia_all)));
        parts.push(match &self.sample {
            Ok(sample) => format!("sample {}, {} types", sample.describe(), sample.types.len()),
            Err(error) => format!("sample {error}"),
        });
        parts.push(match &self.queries {
            Ok(queries) => format!(
                "mismatches -sym {}, -lines {}, -type {}",
                queries.sym, queries.lines, queries.types
            ),
            Err(error) => format!("DIA queries {error}"),
        });
        parts.push(match &self.symbolizer {
            None => "symbolizer: no image".to_owned(),
            Some(Ok(result)) => format!(
                "symbolizer {} of {} differ ({} unresolved)",
                result.mismatches, result.total, result.unresolved
            ),
            Some(Err(error)) => format!("symbolizer {error}"),
        });
        if let Ok(rebuilt) = &self.rebuilt {
            parts.push(if rebuilt.unclaimed.is_empty() {
                "unclaimed streams none".to_owned()
            } else {
                format!("unclaimed streams {:?}", rebuilt.unclaimed)
            });
        }
        parts.join("; ")
    }

    /// The first unexplained differences of the dumps.
    fn details(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for (name, check) in [("llvm-pdbutil", &self.pdbutil), ("DIA -all", &self.dia_all)] {
            if let Ok(check) = check {
                lines.extend(
                    check
                        .comparison
                        .unexplained
                        .iter()
                        .take(SHOWN_DIFFERENCES)
                        .map(|line| format!("{name} {line}")),
                );
            }
        }
        lines
    }
}

/// Describe a dump check.
fn describe(check: &Result<DumpCheck, String>) -> String {
    match check {
        Ok(check) => check.describe(),
        Err(error) => error.clone(),
    }
}

/// Return the path of a found tool, or the result text of a check without the tool.
fn available(tool: &Result<PathBuf, String>) -> Result<&Path, String> {
    tool.as_ref()
        .map(PathBuf::as_path)
        .map_err(|_| not_run("tool missing"))
}

/// Rebuild one PDB and run all checks.
fn check(fixture: &NativeFixture, consumers: &Consumers, directory: &Path, jobs: &Jobs) -> Report {
    if directory.exists()
        && let Err(error) = fs::remove_dir_all(directory)
    {
        return Report::failed(format!("cannot remove {}: {error}", directory.display()));
    }
    let rebuilt = match rebuild(
        &fixture.pdb,
        &directory.join("rebuilt"),
        &fixture.file_name(),
    ) {
        Ok(rebuilt) => rebuilt,
        Err(error) => return Report::failed(error),
    };
    let original = fixture.pdb.as_path();
    let path = rebuilt.path.as_path();
    let ((pdbtool, compare), (pdbutil, dia_all)) = jobs.join(
        || {
            jobs.join(
                || available(&consumers.pdbtool).and_then(|tool| pdbtool_check(tool, path)),
                || {
                    available(&consumers.pdbtool)
                        .and_then(|tool| pdbtool_compare(tool, original, path))
                },
            )
        },
        || {
            jobs.join(
                || {
                    available(&consumers.pdbutil)
                        .and_then(|tool| both(tool, original, path, pdbutil_dump, jobs))
                },
                || {
                    available(&consumers.dia2dump).and_then(|tool| {
                        both(
                            tool,
                            original,
                            path,
                            |tool, pdb| dia2dump(tool, ["-all"], pdb),
                            jobs,
                        )
                    })
                },
            )
        },
    );
    let pdbutil = pdbutil.and_then(|runs| {
        let runs = runs.map(|run| Run {
            code: run.code,
            text: normalize_pdbutil(&run.text),
        });
        compare_dumps(Dump::Pdbutil, &runs, directory, "llvm")
    });
    let (sample, dia_all) = match dia_all {
        Ok(runs) => (
            separated_addresses(original)
                .map(|separated| Sample::select(&runs[0].text, &separated)),
            compare_dumps(Dump::Dia, &runs, directory, "dia-all"),
        ),
        Err(error) => (Err(error.clone()), Err(error)),
    };
    let queries = match (&sample, available(&consumers.dia2dump)) {
        (Ok(sample), Ok(tool)) => query(tool, original, path, sample, directory, jobs),
        (Err(error), _) => Err(error.clone()),
        (_, Err(error)) => Err(error),
    };
    let symbolizer =
        fixture
            .image
            .as_deref()
            .map(|image| match (&sample, available(&consumers.symbolizer)) {
                (Ok(sample), Ok(tool)) => {
                    compare_frames(tool, image, original, path, sample, directory, jobs)
                }
                (Err(error), _) => Err(error.clone()),
                (_, Err(error)) => Err(error),
            });
    Report {
        rebuilt: Ok(rebuilt),
        pdbtool,
        compare,
        pdbutil,
        dia_all,
        sample,
        queries,
        symbolizer,
    }
}

/// Rebuild the PDB at `original` and write it to `directory/<file name>`.
fn rebuild(original: &Path, directory: &Path, file_name: &str) -> Result<Rebuilt, String> {
    let bytes = fs::read(original)
        .map_err(|error| format!("cannot read {}: {error}", original.display()))?;
    let pdbrs = |error: pdbrs::Error| format!("pdbrs: {error}");
    let pdb = Pdb::parse(&bytes).map_err(pdbrs)?;
    let contents = pdb.contents().map_err(pdbrs)?;
    let options = contents.build_options();
    let plan = || -> Result<(Vec<u8>, u32), pdbrs::Error> {
        let builder = contents.to_builder(IdentityPolicy::Preserve)?;
        let plan = builder.plan(options)?;
        Ok((plan.to_vec()?, plan.stream_count()))
    };
    let (first, streams_after) = plan().map_err(pdbrs)?;
    let (second, _) = plan().map_err(pdbrs)?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("cannot make {}: {error}", directory.display()))?;
    let path = directory.join(file_name);
    fs::write(&path, &first)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    Ok(Rebuilt {
        path,
        deterministic: first == second,
        streams_before: pdb.container().stream_count(),
        streams_after,
        unclaimed: contents
            .unclaimed_streams()
            .iter()
            .map(|stream| stream.0)
            .collect(),
    })
}

/// Run a tool on the original and the rebuilt PDB, on two workers if a worker is free.
fn both(
    tool: &Path,
    original: &Path,
    rebuilt: &Path,
    run: impl Fn(&Path, &Path) -> Result<Run, String> + Sync,
    jobs: &Jobs,
) -> Result<[Run; 2], String> {
    let (first, second) = jobs.join(|| run(tool, original), || run(tool, rebuilt));
    Ok([first?, second?])
}

/// Compare the dumps of the original and the rebuilt PDB. Write both dumps and the
/// unexplained differences to `directory` with the file name prefix `name`.
fn compare_dumps(
    dump: Dump,
    runs: &[Run; 2],
    directory: &Path,
    name: &str,
) -> Result<DumpCheck, String> {
    let comparison = Comparison::new(dump, &runs[0].text, &runs[1].text);
    let mut differences = comparison.unexplained.join("\n");
    differences.push('\n');
    for (suffix, text) in [
        ("original.txt", runs[0].text.as_str()),
        ("rebuilt.txt", runs[1].text.as_str()),
        ("diff", differences.as_str()),
    ] {
        write(&directory.join(format!("{name}.{suffix}")), text)?;
    }
    Ok(DumpCheck {
        exits: Exits::new(runs[0].code, runs[1].code),
        comparison,
    })
}

/// Write a text file.
fn write(path: &Path, text: &str) -> Result<(), String> {
    fs::write(path, text).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

/// One DIA query.
enum Query<'a> {
    /// `-sym <rva>` or `-lines <rva>`.
    Address(&'static str, u32),
    /// `-type <name>`.
    Type(&'a str),
}

/// Run the DIA queries of the sample on both PDBs on the workers and count the mismatches.
/// Write the mismatches to `queries.diff`.
fn query(
    tool: &Path,
    original: &Path,
    rebuilt: &Path,
    sample: &Sample,
    directory: &Path,
    jobs: &Jobs,
) -> Result<Queries, String> {
    let mut list: Vec<Query<'_>> = Vec::new();
    for &rva in &sample.rvas {
        list.push(Query::Address("-sym", rva));
        list.push(Query::Address("-lines", rva));
    }
    list.extend(sample.types.iter().map(|name| Query::Type(name)));
    let results = jobs.map(&list, |query| {
        let (option, argument) = match query {
            Query::Address(option, rva) => (*option, format!("0x{rva:x}")),
            Query::Type(name) => ("-type", (*name).to_owned()),
        };
        let runs = both(
            tool,
            original,
            rebuilt,
            |tool, pdb| dia2dump(tool, [option, argument.as_str()], pdb),
            jobs,
        )?;
        Ok::<_, String>((runs[0] != runs[1]).then(|| {
            format!(
                "== {option} {argument}\n-- original (exit {:?})\n{}-- rebuilt (exit {:?})\n{}",
                runs[0].code, runs[0].text, runs[1].code, runs[1].text
            )
        }))
    });
    let mut queries = Queries::default();
    let mut differences = String::new();
    for (query, result) in list.iter().zip(results) {
        if let Some(difference) = result? {
            match query {
                Query::Address("-sym", _) => queries.sym += 1,
                Query::Address(..) => queries.lines += 1,
                Query::Type(_) => queries.types += 1,
            }
            differences.push_str(&difference);
        }
    }
    write(&directory.join("queries.diff"), &differences)?;
    Ok(queries)
}

/// Symbolize the sample with each PDB next to a patched copy of the image and compare the
/// frames. Write the different frames to `symbolizer.diff`.
fn compare_frames(
    tool: &Path,
    image: &Path,
    original: &Path,
    rebuilt: &Path,
    sample: &Sample,
    directory: &Path,
    jobs: &Jobs,
) -> Result<Symbolized, String> {
    let frames = |name: &str, pdb: &Path| {
        let patched = symbolize::prepare(image, pdb, &directory.join(name))?;
        symbolize::symbolize(tool, &patched, &sample.rvas)
    };
    let (original, rebuilt) = jobs.join(
        || frames("symbolize-original", original),
        || frames("symbolize-rebuilt", rebuilt),
    );
    let [original, rebuilt]: [Frames; 2] = [original?, rebuilt?];
    let mut differences = String::new();
    for ((rva, left), right) in sample
        .rvas
        .iter()
        .zip(&original.addresses)
        .zip(&rebuilt.addresses)
    {
        if left != right {
            let _ = write!(
                differences,
                "== 0x{rva:x}\n-- original\n{left}-- rebuilt\n{right}"
            );
        }
    }
    write(&directory.join("symbolizer.diff"), &differences)?;
    Ok(Symbolized {
        mismatches: original.mismatches(&rebuilt),
        total: sample.rvas.len(),
        unresolved: original.unresolved(),
    })
}
