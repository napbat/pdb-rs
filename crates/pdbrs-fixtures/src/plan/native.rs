//! Plans for the MSVC and `clang-cl` builds of `fixtures/native`.

use std::path::{Path, PathBuf};

use super::{Expected, Paths, Plan};
use crate::classify::Kind;
use crate::environment::Environment;
use crate::matrix::{Compiler, Linker, Native};
use crate::step::{Step, quote};
use crate::toolchain::{Toolchains, vcvars_argument};

/// The C++ translation units.
const CPP_SOURCES: [&str; 4] = ["main.cpp", "types.cpp", "control.cpp", "big_enum.cpp"];
/// The C translation units.
const C_SOURCES: [&str; 1] = ["c_api.c"];
/// Options for every compile step.
const COMMON: [&str; 5] = ["/nologo", "/c", "/W3", "/EHsc", "/utf-8"];

/// Make the plan of a native fixture.
pub fn plan(recipe: &Native, paths: &Paths, toolchains: &mut Toolchains) -> Result<Plan, String> {
    let studio = toolchains.visual_studio(recipe.studio)?;
    let environment = toolchains.vcvars(recipe.studio, recipe.arch)?;
    let label = format!(
        "{} {}",
        quote(&studio.vcvarsall().display().to_string()),
        vcvars_argument(std::env::consts::ARCH, recipe.arch).unwrap_or_default()
    );
    let mut plan = Plan::default();
    let compiler = match recipe.compiler {
        Compiler::Msvc => msvc_tool(toolchains, &environment, &label, "cl.exe", &mut plan)?,
        Compiler::ClangCl => llvm_tool(toolchains, studio.llvm_tool("clang-cl.exe")?, &mut plan),
    };
    let linker = match recipe.linker {
        Linker::Msvc => msvc_tool(toolchains, &environment, &label, "link.exe", &mut plan)?,
        Linker::Lld => llvm_tool(toolchains, studio.llvm_tool("lld-link.exe")?, &mut plan),
    };

    let sources = paths.source("native");
    let objects = paths.fixture.join("obj");
    plan.directories.push(objects.clone());
    for (language, files) in [
        ("/std:c++20", &CPP_SOURCES[..]),
        ("/std:c17", &C_SOURCES[..]),
    ] {
        let mut arguments = compile_options(recipe, language, &objects);
        arguments.extend(
            files
                .iter()
                .map(|file| sources.join(file).display().to_string()),
        );
        plan.steps.push(
            Step::new(&compiler, &paths.fixture)
                .args(arguments)
                .environment(&label, &environment),
        );
    }

    let image = paths.fixture.join("demo.exe");
    let pdb = paths.fixture.join("demo.pdb");
    let mut arguments: Vec<String> = vec!["/nologo".to_owned()];
    arguments.extend(recipe.link.iter().map(|option| (*option).to_owned()));
    arguments.push(format!("/OUT:{}", image.display()));
    arguments.push(format!("/PDB:{}", pdb.display()));
    arguments.extend(
        CPP_SOURCES
            .iter()
            .chain(&C_SOURCES)
            .map(|file| object_path(&objects, file).display().to_string()),
    );
    let mut link = Step::new(&linker, &paths.fixture)
        .args(arguments)
        .environment(&label, &environment);
    if recipe.link.contains(&"/DEBUG:FASTLINK") {
        // MSVC 14.50 and later ignore /DEBUG:FASTLINK, warn with LNK4315 and write a full PDB.
        link = link.unsupported_when(
            "LNK4315",
            "the selected link.exe ignores /DEBUG:FASTLINK (warning LNK4315) and writes a \
             /DEBUG:FULL PDB; a Visual Studio 2022 toolset (MSVC 14.4x or older) is needed",
        );
    }
    plan.steps.push(link);
    plan.outputs = vec![
        Expected {
            path: image,
            kind: Kind::Image,
        },
        Expected {
            path: pdb,
            kind: Kind::NativePdb,
        },
    ];
    Ok(plan)
}

/// The options of one compile step, before the source files.
fn compile_options(recipe: &Native, language: &str, objects: &Path) -> Vec<String> {
    let mut options: Vec<String> = COMMON.iter().map(|option| (*option).to_owned()).collect();
    options.extend(recipe.compile.iter().map(|option| (*option).to_owned()));
    options.push(language.to_owned());
    match recipe.compiler {
        Compiler::Msvc => options.push(format!("/Fd{}", objects.join("compile.pdb").display())),
        Compiler::ClangCl => options.push(format!("--target={}", recipe.arch.clang_target())),
    }
    // A trailing separator tells the compiler that /Fo names a directory.
    options.push(format!(
        "/Fo{}{}",
        objects.display(),
        std::path::MAIN_SEPARATOR
    ));
    options
}

fn object_path(objects: &Path, source: &str) -> PathBuf {
    objects.join(Path::new(source).with_extension("obj"))
}

/// Find an MSVC tool on the `PATH` of the environment and record its banner.
fn msvc_tool(
    toolchains: &mut Toolchains,
    environment: &Environment,
    label: &str,
    file: &str,
    plan: &mut Plan,
) -> Result<PathBuf, String> {
    let path = environment
        .find_program(file)
        .ok_or_else(|| format!("{file} is not on the PATH of {label}"))?;
    let name = file.trim_end_matches(".exe").to_owned();
    plan.tools
        .push((name, toolchains.version(&path, &[], Some(environment))));
    Ok(path)
}

/// Record the version of an LLVM tool.
fn llvm_tool(toolchains: &mut Toolchains, path: PathBuf, plan: &mut Plan) -> PathBuf {
    let name = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    plan.tools
        .push((name, toolchains.version(&path, &["--version"], None)));
    path
}
