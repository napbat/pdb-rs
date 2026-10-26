//! Build plans: the tool versions, steps and expected outputs of one fixture.

mod dotnet;
mod hlsl;
mod native;
mod rust;

use std::path::PathBuf;

use crate::classify::Kind;
use crate::matrix::{Fixture, Recipe};
use crate::step::Step;
use crate::toolchain::Toolchains;

/// An output that a plan must produce.
#[derive(Debug)]
pub struct Expected {
    /// The absolute path.
    pub path: PathBuf,
    /// The kind that the build must produce.
    pub kind: Kind,
}

/// Everything that a build of one fixture needs.
#[derive(Debug, Default)]
pub struct Plan {
    /// Tool names and version lines.
    pub tools: Vec<(String, String)>,
    /// Directories to make before the first step.
    pub directories: Vec<PathBuf>,
    /// Generated files to write after the directories and before the first step: the path and
    /// the contents.
    pub files: Vec<(PathBuf, String)>,
    /// The steps, in order.
    pub steps: Vec<Step>,
    /// The outputs.
    pub outputs: Vec<Expected>,
}

/// The directories of one fixture build.
#[derive(Debug)]
pub struct Paths {
    /// The workspace root. The sources are in its `fixtures/` directory.
    pub workspace: PathBuf,
    /// The output directory of the fixture.
    pub fixture: PathBuf,
}

impl Paths {
    /// The source directory below `fixtures/`.
    pub fn source(&self, relative: &str) -> PathBuf {
        let mut path = self.workspace.join("fixtures");
        path.extend(relative.split('/'));
        path
    }
}

/// Make the plan of a fixture.
///
/// The error tells why the toolchain of the fixture is not available.
pub fn plan(fixture: &Fixture, paths: &Paths, toolchains: &mut Toolchains) -> Result<Plan, String> {
    match &fixture.recipe {
        Recipe::Native(recipe) => native::plan(recipe, paths, toolchains),
        Recipe::Rust(recipe) => rust::plan(recipe, paths, toolchains),
        Recipe::Dotnet(recipe) => dotnet::plan(recipe, paths, toolchains),
        Recipe::Hlsl(recipe) => hlsl::plan(recipe, paths, toolchains),
    }
}
