//! `PortableVerifier`, the .NET program in `fixtures/dotnet/verifier/`.
//!
//! The program reads a Portable PDB with `System.Reflection.Metadata` from the shared
//! framework and prints the canonical dump (see `fixtures/README.md`). The build writes the
//! intermediate files to `portable-verifier-build/` in the tools root, so the source directory
//! stays clean.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Missing, Paths, Tool};
use crate::step::Step;
use crate::toolchain::{Toolchains, capture};

/// The tool name and the name of its project.
pub const NAME: &str = "PortableVerifier";

/// The project directory, relative to the workspace root.
const PROJECT: &str = "fixtures/dotnet/verifier";

/// The output directory below `bin/` in the tools root.
const OUTPUT: &str = "portable-verifier";

/// Build the verifier with `dotnet build` into `bin/portable-verifier/` in the tools root.
pub fn build(paths: &Paths, toolchains: &mut Toolchains) -> Tool {
    super::record(NAME, |tool| {
        let workspace =
            paths.root.ancestors().nth(2).ok_or_else(|| {
                Missing::Failed("the tools root has no workspace root".to_owned())
            })?;
        let project = workspace.join(PROJECT);
        let csproj = project.join(format!("{NAME}.csproj"));
        let output = paths.bin.join(OUTPUT);
        let artifacts = paths.root.join("portable-verifier-build");
        tool.source("project", csproj.to_string_lossy());
        tool.source(
            "framework",
            "System.Reflection.Metadata from the shared framework; no NuGet package",
        );
        let sdk = toolchains
            .dotnet_version(&project)
            .map_err(Missing::Unavailable)?;
        tool.version("dotnet_sdk", sdk);

        tool.steps.push(
            Step::new("dotnet", &project)
                .args(["build".to_owned(), csproj.to_string_lossy().into_owned()])
                .args(["-c", "Release", "--artifacts-path"])
                .args([artifacts.to_string_lossy().into_owned()])
                .args(["-o".to_owned(), output.to_string_lossy().into_owned()])
                .args(["--disable-build-servers", "-nologo"])
                .variable("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
                .variable("DOTNET_NOLOGO", "1")
                .variable("DOTNET_CLI_UI_LANGUAGE", "en"),
        );
        super::run_steps(&tool.steps, &paths.log(NAME))?;

        let assembly = output.join(format!("{NAME}.dll"));
        tool.version("runtime", runtime_version(&assembly)?);
        tool.finish(vec![
            assembly,
            output.join(format!("{NAME}.runtimeconfig.json")),
        ])
    })
}

/// The `System.Reflection.Metadata` and .NET runtime versions that the verifier prints.
fn runtime_version(assembly: &Path) -> Result<String, Missing> {
    capture(Command::new("dotnet").arg(assembly).arg("--version"))
        .map(|text| text.trim().to_owned())
        .map_err(Missing::Failed)
}

/// The command that runs the verifier assembly on `pdb`.
pub fn command(assembly: &Path, pdb: &Path) -> Command {
    let mut command = Command::new(PathBuf::from("dotnet"));
    command.arg(assembly).arg(pdb);
    command
}
