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
| `native/` | C++20 and C17 sources for MSVC and `clang-cl`: `main.cpp`, `types.cpp`, `control.cpp`, `big_enum.cpp`, `c_api.c` and two headers. Inputs of the native variants: `demo.natvis` (Natvis visualizers) and `srcsrv.txt` (a `srcsrv` stream). |
| `native/dll/` | A small DLL for the `msvc-x64-dll` fixture: `exports.cpp` and the module-definition file `exports.def`. |
| `native/clr/` | A mixed C++/CLI program for the `msvc-x64-clr` fixture: `managed.cpp`. |
| `rust/` | A standalone Cargo project. Its empty `[workspace]` table keeps it out of the root workspace. |
| `dotnet/csharp/` | A C# console project with four source files: `Program.cs`, `AsyncWork.cs`, `Shapes.cs` and `Locals.cs`. |
| `dotnet/vb/` | A Visual Basic console project with two source files: `Program.vb` and `Tracing.vb`. |
| `dotnet/fsharp/` | An F# console project with three source files: `Tracing.fs`, `Workflows.fs` and `Program.fs`. It needs the `FSharp.Core` package from NuGet. |

Each program runs without arguments, prints a checksum and exits with code 0. MSVC-only
constructs are behind `#if defined(_MSC_VER) && !defined(__clang__)`. The `__m128` and
`__vectorcall` code is only in x86 and x64 builds.

### .NET demos

The .NET demos give the Portable PDB reader and writer data for each debug table (#23, #24,
#26). Each demo has these items:

- async methods and iterators
- more than one source document
- methods without sequence points: abstract methods, extern methods and compiler-generated
  members, for example the kickoff methods of state machines and record members

Each demo also has these language-specific items:

| Demo | Items |
| --- | --- |
| C# | Async and iterator state machines with `StateMachineMethod` rows and async stepping records. Local constants of many types, also an enum type from another assembly. Nested scopes, `dynamic` locals and tuple element names in a method without a state machine. Alias and `using static` imports, records with primary constructors, a `#line hidden` region. In `Mapping.cs`, `#line 100 "Mapped.template"` maps part of one method to another document, so its sequence points change documents (#23). The build with embedded sources embeds `Mapped.template` too. |
| Visual Basic | Async and iterator state machines with `StateMachineMethod` rows. `Static` locals, local constants (also `Date` and `Decimal`), file-level alias and XML namespace imports, project-level imports in `VbDemo.vbproj` and the default namespace from `RootNamespace`. In `Mapping.vb`, two `#ExternalSource` blocks map parts of one function to `Mapped.vb` and `MappedNext.vb`, so its sequence points change documents (#23). The VB compiler hides the other lines of that file. |
| F# | `task { }` state machines, `seq { }` classes, an `async { }` workflow, records and discriminated unions. The generator builds the F# demo with `Optimize=true`, because the F# compiler makes a `task { }` state machine only with optimization. The F# compiler writes no `StateMachineMethod` rows. |

With the argument `stack`, each .NET demo prints `System.Diagnostics.StackTrace(true)` and
exits with code 0. The stack goes through more than one source file, an async method and an
iterator. Use this mode to compare the file names and line numbers of a changed PDB with the
original PDB (#26):

```sh
dotnet target/fixtures/dotnet-csharp-portable/artifacts/bin/CSharpDemo/debug/CSharpDemo.dll stack
```

The output of the F# demo also has frames from `FSharp.Core`. Their file names and line numbers
come from the PDB of `FSharp.Core`, not from the PDB of the demo.

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
| `msvc-x64-clr` | `cl` + `link` | x64 | `/clr /Zi /Od`, `/DEBUG:FULL` |
| `msvc-x64-pgo` | `cl` + `link` | x64 | `/Zi /O2 /GL`, `/LTCG:PGINSTRUMENT`, a training run, `/LTCG:PGOPTIMIZE` |
| `msvc-x64-dll` | `cl` + `link` | x64 | `/Zi /Od`, `/DLL /DEF:exports.def` |
| `msvc-x64-xfg` | `cl` + `link` from Visual Studio 2022 | x64 | `/Zi /Od /guard:xfg`, `/DEBUG:FULL /guard:xfg` |
| `msvc-x64-debugtype` | `cl` + `link` | x64 | `/Zi /Od`, `/DEBUG:FULL /DEBUGTYPE:CV,FIXUP,PDATA` |
| `msvc-x64-stripped` | `cl` + `link` | x64 | `/Zi /Od`, `/DEBUG:FULL /PDBSTRIPPED` |
| `msvc-x64-natvis` | `cl` + `link` | x64 | `/Zi /Od`, `/DEBUG:FULL /NATVIS:demo.natvis` |
| `msvc-x64-srcsrv` | `cl` + `link` + `pdbstr` | x64 | `/Zi /Od`, `/DEBUG:FULL`, then `pdbstr -w -s:srcsrv` on a copy |
| `msvc-x64-pch-z7` | `cl` + `link` | x64 | `/Z7 /Od /Yc /Yu`, `/DEBUG:FULL` |
| `msvc-x64-pch-zi` | `cl` + `link` | x64 | `/Zi /Od /Yc /Yu`, `/DEBUG:FULL` |
| `rust-link-x64-debug` | `rustc` + `link` | x86_64 | debug profile |
| `rust-link-x64-release` | `rustc` + `link` | x86_64 | release profile with full debug information |
| `rust-lld-x64-debug` | `rustc` + `lld-link` | x86_64 | debug profile |
| `rust-link-x86-debug` | `rustc` + `link` | i686 | debug profile |
| `dotnet-csharp-portable` | `dotnet build` | AnyCPU | `DebugType=portable` |
| `dotnet-csharp-embedded` | `dotnet build` | AnyCPU | `DebugType=embedded` |
| `dotnet-csharp-full` | `dotnet build` | AnyCPU | `DebugType=full` |
| `dotnet-csharp-embed-sources` | `dotnet build` | AnyCPU | `DebugType=portable`, `EmbedAllSources=true` |
| `dotnet-vb-portable` | `dotnet build` | AnyCPU | `DebugType=portable` |
| `dotnet-fsharp-portable` | `dotnet build` | AnyCPU | `DebugType=portable`, `Optimize=true` |

The `clang-cl` and `lld-link` tools come from the LLVM component of Visual Studio, not from a
separate LLVM installation. MSVC 14.50 and later ignore `/DEBUG:FASTLINK` (warning LNK4315)
and write a full PDB. For this reason, `msvc-x64-fastlink` uses the tools of a Visual Studio
2022 installation. A FASTLINK PDB keeps the type records in the object files and in
`obj/compile.pdb`, so keep the `obj/` directory with the PDB.

### Native variants

The `msvc-x64-*` variants after `clang-link-x64-debug` give the native reader record kinds and
streams that the other fixtures do not contain (#27). Each variant is small. Most variants use
the sources of `native/`. The table gives the outputs and the items that only the variant
contains. The issue in the last column decodes these items.

| Fixture | Outputs | Items that only this fixture contains | Issue |
| --- | --- | --- | --- |
| `msvc-x64-clr` | `demo.exe`, `demo.pdb` | `S_GMANDATA` global symbols and `S_MANSLOT` module symbols. | #30 |
| `msvc-x64-pgo` | `demo.exe`, `demo.pdb` | `S_POGODATA`, `S_CALLERS`, `S_INLINESITE2` and `S_SEPCODE` (separated code) module symbols. | #30 |
| `msvc-x64-dll` | `demo.dll`, `demo.pdb` | `S_EXPORT` symbols with each flag: explicit ordinal, NONAME, DATA, CONSTANT, PRIVATE and forwarder. | #30 |
| `msvc-x64-xfg` | `demo.exe`, `demo.pdb` | None in the PDB. The object files in `obj/` have the XFG hash subsections `0xFF` and `0x100`. | #31 |
| `msvc-x64-debugtype` | `demo.exe`, `demo.pdb` | The optional debug streams Fixup (slot 2), Xdata (slot 7) and Pdata (slot 8). | #32 |
| `msvc-x64-stripped` | `demo.exe`, `demo.pdb`, `demo.stripped.pdb` | A stripped PDB: no `/names` stream and no type records. 51 of 58 modules have no stream. The other modules have only procedure stubs. The globals are only public symbols and procedure references. | #32 |
| `msvc-x64-natvis` | `demo.exe`, `demo.pdb` | None that is new. `demo.natvis` is an injected source (`/src/headerblock`, `/src/files/...`). The Rust fixtures have Natvis injected sources from `link.exe` and `lld-link` too, but no fixture of C++ code had them. | #32 |
| `msvc-x64-srcsrv` | `demo.exe`, `demo.pdb`, `demo.srcsrv.pdb` | The `srcsrv` named stream. `pdbstr` also increments the age in the PDB Info stream, but not in the DBI stream. | #32 |
| `msvc-x64-pch-z7` | `demo.exe`, `demo.pdb` | None in the PDB. The object files have `LF_ENDPRECOMP` (`obj/main.obj`) and `LF_PRECOMP` (the other C++ objects). | #29 |
| `msvc-x64-pch-zi` | `demo.exe`, `demo.pdb` | None in the PDB. The objects refer to `obj/compile.pdb` with `LF_TYPESERVER2`. | #29 |

Some notes about the variants:

- `msvc-x64-clr` needs the Visual Studio component "C++/CLI support". It compiles with `/EHa`,
  because `/clr` does not accept `/EHsc`. MSVC 14.51 writes no IL line subsections (`0xF9`), no
  metadata token map subsections (`0xFA`, `0xFB`) and no TokenRidMap stream for C++/CLI.
- `msvc-x64-pgo` runs the instrumented `demo.exe` once. The program loads `pgort140.dll` from the
  `PATH` of the `vcvarsall.bat` environment. The optimized link updates the PDB of the
  instrumented link, so the PDB keeps earlier stream directories.
- `msvc-x64-dll` exports `demo_dll_limit` with the obsolete keyword `CONSTANT`. The linker
  writes warning LNK4087, but it sets the constant flag of the `S_EXPORT` record.
- `msvc-x64-xfg` uses the tools of a Visual Studio 2022 installation. MSVC 14.50 and later
  accept `/guard:xfg` but write no XFG hashes. `link.exe` does not copy the XFG hash
  subsections from the objects to the PDB, also with `/Z7` or `/DEBUG:FASTLINK`.
- `msvc-x64-srcsrv` needs `pdbstr.exe` from the Debugging Tools for Windows, in
  `Debuggers\<host>\srcsrv` below the Windows SDK root (`WindowsSdkDir`). `pdbstr` changes the
  PDB in place, so the generator copies `demo.pdb` to `demo.srcsrv.pdb` first. The source file
  entries of `srcsrv.txt` use names, not the absolute paths of the build.
- The precompiled header variants compile `main.cpp` with `/Ycdemo.h` and the other C++ files
  with `/Yudemo.h`. The linker resolves the precompiled types, so the PDB records are the same
  kinds as in `msvc-x64-z7` and `msvc-x64-debug`.

A missing component or tool makes the fixture `unavailable` with the reason. The generator
checks for `msvcmrt.lib` in the `LIB` directories, for `pgort140.dll` on the `PATH` and for
`pdbstr.exe`.

These LLVM options do not give new PDB data, so the matrix has no fixture for them:

- `clang-cl -gcodeview-ghash` with `lld-link /DEBUG:GHASH`: the objects have `.debug$H`
  sections, but the TPI stream is the same as without global type hashes. Only the command
  line strings in the IPI stream are different.
- `lld-link /PDBSTRIPPED`: LLD ignores the option with a warning.
- `lld-link /DEBUGTYPE:CV,FIXUP,PDATA`: LLD writes no Fixup, Pdata or Xdata stream.
- `lld-link` with a module-definition file writes no `S_EXPORT` records.
- `clang-cl /guard:cf` with `lld-link /guard:cf`: no new record kinds.
- `clang-cl` precompiled headers (`/Yc`, `/Yu`): the objects have no `LF_PRECOMP` records.
- `lld-link /NATVIS`: the Rust fixtures that use `lld-link` already contain Natvis injected
  sources.

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
5. It classifies each output by its contents and writes `manifest.json`. With `--only`, it
   keeps the entries of the other fixtures from the earlier manifest.

The generator exits with a failure code if a fixture with an available toolchain fails. A
missing toolchain does not cause a failure. The manifest records the fixture as `unavailable`
with the reason.

## Outputs

The default output root is `target/fixtures/` below the workspace root. Each fixture has its
own directory with a `build.log` file. Native builds put object files in `obj/`. Rust builds
use `target/` in the fixture directory as `CARGO_TARGET_DIR`. .NET builds use `artifacts/` in
the fixture directory as the artifacts path, so no `bin/` or `obj/` directory appears here.

`manifest.json` lists the fixtures in matrix order. A run without `--only` writes the entries
of all fixtures. A run with `--only` reads the earlier `manifest.json` of the output root, if it
exists. It replaces or adds the entries of the selected fixtures and keeps the other entries.
The header fields, for example the commit and the time, always come from the last run. If the
generator cannot read or parse the earlier manifest, it writes a warning and lists only the
selected fixtures.

For each fixture the manifest records:

- the name, producer, language, architecture and configuration
- the status (`built`, `failed` or `unavailable`) and the reason
- the tool versions, for example the `cl` banner, `clang-cl --version`, `rustc -V` and
  `dotnet --version`
- each command with its working directory, environment source and extra variables
- each output with its kind (`native-pdb`, `portable-pdb`, `embedded-in-image`,
  `embedded-pdb-payload` or `image`), its path relative to the output root and its size in bytes

The output kind comes from the file contents: the MSF 7.0 signature, the `BSJB` signature, the
`MPDB` signature or an `MPDB` payload in the PE debug directory. The manifest also records the
workspace commit and the host.

For an `embedded-in-image` output, the generator writes the data of the debug directory entry
of type 17 to a file next to the image, with the extension `.mpdb` (for example
`CSharpDemo.mpdb`). The manifest records this file as an `embedded-pdb-payload` output. Tests
and examples read the payload from this file, so pdbrs does not need a PE parser.

The `consumer_tools` section of `manifest.json` copies the tool records of
`target/tools/manifest.json` (see [Consumer tools](#consumer-tools)). If that file is missing,
or a file of a built tool is missing, the section records the tool as `unavailable` with the
reason. The consumer tools never change the result of the fixture build. The section always
reads `target/tools/` below the workspace root, also with `--out`.

Do not commit generated files. The `target/` directory is ignored by git. The only build
input in this directory that Cargo writes is `rust/Cargo.lock`. It is committed, and the
generator builds with `--locked`, so Cargo does not change it.

## Consumer tools

The `tools` command makes three independent consumers of PDB files. Other work compares the
output of pdbrs with the output of these tools (#21, #22).

| Tool | Source | Use |
| --- | --- | --- |
| `pdbtool` | The `pdbtool` crate 0.1.22 from crates.io. This is the version at the pinned `microsoft/pdb-rs` revision `2081a0dd` in `docs/reference/SOURCES.json`. | `check`, `pdz-encode`, `copy`, `compare`, `container` |
| `DIA2Dump` | The DIA SDK sample of the latest Visual Studio installation with the x64 C++ tools. | Dumps a PDB or an MSFZ file through `msdia140.dll`. |
| `PortableVerifier` | The C# program in `dotnet/verifier/`. It uses `System.Reflection.Metadata` from the .NET shared framework, so it needs no NuGet package. | Prints the canonical dump of a Portable PDB (see [Portable PDB verifier](#portable-pdb-verifier)). |

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
9. It runs `dotnet build fixtures/dotnet/verifier/PortableVerifier.csproj -c Release` with
   `--artifacts-path target/tools/portable-verifier-build` and
   `-o target/tools/bin/portable-verifier`. Then it runs the verifier with `--version`.
10. It writes `target/tools/manifest.json`.

The outputs are in `target/tools/bin/`: `pdbtool.exe`, `DIA2Dump.exe`, `DIA2Dump.pdb`,
`msdia140.dll` and `portable-verifier/PortableVerifier.dll`. Each tool writes its command
output to `target/tools/<tool>.log`.

`target/tools/manifest.json` has the generator, the workspace commit, the host and one
record for each tool. Each record has:

- the name, the status (`built`, `failed` or `unavailable`) and the reason
- the path of the executable and the list of files that the tool needs
- the version: the crate version and `cargo -V` for `pdbtool`; the Visual Studio version, the
  platform toolset and the `msdia140.dll` file version for `DIA2Dump`; the .NET SDK version
  and the `System.Reflection.Metadata` and runtime versions for `PortableVerifier`
- the source: the crate, its repository and the pinned revision for `pdbtool`; the Visual
  Studio, sample, `diaguids.lib`, `msdia140.dll`, MSBuild and copy paths and the two changes
  for `DIA2Dump`; the project file for `PortableVerifier`
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

## Portable PDB verifier

`PortableVerifier` reads a Portable PDB with `System.Reflection.Metadata` (SRM) and prints a
canonical text dump. pdbrs prints the same format. Compare the two dumps of each `portable-pdb`
and each `embedded-pdb-payload` output of `manifest.json` with this command:

```sh
cargo run -p pdbrs-fixtures -- verify-portable [--fixtures <dir>]
```

For an `embedded-pdb-payload` output, pdbrs decodes the `.mpdb` file with
`portable::embedded::decode_embedded`. The verifier reads the `embedded-in-image` output of the
same fixture with `PEReader.ReadEmbeddedPortablePdbDebugDirectoryData`. The verifier also
decompresses the payload with `DeflateStream` to read the stream headers, and checks that its
length is equal to the SRM metadata length.

The command writes `<fixture>.pdbrs.txt` and `<fixture>.srm.txt` to `target/verify-portable/`.
It prints the line count and the number of different lines for each fixture, and the first
different lines. It exits with a failure code if a dump fails or the dumps differ. Run the
verifier alone with `dotnet target/tools/bin/portable-verifier/PortableVerifier.dll <file.pdb>`,
or with `--embedded <image.dll>` for an embedded Portable PDB.

SRM does not expose the stream headers, the `#~` header fields or the external row counts of
the `#Pdb` stream. The verifier reads these values from the bytes and checks them against SRM:
the heap offsets and sizes, the `#Pdb` offset, the PDB ID, the entry point, the local row
counts and the row size of each table. Each row size depends on the index widths, so this
check also confirms the widths. SRM does not expose the `VariableList` and `ConstantList`
columns or the `StateMachineMethod` rows. The verifier reads them from the row bytes and checks
them against the SRM views of the same data. A failed check stops the verifier with an error.

### Rebuild check

The `verify-rebuild` command checks the Portable PDB writer of pdbrs with the same inputs:

```sh
cargo run -p pdbrs-fixtures -- verify-rebuild [--fixtures <dir>]
```

For each input, the command checks these items and prints `yes` or `NO` for each one:

1. `portable::Pdb::copy_exact` gives the same bytes.
2. `Pdb::to_parts` and a plan with `IdPolicy::ContentDerived` give the same bytes and the same
   PDB ID. The fixtures are deterministic builds, so SRM computed the original ID with the
   same algorithm.
3. A second plan gives the same bytes. A plan with the original ID as an explicit ID gives the
   same bytes. The `MPDB` payload of the plan decodes to the same bytes.
4. The verifier dump of the rebuilt file is the same as the verifier dump of the original.

The command writes `<fixture>.pdb`, `<fixture>.original.srm.txt` and
`<fixture>.rebuilt.srm.txt` to `target/verify-rebuild/`. It exits with a failure code if a
check fails.

### Edit check

The `verify-edit` command checks the Portable PDB builder and editor of pdbrs with the same
inputs:

```sh
cargo run -p pdbrs-fixtures -- verify-edit [--fixtures <dir>]
```

For each input, the command checks these items:

1. `Pdb::edit` and `PdbEditor::plan` without a change give the original bytes.
2. `PdbBuilder::plan` of the same records writes new heaps. The decoded lines of its verifier
   dump (all lines after the `row` lines) are the same as those of the original.
3. An edit renames document 1 (suffix `.edited`), moves each visible sequence point 1000 lines
   down, adds the local constant `pdbrsAdded` (`i4=42`) to the last local scope and adds a
   custom debug information value of the unknown kind `70646272-7365-4564-6974-2d636865636b`
   to `MethodDef` 1. The same edit again gives the same bytes. The decoded lines of the
   verifier dump are the expected change of the original lines. The command compares the
   custom debug information lines without their row IDs, as a sorted list.
4. For a standalone fixture, the command copies the program directory to
   `target/verify-edit/<fixture>-<variant>/`, puts in the original, the built (item 2) or the
   moved PDB (item 3 without the other changes) and runs `dotnet <program>.dll stack`. The
   built and moved PDBs use the original PDB ID, so the runtime accepts them. The built PDB
   gives the original trace. The moved PDB gives each line number plus 1000 in the frames of
   the fixture documents; frames of other assemblies, for example `FSharp.Core`, do not change.

The command writes the PDBs, the dumps and the traces to `target/verify-edit/`. It exits with a
failure code if a check fails.

### Dump format

The dump is UTF-8 text with `\n` line ends. Each line has a keyword and `name=value` fields.
Counts, row IDs and GUID indexes are decimal. Offsets, sizes, flags, masks and tokens are
hexadecimal with a `0x` prefix. All offsets are from the start of the metadata root. The lines
come in this order:

| Keyword | Fields |
| --- | --- |
| `portable-dump` | `version`: the format version (1) |
| `metadata` | `version`: the version string in quotes (bytes outside printable ASCII, `"` and `\` as `\xNN`); `streams`: the stream count |
| `stream` | `name`, `offset`, `size`, for each stream header in file order |
| `heap` | `name`, `size`, for `#Strings`, `#US`, `#Blob` and `#GUID`; the size of an absent heap is 0 |
| `pdb` | `id`: the 20-byte PDB ID in hexadecimal; `entry-point`: the entry point token |
| `pdb-rows` | `table`, `rows`, for each external table of the `#Pdb` stream |
| `tables` | `version`, `heap-sizes`, `valid`, `sorted`: the `#~` header |
| `index` | `heap=<name> width`, `table=<name> width` or `coded=<name> width`: the index widths of the heaps, of `MethodDef`, `Document`, `LocalVariable`, `LocalConstant` and `ImportScope`, and of `HasCustomDebugInformation` |
| `table` | `name`, `id`, `rows`, `row-size` and `offset` (only if the table has rows), for the eight debug tables |
| `row` | `table`, `id` and the raw column values, for each row of the eight debug tables in table and row order |

The `row` columns use the names of the Portable PDB specification in lowercase with hyphens:
heap offsets and tokens are hexadecimal, and row IDs and GUID indexes are decimal (0 is nil).

The heap size is the size of its stream. SRM removes the zero padding after the last string of
`#Strings` from its heap size. The verifier accepts a smaller SRM size only if zero bytes follow
it.

#### Debug table lines

The decoded debug table sections come after the `row` lines and before the custom debug
information section (#23). pdbrs decodes the values with `portable::debug`. The verifier uses
the SRM views of documents, sequence points, scopes and imports. SRM does not decode local
constant signatures, so the verifier decodes them with `BlobReader` in
`dotnet/verifier/DebugDump.cs`. Names and text are quoted like the metadata version string.
GUIDs are lowercase text; a nil GUID index gives `00000000-0000-0000-0000-000000000000`.

| Keyword | Fields |
| --- | --- |
| `document` | `id`, `name` (the joined name), `language`, `hash-algorithm` (GUIDs), `hash` (bytes in hexadecimal), for each `Document` row |
| `method-debug` | `id`, `document` (the `Document` column), `local-signature` (the `StandAloneSig` row), `points` (the count), for each method with a sequence point blob |
| `sequence-point` | `method`, `il`, `document`, then `hidden` or `start-line`, `start-column`, `end-line`, `end-column` (decimal), after the `method-debug` line of the method |
| `scope` | `id`, `method`, `import-scope`, `start`, `end` (IL offsets), `variables`, `constants` (the run sizes), for each `LocalScope` row |
| `scope-variable` | `scope`, `id`, `index`, `attributes`, `name`, for each variable of the scope |
| `scope-constant` | `scope`, `id`, `name` and the decoded signature, for each constant of the scope |
| `import-scope` | `id`, `parent`, `imports` (the count), for each `ImportScope` row |
| `import` | `scope`, `kind` (1 to 9) and the fields of the kind: `alias` and `namespace` (quoted bytes), `assembly` (row), `type` (token) |
| `state-machine` | `move-next`, `kickoff` (`MethodDef` rows), in `MoveNext` order |

A decoded constant signature has a `modifier=required:<token>` or `modifier=optional:<token>`
field for each custom modifier, then the value:

- `boolean=true` or `boolean=false`; `char=0xNNNN`; `i1`, `u1`, `i2`, `u2`, `i4`, `u4`, `i8`,
  `u8` in decimal; `r4` and `r8` as the IEEE 754 bits in hexadecimal.
- `string=null` or `string="..."` (the UTF-16 text as UTF-8).
- An integer value with `enum=<token>` for an enum constant.
- `class=<token>` or `valuetype=<token>`, then `default`, `datetime=<ticks>` or
  `decimal=<sign>:<scale>:<high>:<mid>:<low>`.
- `object` for an `OBJECT` constant.

#### Custom debug information lines

The custom debug information section comes last. Each `CustomDebugInformation` row gives a
`cdi` line: `id`, `parent` (token), `kind` (the GUID as lowercase text) and `codec`. The detail
lines of its decoded value follow. Each detail line starts with the `id` of the row. pdbrs
decodes the values with `portable::custom`. The verifier decodes the SRM blobs with
`BlobReader` in `dotnet/verifier/CustomDebugInfoDump.cs`. IL offsets and sizes are hexadecimal;
syntax offsets, state numbers, slot kinds, ordinals and row IDs are signed decimal.

| `codec` | Detail lines and fields |
| --- | --- |
| `async-stepping` | `cdi-async`: `catch-handler` (or `none`); `cdi-async-step`: `yield`, `resume`, `method` |
| `hoisted-scopes` | `cdi-hoisted-scope`: `start`, `length` |
| `dynamic-locals` | `cdi-dynamic`: `flags`, one `0` or `1` for each stored bit, first flag first |
| `tuple-names` | `cdi-tuple-name`: `name` (quoted; empty for an element without a name) |
| `default-namespace`, `source-link` | `cdi-text`: `text` (quoted) |
| `enc-local-slots` | `cdi-enc-slot`: `temporary`, or `kind`, `offset` and `ordinal` (or `none`) |
| `enc-lambdas` | `cdi-enc-lambda-map`: `method-ordinal`; `cdi-enc-closure`: `offset`; `cdi-enc-lambda`: `offset`, `closure` (`this`, `static` or the closure index) |
| `enc-states` | `cdi-enc-state`: `number`, `offset` |
| `embedded-source` | `cdi-embedded-source`: `format` (`raw` or `deflate`), `size` and `fnv1a` (the 64-bit FNV-1a hash of the decoded source) |
| `metadata-references` | `cdi-reference`: `file`, `aliases` (quoted), `flags`, `time-stamp`, `file-size`, `mvid` (16 bytes in hexadecimal) |
| `compilation-options` | `cdi-option`: `name`, `value` (quoted) |
| `type-documents` | `cdi-type-document`: `document` |
| `primary-constructor` | No detail line |
| `unknown` | `cdi-raw`: `size`, `bytes` (in hexadecimal) |

#### Embedded source check lines

The embedded source check section comes after the custom debug information section. Each
embedded source of a document gives a `source` line, in `Document` row order: `document` (the
row ID), `cdi` (the `CustomDebugInformation` row ID), `size` (the decoded size) and `check`.
The `check` field is `verified` if the hash algorithm is SHA-256 and the SHA-256 digest of the
decoded source is equal to the document hash. It is `mismatch` if the digest differs. It is
`not-verified` for other hash algorithms. pdbrs uses `Pdb::document_source`. The verifier
decodes the blob with `DeflateStream` and computes the digest with `SHA256.HashData`.

To extend the format, add new keywords after the `row` lines. Add each new line type to the
verifier and to `crates/pdbrs-fixtures/src/verify/dump.rs` (or a module below it) together,
and increase the version only if an existing line changes.

### Format detection

Open each output of `manifest.json` with the format facade `pdbrs::Pdb` with this command:

```sh
cargo run -p pdbrs-fixtures -- detect [--fixtures <dir>]
```

It prints a Markdown table. A native PDB must open as `native`, and a Portable PDB as
`portable`. An image is not a PDB. The CLI metadata of a .NET image must give the error for
ordinary assembly metadata. An `MPDB` payload must give the error that names the embedded
adapter, and its decoded Portable PDB must open as `portable`. The command exits with a failure
code if a result differs.

## Read the fixtures

Run the full native reader on each native PDB that `manifest.json` lists:

```sh
cargo run -p pdbrs --example native -- target/fixtures/msvc-x64-debug/demo.pdb
```

The reader prints counts, undecoded record kinds and producer inconsistencies. It exits with a
failure code while any record kind, subsection kind or stream has no typed decoder. GitHub issues
#13 (MSVC and Clang C/C++) and #14 (.NET Windows PDBs) list the kinds that these fixtures still
contain.

The native variants contain kinds and streams without a decoder. [Native variants](#native-variants)
gives the issue (#29 to #32) that decodes each item.

Read every row and heap value of a Portable PDB with:

```sh
cargo run -p pdbrs --example portable -- target/fixtures/dotnet-csharp-portable/artifacts/bin/CSharpDemo/debug/CSharpDemo.pdb
```

The example prints the stream headers, the heap sizes, the `#Pdb` fields and the row counts. It
exits with a failure code at the first row or heap value that it cannot read. It decodes each
custom debug information value, encodes it again and counts the values that give the same
bytes (#24). It decodes the debug table blobs (#23). It decodes the embedded source of each
document and checks it against the SHA-256 document hash (#25). It exits with a failure code
if a source does not match its hash. It counts the sources with other hash algorithms as not
verified.

The example also reads an embedded PDB payload. It decodes the `MPDB` envelope first:

```sh
cargo run -p pdbrs --example portable -- target/fixtures/dotnet-csharp-embedded/artifacts/bin/CSharpDemo/debug/CSharpDemo.mpdb
```
