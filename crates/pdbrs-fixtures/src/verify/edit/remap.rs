//! The remap cases of `verify-edit` (#46): `pdbrs::portable::PdbEditor::remap_source_paths`.
//!
//! The case remaps the document names of each Portable PDB input with the build root of the
//! fixtures (see [`BuildRoot`]): the root becomes `/_/`. It uses
//! `IdentityPolicy::ContentDerived`, writes `<fixture>.remap-source-paths.pdb` and its verifier
//! dump `<fixture>.remap-source-paths.srm.txt`, and checks:
//!
//! 1. The decoded dump of the .NET verifier (`System.Reflection.Metadata`) is the source dump
//!    with the new name in each `document` line. All other lines, also the sequence points, are
//!    equal, line for line.
//! 2. The streams whose bytes are different from the source are exactly the streams that the
//!    plan report does not give as [`StreamChange::Copied`].
//! 3. The same edit on a second editor gives the same bytes.
//! 4. A function that keeps every name gives a report with each stream as `Copied` and the
//!    source bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use pdbrs::io::ReadAt;
use pdbrs::portable::{EditOptions, IdentityPolicy, Pdb, StreamChange};

use super::cases::{changed_streams, list};
use super::expect::{self, field};
use super::tasks::quote;
use super::{pdbrs, same, source, write_and_dump};
use crate::jobs::Jobs;
use crate::verify::Comparison;
use crate::verify::rebuild::yes;
use crate::verify::source_paths::BuildRoot;

/// The name of the case in the log and in the output files.
const CASE: &str = "remap-source-paths";

/// The output of the remap of one PDB.
struct Remapped {
    /// The output bytes.
    bytes: Vec<u8>,
    /// The names of the streams that the plan report does not give as copied.
    reported: BTreeSet<String>,
    /// The number of names that got a new value.
    count: usize,
}

/// Remap the document names of `pdb` with `remap` and plan.
fn remapped<S: ReadAt>(
    pdb: &Pdb<S>,
    remap: impl FnMut(&[u8]) -> Option<Vec<u8>>,
) -> Result<Remapped, String> {
    let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
    let count = editor.remap_source_paths(remap).map_err(pdbrs)?;
    let plan = editor.plan(EditOptions::default()).map_err(pdbrs)?;
    let reported = plan
        .report()
        .streams()
        .iter()
        .filter(|(_, change)| *change != StreamChange::Copied)
        .map(|(stream, _)| stream.name().to_owned())
        .collect();
    let bytes = plan.to_vec().map_err(pdbrs)?;
    Ok(Remapped {
        bytes,
        reported,
        count,
    })
}

/// Return the new name of each document of `pdb` that the root changes, by row ID.
fn new_names<S: ReadAt>(pdb: &Pdb<S>, root: &BuildRoot) -> Result<BTreeMap<u32, Vec<u8>>, String> {
    let mut names = BTreeMap::new();
    for document in pdb.documents() {
        let (id, document) = document.map_err(pdbrs)?;
        if let Some(new) = root.remap(&document.name) {
            names.insert(id.0, new);
        }
    }
    Ok(names)
}

/// Return the source lines with the new name in each changed `document` line.
fn expected(lines: &[String], names: &BTreeMap<u32, Vec<u8>>) -> Result<Vec<String>, String> {
    let mut result = lines.to_vec();
    for line in &mut result {
        if !line.starts_with("document ") {
            continue;
        }
        let id: u32 = field(line, "id")
            .and_then(|id| id.parse().ok())
            .ok_or_else(|| format!("no document ID in `{line}`"))?;
        let Some(new) = names.get(&id) else {
            continue;
        };
        let start = line
            .find(" name=\"")
            .ok_or_else(|| format!("no name in `{line}`"))?
            + " name=".len();
        let end = start
            + 1
            + line[start + 1..]
                .find('"')
                .ok_or_else(|| format!("no end of the name in `{line}`"))?
            + 1;
        *line = format!("{}{}{}", &line[..start], quote(new), &line[end..]);
    }
    Ok(result)
}

/// Run the case on one Portable PDB. Write the results to `log`. Return the number of
/// problems.
fn fixture(
    assembly: &Path,
    root: &BuildRoot,
    comparison: &Comparison,
    out: &Path,
    log: &mut String,
) -> Result<usize, String> {
    let fixture = &comparison.fixture;
    let (original, theirs) = source(assembly, comparison)?;
    let pdb = Pdb::parse(&original).map_err(pdbrs)?;
    let _ = writeln!(log, "{fixture}:");
    let kept = remapped(&pdb, |_| None)?;
    let keeps = kept.count == 0 && kept.reported.is_empty() && kept.bytes == original;
    let _ = writeln!(
        log,
        "  {CASE}: a function that keeps every name gives the source bytes: {}",
        yes(keeps)
    );
    let names = new_names(&pdb, root)?;
    let output = remapped(&pdb, |name| root.remap(name))?;
    let dump = write_and_dump(assembly, out, &format!("{fixture}.{CASE}"), &output.bytes)?;
    let expected = expected(&expect::decoded(&theirs), &names)?;
    let actual = expect::decoded(&dump);
    let points = actual
        .iter()
        .filter(|line| line.starts_with("sequence-point "))
        .count();
    let _ = writeln!(
        log,
        "  {CASE}: {} document names, {} new names, {points} sequence points",
        names.len(),
        output.count
    );
    let records = same(log, &format!("{CASE}: decoded dump"), &expected, &actual);
    let changed = changed_streams(&original, &theirs, &output.bytes, &dump)?;
    let agree = changed == output.reported;
    let _ = writeln!(
        log,
        "  {CASE}: streams with other bytes: {}; the report gives the same streams: {}",
        list(&changed),
        yes(agree)
    );
    if !agree {
        let _ = writeln!(log, "    the report gives: {}", list(&output.reported));
    }
    let again = remapped(&pdb, |name| root.remap(name))?.bytes == output.bytes;
    let _ = writeln!(
        log,
        "  {CASE}: the same edit again gives the same bytes: {}",
        yes(again)
    );
    // Each input has documents below the build root.
    let applied = output.count != 0;
    Ok(usize::from(
        !(keeps && records && agree && again && applied),
    ))
}

/// Run the case on each comparison and print the section `remap cases:`.
///
/// Return `false` if a check fails or gives an error.
pub(super) fn run(
    jobs: &Jobs,
    assembly: &Path,
    fixtures: &Path,
    comparisons: &[Comparison],
    out: &Path,
) -> bool {
    println!("remap cases:");
    let root = match BuildRoot::from_manifest(fixtures) {
        Ok(root) => root,
        Err(error) => {
            println!("problem: {error}");
            return false;
        }
    };
    println!("build root: {} -> /_/", root.text());
    let results = jobs.map(comparisons, |comparison| {
        let mut log = String::new();
        let result = fixture(assembly, &root, comparison, out, &mut log);
        (log, result)
    });
    let mut problems = 0;
    for (comparison, (log, result)) in comparisons.iter().zip(results) {
        print!("{log}");
        match result {
            Ok(count) => problems += count,
            Err(error) => {
                println!("{}: error: {error}", comparison.fixture);
                problems += 1;
            }
        }
    }
    println!(
        "remap section: portable PDBs: {}, problems: {problems}",
        comparisons.len()
    );
    problems == 0
}
