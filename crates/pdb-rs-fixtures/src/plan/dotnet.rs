//! Plans for the `dotnet build` runs of `fixtures/dotnet`.

use std::path::PathBuf;

use super::{Expected, Paths, Plan};
use crate::classify::Kind;
use crate::matrix::Dotnet;
use crate::step::Step;
use crate::toolchain::Toolchains;

/// The build configuration. The artifacts layout uses its lowercase name.
const CONFIGURATION: &str = "Debug";

/// Make the plan of a .NET fixture.
pub fn plan(recipe: &Dotnet, paths: &Paths, toolchains: &mut Toolchains) -> Result<Plan, String> {
    let project_directory = paths.source(&format!("dotnet/{}", recipe.directory));
    let dotnet = PathBuf::from("dotnet");
    let version = toolchains.dotnet_version(&project_directory)?;
    let mut plan = Plan::default();
    plan.tools.push(("dotnet".to_owned(), version));

    let project = project_directory.join(format!("{}.{}", recipe.project, recipe.extension));
    let artifacts = paths.fixture.join("artifacts");
    let step = Step::new(&dotnet, &project_directory)
        .args(["build".to_owned(), project.display().to_string()])
        .args(["-c", CONFIGURATION, "--artifacts-path"])
        .args([artifacts.display().to_string()])
        .args(["--disable-build-servers", "-nologo"])
        .args(
            recipe
                .properties
                .iter()
                .map(|property| format!("-p:{property}")),
        )
        .variable("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .variable("DOTNET_NOLOGO", "1")
        .variable("DOTNET_CLI_UI_LANGUAGE", "en");
    plan.steps.push(step);

    let output = artifacts
        .join("bin")
        .join(recipe.project)
        .join(CONFIGURATION.to_ascii_lowercase());
    let image_kind = if recipe.pdb.is_some() {
        Kind::Image
    } else {
        Kind::EmbeddedInImage
    };
    plan.outputs.push(Expected {
        path: output.join(format!("{}.dll", recipe.project)),
        kind: image_kind,
    });
    if let Some(kind) = recipe.pdb {
        plan.outputs.push(Expected {
            path: output.join(format!("{}.pdb", recipe.project)),
            kind,
        });
    }
    Ok(plan)
}
