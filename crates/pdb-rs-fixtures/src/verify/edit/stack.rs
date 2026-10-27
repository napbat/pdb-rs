//! Run a fixture program in its `stack` mode with other PDBs.
//!
//! The program prints `System.Diagnostics.StackTrace(true)`. The runtime reads the PDB next to
//! the assembly and checks its PDB ID against the `CodeView` entry, so each PDB uses the
//! original PDB ID.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;

use super::super::Output;
use super::super::rebuild::yes;
use super::LINE_SHIFT;

/// The text that comes before a line number in a stack frame.
const LINE: &str = ":line ";

/// The PDBs of one fixture and the document names of the original PDB.
pub(super) struct Pdbs<'a> {
    /// The original PDB.
    pub(super) original: &'a [u8],
    /// The PDB of the new editor with new heaps.
    pub(super) built: &'a [u8],
    /// The PDB with the moved sequence points.
    pub(super) moved: &'a [u8],
    /// The document names of the original PDB.
    pub(super) documents: Vec<String>,
}

/// Return the lines of the trace with each line number plus [`LINE_SHIFT`] in the frames of
/// the documents. Frames of other assemblies (for example `FSharp.Core`) do not change.
fn shifted(trace: &str, documents: &[String]) -> Result<Vec<String>, String> {
    trace
        .lines()
        .map(|line| {
            let Some(position) = line.rfind(LINE) else {
                return Ok(line.to_owned());
            };
            let file = line[..position]
                .rsplit_once(" in ")
                .map_or("", |(_, file)| file);
            if !documents.iter().any(|document| document == file) {
                return Ok(line.to_owned());
            }
            let start = position + LINE.len();
            let number: u32 = line[start..]
                .trim()
                .parse()
                .map_err(|error| format!("bad line number in `{line}`: {error}"))?;
            Ok(format!("{}{}", &line[..start], number + LINE_SHIFT))
        })
        .collect()
}

/// Copy the files of `source` to the new directory `target`.
fn copy_directory(source: &Path, target: &Path) -> Result<(), String> {
    drop(fs::remove_dir_all(target));
    fs::create_dir_all(target)
        .map_err(|error| format!("cannot make {}: {error}", target.display()))?;
    let entries = fs::read_dir(source)
        .map_err(|error| format!("cannot read {}: {error}", source.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read {}: {error}", source.display()))?;
        if entry.path().is_file() {
            fs::copy(entry.path(), target.join(entry.file_name()))
                .map_err(|error| format!("cannot copy {}: {error}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// Run the program of `fixture` with each PDB, compare the traces and write the results to
/// `log`. Return `true` if the built PDB gives the original trace and the moved PDB gives the
/// moved trace.
pub(super) fn check(
    fixture: &str,
    outputs: &[Output],
    pdb: &Path,
    out: &Path,
    pdbs: &Pdbs<'_>,
    log: &mut String,
) -> Result<bool, String> {
    let image = outputs
        .iter()
        .find(|output| output.fixture == fixture && output.kind == "image")
        .ok_or_else(|| format!("{fixture}: no image output"))?;
    let (Some(directory), Some(assembly), Some(pdb_name)) =
        (image.path.parent(), image.path.file_name(), pdb.file_name())
    else {
        return Err(format!("{fixture}: the image path has no directory"));
    };
    let mut traces = Vec::new();
    let variants = [
        ("original", pdbs.original),
        ("built", pdbs.built),
        ("moved", pdbs.moved),
    ];
    for (variant, bytes) in variants {
        let target = out.join(format!("{fixture}-{variant}"));
        copy_directory(directory, &target)?;
        fs::write(target.join(pdb_name), bytes)
            .map_err(|error| format!("cannot write the PDB: {error}"))?;
        let output = Command::new("dotnet")
            .arg(target.join(assembly))
            .arg("stack")
            .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
            .output()
            .map_err(|error| format!("cannot run the program: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "the {variant} program failed with {}",
                output.status
            ));
        }
        let trace = String::from_utf8(output.stdout)
            .map_err(|error| format!("the trace is not UTF-8: {error}"))?;
        let path = out.join(format!("{fixture}-{variant}.stack.txt"));
        fs::write(&path, &trace)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        traces.push(trace);
    }
    let frames = traces[0].lines().filter(|line| line.contains(LINE)).count();
    let built_same = traces[1] == traces[0];
    let moved_lines: Vec<String> = traces[2].lines().map(str::to_owned).collect();
    let expected = shifted(&traces[0], &pdbs.documents)?;
    let moved_frames = expected
        .iter()
        .zip(traces[0].lines())
        .filter(|(expected, original)| expected.as_str() != *original)
        .count();
    let moved_same = moved_lines == expected && moved_frames > 0;
    let _ = writeln!(
        log,
        "  stack trace: {frames} frames with a line number, {moved_frames} in the PDB"
    );
    let _ = writeln!(
        log,
        "  stack trace with the built PDB is the same: {}",
        yes(built_same)
    );
    let _ = writeln!(
        log,
        "  stack trace with the moved PDB has each line of the PDB plus {LINE_SHIFT}: {}",
        yes(moved_same)
    );
    Ok(built_same && moved_same)
}
