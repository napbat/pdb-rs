//! The fixture matrix: one entry for each producer, architecture and configuration.

use crate::classify::Kind;

/// A target architecture of a native or Rust fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Arch {
    /// AMD64.
    X64,
    /// 32-bit x86.
    X86,
    /// ARM64.
    Arm64,
}

impl Arch {
    /// The name in fixture names and in the manifest.
    pub const fn name(self) -> &'static str {
        match self {
            Self::X64 => "x64",
            Self::X86 => "x86",
            Self::Arm64 => "arm64",
        }
    }

    /// The `clang-cl` target triple.
    pub fn clang_target(self) -> &'static str {
        match self {
            Self::X64 => "x86_64-pc-windows-msvc",
            Self::X86 => "i686-pc-windows-msvc",
            Self::Arm64 => "aarch64-pc-windows-msvc",
        }
    }

    /// The Rust target triple.
    pub fn rust_target(self) -> &'static str {
        // Rust and clang use the same triples for these targets.
        self.clang_target()
    }
}

/// The Visual Studio installation that provides the MSVC and LLVM tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Studio {
    /// The latest installation with the x64 C++ tools.
    Latest,
    /// The latest Visual Studio 2022 (version 17) installation with the x64 C++ tools.
    Vs2022,
}

impl Studio {
    /// The `vswhere.exe` options that select the installation.
    pub fn vswhere_filter(self) -> &'static [&'static str] {
        match self {
            Self::Latest => &["-latest"],
            Self::Vs2022 => &["-latest", "-version", "[17.0,18.0)"],
        }
    }

    /// The prefix of environment labels in the manifest.
    pub fn label(self) -> &'static str {
        match self {
            Self::Latest => "latest Visual Studio",
            Self::Vs2022 => "Visual Studio 2022",
        }
    }
}

/// The C and C++ compiler of a native fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compiler {
    /// MSVC `cl.exe`.
    Msvc,
    /// `clang-cl.exe` from the Visual Studio LLVM component.
    ClangCl,
}

/// The linker of a native or Rust fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Linker {
    /// MSVC `link.exe`.
    Msvc,
    /// `lld-link.exe` from the Visual Studio LLVM component.
    Lld,
}

/// A native C and C++ build.
#[derive(Debug, Clone, Copy)]
pub struct Native {
    /// The Visual Studio installation.
    pub studio: Studio,
    /// The compiler.
    pub compiler: Compiler,
    /// The linker.
    pub linker: Linker,
    /// The target architecture.
    pub arch: Arch,
    /// Compiler options for debug information, optimization and the runtime library.
    pub compile: &'static [&'static str],
    /// Linker options.
    pub link: &'static [&'static str],
}

/// A Cargo build of `fixtures/rust`.
#[derive(Debug)]
pub struct Rust {
    /// The linker.
    pub linker: Linker,
    /// The target architecture.
    pub arch: Arch,
    /// Build with the release profile.
    pub release: bool,
}

/// A `dotnet build` of one project in `fixtures/dotnet`.
#[derive(Debug)]
pub struct Dotnet {
    /// The project directory below `fixtures/dotnet`.
    pub directory: &'static str,
    /// The project file and assembly name, without extension.
    pub project: &'static str,
    /// The project file extension.
    pub extension: &'static str,
    /// Build properties in `Name=Value` form.
    pub properties: &'static [&'static str],
    /// The expected kind of the separate PDB, or `None` if the image embeds it.
    pub pdb: Option<Kind>,
}

/// How to build a fixture.
#[derive(Debug)]
pub enum Recipe {
    /// A native C and C++ build.
    Native(Native),
    /// A Rust build.
    Rust(Rust),
    /// A .NET build.
    Dotnet(Dotnet),
}

/// One fixture of the matrix.
#[derive(Debug)]
pub struct Fixture {
    /// The unique name. It is also the directory name below the output root.
    pub name: &'static str,
    /// The producer family.
    pub producer: &'static str,
    /// The source languages.
    pub language: &'static str,
    /// The architecture name.
    pub arch: &'static str,
    /// The configuration name.
    pub configuration: &'static str,
    /// How to build the fixture.
    pub recipe: Recipe,
}

const DEBUG: &[&str] = &["/Zi", "/Od", "/MDd"];
const RELEASE: &[&str] = &["/Zi", "/O2", "/MD"];
const LINK_DEBUG: &[&str] = &["/DEBUG:FULL", "/INCREMENTAL:NO"];
const LINK_RELEASE: &[&str] = &["/DEBUG:FULL", "/OPT:REF", "/OPT:ICF", "/INCREMENTAL:NO"];

const fn native(
    name: &'static str,
    producer: &'static str,
    configuration: &'static str,
    native: Native,
) -> Fixture {
    Fixture {
        name,
        producer,
        language: "c++20, c17",
        arch: native.arch.name(),
        configuration,
        recipe: Recipe::Native(native),
    }
}

const fn msvc(
    arch: Arch,
    compile: &'static [&'static str],
    link: &'static [&'static str],
) -> Native {
    Native {
        studio: Studio::Latest,
        compiler: Compiler::Msvc,
        linker: Linker::Msvc,
        arch,
        compile,
        link,
    }
}

const fn clang(
    linker: Linker,
    arch: Arch,
    compile: &'static [&'static str],
    link: &'static [&'static str],
) -> Native {
    Native {
        studio: Studio::Latest,
        compiler: Compiler::ClangCl,
        linker,
        arch,
        compile,
        link,
    }
}

/// Build a native recipe with the tools of another Visual Studio installation.
const fn in_studio(studio: Studio, native: Native) -> Native {
    Native { studio, ..native }
}

const fn rust(name: &'static str, linker: Linker, arch: Arch, release: bool) -> Fixture {
    Fixture {
        name,
        producer: if matches!(linker, Linker::Lld) {
            "rustc + lld-link"
        } else {
            "rustc + link"
        },
        language: "rust",
        arch: arch.name(),
        configuration: if release { "release" } else { "debug" },
        recipe: Recipe::Rust(Rust {
            linker,
            arch,
            release,
        }),
    }
}

const fn csharp(
    name: &'static str,
    configuration: &'static str,
    properties: &'static [&'static str],
    pdb: Option<Kind>,
) -> Fixture {
    dotnet(
        name,
        "c#",
        configuration,
        ("csharp", "CSharpDemo", "csproj"),
        properties,
        pdb,
    )
}

const fn dotnet(
    name: &'static str,
    language: &'static str,
    configuration: &'static str,
    (directory, project, extension): (&'static str, &'static str, &'static str),
    properties: &'static [&'static str],
    pdb: Option<Kind>,
) -> Fixture {
    Fixture {
        name,
        producer: "dotnet",
        language,
        arch: "anycpu",
        configuration,
        recipe: Recipe::Dotnet(Dotnet {
            directory,
            project,
            extension,
            properties,
            pdb,
        }),
    }
}

/// Every fixture, in build order.
pub static FIXTURES: &[Fixture] = &[
    native(
        "msvc-x64-debug",
        "msvc",
        "debug",
        msvc(Arch::X64, DEBUG, LINK_DEBUG),
    ),
    native(
        "msvc-x64-release",
        "msvc",
        "release",
        msvc(Arch::X64, RELEASE, LINK_RELEASE),
    ),
    native(
        "msvc-x64-incremental",
        "msvc",
        "debug, incremental",
        msvc(
            Arch::X64,
            &["/ZI", "/Od", "/MDd"],
            &["/DEBUG:FULL", "/INCREMENTAL"],
        ),
    ),
    native(
        "msvc-x64-fastlink",
        "msvc (vs 2022)",
        "debug, fastlink",
        // MSVC 14.50 and later ignore /DEBUG:FASTLINK, so this fixture needs the 14.4x tools.
        in_studio(
            Studio::Vs2022,
            msvc(Arch::X64, DEBUG, &["/DEBUG:FASTLINK", "/INCREMENTAL:NO"]),
        ),
    ),
    native(
        "msvc-x64-z7",
        "msvc",
        "debug, z7",
        msvc(Arch::X64, &["/Z7", "/Od", "/MDd"], LINK_DEBUG),
    ),
    native(
        "msvc-x86-debug",
        "msvc",
        "debug",
        msvc(Arch::X86, DEBUG, LINK_DEBUG),
    ),
    native(
        "msvc-arm64-debug",
        "msvc",
        "debug",
        msvc(Arch::Arm64, DEBUG, LINK_DEBUG),
    ),
    native(
        "clang-lld-x64-debug",
        "clang-cl + lld-link",
        "debug",
        clang(Linker::Lld, Arch::X64, DEBUG, LINK_DEBUG),
    ),
    native(
        "clang-lld-x64-release",
        "clang-cl + lld-link",
        "release",
        clang(Linker::Lld, Arch::X64, RELEASE, LINK_RELEASE),
    ),
    native(
        "clang-lld-x86-debug",
        "clang-cl + lld-link",
        "debug",
        clang(Linker::Lld, Arch::X86, DEBUG, LINK_DEBUG),
    ),
    native(
        "clang-lld-arm64-debug",
        "clang-cl + lld-link",
        "debug",
        clang(Linker::Lld, Arch::Arm64, DEBUG, LINK_DEBUG),
    ),
    native(
        "clang-link-x64-debug",
        "clang-cl + link",
        "debug",
        clang(Linker::Msvc, Arch::X64, DEBUG, LINK_DEBUG),
    ),
    rust("rust-link-x64-debug", Linker::Msvc, Arch::X64, false),
    rust("rust-link-x64-release", Linker::Msvc, Arch::X64, true),
    rust("rust-lld-x64-debug", Linker::Lld, Arch::X64, false),
    rust("rust-link-x86-debug", Linker::Msvc, Arch::X86, false),
    csharp(
        "dotnet-csharp-portable",
        "debug, portable",
        &["DebugType=portable"],
        Some(Kind::PortablePdb),
    ),
    csharp(
        "dotnet-csharp-embedded",
        "debug, embedded",
        &["DebugType=embedded"],
        None,
    ),
    csharp(
        "dotnet-csharp-full",
        "debug, full",
        &["DebugType=full"],
        Some(Kind::NativePdb),
    ),
    csharp(
        "dotnet-csharp-embed-sources",
        "debug, portable, embed all sources",
        &["DebugType=portable", "EmbedAllSources=true"],
        Some(Kind::PortablePdb),
    ),
    dotnet(
        "dotnet-vb-portable",
        "visual basic",
        "debug, portable",
        ("vb", "VbDemo", "vbproj"),
        &["DebugType=portable"],
        Some(Kind::PortablePdb),
    ),
    dotnet(
        "dotnet-fsharp-portable",
        "f#",
        "debug, portable, optimized",
        ("fsharp", "FSharpDemo", "fsproj"),
        // The F# compiler makes a state machine for `task { }` only with optimization.
        &["DebugType=portable", "Optimize=true"],
        Some(Kind::PortablePdb),
    ),
];

/// Select fixtures by name. An empty list selects every fixture.
pub fn select(names: &[String]) -> Result<Vec<&'static Fixture>, String> {
    if names.is_empty() {
        return Ok(FIXTURES.iter().collect());
    }
    if let Some(unknown) = names.iter().find(|name| find(name).is_none()) {
        return Err(format!(
            "unknown fixture `{unknown}`; use --list to see the names"
        ));
    }
    Ok(FIXTURES
        .iter()
        .filter(|fixture| names.iter().any(|name| name == fixture.name))
        .collect())
}

fn find(name: &str) -> Option<&'static Fixture> {
    FIXTURES.iter().find(|fixture| fixture.name == name)
}

#[cfg(test)]
mod tests {
    use super::{FIXTURES, select};
    use std::collections::BTreeSet;

    #[test]
    fn names_are_unique_directory_names() {
        let names: BTreeSet<_> = FIXTURES.iter().map(|fixture| fixture.name).collect();
        assert_eq!(names.len(), FIXTURES.len());
        assert!(names.iter().all(|name| {
            name.bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        }));
    }

    #[test]
    fn selection_keeps_matrix_order_and_rejects_unknown_names() {
        let names = ["rust-lld-x64-debug".to_owned(), "msvc-x64-debug".to_owned()];
        let selected: Vec<_> = select(&names)
            .unwrap()
            .iter()
            .map(|fixture| fixture.name)
            .collect();
        assert_eq!(selected, ["msvc-x64-debug", "rust-lld-x64-debug"]);
        assert!(select(&["nope".to_owned()]).unwrap_err().contains("`nope`"));
        assert_eq!(select(&[]).unwrap().len(), FIXTURES.len());
    }
}
