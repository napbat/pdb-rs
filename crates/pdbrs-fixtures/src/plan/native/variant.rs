//! Plans of the native variants with other sources or extra steps.

use std::path::{Path, PathBuf};

use super::{Build, demo};
use crate::classify::Kind;
use crate::environment::Environment;
use crate::plan::Plan;
use crate::tools::file_version::file_version;

/// The source of the DLL variant.
const DLL_SOURCE: &str = "dll/exports.cpp";
/// The source of the C++/CLI variant.
const CLR_SOURCE: &str = "clr/managed.cpp";
/// The source of the annotation variant.
const ANNOTATION_SOURCE: &str = "annotation/annotation.cpp";

/// Plan the demo, then add a `srcsrv` stream to a copy of its PDB with `pdbstr`.
///
/// The error tells why `pdbstr` or `cmd.exe` is not available.
pub fn source_server(build: &Build<'_>, plan: &mut Plan) -> Result<(), String> {
    let pdbstr = pdbstr(&build.environment)?;
    let shell = build
        .environment
        .find_program("cmd.exe")
        .ok_or_else(|| format!("cmd.exe is not on the PATH of {}", build.label))?;
    let version = std::fs::read(&pdbstr)
        .ok()
        .and_then(|bytes| file_version(&bytes))
        .map_or_else(
            || "pdbstr, no file version".to_owned(),
            |version| format!("pdbstr file version {version}"),
        );
    plan.tools.push(("pdbstr".to_owned(), version));
    demo(build, plan);
    // pdbstr changes the PDB in place. Change a copy, so that the linker output stays as it is.
    let copy = build.fixture.join("demo.srcsrv.pdb");
    plan.steps.push(build.step(&shell).args([
        "/d",
        "/c",
        "copy",
        "/y",
        "demo.pdb",
        "demo.srcsrv.pdb",
    ]));
    plan.steps.push(build.step(&pdbstr).args([
        "-w".to_owned(),
        format!("-p:{}", copy.display()),
        format!("-i:{}", build.source("srcsrv.txt").display()),
        "-s:srcsrv".to_owned(),
    ]));
    plan.outputs
        .push(build.output("demo.srcsrv.pdb", Kind::NativePdb));
    Ok(())
}

/// Find `pdbstr.exe` of the Debugging Tools for Windows in the Windows SDK.
fn pdbstr(environment: &Environment) -> Result<PathBuf, String> {
    let kits = environment
        .get("WindowsSdkDir")
        .ok_or("the environment has no WindowsSdkDir variable, so the Windows SDK is missing")?;
    let host = if std::env::consts::ARCH == "aarch64" {
        "arm64"
    } else {
        "x64"
    };
    let path = Path::new(kits)
        .join("Debuggers")
        .join(host)
        .join("srcsrv")
        .join("pdbstr.exe");
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "{} is missing; install the Debugging Tools for Windows feature of the Windows SDK",
            path.display()
        ))
    }
}

/// Plan a profile-guided build of the demo.
///
/// The first link writes an instrumented program and `demo.pgd`. The training run of the
/// program writes `demo!1.pgc`. The second link uses the profile and replaces `demo.exe` and
/// `demo.pdb`. The error tells why the instrumentation runtime is not available.
pub fn profile_guided(build: &Build<'_>, plan: &mut Plan) -> Result<(), String> {
    // The instrumented program loads the runtime from the PATH.
    if build.environment.find_program("pgort140.dll").is_none() {
        return Err(format!(
            "pgort140.dll is not on the PATH of {}; the instrumented program cannot run",
            build.label
        ));
    }
    let objects = build.compile_demo(plan);
    let image = build.fixture.join("demo.exe");
    let pdb = build.fixture.join("demo.pdb");
    let database = format!("/PGD:{}", build.fixture.join("demo.pgd").display());
    plan.steps.push(build.link(
        &["/LTCG:PGINSTRUMENT".to_owned(), database.clone()],
        &image,
        &pdb,
        &objects,
    ));
    plan.steps.push(build.step(&image));
    plan.steps.push(build.link(
        &["/LTCG:PGOPTIMIZE".to_owned(), database],
        &image,
        &pdb,
        &objects,
    ));
    plan.outputs = vec![
        build.output("demo.exe", Kind::Image),
        build.output("demo.pdb", Kind::NativePdb),
    ];
    Ok(())
}

/// Plan the DLL `demo.dll` with the exports of `dll/exports.def`.
pub fn dll(build: &Build<'_>, plan: &mut Plan) {
    plan.steps
        .push(build.compile("/std:c++20", &[], &[DLL_SOURCE]));
    let extra = [
        "/DLL".to_owned(),
        format!("/DEF:{}", build.source("dll/exports.def").display()),
        format!("/IMPLIB:{}", build.objects.join("demo.lib").display()),
    ];
    plan.steps.push(build.link(
        &extra,
        &build.fixture.join("demo.dll"),
        &build.fixture.join("demo.pdb"),
        &[build.object(DLL_SOURCE)],
    ));
    plan.outputs = vec![
        build.output("demo.dll", Kind::Image),
        build.output("demo.pdb", Kind::NativePdb),
    ];
}

/// Plan the C++/CLI program `demo.exe`.
///
/// The error tells why the C++/CLI component of Visual Studio is not available.
pub fn clr(build: &Build<'_>, plan: &mut Plan) -> Result<(), String> {
    // The C++/CLI support component installs the libraries of the mixed-mode startup code.
    if build.environment.find_file("LIB", "msvcmrt.lib").is_none() {
        return Err(format!(
            "msvcmrt.lib is not in the LIB directories of {}; install the Visual Studio \
             component \"C++/CLI support\"",
            build.label
        ));
    }
    single_source(build, plan, CLR_SOURCE);
    Ok(())
}

/// Plan the program `demo.exe` from `annotation/annotation.cpp`.
pub fn annotation(build: &Build<'_>, plan: &mut Plan) {
    single_source(build, plan, ANNOTATION_SOURCE);
}

/// Compile one C++ source below `fixtures/native` and link `demo.exe` and `demo.pdb`.
fn single_source(build: &Build<'_>, plan: &mut Plan, source: &str) {
    plan.steps.push(build.compile("/std:c++20", &[], &[source]));
    plan.steps.push(build.link(
        &[],
        &build.fixture.join("demo.exe"),
        &build.fixture.join("demo.pdb"),
        &[build.object(source)],
    ));
    plan.outputs = vec![
        build.output("demo.exe", Kind::Image),
        build.output("demo.pdb", Kind::NativePdb),
    ];
}
