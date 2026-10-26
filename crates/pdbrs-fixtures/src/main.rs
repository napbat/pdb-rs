//! Build the demo programs in `fixtures/` and record their PDB outputs.
//!
//! The generator builds each fixture of the matrix in its own directory below the output
//! root. It writes `manifest.json` with the status, tool versions, commands and outputs of
//! each fixture. A run with `--only` keeps the entries of the other fixtures from the earlier
//! manifest. It exits with a failure code if a fixture with an available toolchain fails.
//!
//! The `tools` command installs `pdbtool` and builds `DIA2Dump` and the Portable PDB verifier
//! into `target/tools/bin`. The fixture manifest copies the tool records of
//! `target/tools/manifest.json`.
//!
//! The `verify-portable` command compares the pdbrs and verifier dumps of the Portable PDB
//! fixtures. The `detect` command opens each fixture output with the format facade of pdbrs.

mod args;
mod classify;
mod environment;
mod json;
mod manifest;
mod matrix;
mod plan;
mod step;
mod toolchain;
mod tools;
mod verify;

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::args::{Action, Options, USAGE};
use crate::classify::{Kind, classify, embedded_pdb_payload};
use crate::manifest::{Header, Output, Record, Status, relative_path};
use crate::matrix::{FIXTURES, Fixture};
use crate::plan::{Paths, Plan};
use crate::step::StepError;
use crate::toolchain::Toolchains;

fn main() -> ExitCode {
    match args::parse(std::env::args().skip(1)) {
        Ok(Action::Help) => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Action::List) => {
            for fixture in FIXTURES {
                println!(
                    "{:<30} {:<22} {:<7} {}",
                    fixture.name, fixture.producer, fixture.arch, fixture.configuration
                );
            }
            ExitCode::SUCCESS
        }
        Ok(Action::Build(options)) => exit_code(build(options)),
        Ok(Action::Tools) => exit_code(tools::run(&workspace_root())),
        Ok(Action::VerifyPortable(fixtures)) => {
            let workspace = workspace_root();
            let fixtures = fixtures.unwrap_or_else(|| workspace.join("target").join("fixtures"));
            exit_code(verify::verify_portable(&workspace, &fixtures))
        }
        Ok(Action::Detect(fixtures)) => {
            let fixtures =
                fixtures.unwrap_or_else(|| workspace_root().join("target").join("fixtures"));
            exit_code(verify::detect(&fixtures))
        }
        Err(error) => {
            eprintln!("error: {error}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// The exit code of a command that returns `false` if a part failed.
fn exit_code(result: Result<bool, String>) -> ExitCode {
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The workspace root, two levels above the manifest directory of this crate.
fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .nth(2)
        .unwrap_or(manifest)
        .to_path_buf()
}

/// Build the selected fixtures and write the manifest.
///
/// Return `false` if a fixture with an available toolchain failed.
fn build(options: Options) -> Result<bool, String> {
    let workspace = workspace_root();
    let selected = matrix::select(&options.only)?;
    let out = options
        .out
        .unwrap_or_else(|| workspace.join("target").join("fixtures"));
    fs::create_dir_all(&out).map_err(|error| format!("cannot make {}: {error}", out.display()))?;
    let out = fs::canonicalize(&out)
        .map(simplify)
        .map_err(|error| format!("cannot resolve {}: {error}", out.display()))?;

    let mut toolchains = Toolchains::default();
    let mut records = Vec::new();
    for fixture in selected {
        println!("building {} ...", fixture.name);
        let record = build_fixture(fixture, &workspace, &out, &mut toolchains);
        println!(
            "  {}{}",
            record.status.name(),
            record
                .reason
                .as_deref()
                .map(|reason| format!(": {reason}"))
                .unwrap_or_default()
        );
        records.push(record);
    }

    let header = Header::now(&workspace, &out);
    let consumer_tools = tools::consumer_tools(&workspace);
    let manifest = out.join("manifest.json");
    // A run with `--only` keeps the entries of the other fixtures from the earlier manifest.
    let previous = if options.only.is_empty() {
        None
    } else {
        manifest::read_previous(&manifest)
    };
    let fixtures = manifest::fixture_entries(previous.as_ref(), &records);
    fs::write(
        &manifest,
        manifest::render(&header, fixtures, consumer_tools),
    )
    .map_err(|error| format!("cannot write {}: {error}", manifest.display()))?;
    println!("wrote {}", manifest.display());
    Ok(records.iter().all(|record| record.status != Status::Failed))
}

/// Remove the `\\?\` prefix that `fs::canonicalize` adds on Windows.
///
/// Some compilers and linkers do not accept the prefix in their arguments.
fn simplify(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
        Some(stripped) if !stripped.starts_with("UNC\\") => PathBuf::from(stripped),
        _ => path,
    }
}

/// Plan, run and check one fixture.
fn build_fixture(
    fixture: &'static Fixture,
    workspace: &Path,
    out: &Path,
    toolchains: &mut Toolchains,
) -> Record {
    let paths = Paths {
        workspace: workspace.to_path_buf(),
        fixture: out.join(fixture.name),
    };
    let plan = match plan::plan(fixture, &paths, toolchains) {
        Ok(plan) => plan,
        Err(reason) => return Record::unavailable(fixture, reason),
    };
    let mut record = Record {
        fixture,
        status: Status::Built,
        reason: None,
        tools: plan.tools.clone(),
        steps: plan.steps.clone(),
        outputs: Vec::new(),
    };
    let result = run(&plan, &paths.fixture)
        .and_then(|()| check_outputs(&plan, out).map_err(StepError::Failed));
    match result {
        Ok((outputs, notes)) => {
            record.outputs = outputs;
            record.reason = (!notes.is_empty()).then(|| notes.join("; "));
        }
        Err(StepError::Failed(reason)) => {
            record.status = Status::Failed;
            record.reason = Some(reason);
        }
        Err(StepError::Unsupported(reason)) => {
            record.status = Status::Unavailable;
            record.reason = Some(reason);
        }
    }
    record
}

/// Make a clean fixture directory and run each step. The steps write to `build.log`.
fn run(plan: &Plan, directory: &Path) -> Result<(), StepError> {
    let failed = |action: &str, path: &Path, error: std::io::Error| {
        StepError::Failed(format!("cannot {action} {}: {error}", path.display()))
    };
    if directory.exists() {
        fs::remove_dir_all(directory).map_err(|error| failed("clean", directory, error))?;
    }
    for path in std::iter::once(directory).chain(plan.directories.iter().map(PathBuf::as_path)) {
        fs::create_dir_all(path).map_err(|error| failed("make", path, error))?;
    }
    let log_path = directory.join("build.log");
    let mut log = File::create(&log_path).map_err(|error| failed("make", &log_path, error))?;
    for step in &plan.steps {
        step.run(&mut log).map_err(|error| match error {
            StepError::Failed(reason) => {
                StepError::Failed(format!("{reason} (log: {})", log_path.display()))
            }
            unsupported @ StepError::Unsupported(_) => unsupported,
        })?;
    }
    Ok(())
}

/// Check that each output exists and classify its contents.
///
/// For an image with an embedded Portable PDB, also write its `MPDB` payload to a file next to
/// the image, with the extension `mpdb`. Return the outputs and notes about outputs whose kind
/// differs from the plan.
fn check_outputs(plan: &Plan, out: &Path) -> Result<(Vec<Output>, Vec<String>), String> {
    let mut outputs = Vec::new();
    let mut notes = Vec::new();
    for expected in &plan.outputs {
        let bytes = fs::read(&expected.path).map_err(|error| {
            format!(
                "expected output {} is missing: {error}",
                expected.path.display()
            )
        })?;
        let kind = classify(&bytes);
        let path = relative_path(out, &expected.path);
        if kind != expected.kind {
            notes.push(format!(
                "{path}: expected {}, the contents show {}",
                expected.kind.name(),
                kind.name()
            ));
        }
        outputs.push(output(kind, path, &bytes));
        if kind == Kind::EmbeddedInImage
            && let Some(payload) = embedded_pdb_payload(&bytes)
        {
            let payload_path = expected.path.with_extension("mpdb");
            fs::write(&payload_path, payload)
                .map_err(|error| format!("cannot write {}: {error}", payload_path.display()))?;
            outputs.push(output(
                Kind::EmbeddedPdbPayload,
                relative_path(out, &payload_path),
                payload,
            ));
        }
    }
    Ok((outputs, notes))
}

/// Make the record of one output file.
fn output(kind: Kind, path: String, bytes: &[u8]) -> Output {
    Output {
        kind,
        path,
        size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
    }
}
