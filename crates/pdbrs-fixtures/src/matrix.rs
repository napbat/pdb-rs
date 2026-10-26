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
}

impl Variant {
    /// The source languages.
    pub const fn language(self) -> &'static str {
        match self {
            Self::Dll | Self::Annotation | Self::Generated => "c++20",
            Self::Clr => "c++/cli",
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
    /// except [`PDZ_EXCLUDED`].
    pub sources: &'static [&'static str],
}

/// The fixtures that a PDZ fixture does not convert, even with an empty source list.
///
/// `pdbtool` does not open the DXC PDBs: their stream pages are after the declared page
/// count ("Page is invalid; it is out of range").
pub const PDZ_EXCLUDED: &[&str] = &["hlsl-dxc-debug"];

/// The sources of the PDZ fixtures with one option: one fixture of each producer.
const PDZ_SAMPLE: &[&str] = &[
    "msvc-x64-debug",
    "clang-lld-x64-debug",
    "rust-link-x64-debug",
    "dotnet-csharp-full",
];

/// The sources of the `msfz-*` fixtures: [`PDZ_SAMPLE`] and a large PDB, whose streams cross
/// chunks also with the default chunk size.
const MSFZ_SAMPLE: &[&str] = &[
    "msvc-x64-debug",
    "clang-lld-x64-debug",
    "rust-link-x64-debug",
    "dotnet-csharp-full",
    "msvc-x64-large",
];

/// The sources of the `--pad16k` fixture: PDBs that give a PDZ file smaller than 16 KiB.
const PDZ_SMALL: &[&str] = &["hlsl-fxc-sm50-debug", "dotnet-csharp-full"];

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

const DEBUG: &[&str] = &["/Zi", "/Od", "/MDd"];
const RELEASE: &[&str] = &["/Zi", "/O2", "/MD"];
const LINK_DEBUG: &[&str] = &["/DEBUG:FULL", "/INCREMENTAL:NO"];
const LINK_RELEASE: &[&str] = &["/DEBUG:FULL", "/OPT:REF", "/OPT:ICF", "/INCREMENTAL:NO"];
// `link` and `lld-link` accept the page sizes 4096, 8192, 16384 and 32768 only.
const LINK_PAGE_8192: &[&str] = &["/DEBUG:FULL", "/INCREMENTAL:NO", "/PDBPAGESIZE:8192"];
const LINK_PAGE_16384: &[&str] = &["/DEBUG:FULL", "/INCREMENTAL:NO", "/PDBPAGESIZE:16384"];
const LINK_PAGE_32768: &[&str] = &["/DEBUG:FULL", "/INCREMENTAL:NO", "/PDBPAGESIZE:32768"];

const fn native(
    name: &'static str,
    producer: &'static str,
    configuration: &'static str,
    native: Native,
) -> Fixture {
    Fixture {
        name,
        producer,
        language: native.variant.language(),
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
        variant: Variant::Demo,
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
        variant: Variant::Demo,
    }
}

/// Build a native recipe with the tools of another Visual Studio installation.
const fn in_studio(studio: Studio, native: Native) -> Native {
    Native { studio, ..native }
}

/// Build a native recipe with other sources or steps.
const fn with_variant(variant: Variant, native: Native) -> Native {
    Native { variant, ..native }
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

const fn hlsl(
    name: &'static str,
    configuration: &'static str,
    compiler: ShaderCompiler,
    model: &'static str,
    options: &'static [&'static str],
) -> Fixture {
    let (producer, arch) = match compiler {
        ShaderCompiler::Fxc => ("fxc", "dxbc"),
        ShaderCompiler::Dxc => ("dxc", "dxil"),
    };
    Fixture {
        name,
        producer,
        language: "hlsl",
        arch,
        configuration,
        recipe: Recipe::Hlsl(Hlsl {
            compiler,
            model,
            options,
        }),
    }
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

const fn pdz(
    name: &'static str,
    configuration: &'static str,
    options: &'static [&'static str],
    sources: &'static [&'static str],
) -> Fixture {
    Fixture {
        name,
        producer: "pdbtool pdz-encode",
        language: "native PDB in MSFZ",
        arch: "any",
        configuration,
        recipe: Recipe::Pdz(Pdz {
            encoder: Encoder::PdzEncode,
            options,
            sources,
        }),
    }
}

const fn msfz(
    name: &'static str,
    configuration: &'static str,
    options: &'static [&'static str],
) -> Fixture {
    Fixture {
        name,
        producer: "msfz-encode",
        language: "native PDB in MSFZ",
        arch: "any",
        configuration,
        recipe: Recipe::Pdz(Pdz {
            encoder: Encoder::MsfzEncode,
            options,
            sources: MSFZ_SAMPLE,
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
    native(
        "msvc-x64-clr",
        "msvc",
        "debug, c++/cli",
        with_variant(
            Variant::Clr,
            msvc(Arch::X64, &["/clr", "/Zi", "/Od", "/MDd"], LINK_DEBUG),
        ),
    ),
    native(
        "msvc-x64-pgo",
        "msvc",
        "release, profile-guided optimization",
        with_variant(
            Variant::ProfileGuided,
            msvc(Arch::X64, &["/Zi", "/O2", "/MD", "/GL"], LINK_RELEASE),
        ),
    ),
    native(
        "msvc-x64-dll",
        "msvc",
        "debug, dll with exports",
        with_variant(Variant::Dll, msvc(Arch::X64, DEBUG, LINK_DEBUG)),
    ),
    native(
        "msvc-x64-xfg",
        "msvc (vs 2022)",
        "debug, xfg",
        // MSVC 14.50 and later accept /guard:xfg but write no XFG data, so this fixture needs
        // the 14.4x tools.
        in_studio(
            Studio::Vs2022,
            msvc(
                Arch::X64,
                &["/Zi", "/Od", "/MDd", "/guard:xfg"],
                &["/DEBUG:FULL", "/INCREMENTAL:NO", "/guard:xfg"],
            ),
        ),
    ),
    native(
        "msvc-x64-debugtype",
        "msvc",
        "debug, debugtype cv fixup pdata",
        msvc(
            Arch::X64,
            DEBUG,
            &[
                "/DEBUG:FULL",
                "/INCREMENTAL:NO",
                "/DEBUGTYPE:CV,FIXUP,PDATA",
            ],
        ),
    ),
    native(
        "msvc-arm64-debugtype",
        "msvc",
        "debug, debugtype cv fixup pdata",
        msvc(
            Arch::Arm64,
            DEBUG,
            &[
                "/DEBUG:FULL",
                "/INCREMENTAL:NO",
                "/DEBUGTYPE:CV,FIXUP,PDATA",
            ],
        ),
    ),
    native(
        "msvc-x64-stripped",
        "msvc",
        "debug, pdbstripped",
        with_variant(Variant::Stripped, msvc(Arch::X64, DEBUG, LINK_DEBUG)),
    ),
    native(
        "msvc-x64-natvis",
        "msvc",
        "debug, natvis",
        with_variant(Variant::Natvis, msvc(Arch::X64, DEBUG, LINK_DEBUG)),
    ),
    native(
        "msvc-x64-srcsrv",
        "msvc + pdbstr",
        "debug, srcsrv stream",
        with_variant(Variant::SourceServer, msvc(Arch::X64, DEBUG, LINK_DEBUG)),
    ),
    native(
        "msvc-x64-pch-z7",
        "msvc",
        "debug, z7, precompiled header",
        with_variant(
            Variant::PrecompiledHeader,
            msvc(Arch::X64, &["/Z7", "/Od", "/MDd"], LINK_DEBUG),
        ),
    ),
    native(
        "msvc-x64-pch-zi",
        "msvc",
        "debug, precompiled header",
        with_variant(
            Variant::PrecompiledHeader,
            msvc(Arch::X64, DEBUG, LINK_DEBUG),
        ),
    ),
    native(
        "msvc-x64-page8192",
        "msvc",
        "debug, pdb page size 8192",
        msvc(Arch::X64, DEBUG, LINK_PAGE_8192),
    ),
    native(
        "msvc-x64-page16384",
        "msvc",
        "debug, pdb page size 16384",
        msvc(Arch::X64, DEBUG, LINK_PAGE_16384),
    ),
    native(
        "msvc-x64-page32768",
        "msvc",
        "debug, pdb page size 32768",
        msvc(Arch::X64, DEBUG, LINK_PAGE_32768),
    ),
    native(
        "clang-lld-x64-page8192",
        "clang-cl + lld-link",
        "debug, pdb page size 8192",
        clang(Linker::Lld, Arch::X64, DEBUG, LINK_PAGE_8192),
    ),
    native(
        "clang-lld-x64-page16384",
        "clang-cl + lld-link",
        "debug, pdb page size 16384",
        clang(Linker::Lld, Arch::X64, DEBUG, LINK_PAGE_16384),
    ),
    native(
        "clang-lld-x64-page32768",
        "clang-cl + lld-link",
        "debug, pdb page size 32768",
        clang(Linker::Lld, Arch::X64, DEBUG, LINK_PAGE_32768),
    ),
    native(
        "msvc-x64-large",
        "msvc",
        "debug, generated sources, more than one free page map interval",
        with_variant(
            Variant::Generated,
            msvc(Arch::X64, &["/Zi", "/Od", "/MDd", "/MP"], LINK_DEBUG),
        ),
    ),
    native(
        "msvc-x64-sha1",
        "msvc",
        "debug, sha1 source checksums",
        msvc(Arch::X64, &["/Zi", "/Od", "/MDd", "/ZH:SHA1"], LINK_DEBUG),
    ),
    native(
        "msvc-x64-annotation",
        "msvc",
        "debug, __annotation",
        with_variant(Variant::Annotation, msvc(Arch::X64, DEBUG, LINK_DEBUG)),
    ),
    native(
        "msvc-x64-vc120",
        "msvc",
        "debug, vc120 pdb format",
        // The undocumented option /DEBUG:VC120 writes the Info feature VC110 instead of VC140.
        msvc(Arch::X64, DEBUG, &["/DEBUG:FULL,VC120", "/INCREMENTAL:NO"]),
    ),
    native(
        "msvc-x64-lazy",
        "msvc",
        "debug, lazy (no type merge)",
        // The undocumented option /DEBUG:LAZY writes the Info feature NOTM. The PDB has no type
        // records; the module symbols keep the type indices of the object files.
        msvc(Arch::X64, DEBUG, &["/DEBUG:LAZY", "/INCREMENTAL:NO"]),
    ),
    native(
        "clang-lld-x64-static",
        "clang-cl + lld-link",
        "debug, static runtime library",
        // The static runtime has objects from MASM (`ml64`). Some of their `LF_STRUCTURE`
        // records end with `f1` and two stale bytes instead of the `LF_PAD` run (#29).
        clang(Linker::Lld, Arch::X64, &["/Zi", "/Od", "/MTd"], LINK_DEBUG),
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
    // The Windows PDB of the VB demo: VB `Date` constants give `LF_DATE` in `S_MANCONSTANT`.
    dotnet(
        "dotnet-vb-full",
        "visual basic",
        "debug, full",
        ("vb", "VbDemo", "vbproj"),
        &["DebugType=full"],
        Some(Kind::NativePdb),
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
    hlsl(
        "hlsl-fxc-sm50-debug",
        "debug, shader model 5.0",
        ShaderCompiler::Fxc,
        "5_0",
        &["/Zi", "/Od"],
    ),
    hlsl(
        "hlsl-fxc-sm51-debug",
        "debug, shader model 5.1",
        ShaderCompiler::Fxc,
        "5_1",
        &["/Zi", "/Od"],
    ),
    hlsl(
        "hlsl-dxc-debug",
        "debug, shader model 6.0",
        ShaderCompiler::Dxc,
        "6_0",
        &["-Zi", "-Od"],
    ),
    // The PDZ fixtures read the outputs of the fixtures above, so they come last.
    pdz("pdz-default", "pdz-encode defaults", &[], &[]),
    pdz(
        "pdz-compress-stream-dir",
        "--compress-stream-dir",
        &["--compress-stream-dir"],
        PDZ_SAMPLE,
    ),
    pdz(
        "pdz-one-stream-per-chunk",
        "--one-stream-per-chunk",
        &["--one-stream-per-chunk"],
        PDZ_SAMPLE,
    ),
    // 4 KiB is the smallest chunk size of the writer. Large streams then cross chunks.
    pdz(
        "pdz-small-chunks",
        "--max-chunk-size 4K",
        &["--max-chunk-size", "4K"],
        PDZ_SAMPLE,
    ),
    pdz("pdz-pad16k", "--pad16k", &["--pad16k"], PDZ_SMALL),
    // `msfz-encode` writes the forms of Microsoft's writer that `pdz-encode` does not write.
    msfz(
        "msfz-deflate",
        "--chunk-codec deflate",
        &["--chunk-codec", "deflate"],
    ),
    msfz(
        "msfz-deflate-dir",
        "--stream-dir-codec deflate",
        &["--stream-dir-codec", "deflate"],
    ),
    msfz("msfz-uncompressed", "--uncompressed", &["--uncompressed"]),
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
    use super::{FIXTURES, PDZ_EXCLUDED, Recipe, select};
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

    #[test]
    fn pdz_sources_are_earlier_fixtures_that_are_not_conversions() {
        let position = |name: &str| FIXTURES.iter().position(|fixture| fixture.name == name);
        for name in PDZ_EXCLUDED {
            assert!(position(name).is_some(), "{name}");
        }
        for (index, fixture) in FIXTURES.iter().enumerate() {
            let Recipe::Pdz(pdz) = &fixture.recipe else {
                continue;
            };
            for source in pdz.sources {
                let source_index = position(source).expect(source);
                assert!(
                    source_index < index,
                    "{source} comes after {}",
                    fixture.name
                );
                assert!(!matches!(FIXTURES[source_index].recipe, Recipe::Pdz(_)));
            }
        }
    }
}
