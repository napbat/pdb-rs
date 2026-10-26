//! The fixture matrix: one entry for each producer, architecture and configuration.

mod recipe;

pub use recipe::{
    Arch, Compiler, Dotnet, Encoder, Fixture, Hlsl, Linker, Native, Pdz, Recipe, Rust,
    ShaderCompiler, Studio, Variant,
};

use crate::classify::Kind;

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

const DEBUG: &[&str] = &["/Zi", "/Od", "/MDd"];
const RELEASE: &[&str] = &["/Zi", "/O2", "/MD"];
const LINK_DEBUG: &[&str] = &["/DEBUG:FULL", "/INCREMENTAL:NO"];
const LINK_RELEASE: &[&str] = &["/DEBUG:FULL", "/OPT:REF", "/OPT:ICF", "/INCREMENTAL:NO"];
// `link` and `lld-link` accept the page sizes 4096, 8192, 16384 and 32768 only.
const LINK_PAGE_8192: &[&str] = &["/DEBUG:FULL", "/INCREMENTAL:NO", "/PDBPAGESIZE:8192"];
const LINK_PAGE_16384: &[&str] = &["/DEBUG:FULL", "/INCREMENTAL:NO", "/PDBPAGESIZE:16384"];
const LINK_PAGE_32768: &[&str] = &["/DEBUG:FULL", "/INCREMENTAL:NO", "/PDBPAGESIZE:32768"];
// The undocumented option /DEBUG:LAZY writes the Info feature NOTM: the module type streams keep
// the type records and the symbols of the object files.
const LINK_LAZY: &[&str] = &["/DEBUG:LAZY", "/INCREMENTAL:NO"];
// amp.h stops with an error for C++17 unless the program accepts the deprecation of C++ AMP.
const AMP_DEBUG: &[&str] = &["/Zi", "/Od", "/MDd", "/D_SILENCE_AMP_DEPRECATION_WARNINGS"];

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

/// Build a MASM recipe: assemble with `/Zi` and link with `link`.
const fn masm(arch: Arch) -> Native {
    Native {
        compiler: Compiler::Masm,
        variant: Variant::Masm,
        ..msvc(arch, &["/Zi"], LINK_DEBUG)
    }
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
    // The local producers of #39: each fixture holds record kinds or stream forms that the
    // fixtures above do not have.
    native(
        "msvc-x64-amp",
        "msvc (vs 2022)",
        "debug, c++ amp",
        // MSVC 14.50 removed amp.h, so this fixture needs the 14.4x tools.
        in_studio(
            Studio::Vs2022,
            with_variant(Variant::Amp, msvc(Arch::X64, AMP_DEBUG, LINK_DEBUG)),
        ),
    ),
    native(
        "msvc-x64-amp-lazy",
        "msvc (vs 2022)",
        "debug, c++ amp, lazy (no type merge)",
        // Only /DEBUG:LAZY keeps the S_LPROC32_DPC_ID records of the object file.
        in_studio(
            Studio::Vs2022,
            with_variant(Variant::Amp, msvc(Arch::X64, AMP_DEBUG, LINK_LAZY)),
        ),
    ),
    native(
        "msvc-x64-modern",
        "msvc",
        "debug, c++20 coroutine and structured bindings",
        with_variant(Variant::Modern, msvc(Arch::X64, DEBUG, LINK_DEBUG)),
    ),
    native(
        "msvc-x86-modern-zi",
        "msvc",
        "debug, edit and continue, c++20 coroutine and structured bindings",
        with_variant(
            Variant::Modern,
            msvc(
                Arch::X86,
                &["/ZI", "/Od", "/MDd"],
                &["/DEBUG:FULL", "/INCREMENTAL"],
            ),
        ),
    ),
    native(
        "msvc-x64-modern-lazy",
        "msvc",
        "release, lazy (no type merge), c++20 function-local statics",
        // `link` drops the S_STATICLOCAL records of the object file unless /DEBUG:LAZY is set.
        with_variant(Variant::Modern, msvc(Arch::X64, RELEASE, LINK_LAZY)),
    ),
    native(
        "masm-x64-debug",
        "masm (ml64) + link",
        "debug",
        masm(Arch::X64),
    ),
    native(
        "masm-x86-debug",
        "masm (ml) + link",
        "debug",
        masm(Arch::X86),
    ),
    native(
        "msvc-arm64ec-debug",
        "msvc",
        "debug, arm64ec",
        with_variant(
            Variant::Hybrid,
            msvc(
                Arch::Arm64Ec,
                &["/Zi", "/Od", "/MDd", "/arm64EC"],
                &["/DEBUG:FULL", "/INCREMENTAL:NO", "/MACHINE:ARM64EC"],
            ),
        ),
    ),
    native(
        "clang-flang-lld-x64-primitives",
        "clang-cl + flang + lld-link",
        "debug, primitive types of c and fortran",
        // -mlong-double-80 gives a 16-byte long double, which LLVM writes as T_REAL128.
        with_variant(
            Variant::Primitives,
            clang(
                Linker::Lld,
                Arch::X64,
                &["/Zi", "/Od", "/MD", "/clang:-mlong-double-80"],
                LINK_DEBUG,
            ),
        ),
    ),
    native(
        "msvc-x64-sourcelink",
        "msvc",
        "debug, sourcelink",
        with_variant(Variant::SourceLink, msvc(Arch::X64, DEBUG, LINK_DEBUG)),
    ),
    native(
        "msvc-x64-fastlink-full",
        "msvc (vs 2022) + mspdbcmf",
        "debug, fastlink converted to a full pdb",
        // `mspdbcmf` writes TPI hash adjusters. MSVC 14.50 and later ignore /DEBUG:FASTLINK.
        in_studio(
            Studio::Vs2022,
            with_variant(
                Variant::FastLinkConverted,
                msvc(Arch::X64, DEBUG, &["/DEBUG:FASTLINK", "/INCREMENTAL:NO"]),
            ),
        ),
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
