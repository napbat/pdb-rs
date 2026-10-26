//! `msfz-encode`, the Cargo project in `fixtures/tools/msfz-encode/`.
//!
//! The program writes an MSFZ file from an MSF 7.0 PDB with Microsoft's `ms-pdb-msfz` writer.
//! It makes the `msfz-*` fixtures: forms that `pdbtool pdz-encode` does not write. The project
//! has its own empty `[workspace]` table, so it is not part of the pdbrs workspace, and it does
//! not use pdbrs.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::{Missing, Paths, Tool};
use crate::step::Step;
use crate::toolchain::{Toolchains, capture};

/// The tool name and the name of its package.
pub const NAME: &str = "msfz-encode";

/// The project directory, relative to the workspace root.
const PROJECT: &str = "fixtures/tools/msfz-encode";

/// The upstream repository of the Microsoft crates.
const REPOSITORY: &str = "https://github.com/microsoft/pdb-rs";

/// The pinned `microsoft/pdb-rs` revision in `docs/reference/SOURCES.json`.
const REVISION: &str = "2081a0dd6f1150cf965dec10855e75cfecfde176";

/// The Microsoft crates and their versions at [`REVISION`]. `Cargo.lock` of the project must
/// have these versions.
const CRATES: [(&str, &str); 2] = [("ms-pdb-msfz", "0.1.10"), ("ms-pdb-msf", "0.1.7")];

/// Install `msfz-encode` into the tools root with `cargo install --path --locked --force`.
///
/// The build directory `msfz-encode-build/` stays in the tools root, so a later run only
/// relinks the executable.
pub fn build(paths: &Paths, toolchains: &mut Toolchains) -> Tool {
    super::record(NAME, |tool| {
        let workspace =
            paths.root.ancestors().nth(2).ok_or_else(|| {
                Missing::Failed("the tools root has no workspace root".to_owned())
            })?;
        let mut project = workspace.to_path_buf();
        project.extend(PROJECT.split('/'));
        let manifest = project.join("Cargo.toml");
        if !manifest.is_file() {
            return Err(Missing::Unavailable(format!(
                "{} does not exist",
                manifest.display()
            )));
        }
        // `cargo run` sets `CARGO` to the Cargo that runs the generator.
        let cargo = std::env::var_os("CARGO").map_or_else(|| PathBuf::from("cargo"), PathBuf::from);
        tool.source("project", manifest.to_string_lossy());
        tool.source("repository", REPOSITORY);
        tool.source("pinned_revision", REVISION);
        tool.version("cargo", toolchains.version(&cargo, &["-V"], None));
        let lock = project.join("Cargo.lock");
        let lock = fs::read_to_string(&lock)
            .map_err(|error| Missing::Failed(format!("cannot read {}: {error}", lock.display())))?;
        for (name, expected) in CRATES {
            let locked = locked_version(&lock, name).ok_or_else(|| {
                Missing::Failed(format!("Cargo.lock of {NAME} does not list {name}"))
            })?;
            if locked != expected {
                return Err(Missing::Failed(format!(
                    "Cargo.lock of {NAME} has {name} {locked}, not {expected}"
                )));
            }
            tool.version(name, locked);
        }

        let root = paths.root.to_string_lossy().into_owned();
        let build = paths.root.join("msfz-encode-build");
        tool.steps.push(Step::new(&cargo, &paths.root).args([
            "install".to_owned(),
            "--path".to_owned(),
            project.to_string_lossy().into_owned(),
            "--locked".to_owned(),
            "--force".to_owned(),
            "--root".to_owned(),
            root,
            "--target-dir".to_owned(),
            build.to_string_lossy().into_owned(),
        ]));
        super::run_steps(&tool.steps, &paths.log(NAME))?;

        let program = paths
            .bin
            .join(format!("{NAME}{}", std::env::consts::EXE_SUFFIX));
        let version = capture(Command::new(&program).arg("--version")).map_err(Missing::Failed)?;
        tool.version("crate", version.trim().trim_start_matches(NAME).trim());
        tool.finish(vec![program])
    })
}

/// Find the version of the package `name` in the text of a `Cargo.lock` file.
///
/// Each package has a `[[package]]` table with a `name = "..."` line and then a
/// `version = "..."` line.
fn locked_version<'a>(lock: &'a str, name: &str) -> Option<&'a str> {
    let wanted = format!("name = \"{name}\"");
    let mut lines = lock.lines().map(str::trim);
    lines.find(|line| *line == wanted)?;
    lines
        .next()?
        .strip_prefix("version = \"")?
        .strip_suffix('"')
}

#[cfg(test)]
mod tests {
    use super::locked_version;

    #[test]
    fn finds_the_locked_version() {
        let lock = "[[package]]\r\nname = \"ms-pdb-msf\"\r\nversion = \"0.1.7\"\r\n\r\n\
                    [[package]]\nname = \"ms-pdb-msfz\"\nversion = \"0.1.10\"\n";
        assert_eq!(locked_version(lock, "ms-pdb-msf"), Some("0.1.7"));
        assert_eq!(locked_version(lock, "ms-pdb-msfz"), Some("0.1.10"));
        assert_eq!(locked_version(lock, "ms-pdb"), None);
        assert_eq!(locked_version("name = \"ms-pdb\"\n", "ms-pdb"), None);
    }
}
