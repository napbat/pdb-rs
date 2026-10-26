//! Microsoft's `pdbtool`, installed from crates.io with `cargo install`.

use std::path::PathBuf;
use std::process::Command;

use super::{Missing, Paths, Tool};
use crate::step::Step;
use crate::toolchain::{Toolchains, capture};

/// The tool and crate name.
pub const NAME: &str = "pdbtool";

/// The crate version. It is the version of `pdbtool` at the pinned `microsoft/pdb-rs`
/// revision in `docs/reference/SOURCES.json`.
const VERSION: &str = "0.1.22";

/// The upstream repository of the crate.
const REPOSITORY: &str = "https://github.com/microsoft/pdb-rs";

/// The pinned `microsoft/pdb-rs` revision in `docs/reference/SOURCES.json`.
const REVISION: &str = "2081a0dd6f1150cf965dec10855e75cfecfde176";

/// Install `pdbtool` into the tools root with `cargo install --locked --force`.
///
/// The build directory `pdbtool-build/` stays in the tools root, so a later run only
/// relinks the executable.
pub fn build(paths: &Paths, toolchains: &mut Toolchains) -> Tool {
    super::record(NAME, |tool| {
        // `cargo run` sets `CARGO` to the Cargo that runs the generator.
        let cargo = std::env::var_os("CARGO").map_or_else(|| PathBuf::from("cargo"), PathBuf::from);
        tool.source("registry", "crates.io");
        tool.source("crate", format!("{NAME} {VERSION}"));
        tool.source("repository", REPOSITORY);
        tool.source("pinned_revision", REVISION);
        tool.version("cargo", toolchains.version(&cargo, &["-V"], None));

        let root = paths.root.to_string_lossy().into_owned();
        let build = paths.root.join("pdbtool-build");
        let build = build.to_string_lossy().into_owned();
        tool.steps.push(Step::new(&cargo, &paths.root).args([
            "install",
            NAME,
            "--version",
            VERSION,
            "--locked",
            "--force",
            "--root",
            root.as_str(),
            "--target-dir",
            build.as_str(),
        ]));
        super::run_steps(&tool.steps, &paths.log(NAME))?;

        let list = capture(Command::new(&cargo).args(["install", "--list", "--root", &root]))
            .map_err(Missing::Failed)?;
        let installed = installed_version(&list, NAME).ok_or_else(|| {
            Missing::Failed(format!("`cargo install --list` does not list {NAME}"))
        })?;
        if installed != VERSION {
            return Err(Missing::Failed(format!(
                "`cargo install --list` shows {NAME} {installed}, not {VERSION}"
            )));
        }
        tool.version("crate", installed);
        tool.finish(vec![
            paths
                .bin
                .join(format!("{NAME}{}", std::env::consts::EXE_SUFFIX)),
        ])
    })
}

/// Find the version of `name` in the output of `cargo install --list`.
///
/// Each package has a line of the form `name v1.2.3:`, followed by indented executable names.
fn installed_version<'a>(list: &'a str, name: &str) -> Option<&'a str> {
    list.lines().find_map(|line| {
        line.trim_end()
            .strip_prefix(name)?
            .strip_prefix(" v")?
            .strip_suffix(':')
    })
}

#[cfg(test)]
mod tests {
    use super::installed_version;

    #[test]
    fn finds_the_installed_version() {
        let list = "pdbtool v0.1.22:\r\n    pdbtool.exe\r\npdbtool-extra v9.0.0:\n    x.exe\n";
        assert_eq!(installed_version(list, "pdbtool"), Some("0.1.22"));
        assert_eq!(installed_version(list, "pdbtool-extra"), Some("9.0.0"));
        assert_eq!(installed_version(list, "pdb"), None);
        assert_eq!(installed_version("", "pdbtool"), None);
    }
}
