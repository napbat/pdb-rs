//! Plans for the Cargo builds of `fixtures/rust`.

use std::path::PathBuf;

use super::{Expected, Paths, Plan};
use crate::classify::Kind;
use crate::matrix::{Linker, Rust, Studio};
use crate::step::Step;
use crate::toolchain::Toolchains;

/// The package name, which is also the name of the executable.
const PACKAGE: &str = "pdbrs-rust-demo";

/// Make the plan of a Rust fixture.
pub fn plan(recipe: &Rust, paths: &Paths, toolchains: &mut Toolchains) -> Result<Plan, String> {
    let target = recipe.arch.rust_target();
    let sysroot = toolchains.rust_sysroot()?;
    if !sysroot
        .join("lib")
        .join("rustlib")
        .join(target)
        .join("lib")
        .is_dir()
    {
        return Err(format!(
            "the Rust standard library for {target} is not installed; run `rustup target add {target}`"
        ));
    }
    let cargo = std::env::var_os("CARGO").map_or_else(|| PathBuf::from("cargo"), PathBuf::from);
    let mut plan = Plan::default();
    plan.tools.push((
        "rustc".to_owned(),
        toolchains.version("rustc".as_ref(), &["-V"], None),
    ));
    plan.tools.push((
        "cargo".to_owned(),
        toolchains.version(&cargo, &["-V"], None),
    ));

    let source = paths.source("rust");
    let target_dir = paths.fixture.join("target");
    let manifest = source.join("Cargo.toml");
    let mut step = Step::new(&cargo, &source)
        .args(["build", "--locked", "--manifest-path"])
        .args([manifest.display().to_string()])
        .args(["--target", target])
        .variable("CARGO_TARGET_DIR", &target_dir.display().to_string());
    if recipe.release {
        step = step.args(["--release"]);
    }
    match recipe.linker {
        Linker::Msvc => record_link_banner(recipe, toolchains, &mut plan),
        Linker::Lld => {
            let lld = toolchains
                .visual_studio(Studio::Latest)?
                .llvm_tool("lld-link.exe")?;
            let version = toolchains.version(&lld, &["--version"], None);
            plan.tools.push(("lld-link".to_owned(), version));
            step = step.variable(&linker_variable(target), &lld.display().to_string());
        }
    }
    plan.steps.push(step);

    let profile = if recipe.release { "release" } else { "debug" };
    let output = target_dir.join(target).join(profile);
    plan.outputs = vec![
        Expected {
            path: output.join(format!("{PACKAGE}.exe")),
            kind: Kind::Image,
        },
        Expected {
            path: output.join(format!("{}.pdb", PACKAGE.replace('-', "_"))),
            kind: Kind::NativePdb,
        },
    ];
    Ok(plan)
}

/// Record the banner of the `link.exe` that `rustc` most likely selects.
///
/// `rustc` finds `link.exe` itself. The banner comes from the `vcvarsall.bat` environment of
/// the same architecture, so it names the latest Visual Studio tools.
fn record_link_banner(recipe: &Rust, toolchains: &mut Toolchains, plan: &mut Plan) {
    let version = match toolchains.vcvars(Studio::Latest, recipe.arch) {
        Ok(environment) => match environment.find_program("link.exe") {
            Some(link) => toolchains.version(&link, &[], Some(&environment)),
            None => "link.exe is not on the vcvarsall.bat PATH".to_owned(),
        },
        Err(reason) => format!("unknown: {reason}"),
    };
    plan.tools.push(("link".to_owned(), version));
}

/// The Cargo variable that selects the linker of a target.
fn linker_variable(target: &str) -> String {
    format!(
        "CARGO_TARGET_{}_LINKER",
        target.to_ascii_uppercase().replace('-', "_")
    )
}

#[cfg(test)]
mod tests {
    use super::linker_variable;

    #[test]
    fn names_the_cargo_linker_variable() {
        assert_eq!(
            linker_variable("x86_64-pc-windows-msvc"),
            "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER"
        );
    }
}
