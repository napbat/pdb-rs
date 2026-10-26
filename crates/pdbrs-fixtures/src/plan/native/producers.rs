//! Plans of the native variants for the local producers of #39: C++ AMP, C++20 features,
//! MASM, ARM64EC, the primitive types of `clang-cl` and `flang`, and a FASTLINK PDB that
//! `mspdbcmf` converts to a full PDB.

use std::path::Path;

use super::variant::single_source;
use super::{Build, demo};
use crate::classify::Kind;
use crate::matrix::Arch;
use crate::plan::Plan;
use crate::tools::file_version::file_version;

/// The source of the C++ AMP variant.
const AMP_SOURCE: &str = "amp/amp.cpp";
/// The source of the C++20 variant.
const MODERN_SOURCE: &str = "modern/modern.cpp";
/// The source of the ARM64EC variant.
const HYBRID_SOURCE: &str = "arm64ec/hybrid.cpp";
/// The C source of the primitive types variant.
const PRIMITIVES_C_SOURCE: &str = "primitives/primitives.c";
/// The Fortran source of the primitive types variant.
const PRIMITIVES_FORTRAN_SOURCE: &str = "primitives/kinds.f90";

/// Plan the C++ AMP program `demo.exe`.
///
/// The error tells why `amp.h` is not available.
pub fn amp(build: &Build<'_>, plan: &mut Plan) -> Result<(), String> {
    if build.environment.find_file("INCLUDE", "amp.h").is_none() {
        return Err(format!(
            "amp.h is not in the INCLUDE directories of {}; C++ AMP needs MSVC 14.4x or older \
             (MSVC 14.50 removed it)",
            build.label
        ));
    }
    // The `restrict(amp)` lambdas do not compile with /std:c++20.
    single_source(build, plan, AMP_SOURCE, "/std:c++17");
    Ok(())
}

/// Plan the C++20 program `demo.exe` from `modern/modern.cpp`.
pub fn modern(build: &Build<'_>, plan: &mut Plan) {
    single_source(build, plan, MODERN_SOURCE, "/std:c++20");
}

/// Plan the ARM64EC program `demo.exe` from `arm64ec/hybrid.cpp`.
pub fn hybrid(build: &Build<'_>, plan: &mut Plan) {
    single_source(build, plan, HYBRID_SOURCE, "/std:c++20");
}

/// Plan the MASM program `demo.exe`: assemble one source, and link it with `kernel32.lib`
/// and no C runtime library.
///
/// The error tells that MASM has no source for the architecture.
pub fn masm(build: &Build<'_>, plan: &mut Plan) -> Result<(), String> {
    // The x86 source names its entry point in its END directive.
    let (source, entry) = match build.recipe.arch {
        Arch::X64 => ("masm/x64.asm", Some("mainCRTStartup")),
        Arch::X86 => ("masm/x86.asm", None),
        other => {
            return Err(format!(
                "the MASM variant has no source for {}; ml64 and ml assemble x64 and x86",
                other.name()
            ));
        }
    };
    let object = build.object(source);
    let mut arguments = vec!["/nologo".to_owned(), "/c".to_owned()];
    arguments.extend(
        build
            .recipe
            .compile
            .iter()
            .map(|option| (*option).to_owned()),
    );
    arguments.push(format!("/Fo{}", object.display()));
    arguments.push(build.source(source).display().to_string());
    plan.steps.push(build.step(&build.compiler).args(arguments));
    let mut extra = vec!["/SUBSYSTEM:CONSOLE".to_owned()];
    extra.extend(entry.map(|entry| format!("/ENTRY:{entry}")));
    extra.push("kernel32.lib".to_owned());
    plan.steps.push(build.link(
        &extra,
        &build.fixture.join("demo.exe"),
        &build.fixture.join("demo.pdb"),
        &[object],
    ));
    plan.outputs = vec![
        build.output("demo.exe", Kind::Image),
        build.output("demo.pdb", Kind::NativePdb),
    ];
    Ok(())
}

/// Plan the program `demo.exe` from `primitives/primitives.c` (`clang-cl`) and
/// `primitives/kinds.f90` (`flang`), linked with `lld-link`.
///
/// The Fortran module uses no Fortran runtime function, so the link ignores the default
/// libraries that `flang` names in the object file.
pub fn primitives(build: &Build<'_>, flang: &Path, plan: &mut Plan) {
    plan.steps
        .push(build.compile("/std:c17", &[], &[PRIMITIVES_C_SOURCE]));
    let object = build.object(PRIMITIVES_FORTRAN_SOURCE);
    plan.steps.push(
        build.step(flang).args([
            "-g".to_owned(),
            "-c".to_owned(),
            format!("--target={}", build.recipe.arch.clang_target()),
            "-module-dir".to_owned(),
            build.objects.display().to_string(),
            "-o".to_owned(),
            object.display().to_string(),
            build
                .source(PRIMITIVES_FORTRAN_SOURCE)
                .display()
                .to_string(),
        ]),
    );
    let extra = [
        "/NODEFAULTLIB:flang_rt.runtime.static.lib".to_owned(),
        "/NODEFAULTLIB:libcmt".to_owned(),
        format!(
            "/NODEFAULTLIB:clang_rt.builtins-{}.lib",
            build
                .recipe
                .arch
                .clang_target()
                .split('-')
                .next()
                .unwrap_or_default()
        ),
    ];
    plan.steps.push(build.link(
        &extra,
        &build.fixture.join("demo.exe"),
        &build.fixture.join("demo.pdb"),
        &[build.object(PRIMITIVES_C_SOURCE), object],
    ));
    plan.outputs = vec![
        build.output("demo.exe", Kind::Image),
        build.output("demo.pdb", Kind::NativePdb),
    ];
}

/// Plan the demo with `/DEBUG:FASTLINK`, then convert `demo.pdb` to a full PDB in place
/// with `mspdbcmf`.
///
/// The error tells why `mspdbcmf` is not available.
pub fn fastlink_converted(build: &Build<'_>, plan: &mut Plan) -> Result<(), String> {
    let converter = build
        .environment
        .find_program("mspdbcmf.exe")
        .ok_or_else(|| format!("mspdbcmf.exe is not on the PATH of {}", build.label))?;
    let version = std::fs::read(&converter)
        .ok()
        .and_then(|bytes| file_version(&bytes))
        .map_or_else(
            || "mspdbcmf, no file version".to_owned(),
            |version| format!("mspdbcmf file version {version}"),
        );
    plan.tools.push(("mspdbcmf".to_owned(), version));
    demo(build, plan);
    plan.steps.push(build.step(&converter).args([
        "/nologo".to_owned(),
        build.fixture.join("demo.pdb").display().to_string(),
    ]));
    Ok(())
}
