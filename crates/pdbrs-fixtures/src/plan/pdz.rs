//! Plans for the conversion of MSF 7.0 PDB fixtures to MSFZ with `pdbtool pdz-encode` or
//! `msfz-encode`.
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
/// The fixture is unavailable if its encoder is not built or if the toolchain of a source
/// fixture is not available.
pub fn plan(recipe: &Pdz, paths: &Paths, toolchains: &mut Toolchains) -> Result<Plan, String> {
    let name = recipe.encoder.tool();
    let record = tools::built_record(&paths.workspace, name)?;
    let encoder = tools::built_tool(&paths.workspace, name)?;
    let mut plan = Plan::default();
    plan.tools.push((name.to_owned(), version(name, &record)));
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
                Step::new(&encoder, &paths.fixture)
                    .args(recipe.encoder.command().iter().copied())
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

/// Return the version line of the encoder record of the tools manifest.
///
/// The line has the crate version and the versions of the Microsoft libraries that the
/// record lists, for example `msfz-encode 0.1.0, ms-pdb-msfz 0.1.10, ms-pdb-msf 0.1.7`.
fn version(name: &str, record: &Value) -> String {
    let field = |object: &str, name: &str| {
        record
            .get(object)
            .and_then(|value| value.get(name))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let unknown = || "unknown".to_owned();
    let mut parts = vec![format!(
        "{name} {}",
        field("version", "crate").unwrap_or_else(unknown)
    )];
    parts.extend(
        ["ms-pdb-msfz", "ms-pdb-msf"]
            .into_iter()
            .filter_map(|library| Some(format!("{library} {}", field("version", library)?))),
    );
    format!(
        "{} ({} at {})",
        parts.join(", "),
        field("source", "repository").unwrap_or_else(unknown),
        field("source", "pinned_revision").unwrap_or_else(unknown)
    )
}
