//! Plans for the conversion of MSF 7.0 PDB fixtures to MSFZ with `pdbtool pdz-encode`.
//!
//! A PDZ fixture reads the outputs of other fixtures in the same output root. Thus the matrix
//! puts the PDZ fixtures after their sources. The output of each source PDB is
//! `<fixture>/<source fixture>/<file name>`. The name keeps the `.pdb` extension, because DIA
//! opens only files with that extension.

use std::path::Path;

use super::{Expected, Paths, Plan};
use crate::classify::Kind;
use crate::json::Value;
use crate::matrix::{FIXTURES, Fixture, PDZ_EXCLUDED, Pdz, Recipe};
use crate::step::Step;
use crate::toolchain::Toolchains;
use crate::tools;

/// Make the plan of a PDZ fixture.
///
/// The fixture is unavailable if `pdbtool` is not built or if the toolchain of a source
/// fixture is not available.
pub fn plan(recipe: &Pdz, paths: &Paths, toolchains: &mut Toolchains) -> Result<Plan, String> {
    let record = tools::built_record(&paths.workspace, "pdbtool")?;
    let pdbtool = tools::built_tool(&paths.workspace, "pdbtool")?;
    let mut plan = Plan::default();
    plan.tools.push(("pdbtool".to_owned(), version(&record)));
    let root = paths
        .fixture
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", paths.fixture.display()))?;
    for source in sources(recipe)? {
        let source_paths = Paths {
            workspace: paths.workspace.clone(),
            fixture: root.join(source.name),
        };
        let source_plan = super::plan(source, &source_paths, toolchains)
            .map_err(|reason| format!("source fixture {}: {reason}", source.name))?;
        let directory = paths.fixture.join(source.name);
        let mut converted = false;
        for input in source_plan
            .outputs
            .iter()
            .filter(|output| output.kind == Kind::NativePdb)
        {
            let output = directory.join(file_name(&input.path)?);
            plan.steps.push(
                Step::new(&pdbtool, &paths.fixture)
                    .args(["--quiet", "pdz-encode"])
                    .args(recipe.options.iter().copied())
                    .args([
                        input.path.display().to_string(),
                        output.display().to_string(),
                    ]),
            );
            plan.outputs.push(Expected {
                path: output,
                kind: Kind::NativePdz,
            });
            converted = true;
        }
        if converted {
            plan.directories.push(directory);
        }
    }
    if plan.outputs.is_empty() {
        return Err("the sources have no MSF 7.0 PDB output".to_owned());
    }
    Ok(plan)
}

/// Return the source fixtures of `recipe` in matrix order.
///
/// An empty source list selects each fixture that is not a conversion, except
/// [`PDZ_EXCLUDED`]. The plan keeps only the MSF 7.0 PDB outputs of each source.
fn sources(recipe: &Pdz) -> Result<Vec<&'static Fixture>, String> {
    if recipe.sources.is_empty() {
        return Ok(FIXTURES
            .iter()
            .filter(|fixture| {
                !matches!(fixture.recipe, Recipe::Pdz(_)) && !PDZ_EXCLUDED.contains(&fixture.name)
            })
            .collect());
    }
    recipe
        .sources
        .iter()
        .map(|name| {
            FIXTURES
                .iter()
                .find(|fixture| fixture.name == *name)
                .ok_or_else(|| format!("unknown source fixture `{name}`"))
        })
        .collect()
}

/// Return the file name of `path`.
fn file_name(path: &Path) -> Result<&std::ffi::OsStr, String> {
    path.file_name()
        .ok_or_else(|| format!("{} has no file name", path.display()))
}

/// Return the version line of the `pdbtool` record of the tools manifest.
fn version(record: &Value) -> String {
    let field = |object: &str, name: &str| {
        record
            .get(object)
            .and_then(|value| value.get(name))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned()
    };
    format!(
        "pdbtool {} ({} at {})",
        field("version", "crate"),
        field("source", "repository"),
        field("source", "pinned_revision")
    )
}
