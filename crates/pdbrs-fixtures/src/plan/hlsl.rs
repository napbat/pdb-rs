//! Plans for the `fxc` and `dxc` builds of `fixtures/hlsl`.

use std::path::Path;

use super::{Expected, Paths, Plan};
use crate::classify::Kind;
use crate::matrix::{Arch, Hlsl, ShaderCompiler, Studio};
use crate::step::{Step, quote};
use crate::toolchain::{Toolchains, vcvars_argument};
use crate::tools::file_version::file_version;

/// The shader stages: the source file name without extension and the profile prefix.
const STAGES: [(&str, &str); 3] = [("vertex", "vs"), ("pixel", "ps"), ("compute", "cs")];

/// Make the plan of an HLSL fixture: one compile step and one PDB for each shader stage.
///
/// The error tells why the shader compiler is not available.
pub fn plan(recipe: &Hlsl, paths: &Paths, toolchains: &mut Toolchains) -> Result<Plan, String> {
    // The `vcvarsall.bat` environment puts the tools of the Windows SDK on the PATH.
    let studio = toolchains.visual_studio(Studio::Latest)?;
    let environment = toolchains.vcvars(Studio::Latest, Arch::X64)?;
    let label = format!(
        "{} {}",
        quote(&studio.vcvarsall().display().to_string()),
        vcvars_argument(std::env::consts::ARCH, Arch::X64).unwrap_or_default()
    );
    let (name, file) = match recipe.compiler {
        ShaderCompiler::Fxc => ("fxc", "fxc.exe"),
        ShaderCompiler::Dxc => ("dxc", "dxc.exe"),
    };
    let compiler = environment
        .find_program(file)
        .ok_or_else(|| format!("{file} is not on the PATH of {label}; install the Windows SDK"))?;
    let version = match recipe.compiler {
        ShaderCompiler::Fxc => fxc_version(&compiler),
        ShaderCompiler::Dxc => toolchains.version(&compiler, &["--version"], Some(&environment)),
    };
    let mut plan = Plan::default();
    plan.tools.push((name.to_owned(), version));
    let sources = paths.source("hlsl");
    for (stage, prefix) in STAGES {
        let pdb = paths.fixture.join(format!("{stage}.pdb"));
        let arguments = arguments(
            recipe,
            &format!("{prefix}_{}", recipe.model),
            &sources.join(format!("{stage}.hlsl")),
            &pdb,
            &paths.fixture.join(format!("{stage}.cso")),
        );
        plan.steps.push(
            Step::new(&compiler, &paths.fixture)
                .args(arguments)
                .environment(&label, &environment),
        );
        plan.outputs.push(Expected {
            path: pdb,
            kind: Kind::NativePdb,
        });
    }
    Ok(plan)
}

/// The arguments of one compile step. The step writes the PDB and the shader object.
fn arguments(
    recipe: &Hlsl,
    profile: &str,
    source: &Path,
    pdb: &Path,
    object: &Path,
) -> Vec<String> {
    let mut arguments: Vec<String> = Vec::new();
    match recipe.compiler {
        ShaderCompiler::Fxc => {
            arguments.extend(["/nologo", "/T", profile, "/E", "main"].map(str::to_owned));
            arguments.extend(recipe.options.iter().map(|option| (*option).to_owned()));
            arguments.push(format!("/Fd{}", pdb.display()));
            arguments.push(format!("/Fo{}", object.display()));
        }
        ShaderCompiler::Dxc => {
            arguments.extend(["-nologo", "-T", profile, "-E", "main"].map(str::to_owned));
            arguments.extend(recipe.options.iter().map(|option| (*option).to_owned()));
            arguments.extend(["-Fd".to_owned(), pdb.display().to_string()]);
            arguments.extend(["-Fo".to_owned(), object.display().to_string()]);
        }
    }
    arguments.push(source.display().to_string());
    arguments
}

/// The file version of `fxc.exe`. Its banner names only the compiler version 10.1.
fn fxc_version(program: &Path) -> String {
    std::fs::read(program)
        .ok()
        .and_then(|bytes| file_version(&bytes))
        .map_or_else(
            || "fxc, no file version".to_owned(),
            |version| format!("fxc file version {version}"),
        )
}

#[cfg(test)]
mod tests {
    use super::arguments;
    use crate::matrix::{Hlsl, ShaderCompiler};
    use std::path::Path;

    #[test]
    fn each_compiler_gets_its_option_syntax() {
        let fxc = Hlsl {
            compiler: ShaderCompiler::Fxc,
            model: "5_1",
            options: &["/Zi"],
        };
        let paths = (Path::new("a.hlsl"), Path::new("a.pdb"), Path::new("a.cso"));
        assert_eq!(
            arguments(&fxc, "vs_5_1", paths.0, paths.1, paths.2),
            [
                "/nologo", "/T", "vs_5_1", "/E", "main", "/Zi", "/Fda.pdb", "/Foa.cso", "a.hlsl"
            ]
        );
        let dxc = Hlsl {
            compiler: ShaderCompiler::Dxc,
            model: "6_0",
            options: &["-Zi"],
        };
        assert_eq!(
            arguments(&dxc, "cs_6_0", paths.0, paths.1, paths.2),
            [
                "-nologo", "-T", "cs_6_0", "-E", "main", "-Zi", "-Fd", "a.pdb", "-Fo", "a.cso",
                "a.hlsl"
            ]
        );
    }
}
