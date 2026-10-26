# PDB fixtures

This directory holds small demo programs. The `pdbrs-fixtures` generator builds them with
different producers. The resulting PDBs are local test inputs for the native PDB reader and
for future Portable PDB work.

The demos use record kinds that Rust programs do not produce. Examples are C++ virtual
inheritance, bitfields, a field list with `LF_INDEX` continuations, thread storage, structured
exception handling and .NET async state machines.

## Sources

| Directory | Contents |
| --- | --- |
| `native/` | C++20 and C17 sources for MSVC and `clang-cl`: `main.cpp`, `types.cpp`, `control.cpp`, `big_enum.cpp`, `c_api.c` and two headers. |
| `rust/` | A standalone Cargo project. Its empty `[workspace]` table keeps it out of the root workspace. |
| `dotnet/csharp/` | A C# console project with three source files. |
| `dotnet/vb/` | A Visual Basic console project. |
| `dotnet/fsharp/` | An F# console project. It needs the `FSharp.Core` package from NuGet. |

Each program runs without arguments, prints a checksum and exits with code 0. MSVC-only
constructs are behind `#if defined(_MSC_VER) && !defined(__clang__)`. The `__m128` and
`__vectorcall` code is only in x86 and x64 builds.

## Matrix

| Fixture | Producer | Architecture | Configuration |
| --- | --- | --- | --- |
| `msvc-x64-debug` | `cl` + `link` | x64 | `/Zi /Od`, `/DEBUG:FULL` |
| `msvc-x64-release` | `cl` + `link` | x64 | `/Zi /O2`, `/DEBUG:FULL /OPT:REF /OPT:ICF` |
| `msvc-x64-incremental` | `cl` + `link` | x64 | `/ZI /Od`, `/DEBUG:FULL /INCREMENTAL` |
| `msvc-x64-fastlink` | `cl` + `link` from Visual Studio 2022 | x64 | `/Zi /Od`, `/DEBUG:FASTLINK` |
| `msvc-x64-z7` | `cl` + `link` | x64 | `/Z7 /Od`, `/DEBUG:FULL` |
| `msvc-x86-debug` | `cl` + `link` | x86 | `/Zi /Od`, `/DEBUG:FULL` |
| `msvc-arm64-debug` | `cl` + `link` | ARM64 | `/Zi /Od`, `/DEBUG:FULL` |
| `clang-lld-x64-debug` | `clang-cl` + `lld-link` | x64 | `/Zi /Od`, `/DEBUG:FULL` |
| `clang-lld-x64-release` | `clang-cl` + `lld-link` | x64 | `/Zi /O2`, `/DEBUG:FULL /OPT:REF /OPT:ICF` |
| `clang-lld-x86-debug` | `clang-cl` + `lld-link` | x86 | `/Zi /Od`, `/DEBUG:FULL` |
| `clang-lld-arm64-debug` | `clang-cl` + `lld-link` | ARM64 | `/Zi /Od`, `/DEBUG:FULL` |
| `clang-link-x64-debug` | `clang-cl` + `link` | x64 | `/Zi /Od`, `/DEBUG:FULL` |
| `rust-link-x64-debug` | `rustc` + `link` | x86_64 | debug profile |
| `rust-link-x64-release` | `rustc` + `link` | x86_64 | release profile with full debug information |
| `rust-lld-x64-debug` | `rustc` + `lld-link` | x86_64 | debug profile |
| `rust-link-x86-debug` | `rustc` + `link` | i686 | debug profile |
| `dotnet-csharp-portable` | `dotnet build` | AnyCPU | `DebugType=portable` |
| `dotnet-csharp-embedded` | `dotnet build` | AnyCPU | `DebugType=embedded` |
| `dotnet-csharp-full` | `dotnet build` | AnyCPU | `DebugType=full` |
| `dotnet-csharp-embed-sources` | `dotnet build` | AnyCPU | `DebugType=portable`, `EmbedAllSources=true` |
| `dotnet-vb-portable` | `dotnet build` | AnyCPU | `DebugType=portable` |
| `dotnet-fsharp-portable` | `dotnet build` | AnyCPU | `DebugType=portable` |

The `clang-cl` and `lld-link` tools come from the LLVM component of Visual Studio, not from a
separate LLVM installation. MSVC 14.50 and later ignore `/DEBUG:FASTLINK` (warning LNK4315)
and write a full PDB. For this reason, `msvc-x64-fastlink` uses the tools of a Visual Studio
2022 installation. A FASTLINK PDB keeps the type records in the object files and in
`obj/compile.pdb`, so keep the `obj/` directory with the PDB.

## Run the generator

Run these commands from the workspace root. You do not need a developer command prompt.

```sh
cargo run -p pdbrs-fixtures -- --list
cargo run -p pdbrs-fixtures
cargo run -p pdbrs-fixtures -- --only msvc-x64-debug --only rust-lld-x64-debug
cargo run -p pdbrs-fixtures -- --out D:/pdb-fixtures
```

The generator does these steps:

1. It finds Visual Studio with `vswhere.exe`.
2. It runs `vcvarsall.bat` once for each architecture and keeps the environment.
3. It gives that environment to each compiler and linker process.
4. It deletes the directory of each selected fixture and builds the fixture again.
5. It classifies each output by its contents and writes `manifest.json`.

The generator exits with a failure code if a fixture with an available toolchain fails. A
missing toolchain does not cause a failure. The manifest records the fixture as `unavailable`
with the reason.

## Outputs

The default output root is `target/fixtures/` below the workspace root. Each fixture has its
own directory with a `build.log` file. Native builds put object files in `obj/`. Rust builds
use `target/` in the fixture directory as `CARGO_TARGET_DIR`. .NET builds use `artifacts/` in
the fixture directory as the artifacts path, so no `bin/` or `obj/` directory appears here.

`manifest.json` lists the selected fixtures of the last run. For each fixture it records:

- the name, producer, language, architecture and configuration
- the status (`built`, `failed` or `unavailable`) and the reason
- the tool versions, for example the `cl` banner, `clang-cl --version`, `rustc -V` and
  `dotnet --version`
- each command with its working directory, environment source and extra variables
- each output with its kind (`native-pdb`, `portable-pdb`, `embedded-in-image` or `image`),
  its path relative to the output root and its size in bytes

The output kind comes from the file contents: the MSF 7.0 signature, the `BSJB` signature or
an `MPDB` payload in the PE debug directory. The manifest also records the workspace commit
and the host.

The `consumer_tools` section of `manifest.json` copies the tool records of
`target/tools/manifest.json` (see [Consumer tools](#consumer-tools)). If that file is missing,
or a file of a built tool is missing, the section records the tool as `unavailable` with the
reason. The consumer tools never change the result of the fixture build. The section always
reads `target/tools/` below the workspace root, also with `--out`.

Do not commit generated files. The `target/` directory is ignored by git. The only build
input in this directory that Cargo writes is `rust/Cargo.lock`. It is committed, and the
generator builds with `--locked`, so Cargo does not change it.

## Consumer tools

The `tools` command makes two independent consumers of PDB files. Other work compares the
output of pdbrs with the output of these tools (#21).

| Tool | Source | Use |
| --- | --- | --- |
| `pdbtool` | The `pdbtool` crate 0.1.22 from crates.io. This is the version at the pinned `microsoft/pdb-rs` revision `2081a0dd` in `docs/reference/SOURCES.json`. | `check`, `pdz-encode`, `copy`, `compare`, `container` |
| `DIA2Dump` | The DIA SDK sample of the latest Visual Studio installation with the x64 C++ tools. | Dumps a PDB or an MSFZ file through `msdia140.dll`. |

Run this command from the workspace root. It takes no other arguments:

```sh
cargo run -p pdbrs-fixtures -- tools
```

The command does these steps:

1. It runs `cargo install pdbtool --version 0.1.22 --locked --force` with `--root target/tools`
   and `--target-dir target/tools/pdbtool-build`. A later run only links the executable again.
2. It reads the installed version from `cargo install --list`.
3. It finds Visual Studio with `vswhere.exe`.
4. It deletes `target/tools/dia2dump/` and copies `<VS>/DIA SDK/Samples/DIA2Dump` into it.
   It never writes to the Visual Studio installation.
5. In `DIA2Dump.cpp` of the copy, it replaces `CoCreateInstance(__uuidof(DiaSource), ...)`
   with `NoRegCoCreate(L"msdia140.dll", __uuidof(DiaSource), __uuidof(IDiaDataSource), ...)`.
   `msdia140.dll` is not registered, so `CoCreateInstance` fails with `0x80040154`.
6. In `stdafx.h` of the copy, it includes `diacreate.h` and links
   `<VS>/DIA SDK/lib/amd64/diaguids.lib` with `#pragma comment(lib, ...)`.
7. It builds the copy with `<VS>/MSBuild/Current/Bin/MSBuild.exe` for `Release|x64`. The
   platform toolset comes from the `Microsoft.VCToolsVersion.v<NNN>.default.txt` file of the
   installation, for example `v145` for Visual Studio 18.
8. It copies `<VS>/DIA SDK/bin/amd64/msdia140.dll` next to `DIA2Dump.exe`.
9. It writes `target/tools/manifest.json`.

The outputs are in `target/tools/bin/`: `pdbtool.exe`, `DIA2Dump.exe`, `DIA2Dump.pdb` and
`msdia140.dll`. Each tool writes its command output to `target/tools/<tool>.log`.

`target/tools/manifest.json` has the generator, the workspace commit, the host and one
record for each tool. Each record has:

- the name, the status (`built`, `failed` or `unavailable`) and the reason
- the path of the executable and the list of files that the tool needs
- the version: the crate version and `cargo -V` for `pdbtool`; the Visual Studio version, the
  platform toolset and the `msdia140.dll` file version for `DIA2Dump`
- the source: the crate, its repository and the pinned revision for `pdbtool`; the Visual
  Studio, sample, `diaguids.lib`, `msdia140.dll`, MSBuild and copy paths and the two changes
  for `DIA2Dump`
- each command with its working directory

A tool without a prerequisite, for example Visual Studio or the DIA SDK, is `unavailable`. A
tool whose step fails, for example `cargo install` without network access, is `failed`. The
command writes the manifest in all cases. It exits with a failure code only if a tool is
`failed`.

Use the tools like this:

```sh
target/tools/bin/pdbtool check target/fixtures/msvc-x64-debug/demo.pdb
target/tools/bin/pdbtool pdz-encode target/fixtures/msvc-x64-debug/demo.pdb target/scratch/demo-msfz.pdb
target/tools/bin/DIA2Dump -m target/scratch/demo-msfz.pdb
```

`pdbtool check` exits with code 0 also when a PDB has errors. Read the line
`Number of PDBs with errors` of its output. Give an MSFZ file a name that ends in `.pdb`.
Otherwise DIA2Dump reads the file as an executable. `llvm-pdbutil` does not read MSFZ files.

## Read the fixtures

Run the full native reader on each native PDB that `manifest.json` lists:

```sh
cargo run -p pdbrs --example native -- target/fixtures/msvc-x64-debug/demo.pdb
```

The reader prints counts, undecoded record kinds and producer inconsistencies. It exits with a
failure code while any record kind, subsection kind or stream has no typed decoder. GitHub issues
#13 (MSVC and Clang C/C++) and #14 (.NET Windows PDBs) list the kinds that these fixtures still
contain. `format::probe` identifies the portable PDBs as CLI metadata; GitHub issue #17 tracks
Portable PDB parsing.
