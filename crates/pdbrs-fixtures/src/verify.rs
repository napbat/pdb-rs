//! The `verify-portable` and `detect` commands.
//!
//! `verify-portable` prints the canonical dump of each Portable PDB of the fixture manifest
//! twice: with pdbrs and with the .NET verifier (`System.Reflection.Metadata`). It writes both
//! dumps to `target/verify-portable/` below the workspace root and compares them line by line.
//! For an embedded Portable PDB, pdbrs decodes the `MPDB` payload file with `decode_embedded`,
//! and the verifier reads the image with `PEReader.ReadEmbeddedPortablePdbDebugDirectoryData`.
//!
//! `detect` opens each output of the fixture manifest with the format facade `pdbrs::Pdb`
//! and compares the detected format with the kind that the manifest records. The CLI metadata
//! of each image must give the "ordinary assembly metadata" error, not a Portable PDB.
//!
//! `verify-rebuild` (see [`rebuild`]) rebuilds each Portable PDB with the pdbrs writer and
//! compares the bytes, the ID and the verifier dump with the original.

mod dump;
mod edit;
mod rebuild;

pub use edit::verify_edit;
pub use rebuild::verify_rebuild;

use std::fs;
use std::path::{Path, PathBuf};

use pdbrs::error::PortableLocation;
use pdbrs::portable::embedded::decode_embedded;
use pdbrs::portable::metadata::MetadataStream;
use pdbrs::{ErrorKind, Format, Pdb};

use crate::classify::cli_metadata;
use crate::json::{self, Value};
use crate::tools::{self, verifier};

/// The decompression limit for `MPDB` payloads.
const EMBEDDED_LIMIT: usize = 64 * 1024 * 1024;

/// The number of differences that the comparison prints for each fixture.
const SHOWN_DIFFERENCES: usize = 10;

/// One output file of the fixture manifest.
struct Output {
    fixture: String,
    /// `true` if the producer is `dotnet`, so an image has CLI metadata.
    managed: bool,
    kind: String,
    path: PathBuf,
}

/// Read the outputs of the built fixtures from `manifest.json` in `fixtures`.
fn outputs(fixtures: &Path) -> Result<Vec<Output>, String> {
    let path = fixtures.join("manifest.json");
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let manifest = json::parse(&text)?;
    let Some(Value::Array(entries)) = manifest.get("fixtures") else {
        return Err(format!("{} has no `fixtures` array", path.display()));
    };
    let mut outputs = Vec::new();
    for entry in entries {
        let fixture = entry.get("name").and_then(Value::as_str).unwrap_or("?");
        let managed = entry.get("producer").and_then(Value::as_str) == Some("dotnet");
        let Some(Value::Array(files)) = entry.get("outputs") else {
            continue;
        };
        for file in files {
            let (Some(kind), Some(relative)) = (
                file.get("kind").and_then(Value::as_str),
                file.get("path").and_then(Value::as_str),
            ) else {
                continue;
            };
            outputs.push(Output {
                fixture: fixture.to_owned(),
                managed,
                kind: kind.to_owned(),
                path: fixtures.join(relative),
            });
        }
    }
    Ok(outputs)
}

/// The inputs of one comparison.
enum Input {
    /// A standalone Portable PDB. Both readers read this file.
    Standalone(PathBuf),
    /// An embedded Portable PDB. pdbrs reads the payload file, the verifier reads the image.
    Embedded { payload: PathBuf, image: PathBuf },
}

/// One comparison of the pdbrs and verifier dumps.
struct Comparison {
    fixture: String,
    input: Input,
}

/// Return the comparisons of the outputs: one for each `portable-pdb` output and one for each
/// `embedded-pdb-payload` output with the `embedded-in-image` output of its fixture.
fn comparisons(outputs: &[Output]) -> Result<Vec<Comparison>, String> {
    let mut comparisons = Vec::new();
    for output in outputs {
        let input = match output.kind.as_str() {
            "portable-pdb" => Input::Standalone(output.path.clone()),
            "embedded-pdb-payload" => {
                let image = outputs
                    .iter()
                    .find(|image| {
                        image.fixture == output.fixture && image.kind == "embedded-in-image"
                    })
                    .ok_or_else(|| {
                        format!(
                            "{}: the payload has no embedded-in-image output",
                            output.fixture
                        )
                    })?;
                Input::Embedded {
                    payload: output.path.clone(),
                    image: image.path.clone(),
                }
            }
            _ => continue,
        };
        comparisons.push(Comparison {
            fixture: output.fixture.clone(),
            input,
        });
    }
    Ok(comparisons)
}

/// Compare the pdbrs and verifier dumps of each Portable PDB fixture.
///
/// Return `false` if a dump fails or the dumps differ.
pub fn verify_portable(workspace: &Path, fixtures: &Path) -> Result<bool, String> {
    let assembly = tools::built_tool(workspace, verifier::NAME)?;
    let out = workspace.join("target").join("verify-portable");
    fs::create_dir_all(&out).map_err(|error| format!("cannot make {}: {error}", out.display()))?;
    let comparisons = comparisons(&outputs(fixtures)?)?;
    if comparisons.is_empty() {
        return Err(
            "the fixture manifest has no portable-pdb or embedded-pdb-payload output".to_owned(),
        );
    }
    let mut passed = true;
    for comparison in &comparisons {
        match compare(&assembly, comparison, &out) {
            Ok(true) => {}
            Ok(false) => passed = false,
            Err(error) => {
                println!("{}: error: {error}", comparison.fixture);
                passed = false;
            }
        }
    }
    println!("wrote the dumps to {}", out.display());
    Ok(passed)
}

/// Read a file of the fixture manifest.
fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

/// Dump one Portable PDB with both readers, write the dumps and print the differences.
fn compare(assembly: &Path, comparison: &Comparison, out: &Path) -> Result<bool, String> {
    let (bytes, mut command) = match &comparison.input {
        Input::Standalone(path) => (read(path)?, verifier::command(assembly, path)),
        Input::Embedded { payload, image } => (
            decode_embedded(&read(payload)?, EMBEDDED_LIMIT)
                .map_err(|error| format!("pdbrs: {error}"))?,
            verifier::embedded_command(assembly, image),
        ),
    };
    let ours = pdbrs::portable::Pdb::parse(&bytes)
        .and_then(|reader| dump::dump(&reader))
        .map_err(|error| format!("pdbrs: {error}"))?;
    let output = command
        .output()
        .map_err(|error| format!("cannot run the verifier: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "the verifier failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let theirs = String::from_utf8(output.stdout)
        .map_err(|error| format!("the verifier output is not UTF-8: {error}"))?;
    for (suffix, text) in [("pdbrs", &ours), ("srm", &theirs)] {
        let path = out.join(format!("{}.{suffix}.txt", comparison.fixture));
        fs::write(&path, text)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }

    let differences = differences(&ours, &theirs);
    println!(
        "{}: {} lines, {} different lines",
        comparison.fixture,
        theirs.lines().count(),
        differences.len()
    );
    for (line, ours, theirs) in differences.iter().take(SHOWN_DIFFERENCES) {
        println!("  line {line}:\n    pdbrs: {ours}\n    srm:   {theirs}");
    }
    Ok(differences.is_empty())
}

/// Return each line number (1-based) where the texts differ, with the two lines. A missing
/// line shows as `<none>`.
fn differences<'a>(ours: &'a str, theirs: &'a str) -> Vec<(usize, &'a str, &'a str)> {
    let ours: Vec<&str> = ours.lines().collect();
    let theirs: Vec<&str> = theirs.lines().collect();
    (0..ours.len().max(theirs.len()))
        .filter_map(|index| {
            let left = ours.get(index).copied().unwrap_or("<none>");
            let right = theirs.get(index).copied().unwrap_or("<none>");
            (left != right || ours.get(index).is_none() || theirs.get(index).is_none()).then_some((
                index + 1,
                left,
                right,
            ))
        })
        .collect()
}

/// Open each fixture output with the format facade and print the result table.
///
/// Return `false` if a result differs from the expected result.
pub fn detect(fixtures: &Path) -> Result<bool, String> {
    let outputs = outputs(fixtures)?;
    if outputs.is_empty() {
        return Err("the fixture manifest has no outputs".to_owned());
    }
    println!("| Fixture | Input | Expected | pdbrs::Pdb result | Pass |");
    println!("| --- | --- | --- | --- | --- |");
    let mut passed = true;
    for output in &outputs {
        let bytes = fs::read(&output.path)
            .map_err(|error| format!("cannot read {}: {error}", output.path.display()))?;
        let file = output
            .path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        for (input, expected, result) in checks(output, &file, &bytes) {
            let pass = expected == result;
            passed &= pass;
            println!(
                "| {} | {input} | {expected} | {result} | {} |",
                output.fixture,
                if pass { "yes" } else { "NO" }
            );
        }
    }
    Ok(passed)
}

/// The result for an image without CLI metadata.
const NO_METADATA: &str = "no CLI metadata in the image";

/// Return the inputs of one output with the expected and the actual result of each.
///
/// An image of a managed fixture must have CLI metadata, and the facade must reject it as
/// assembly metadata. A native image has no CLI metadata.
fn checks(output: &Output, file: &str, bytes: &[u8]) -> Vec<(String, &'static str, String)> {
    match output.kind.as_str() {
        "native-pdb" => vec![(file.to_owned(), "native", outcome(bytes))],
        "portable-pdb" => vec![(file.to_owned(), "portable", outcome(bytes))],
        "image" | "embedded-in-image" => {
            let metadata = cli_metadata(bytes).map_or_else(|| NO_METADATA.to_owned(), outcome);
            let expected = if output.managed {
                "rejected: assembly metadata"
            } else {
                NO_METADATA
            };
            vec![
                (
                    file.to_owned(),
                    "rejected: no PDB signature",
                    outcome(bytes),
                ),
                (format!("{file} CLI metadata"), expected, metadata),
            ]
        }
        "embedded-pdb-payload" => {
            let decoded = decode_embedded(bytes, EMBEDDED_LIMIT).map_or_else(
                |error| format!("decode error: {error}"),
                |pdb| outcome(&pdb),
            );
            vec![
                (file.to_owned(), "rejected: MPDB payload", outcome(bytes)),
                (format!("{file} decoded"), "portable", decoded),
            ]
        }
        _ => vec![(
            file.to_owned(),
            "a known output kind",
            format!("kind {}", output.kind),
        )],
    }
}

/// Open bytes with the format facade and describe the result.
fn outcome(bytes: &[u8]) -> String {
    match Pdb::parse(bytes) {
        Ok(pdb) => match pdb.format() {
            Format::Native => "native".to_owned(),
            Format::Portable => "portable".to_owned(),
            other => format!("{other:?}"),
        },
        Err(error) => {
            let stream = error.portable_location().and_then(PortableLocation::stream);
            match error.kind() {
                ErrorKind::MissingStream if stream == Some(MetadataStream::Pdb) => {
                    "rejected: assembly metadata".to_owned()
                }
                ErrorKind::InvalidFormat
                    if error
                        .context()
                        .is_some_and(|context| context.contains("MPDB")) =>
                {
                    "rejected: MPDB payload".to_owned()
                }
                ErrorKind::InvalidSignature => "rejected: no PDB signature".to_owned(),
                _ => format!("error: {error}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Input, Output, comparisons, differences};

    fn output(fixture: &str, kind: &str, path: &str) -> Output {
        Output {
            fixture: fixture.to_owned(),
            managed: true,
            kind: kind.to_owned(),
            path: PathBuf::from(path),
        }
    }

    #[test]
    fn differences_include_changed_and_missing_lines() {
        assert_eq!(differences("a\nb\n", "a\nb\n"), Vec::new());
        assert_eq!(
            differences("a\nb\n", "a\nc\nd\n"),
            vec![(2, "b", "c"), (3, "<none>", "d")]
        );
    }

    #[test]
    fn embedded_payloads_pair_with_the_image_of_their_fixture() {
        let outputs = [
            output("portable", "image", "p.dll"),
            output("portable", "portable-pdb", "p.pdb"),
            output("other", "embedded-in-image", "o.dll"),
            output("embedded", "embedded-in-image", "e.dll"),
            output("embedded", "embedded-pdb-payload", "e.mpdb"),
        ];
        let found = comparisons(&outputs).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].fixture, "portable");
        assert!(matches!(&found[0].input, Input::Standalone(path) if path == "p.pdb"));
        assert_eq!(found[1].fixture, "embedded");
        assert!(matches!(
            &found[1].input,
            Input::Embedded { payload, image } if payload == "e.mpdb" && image == "e.dll"
        ));
        // A payload without its image is an error.
        assert!(comparisons(&outputs[4..]).is_err());
    }
}
