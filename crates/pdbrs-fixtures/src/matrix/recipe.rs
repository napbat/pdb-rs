//! The recipe types of the fixture matrix: producers, architectures and build steps.

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
    /// ARM64EC: x64-compatible ARM64 code (`cl /arm64EC`, `link /MACHINE:ARM64EC`). The
    /// ARM64 tools build it.
    Arm64Ec,
}

impl Arch {
    /// The name in fixture names and in the manifest.
    pub const fn name(self) -> &'static str {
        match self {
            Self::X64 => "x64",
            Self::X86 => "x86",
            Self::Arm64 => "arm64",
            Self::Arm64Ec => "arm64ec",
        }
    }

    /// The `clang-cl` target triple.
    pub fn clang_target(self) -> &'static str {
        match self {
            Self::X64 => "x86_64-pc-windows-msvc",
            Self::X86 => "i686-pc-windows-msvc",
            Self::Arm64 => "aarch64-pc-windows-msvc",
            Self::Arm64Ec => "arm64ec-pc-windows-msvc",
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
    /// The Microsoft macro assembler: `ml64.exe` for x64 and `ml.exe` for x86.
    Masm,
}

/// The linker of a native or Rust fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Linker {
    /// MSVC `link.exe`.
    Msvc,
    /// `lld-link.exe` from the Visual Studio LLVM component.
    Lld,
}

/// The sources and steps of a native fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// Compile the demo and link `demo.exe`.
    Demo,
    /// Like `Demo`, but `main.cpp` makes a precompiled header from `demo.h` (`/Yc`) and the
    /// other C++ files use it (`/Yu`).
    PrecompiledHeader,
    /// Like `Demo`, and the linker also writes `demo.stripped.pdb` (`/PDBSTRIPPED`).
    Stripped,
    /// Like `Demo`, and the linker adds `demo.natvis` to the PDB (`/NATVIS`).
    Natvis,
    /// Like `Demo`, then `pdbstr` adds a `srcsrv` stream to `demo.srcsrv.pdb`, a copy of the
    /// PDB.
    SourceServer,
    /// Profile-guided optimization of the demo: an instrumented link, a training run of the
    /// instrumented program and an optimized link.
    ProfileGuided,
    /// The DLL `dll/exports.cpp` with the exports of `dll/exports.def`.
    Dll,
    /// The C++/CLI program `clr/managed.cpp`.
    Clr,
    /// The program `annotation/annotation.cpp`, which calls the `__annotation` intrinsic.
    Annotation,
    /// A large program from C++ sources that the generator writes. Its PDB has more pages than
    /// one free page map interval covers.
    Generated,
    /// The C++ AMP program `amp/amp.cpp` (C++17). MSVC 14.50 removed `amp.h`.
    Amp,
    /// The C++20 program `modern/modern.cpp`: a coroutine, structured bindings, references
    /// to temporaries and function-local statics.
    Modern,
    /// The MASM program `masm/x64.asm` or `masm/x86.asm`, without a C runtime library.
    Masm,
    /// The ARM64EC program `arm64ec/hybrid.cpp`.
    Hybrid,
    /// The C program `primitives/primitives.c` (`clang-cl`) and the Fortran module
    /// `primitives/kinds.f90` (`flang` of the Visual Studio LLVM component), linked with
    /// `lld-link`.
    Primitives,
    /// Like `Demo`, and the linker adds the Source Link files `sourcelink/first.json` and
    /// `sourcelink/second.json` (`/SOURCELINK`).
    SourceLink,
    /// Like `Demo` with `/DEBUG:FASTLINK`, then `mspdbcmf` converts `demo.pdb` to a full PDB.
    FastLinkConverted,
}

impl Variant {
    /// The source languages.
    pub const fn language(self) -> &'static str {
        match self {
            Self::Dll | Self::Annotation | Self::Generated | Self::Modern | Self::Hybrid => "c++20",
            Self::Clr => "c++/cli",
            Self::Amp => "c++17 (c++ amp)",
            Self::Masm => "masm",
            Self::Primitives => "c17, fortran",
            _ => "c++20, c17",
        }
    }
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
    /// The sources and steps.
    pub variant: Variant,
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

/// An HLSL shader compiler from the Windows SDK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderCompiler {
    /// `fxc.exe`, the DXBC compiler for shader models 5.1 and older.
    Fxc,
    /// `dxc.exe`, the DXIL compiler for shader model 6.
    Dxc,
}

/// A build of each shader stage in `fixtures/hlsl` with one compiler.
#[derive(Debug)]
pub struct Hlsl {
    /// The compiler.
    pub compiler: ShaderCompiler,
    /// The shader model in profile form, for example `5_0`.
    pub model: &'static str,
    /// Compiler options for debug information and optimization.
    pub options: &'static [&'static str],
}

/// The program that writes the MSFZ file of a [`Pdz`] fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    /// `pdbtool pdz-encode`.
    PdzEncode,
    /// `msfz-encode` in `fixtures/tools/msfz-encode/`: Microsoft's `ms-pdb-msfz` writer with
    /// options that `pdz-encode` does not give.
    MsfzEncode,
}

impl Encoder {
    /// The tool name in the tools manifest.
    pub fn tool(self) -> &'static str {
        match self {
            Self::PdzEncode => "pdbtool",
            Self::MsfzEncode => "msfz-encode",
        }
    }

    /// The arguments before the options of the recipe.
    pub fn command(self) -> &'static [&'static str] {
        match self {
            Self::PdzEncode => &["--quiet", "pdz-encode"],
            Self::MsfzEncode => &[],
        }
    }
}

/// A conversion of the MSF 7.0 PDBs of other fixtures to MSFZ.
#[derive(Debug)]
pub struct Pdz {
    /// The program that writes the MSFZ files.
    pub encoder: Encoder,
    /// The options of the encoder.
    pub options: &'static [&'static str],
    /// The source fixtures. An empty list selects each fixture with an MSF 7.0 PDB output,
    /// except [`PDZ_EXCLUDED`](super::PDZ_EXCLUDED).
    pub sources: &'static [&'static str],
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
    /// An HLSL shader build.
    Hlsl(Hlsl),
    /// A conversion to MSFZ.
    Pdz(Pdz),
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
