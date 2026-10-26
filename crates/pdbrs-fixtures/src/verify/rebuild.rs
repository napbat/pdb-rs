//! The `verify-rebuild` command.
//!
//! The command reads each Portable PDB of the fixture manifest (a standalone file or the
//! decoded `MPDB` payload of an image) and checks the pdbrs writer:
//!
//! 1. `Pdb::copy_exact` gives the same bytes.
//! 2. `Pdb::to_parts` and a plan with the content-derived ID give the same bytes and the same
//!    PDB ID. The fixtures are deterministic Roslyn and F# builds, so `System.Reflection.Metadata`
//!    computed the original ID with the same algorithm.
//! 3. A second plan gives the same bytes, a plan with the original ID as an explicit ID gives
//!    the same bytes, and the `MPDB` payload of the plan decodes to the same bytes.
//! 4. The verifier dump of the rebuilt file in `target/verify-rebuild/` is the same as the
//!    verifier dump of the original.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use pdbrs::io::copy_exact;
use pdbrs::portable::embedded::decode_embedded;
use pdbrs::portable::write::ChecksumAlgorithm;
use pdbrs::portable::{IdentityPolicy, Pdb};

use super::{
    Comparison, EMBEDDED_LIMIT, Input, SHOWN_DIFFERENCES, comparisons, differences, outputs, read,
};
use crate::tools::{self, verifier};

/// Rebuild each Portable PDB fixture and compare it with the original.
///
/// Return `false` if a check fails.
pub fn verify_rebuild(workspace: &Path, fixtures: &Path) -> Result<bool, String> {
    let assembly = tools::built_tool(workspace, verifier::NAME)?;
    let out = workspace.join("target").join("verify-rebuild");
    fs::create_dir_all(&out).map_err(|error| format!("cannot make {}: {error}", out.display()))?;
    let comparisons = comparisons(&outputs(fixtures)?)?;
    if comparisons.is_empty() {
        return Err(
            "the fixture manifest has no portable-pdb or embedded-pdb-payload output".to_owned(),
        );
    }
    let mut passed = true;
    for comparison in &comparisons {
        match check(&assembly, comparison, &out) {
            Ok(true) => {}
            Ok(false) => passed = false,
            Err(error) => {
                println!("{}: error: {error}", comparison.fixture);
                passed = false;
            }
        }
    }
    println!("wrote the rebuilt files to {}", out.display());
    Ok(passed)
}

/// Return the bytes as lowercase hexadecimal text.
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// Return `yes` or `NO`.
pub(super) const fn yes(value: bool) -> &'static str {
    if value { "yes" } else { "NO" }
}

/// Rebuild one Portable PDB and print the result of each check.
fn check(assembly: &Path, comparison: &Comparison, out: &Path) -> Result<bool, String> {
    let pdbrs = |error: pdbrs::Error| format!("pdbrs: {error}");
    let (original, mut command) = match &comparison.input {
        Input::Standalone(path) => (read(path)?, verifier::command(assembly, path)),
        Input::Embedded { payload, image } => (
            decode_embedded(&read(payload)?, EMBEDDED_LIMIT).map_err(pdbrs)?,
            verifier::embedded_command(assembly, image),
        ),
    };
    let pdb = Pdb::parse(&original).map_err(pdbrs)?;
    let mut copy = Vec::new();
    copy_exact(pdb.source(), &mut copy).map_err(pdbrs)?;
    let plan = pdb
        .to_parts()
        .and_then(|parts| parts.plan(IdentityPolicy::ContentDerived))
        .map_err(pdbrs)?;
    let rebuilt = plan.to_vec().map_err(pdbrs)?;
    let again = pdb
        .to_parts()
        .and_then(|parts| parts.plan(IdentityPolicy::ContentDerived))
        .and_then(|plan| plan.to_vec())
        .map_err(pdbrs)?;
    let explicit = pdb
        .to_parts()
        .and_then(|parts| parts.plan(IdentityPolicy::Explicit(pdb.identity())))
        .and_then(|plan| plan.to_vec())
        .map_err(pdbrs)?;
    let embedded = plan
        .to_embedded()
        .and_then(|payload| decode_embedded(&payload, EMBEDDED_LIMIT))
        .map_err(pdbrs)?;
    let path = out.join(format!("{}.pdb", comparison.fixture));
    fs::write(&path, &rebuilt)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;

    let checks = [
        ("exact copy", copy == original),
        ("rebuild has the original bytes", rebuilt == original),
        (
            "content-derived ID is the original ID",
            plan.identity() == pdb.identity(),
        ),
        ("second plan has the same bytes", again == rebuilt),
        (
            "explicit original ID has the original bytes",
            explicit == original,
        ),
        ("MPDB payload decodes to the rebuild", embedded == rebuilt),
    ];
    println!(
        "{}: {} bytes, ID {}, SHA256 checksum {}",
        comparison.fixture,
        rebuilt.len(),
        hex(&plan.identity().to_bytes()),
        hex(plan.checksum(ChecksumAlgorithm::Sha256).digest()),
    );
    let mut passed = true;
    for (name, result) in checks {
        println!("  {name}: {}", yes(result));
        passed &= result;
    }
    passed &= compare_dumps(assembly, &mut command, &path, &comparison.fixture, out)?;
    Ok(passed)
}

/// Compare the verifier dumps of the original and of the rebuilt file. Write both dumps.
fn compare_dumps(
    assembly: &Path,
    original: &mut std::process::Command,
    rebuilt: &Path,
    fixture: &str,
    out: &Path,
) -> Result<bool, String> {
    let mut texts = Vec::new();
    for command in [original, &mut verifier::command(assembly, rebuilt)] {
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
        texts.push(
            String::from_utf8(output.stdout)
                .map_err(|error| format!("the verifier output is not UTF-8: {error}"))?,
        );
    }
    for (suffix, text) in [("original", &texts[0]), ("rebuilt", &texts[1])] {
        let path = out.join(format!("{fixture}.{suffix}.srm.txt"));
        fs::write(&path, text)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    let differences = differences(&texts[0], &texts[1]);
    println!(
        "  verifier dump: {} lines, {} different lines",
        texts[0].lines().count(),
        differences.len()
    );
    for (line, original, rebuilt) in differences.iter().take(SHOWN_DIFFERENCES) {
        println!("    line {line}:\n      original: {original}\n      rebuilt:  {rebuilt}");
    }
    Ok(differences.is_empty())
}
