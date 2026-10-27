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
| `dotnet/csharp/` | A C# console project with six source files: `Program.cs`, `AsyncWork.cs`, `Shapes.cs`, `Locals.cs`, `Mapping.cs` and `Constants.cs`. |
| `dotnet/vb/` | A Visual Basic console project with three source files: `Program.vb`, `Tracing.vb` and `Mapping.vb`. |
| `dotnet/fsharp/` | An F# console project with three source files: `Tracing.fs`, `Workflows.fs` and `Program.fs`. It needs the `FSharp.Core` package from NuGet. |
| `hlsl/` | HLSL shaders for `fxc` and `dxc`: `vertex.hlsl`, `pixel.hlsl`, `compute.hlsl` and the shared header `common.hlsli`. |
| `tools/msfz-encode/` | Not a demo program: the standalone Cargo project of `msfz-encode`, the producer of the `msfz-*` fixtures (see [Consumer tools](#consumer-tools)). It uses Microsoft's `ms-pdb-msf` and `ms-pdb-msfz` crates, not pdbrs. |

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
| C# | Async and iterator state machines with `StateMachineMethod` rows and async stepping records. Local constants of many types, also an enum type from another assembly. Nested scopes, `dynamic` locals and tuple element names in a method without a state machine. Alias and `using static` imports, records with primary constructors, a `#line hidden` region. In `Mapping.cs`, `#line 100 "Mapped.template"` maps part of one method to another document, so its sequence points change documents (#23). The build with embedded sources embeds `Mapped.template` too. `Constants.cs` has a local constant of each constant type: `float` and `double` (also NaN, infinities and negative zero), `decimal` (also scales and the limits), `string` (also empty, non-ASCII and with a NUL), `char`, `sbyte` to `ulong` and `nint`/`nuint` at their limits, `bool`, null references and enums with `sbyte`, `int`, `long` and `ulong` values. In the Windows PDB, these give the numeric leaves `LF_REAL32`, `LF_REAL64`, `LF_DECIMAL`, `LF_VARSTRING`, `LF_CHAR` to `LF_UQUADWORD` and immediate values in `S_MANCONSTANT` (#39). |
| Visual Basic | Async and iterator state machines with `StateMachineMethod` rows. `Static` locals, local constants (also `Date` and `Decimal`), file-level alias and XML namespace imports, project-level imports in `VbDemo.vbproj` and the default namespace from `RootNamespace`. In `Mapping.vb`, two `#ExternalSource` blocks map parts of one function to `Mapped.vb` and `MappedNext.vb`, so its sequence points change documents (#23). The VB compiler hides the other lines of that file. `ConstantValues` in `Program.vb` has `Const` values of each constant type of Visual Basic. In the Windows PDB (`dotnet-vb-full`), a `Date` constant gives `LF_DATE` with the .NET tick count, not an OLE automation date (#39). |
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
| `msvc-arm64-debugtype` | `cl` + `link` | ARM64 | `/Zi /Od`, `/DEBUG:FULL /DEBUGTYPE:CV,FIXUP,PDATA` |
| `msvc-x64-stripped` | `cl` + `link` | x64 | `/Zi /Od`, `/DEBUG:FULL /PDBSTRIPPED` |
| `msvc-x64-natvis` | `cl` + `link` | x64 | `/Zi /Od`, `/DEBUG:FULL /NATVIS:demo.natvis` |
| `msvc-x64-srcsrv` | `cl` + `link` + `pdbstr` | x64 | `/Zi /Od`, `/DEBUG:FULL`, then `pdbstr -w -s:srcsrv` on a copy |
| `msvc-x64-pch-z7` | `cl` + `link` | x64 | `/Z7 /Od /Yc /Yu`, `/DEBUG:FULL` |
| `msvc-x64-pch-zi` | `cl` + `link` | x64 | `/Zi /Od /Yc /Yu`, `/DEBUG:FULL` |
| `msvc-x64-page8192` | `cl` + `link` | x64 | `/Zi /Od`, `/DEBUG:FULL /PDBPAGESIZE:8192` |
| `msvc-x64-page16384` | `cl` + `link` | x64 | `/Zi /Od`, `/DEBUG:FULL /PDBPAGESIZE:16384` |
| `msvc-x64-page32768` | `cl` + `link` | x64 | `/Zi /Od`, `/DEBUG:FULL /PDBPAGESIZE:32768` |
| `clang-lld-x64-page8192` | `clang-cl` + `lld-link` | x64 | `/Zi /Od`, `/DEBUG:FULL /PDBPAGESIZE:8192` |
| `clang-lld-x64-page16384` | `clang-cl` + `lld-link` | x64 | `/Zi /Od`, `/DEBUG:FULL /PDBPAGESIZE:16384` |
| `clang-lld-x64-page32768` | `clang-cl` + `lld-link` | x64 | `/Zi /Od`, `/DEBUG:FULL /PDBPAGESIZE:32768` |
| `msvc-x64-large` | `cl` + `link` | x64 | `/Zi /Od /MP` on generated sources, `/DEBUG:FULL` |
| `msvc-x64-sha1` | `cl` + `link` | x64 | `/Zi /Od /ZH:SHA1`, `/DEBUG:FULL` |
| `msvc-x64-annotation` | `cl` + `link` | x64 | `/Zi /Od`, `/DEBUG:FULL` |
| `msvc-x64-vc120` | `cl` + `link` | x64 | `/Zi /Od`, `/DEBUG:FULL,VC120` |
| `msvc-x64-lazy` | `cl` + `link` | x64 | `/Zi /Od`, `/DEBUG:LAZY` |
| `clang-lld-x64-static` | `clang-cl` + `lld-link` | x64 | `/Zi /Od /MTd`, `/DEBUG:FULL` |
| `rust-link-x64-debug` | `rustc` + `link` | x86_64 | debug profile |
| `rust-link-x64-release` | `rustc` + `link` | x86_64 | release profile with full debug information |
| `rust-lld-x64-debug` | `rustc` + `lld-link` | x86_64 | debug profile |
| `rust-link-x86-debug` | `rustc` + `link` | i686 | debug profile |
| `dotnet-csharp-portable` | `dotnet build` | AnyCPU | `DebugType=portable` |
| `dotnet-csharp-embedded` | `dotnet build` | AnyCPU | `DebugType=embedded` |
| `dotnet-csharp-full` | `dotnet build` | AnyCPU | `DebugType=full` |
| `dotnet-csharp-embed-sources` | `dotnet build` | AnyCPU | `DebugType=portable`, `EmbedAllSources=true` |
| `dotnet-vb-portable` | `dotnet build` | AnyCPU | `DebugType=portable` |
| `dotnet-vb-full` | `dotnet build` | AnyCPU | `DebugType=full` |
| `dotnet-fsharp-portable` | `dotnet build` | AnyCPU | `DebugType=portable`, `Optimize=true` |
| `hlsl-fxc-sm50-debug` | `fxc` | DXBC | `/Zi /Od`, profiles `vs_5_0`, `ps_5_0`, `cs_5_0` |
| `hlsl-fxc-sm51-debug` | `fxc` | DXBC | `/Zi /Od`, profiles `vs_5_1`, `ps_5_1`, `cs_5_1` |
| `hlsl-dxc-debug` | `dxc` | DXIL | `-Zi -Od`, profiles `vs_6_0`, `ps_6_0`, `cs_6_0` |

The `clang-cl` and `lld-link` tools come from the LLVM component of Visual Studio, not from a
separate LLVM installation. MSVC 14.50 and later ignore `/DEBUG:FASTLINK` (warning LNK4315)
and write a full PDB. For this reason, `msvc-x64-fastlink` uses the tools of a Visual Studio
2022 installation. A FASTLINK PDB keeps the type records in the object files and in
`obj/compile.pdb`, so keep the `obj/` directory with the PDB.

### Native variants

The native variants after `clang-link-x64-debug` give the native reader record kinds, streams
and layouts that the other fixtures do not contain (#1, #27). Each variant except
`msvc-x64-large` is small. Most variants use the sources of `native/`. The table gives the
outputs and the items that only the variant contains. The issue in the last column decodes
these items.

| Fixture | Outputs | Items that only this fixture contains | Issue |
| --- | --- | --- | --- |
| `msvc-x64-clr` | `demo.exe`, `demo.pdb` | `S_GMANDATA` global symbols and `S_MANSLOT` module symbols. | #30 |
| `msvc-x64-pgo` | `demo.exe`, `demo.pdb` | `S_POGODATA`, `S_CALLERS`, `S_INLINESITE2` and `S_SEPCODE` (separated code) module symbols. | #30 |
| `msvc-x64-dll` | `demo.dll`, `demo.pdb` | `S_EXPORT` symbols with each flag: explicit ordinal, NONAME, DATA, CONSTANT, PRIVATE and forwarder. | #30 |
| `msvc-x64-xfg` | `demo.exe`, `demo.pdb` | None in the PDB. The object files in `obj/` have the XFG hash subsections `0xFF` and `0x100`. | #31 |
| `msvc-x64-debugtype` | `demo.exe`, `demo.pdb` | The optional debug streams Fixup (slot 2), Xdata (slot 7) and Pdata (slot 8). | #32 |
| `msvc-arm64-debugtype` | `demo.exe`, `demo.pdb` | The same optional debug streams as `msvc-x64-debugtype`, with the ARM64 forms of the `.pdata` and `.xdata` data. | #32 |
| `msvc-x64-stripped` | `demo.exe`, `demo.pdb`, `demo.stripped.pdb` | A stripped PDB: no `/names` stream and no type records. 51 of 58 modules have no stream. The other modules have only procedure stubs. The globals are only public symbols and procedure references. | #32 |
| `msvc-x64-natvis` | `demo.exe`, `demo.pdb` | None that is new. `demo.natvis` is an injected source (`/src/headerblock`, `/src/files/...`). The Rust fixtures have Natvis injected sources from `link.exe` and `lld-link` too, but no fixture of C++ code had them. | #32 |
| `msvc-x64-srcsrv` | `demo.exe`, `demo.pdb`, `demo.srcsrv.pdb` | The `srcsrv` named stream. `pdbstr` also increments the age in the PDB Info stream, but not in the DBI stream. | #32 |
| `msvc-x64-pch-z7` | `demo.exe`, `demo.pdb` | None in the PDB. The object files have `LF_ENDPRECOMP` (`obj/main.obj`) and `LF_PRECOMP` (the other C++ objects). | #29 |
| `msvc-x64-pch-zi` | `demo.exe`, `demo.pdb` | None in the PDB. The objects refer to `obj/compile.pdb` with `LF_TYPESERVER2`. | #29 |
| `msvc-x64-page8192`, `msvc-x64-page16384`, `msvc-x64-page32768` | `demo.exe`, `demo.pdb` | An MSF page size of 8192, 16384 or 32768 bytes from `link`. | #1 |
| `clang-lld-x64-page8192`, `clang-lld-x64-page16384`, `clang-lld-x64-page32768` | `demo.exe`, `demo.pdb` | The same page sizes from `lld-link`. The superblock of `lld-link` selects free page map 2, the superblock of `link` selects free page map 1. | #1 |
| `msvc-x64-large` | `demo.exe`, `demo.pdb` | More than 4096 pages of 4096 bytes, so the free page map has a second interval. | #1 |
| `msvc-x64-sha1` | `demo.exe`, `demo.pdb` | SHA-1 file checksums (`DEBUG_S_FILECHKSMS` kind 2) for the demo sources. The runtime library sources keep SHA-256. | #31 |
| `msvc-x64-annotation` | `demo.exe`, `demo.pdb` | `S_ANNOTATION` module symbols and `S_ANNOTATIONREF` global symbols. | #30 |
| `msvc-x64-vc120` | `demo.exe`, `demo.pdb` | The PDB Info feature `VC110` (20091201) instead of `VC140`. | #32 |
| `msvc-x64-lazy` | `demo.exe`, `demo.pdb` | The PDB Info feature `NOTM` (no type merge). The TPI and IPI streams are empty. The module streams have `S_MOD_TYPEREF`, `S_GPROC32_ID`, `S_LPROC32_ID` and `S_PROC_ID_END` records. The type and ID indices of the module symbols refer to the records in the object files. | #30, #32 |
| `clang-lld-x64-static` | `demo.exe`, `demo.pdb` | The type records of the static runtime library (`/MTd`). MASM (`ml64`) wrote some of them: `LF_STRUCTURE` records such as `CsFrameEncoded` and `ThunkFrame` end with `f1` and two stale bytes, not with the `LF_PAD` run `f3 f2 f1`. | #29 |

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
- `link` and `lld-link` accept only the page sizes 4096, 8192, 16384 and 32768 for
  `/PDBPAGESIZE` (`link` error LNK1399; `lld-link` "invalid argument"). An MSF free page map
  interval covers as many pages as a page has bytes. Thus the smallest PDB with a second
  interval is larger than 16 MiB. The generator writes the sources of `msvc-x64-large` into
  `src/` in the fixture directory: 28000 structures and functions in 8 files and `main.cpp`.
- `/DEBUG:VC120` and `/DEBUG:LAZY` are options of `link` 14.51 that its help text does not
  show. `msvc-x64-lazy` keeps the type records in the object files, so keep the `obj/` directory
  with the PDB.
- `link /DEBUG:CTYPES` is accepted, but the DBI header flag `fCTypes` stays 0 with C and C++
  objects from `/Zi` and `/Z7`.

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

### HLSL shaders

The HLSL fixtures give the native reader the HLSL record kinds (#27). The shaders use a
constant buffer with `row_major` and `column_major` matrices and vectors, textures, samplers,
structured, RW and byte address buffers, `groupshared` memory, `min16float` and `half` types
and an inlined helper function. Each fixture compiles each shader stage to `<stage>.pdb` and
`<stage>.cso`. `fxc.exe` and `dxc.exe` come from the Windows SDK on the `PATH` of the x64
`vcvarsall.bat` environment. If a compiler is missing, the fixture is `unavailable`.

| Fixture | Items |
| --- | --- |
| `hlsl-fxc-sm50-debug` | `LF_VECTOR`, `LF_MATRIX`, `LF_MODIFIER_EX`, `LF_HLSL`, `LF_STRIDED_ARRAY`, `S_GDATA_HLSL`, `S_LDATA_HLSL` (`groupshared`), `S_DEFRANGE_HLSL`, `S_INLINESITE`, `S_ENVBLOCK` with the HLSL target. |
| `hlsl-fxc-sm51-debug` | The same kinds, but `S_GDATA_HLSL32_EX` and `S_LDATA_HLSL32_EX` instead of `S_GDATA_HLSL` and `S_LDATA_HLSL`. |
| `hlsl-dxc-debug` | No CodeView data. The TPI, DBI and IPI streams are empty. Stream 5 holds a DXIL container with the parts `ILDN`, `HASH`, `SRCI`, `VERS` and `ILDB`. |

Some producer properties of these PDBs:

- `fxc` writes MSF 7.0 PDBs with 512-byte pages. The names of `S_LOCAL` records have zero bytes
  after the terminating zero, not `LF_PAD` bytes. Some `LF_HLSL` records have nonzero bits in
  the unused part of the property field.
- `dxc` declares one page less in the MSF header than the streams use. `pdbtool check` and
  DIA2Dump reject these PDBs. `llvm-pdbutil` reads the summary and the stream list, but not the
  empty TPI stream.

### PDZ (MSFZ) conversions

The `pdz-*` fixtures convert the MSF 7.0 PDBs of the fixtures above to MSFZ with
`pdbtool pdz-encode` (pinned `microsoft/pdb-rs` at `2081a0dd`, #37). They come last in the
matrix, because they read the outputs of the other fixtures in the same output root. They need
`pdbtool` in the tools manifest. Each output is `<pdz fixture>/<source fixture>/<file name>`
with the `.pdb` extension of the source, because DIA opens only files with that extension.
The manifest gives the kind `native-pdz`, the option set as the configuration and each
`pdz-encode` command.

| Fixture | `pdz-encode` options | Sources |
| --- | --- | --- |
| `pdz-default` | none (defaults) | Each MSF 7.0 PDB of each fixture, except `hlsl-dxc-debug` (48 files) |
| `pdz-compress-stream-dir` | `--compress-stream-dir` | `msvc-x64-debug`, `clang-lld-x64-debug`, `rust-link-x64-debug`, `dotnet-csharp-full` |
| `pdz-one-stream-per-chunk` | `--one-stream-per-chunk` | the same four |
| `pdz-small-chunks` | `--max-chunk-size 4K` (the smallest size): large streams cross chunks | the same four |
| `pdz-pad16k` | `--pad16k` | `hlsl-fxc-sm50-debug` (3 files), `dotnet-csharp-full`: PDZ files smaller than 16 KiB |

`pdbtool` does not open the `hlsl-dxc-debug` PDBs ("Page is invalid; it is out of range"),
so no PDZ fixture converts them. `pdz-encode` writes stream 0 (the previous MSF stream
directory) as an empty stream; compare streams from stream 1, as `pdbtool compare` does.

### MSFZ forms of Microsoft's writer

The `msfz-*` fixtures hold MSFZ forms that `pdz-encode` does not write (#1). The `msfz-encode`
tool writes them with Microsoft's `MsfzWriter` of `ms-pdb-msfz` 0.1.10 (see
[Consumer tools](#consumer-tools)). The fixtures need `msfz-encode` in the tools manifest. The
output paths and the manifest entries are the same as for the `pdz-*` fixtures; the producer is
`msfz-encode`. `msfz-encode` copies each stream from stream 1 and writes stream 0 as an empty
stream, as `pdz-encode` does. It fills each chunk to the maximum chunk size (1 MiB by default).
Thus the streams of `msvc-x64-large` cross chunks.

| Fixture | `msfz-encode` options | Form |
| --- | --- | --- |
| `msfz-deflate` | `--chunk-codec deflate` | Raw DEFLATE chunks (code 2) and an uncompressed stream directory |
| `msfz-deflate-dir` | `--stream-dir-codec deflate` | zstd chunks and a raw DEFLATE stream directory (code 2) |
| `msfz-uncompressed` | `--uncompressed` | Each stream as uncompressed fragments; the file has no chunks and an empty Chunk Table |

Each fixture converts `msvc-x64-debug`, `clang-lld-x64-debug`, `rust-link-x64-debug`,
`dotnet-csharp-full` and `msvc-x64-large`.

DIA rejects the `msfz-deflate` and `msfz-deflate-dir` files (`loadDataFromPdb failed -
HRESULT = 806D000C`). DIA reads the `msfz-uncompressed` files and gives the same
`DIA2Dump -all` output as for the source. `pdbtool` and pdbrs read all three forms.

Microsoft's writer cannot make these forms of the MSFZ specification. Thus no fixture has them:

- A stored chunk (compression code 0). The `Compression` enum has only `Zstd` and `Deflate`,
  and `MsfzWriterFile::finish_current_chunk` compresses each chunk with one of them.
- A compressed fragment across chunk boundaries. `MsfzWriterFile::write_to_chunks` puts all
  bytes of one write call into one chunk, and `add_fragment_compressed` joins two fragments only
  in the same chunk. The reader of `ms-pdb-msfz` also rejects such a fragment.
- A stream larger than 4 GiB. `StreamWriter::write` limits a stream to less than
  `NIL_STREAM_SIZE` (`0xffffffff`) bytes.

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
  `embedded-pdb-payload` or `image`), its path relative to the output root, its size in bytes
  and the SHA-256 digest of its contents (`sha256`, 64 lowercase hexadecimal digits)

The generator calculates the digest with its own SHA-256 code
(`crates/pdbrs-fixtures/src/sha256.rs`). It does not use the `pdbrs` library or a host tool for
the digest.

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

Do not commit generated files. The `target/` directory is ignored by git. The build inputs in
this directory that Cargo writes are `rust/Cargo.lock` and `tools/msfz-encode/Cargo.lock`.
They are committed, and the generator builds with `--locked`, so Cargo does not change them.

## Consumer tools

The `tools` command makes three independent consumers of PDB files and one MSFZ producer.
Other work compares the output of pdbrs with the output of the consumers (#21, #22).

| Tool | Source | Use |
| --- | --- | --- |
| `pdbtool` | The `pdbtool` crate 0.1.22 from crates.io. This is the version at the pinned `microsoft/pdb-rs` revision `2081a0dd` in `docs/reference/SOURCES.json`. | `check`, `pdz-encode`, `copy`, `compare`, `container` |
| `DIA2Dump` | The DIA SDK sample of the latest Visual Studio installation with the x64 C++ tools. | Dumps a PDB or an MSFZ file through `msdia140.dll`. |
| `PortableVerifier` | The C# program in `dotnet/verifier/`. It uses `System.Reflection.Metadata` from the .NET shared framework, so it needs no NuGet package. | Prints the canonical dump of a Portable PDB (see [Portable PDB verifier](#portable-pdb-verifier)). |
| `msfz-encode` | The Cargo project in `tools/msfz-encode/`. It uses `ms-pdb-msf` 0.1.7 and `ms-pdb-msfz` 0.1.10 of the pinned `microsoft/pdb-rs` revision `2081a0dd`, not pdbrs. Its empty `[workspace]` table keeps it out of the root workspace, and its `Cargo.lock` is committed. | Writes an MSFZ file from an MSF 7.0 PDB with Microsoft's `MsfzWriter` (see [MSFZ forms of Microsoft's writer](#msfz-forms-of-microsofts-writer)). |

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
10. It runs `cargo install --path fixtures/tools/msfz-encode --locked --force` with
    `--root target/tools` and `--target-dir target/tools/msfz-encode-build`. Before the build, it
    reads the versions of `ms-pdb-msfz` and `ms-pdb-msf` from `Cargo.lock` of the project; other
    versions make the tool `failed`. Then it runs `msfz-encode --version`. If the project
    directory does not exist, the tool is `unavailable`.
11. It writes `target/tools/manifest.json`.

The outputs are in `target/tools/bin/`: `pdbtool.exe`, `DIA2Dump.exe`, `DIA2Dump.pdb`,
`msdia140.dll`, `portable-verifier/PortableVerifier.dll` and `msfz-encode.exe`. Each tool
writes its command output to `target/tools/<tool>.log`.

`target/tools/manifest.json` has the generator, the workspace commit, the host and one
record for each tool. Each record has:

- the name, the status (`built`, `failed` or `unavailable`) and the reason
- the path of the executable and the list of files that the tool needs
- the SHA-256 digest of each file (`sha256`, an object with the file path as the key)
- the version: the crate version and `cargo -V` for `pdbtool`; the Visual Studio version, the
  platform toolset and the `msdia140.dll` file version for `DIA2Dump`; the .NET SDK version
  and the `System.Reflection.Metadata` and runtime versions for `PortableVerifier`; the crate
  version, `cargo -V` and the `ms-pdb-msfz` and `ms-pdb-msf` versions for `msfz-encode`
- the source: the crate, its repository and the pinned revision for `pdbtool`; the Visual
  Studio, sample, `diaguids.lib`, `msdia140.dll`, MSBuild and copy paths and the two changes
  for `DIA2Dump`; the project file for `PortableVerifier`; the project file, the repository
  and the pinned revision for `msfz-encode`
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
target/tools/bin/msfz-encode --chunk-codec deflate target/fixtures/msvc-x64-debug/demo.pdb target/scratch/demo-deflate.pdb
```

`msfz-encode` has these options:

- `--chunk-codec zstd|deflate`: the codec of the chunks (default `zstd`)
- `--stream-dir-codec none|zstd|deflate`: the codec of the stream directory (default `none`)
- `--uncompressed`: write each stream as uncompressed fragments, without chunks
- `--max-chunk-size <bytes>`: the maximum uncompressed chunk size, with an optional `K` or `M`
  suffix (default 1M; the writer clamps the value to 4K..1G)
- `--pad16k`: make the file at least 16 KiB long

`pdbtool check` exits with code 0 also when a PDB has errors. Read the line
`Number of PDBs with errors` of its output. Give an MSFZ file a name that ends in `.pdb`.
Otherwise DIA2Dump reads the file as an executable. `llvm-pdbutil` does not read MSFZ files.

## Worker threads

The commands `detect`, `verify-portable`, `verify-rebuild`, `verify-edit`,
`verify-native-build`, `verify-native-edit`, `verify-msfz` and `verify-remap` do their work on
worker threads (#45). The option `--jobs <n>` sets the number of workers:

```sh
cargo run -p pdbrs-fixtures --release -- verify-native-build --jobs 8
```

- `<n>` must be a whole number greater than 0.
- The default is the available parallelism of the machine (`std::thread::available_parallelism`).
- `--jobs 1` runs all work on one thread, one item after the other, and starts no worker
  thread. Use it to find a defect that occurs only in parallel runs.

All levels of work in one command share the same `<n>` workers. For example,
`verify-native-build` runs these items on the same workers: the fixtures, the four consumer
checks of one fixture, the runs of one tool on the original and the rebuilt PDB, and the DIA
queries of one fixture. At most `<n>` threads do work at the same time.

Each work item writes only to the paths of its own fixture (and of its PDB file stem or option
set), so two workers never write the same file. A worker does not print. The command prints the
results in fixture order after the workers finish, so the output does not depend on the
schedule. The output of `--jobs 1` and of the default is the same, line for line.

All consumer tools (`pdbtool`, `DIA2Dump`, `llvm-pdbutil`, `llvm-symbolizer`, `pdbstr`, the
Portable PDB verifier and the fixture programs) run as separate processes with their own input
and output files. They accept parallel runs, also on the same input PDB, so no tool is limited
to one worker.

## Portable PDB verifier

`PortableVerifier` reads a Portable PDB with `System.Reflection.Metadata` (SRM) and prints a
canonical text dump. pdbrs prints the same format. Compare the two dumps of each `portable-pdb`
and each `embedded-pdb-payload` output of `manifest.json` with this command:

```sh
cargo run -p pdbrs-fixtures -- verify-portable [--fixtures <dir>] [--jobs <n>]
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
cargo run -p pdbrs-fixtures -- verify-rebuild [--fixtures <dir>] [--jobs <n>]
```

For each input, the command checks these items and prints `yes` or `NO` for each one:

1. `io::copy_exact` with `portable::Pdb::source` gives the same bytes.
2. `Pdb::to_parts` and a plan with `IdentityPolicy::ContentDerived` give the same bytes and the same
   PDB ID. The fixtures are deterministic builds, so SRM computed the original ID with the
   same algorithm.
3. A second plan gives the same bytes. A plan with the original ID as an explicit ID gives the
   same bytes. The `MPDB` payload of the plan decodes to the same bytes.
4. The verifier dump of the rebuilt file is the same as the verifier dump of the original.

The command writes `<fixture>.pdb`, `<fixture>.original.srm.txt` and
`<fixture>.rebuilt.srm.txt` to `target/verify-rebuild/`. It exits with a failure code if a
check fails.

### Edit check

The `verify-edit` command checks the Portable PDB editor of pdbrs with the same inputs:

```sh
cargo run -p pdbrs-fixtures -- verify-edit [--fixtures <dir>] [--jobs <n>]
```

For each input, the command checks these items:

1. `Pdb::edit` and `PdbEditor::plan` without a change give the original bytes.
2. A new `PdbEditor` (`PdbEditor::new`) with the same records writes new heaps. The decoded
   lines of its verifier dump (all lines after the `row` lines) are the same as those of the
   original.
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

After these items, the section `change and removal cases:` applies one change in each case
below (#42). Each case starts from `Pdb::edit` with `IdentityPolicy::ContentDerived` and changes
the first record that applies. A case that does not apply gives a `note:` line and no output.

| Case | Change |
| --- | --- |
| `sequence-point-line` | In method order, the first visible sequence point: start line + 1 and end line + 1. |
| `document-name` | Document 1: the name gets the suffix `.edited`. |
| `local-variable-name` | In row order, the first local variable: the name gets the suffix `.edited`. |
| `remove-local-constant` | In row order, the first local constant: removed. |
| `remove-custom-debug-information` | In row order, the first custom debug information value whose kind is not embedded source: removed. |

For each case, the command checks these items:

1. The decoded lines of the verifier dump are the expected change of the original lines: the
   changed line, or the removed lines with the later row IDs, the constant count of the scope
   and the `cdi` field of the `source` lines renumbered. For a removed custom debug information
   value, the command compares the custom debug information lines as in item 3.
2. The streams whose bytes are different from the original are exactly the streams that
   `PlanReport` does not give as `StreamChange::Copied`. `pdbtool compare` does not read
   Portable PDBs. Thus the command takes the stream headers (`stream` lines) from the verifier
   dumps of the original and the output, and compares the bytes of each stream.
3. The same edit again gives the same bytes.

After the section `change and removal cases:`, the section `task cases:` applies the editing
tasks of `PdbEditor` (#41), each from `Pdb::edit` with `IdentityPolicy::ContentDerived`:

| Case | Task |
| --- | --- |
| `source-link` | `set_source_link`: the Source Link record of the module gets a new JSON text. |
| `embed-source` | `embed_source` (DEFLATE): the first document without embedded source whose file below `fixtures/dotnet/` has the SHA-256 hash of the document. A note if the PDB has no such document. |
| `replace-embedded-source` | `embed_source`: the first document with embedded source gets its decoded source again in the other form (raw for DEFLATE, DEFLATE for raw). |

Each task case gets checks 2 and 3 above. For check 1, the command compares the lines without
custom debug information line for line, and the custom debug information values as in item 3.
The verifier decodes the Source Link text and each embedded source, and the `source` line of
an embedded source must give `check=verified` (the SHA-256 hash of the document).

After the task section, the section `remap cases:` (#46) runs `remap_source_paths` on each
input with the build root of `verify-native-edit` (see "Native edit check") and
`IdentityPolicy::ContentDerived`. It writes `<fixture>.remap-source-paths.pdb` and its verifier
dump. A function that keeps every name must give the source bytes. The decoded dump of the
output must be the source dump with the new name in each `document` line, line for line, so
the sequence points do not change. The case also gets checks 2 and 3 above.

The command writes the PDBs, the dumps and the traces to `target/verify-edit/`. A case writes
`<fixture>.<case>.pdb` and `<fixture>.<case>.srm.txt`, and a task case writes
`<fixture>.task-<case>.pdb` and `<fixture>.task-<case>.srm.txt`. The command exits with a
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
cargo run -p pdbrs-fixtures -- detect [--fixtures <dir>] [--jobs <n>]
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

### Input modes

The `native` and `portable` examples read the file in one of three modes. Select the mode with
`--input` before the path:

| Mode | Reader |
| --- | --- |
| `file` (default) | `Pdb::open`: positional reads through `pdbrs::io::FileReader` |
| `slice` | `Pdb::parse`: the example reads all file bytes, and the reader borrows them |
| `seek` | `Pdb::read` with `pdbrs::io::SeekReader` around `std::fs::File` |

```sh
cargo run -p pdbrs --example native -- --input seek target/fixtures/msvc-x64-debug/demo.pdb
```

For an `MPDB` payload, the `portable` example writes the decoded PDB to a temporary file. Then
it reads that file in the selected mode and deletes it. All modes must give the same output
(#36).

## Native build check

The `verify-native-build` command checks the native rebuild of pdbrs (#33, #42) with each
`native-pdb` output of `manifest.json`:

```sh
cargo run -p pdbrs-fixtures --release -- verify-native-build [--fixtures <dir>] [--jobs <n>]
```

The command skips `hlsl-dxc-debug`, because `pdbtool`, `llvm-pdbutil` and `DIA2Dump` reject
its original PDBs. For each other PDB, it does these steps:

1. Rebuild the PDB with `Pdb::edit` (`IdentityPolicy::Preserve`), `PdbEditor::decode_all` and
   `PdbEditor::plan` with `EditOptions::default()`. A second rebuild must give the same
   bytes. The rebuilt PDB has the file name of the original, in
   `target/verify-native-build/<fixture>/<PDB file stem>/rebuilt/`.
2. Run `pdbtool check` on the rebuilt PDB. The line `Number of PDBs with errors` must show 0.
   Run `pdbtool compare` on both PDBs to get the stream counts.
3. Run `llvm-pdbutil dump --all` on both PDBs. Remove the container layout before the
   comparison: the `Number of blocks` and `Blocks:` lines and the stream directory entries
   `Stream N (...)`. Replace the stream numbers of `Stream N, M bytes`, `debug stream: N` and
   `Stream Index: N` with `<n>`.
4. Run `DIA2Dump -all` on both PDBs. Replace each word of 16 hexadecimal digits (a pointer)
   with `<ptr>` before the comparison.
5. Select a sample from the `DIA2Dump -all` output of the original. The addresses are the start
   and the middle of each `Function` and `Data` range and each `PublicSymbol` address. While
   the sample has fewer than 200 addresses and the ranges have more, a pseudo-random generator
   with a fixed seed adds one address in each range. The sample keeps a maximum of 600
   addresses. The type names come from the `UserDefinedType:` lines, with a maximum of 250.
6. Run `DIA2Dump -sym 0x<rva>` and `-lines 0x<rva>` for each address and `-type <name>` for
   each type name on both PDBs, on the workers (see [Worker threads](#worker-threads)). The
   masked outputs must be the same.
7. For a fixture with an `image` output, run `llvm-symbolizer --inlines --relative-address
   --functions=linkage` with the sample addresses for each PDB. The command copies the PDB and
   the image into one directory for each PDB. It replaces the PDB path of the `RSDS` record of
   the image copy with a path that does not exist, so `llvm-symbolizer` loads the PDB next to
   the image. The frames of each address must be the same.

The command reads `pdbtool` and `DIA2Dump` from `target/tools/manifest.json` of the workspace.
If the workspace has no built tool, it reads the `consumer_tools` records of the fixture
manifest. It reads `llvm-pdbutil` and `llvm-symbolizer` from `%ProgramFiles%\LLVM\bin`. A
missing tool makes its checks fail; the other checks still run.

These differences are explained and do not make a check fail:

- Container layout: the `Number of streams` and `Block Size` lines. The rebuild drops nil
  streams and streams that nothing names.
- LLD public bucket order: `S_PUB32` lines of the `Public Symbols` section and `PublicSymbol:`
  lines that both outputs have at other positions. LLD puts some 8-byte names in another hash
  bucket; the rebuild uses the bucket of the name.
- The same failure: both `llvm-pdbutil` runs fail with the same exit code, for example for
  `msvc-x64-fastlink` and for `S_MANCONSTANT` in `dotnet-csharp-full`.

The command prints one line for each PDB. The line shows the rebuild, the `pdbtool` errors,
the classified `llvm-pdbutil` and `DIA2Dump -all` differences, the sample size, the mismatch
counts of the DIA queries and the symbolizer, and the source streams that the rebuild drops
(`PlanReport::dropped`). A sample with fewer than 200 addresses tells why. The
command writes the dumps (`*.original.txt`, `*.rebuilt.txt`) and the differences (`*.diff`) to
the directory of the PDB. It exits with a failure code if a rebuild fails, a check cannot run
or a difference is unexplained.

## Native edit check

The `verify-native-edit` command checks the native PDB editor of pdbrs (#34, #42) with each
`native-pdb` output of `manifest.json`:

```sh
cargo run -p pdbrs-fixtures --release -- verify-native-edit [--fixtures <dir>] [--jobs <n>]
```

The command skips `hlsl-dxc-debug`, as `verify-native-build` does. It applies each edit to
each other PDB with `Pdb::edit` and `PdbEditor::plan`:

| Edit | Changes | Identity |
| --- | --- | --- |
| `named` | Add or replace `srcsrv`, `sourcelink` and the raw stream `pdbrs-edit`. Remove `/LinkInfo` if present | `Preserve` |
| `add-symbols` | Add the public symbol `pdbrs_edit_public` and the `S_GDATA32` global `pdbrs_edit_global` | `IncrementAge` |
| `rename-symbols` | Append `_pdbrs_renamed` to a unique public name and to a unique data, constant or UDT global name. The later records move | `IncrementAge` |
| `types` | Add a field list, the structure `PdbrsEditType`, its `S_UDT` global and the `LF_STRING_ID` record `pdbrs edit string` | `IncrementAge` |
| `identity` | Write the signature `0x52444250`, the GUID `pdbrs-edit-guid!` and age 7 | `Replace` |
| `strip` | `EditOptions::with_strip_private(true)` | `Preserve` |

The edited PDB is `target/verify-native-edit/<fixture>/<PDB file stem>/<edit>.pdb`. For each
edited PDB, the command does these checks:

1. `pdbtool check` shows 0 PDBs with errors.
2. `llvm-pdbutil dump --all` and `DIA2Dump -all` succeed, or fail as on the source PDB.
3. `pdbtool compare --continue-on-differences` shows exactly the streams of
   `PlanReport::changed`, and the stream counts of the added streams.
4. The independent lookups of the edit:
   - `llvm-pdbutil --named-streams` lists the added streams and not the removed ones.
     `pdbstr -r -s:srcsrv` reads the written `srcsrv` text.
   - `llvm-pdbutil --publics` and `DIA2Dump -p` list the added and renamed public names, and
     not the old names. `llvm-pdbutil --globals` and `DIA2Dump -sym` find the added and renamed
     global names. For an old global name, `DIA2Dump -sym` finds one symbol less than on the
     source PDB.
   - `llvm-pdbutil --types` and `DIA2Dump -type` find `PdbrsEditType`, and `llvm-pdbutil --ids`
     finds the added string.
   - `llvm-pdbutil --summary` shows the new signature, age and GUID, and
     `PdbWritePlan::identity` gives the GUID and the age.

The command also compares the strip of `msvc-x64-stripped/demo.pdb` with
`demo.stripped.pdb`, which `link /PDBSTRIPPED` wrote. These `llvm-pdbutil` sections must be
equal: `--symbols`, `--section-contribs`, `--section-map`, `--types`, `--ids`, `--files`,
`--section-headers`, `--fpo` and `--type-stats`; `--summary` without the stream count;
`--named-streams` without the stream indices; `--modules` without the stream numbers; and the
records of `--globals` and `--publics` without their offsets.

The command prints each explained class with its PDB count. It exits with a failure for each
other difference, for each edit that the plan rejects and for each missing tool. The explained
classes are:

- A full dump that fails on the edited PDB as on the source (`msvc-x64-fastlink`, `dotnet-csharp-full`),
  or that succeeds where it fails on the source (the strip of `dotnet-csharp-full`).
- `llvm-pdbutil --publics` fails on the source PDB too (`msvc-x64-fastlink`). DIA2Dump finds the names.
- `DIA2Dump -t` lists no UDT of the source PDB (FASTLINK and managed PDBs), so `-type` finds no added type.
- The stripped PDB keeps the source stream IDs, and `link.exe` writes the stripped symbol
  records in another order, so the stream numbers and the symbol record offsets differ.

After the summary line of these edits, the command prints the section
`change and removal cases:` (#42). It runs 8 more cases on each PDB, each with
`IdentityPolicy::IncrementAge`. A case that does not apply to a PDB gives a `note:` line and no
output.

| Case | Change (the first record that applies) |
| --- | --- |
| `public-address` | The first public symbol with a unique name: address offset + 1 |
| `global-type` | The first `S_GDATA32` or `S_LDATA32` global with a unique name: type `T_INT4` (`T_UINT4` if it is `T_INT4`) |
| `local-type` | In module order, the first module symbol in pre-order of kind `S_REGREL32`, `S_BPREL32`, `S_LOCAL` or `S_LDATA32`: the same type change |
| `line-number` | In module order, the first `DEBUG_S_LINES` line entry with a line number below `0xF00F00`: line number + 1 |
| `member-offset` | In index order, the first `LF_FIELDLIST` with an `LF_MEMBER` offset below `0x7FFF`: offset + 1. The plan writes the TPI stream and its hash stream again |
| `remove-public` | The public symbol of `public-address`: removed |
| `remove-global` | The first data, constant or UDT global with a unique name: removed |
| `remove-module-symbol` | In module order, the first record in pre-order of kind `S_LABEL32`, `S_REGREL32` or `S_BPREL32`: removed |

Each case writes `target/verify-native-edit/<fixture>/<PDB file stem>/<case>.pdb` and gets
checks 1 to 3 above, the name lookups of the removed names, and a read of the output with
pdbrs that finds the new value (or no record).

The section then has two more steps:

- Scope trees: the command decodes each module of each PDB with `PdbEditor::module_mut`. Each
  module with a symbol region must form a scope tree; an opaque module is a problem. A plan
  without a change must give an empty `PlanReport::changed`: each module stream encodes
  again to its source bytes.
- Read count: on `msvc-x64-large`, a counting source maps each read to the pages of the stream
  directory. The plan of a new `srcsrv` stream must read only the Info stream and the first 56
  bytes of the TPI stream. `write_to` must read each page of each `Copied` stream exactly once
  and no other page.

The last line of the section gives the counts. The command fails for a problem in the section.

After the summary line of that section, the command prints the section `task cases:` (#41). It
runs each editing task of `PdbEditor` on each PDB. A case that does not apply to a PDB gives a
`note:` line and no output.

| Case | Task | Identity |
| --- | --- | --- |
| `set-source-link` | `set_source_link` with a Source Link file; `pdbstr -r -s:sourcelink` must read it | `Preserve` |
| `set-source-server` | `set_source_server` with a `srcsrv` stream; `pdbstr -r -s:srcsrv` must read it | `Preserve` |
| `remove-named-stream` | `remove_named_stream` of the first named stream in map order that the editor can remove (not `/names`, `/TMCache`, `/UDTSRCLINEUNDONE` or `/src/`); `llvm-pdbutil --named-streams` must not list it | `Preserve` |
| `rename-symbols` | `rename_symbols` gives the suffix `_pdbrs_task` to the first unique public name, the first unique data, constant or UDT global name and the first unique `S_PROCREF` or `S_LPROCREF` name | `IncrementAge` |

Each case writes `target/verify-native-edit/<fixture>/<PDB file stem>/task-<case>.pdb` and gets
checks 1 to 3 above. For `rename-symbols`, `llvm-pdbutil --publics` and `--globals` and
`DIA2Dump -p` must list the new names and not the old names, `DIA2Dump -sym` must find a
function with the new procedure name and no function with the old name, and a read with pdbrs
must find the new name in each renamed reference record and in the procedure that it names.
The last line of the section gives the counts. The command fails for a problem in the section.

After the summary line of the task section, the command prints the section `remap cases:`
(#46). The build root is the `output_root` of the manifest without `target\fixtures`: the
directory of the fixture sources and the build outputs. The case `remap-source-paths` runs
`remap_source_paths` on each PDB with `IdentityPolicy::Preserve`. The function replaces the root
with `/_/` if the root starts the path and a separator or the end follows; the comparison
ignores the ASCII case and the separator kind. The case writes
`target/verify-native-edit/<fixture>/<PDB file stem>/remap-source-paths.pdb` and checks:

1. A function that keeps every path gives a plan without a changed stream.
2. A byte search for the root (without case, `\` or `/`) over all streams of the output finds the
   root only in the fields that the decision comment of #46 excludes: inside a command-line
   string of `S_ENVBLOCK` or `LF_STRING_ID` (not at its start), inside the `/LinkInfo` command,
   after ` at ` in a type, ID or symbol name (Clang names a lambda after its declaration place),
   or in `sourcelink`. The section prints the count of each class.
3. Checks 1 to 3 above, with the new `/src/files/` names in `llvm-pdbutil --named-streams`, the
   old names absent, and the `srcsrv` text with the new paths through `pdbstr -r`.
4. `DIA2Dump -l` of the output is `DIA2Dump -l` of the source with the function applied to
   each file name: the lines of each procedure have the new file names and the same line
   numbers.

The last line of the section gives the counts. The command fails for a problem in the section.

After the summary line of that section, the command prints the section `type removal cases:`
(#47). It runs two cases on each PDB. Each case removes records that nothing refers to and
plans with `IdentityPolicy::IncrementAge` and `EditOptions::default()`:

| Case | Removed records | Method |
| --- | --- | --- |
| `remove-types` | The TPI records that pdbrs decodes and that are not UDTs (`LF_CLASS`, `LF_STRUCTURE`, `LF_INTERFACE`, `LF_UNION`, `LF_ENUM` and their `2` variants) or modifiers of UDTs | `TypeList::remove` |
| `remove-ids` | The `LF_UDT_SRC_LINE` and `LF_UDT_MOD_SRC_LINE` records of the IPI stream | `IdList::remove` |

DIA lists the UDTs, and it lists a modifier of a UDT as a UDT, so `remove-types` keeps them.
A debugger finds a UDT source line record through the IPI hash, and no index refers to it.

The references are an over-approximation:

- Type references: the indices of `TypeLeaf::for_each_reference` of each TPI record, the type
  fields of the IPI records, each hexadecimal value of at least `0x1000` in the IPI and symbol
  sections of `llvm-pdbutil dump --types --ids --symbols --globals` (not the record header
  indices), the types of the HLSL data symbols (`llvm-pdbutil` does not print them), the values
  of the XFG hash, token map and cross-scope export subsections, the type map streams of
  `/TMCache` and `/UDTSRCLINEUNDONE`.
- ID references: the ID fields of the IPI records (`LF_FUNC_ID` scope, `LF_STRING_ID`
  substring list, `LF_SUBSTR_LIST`, `LF_BUILDINFO` arguments, `LF_UDT_SRC_LINE` source file),
  each value of the dump that is not in a type field, the inlinee lines, the token maps, the
  cross-scope exports and imports, and the ID maps of the type map streams.

Each case writes `target/verify-native-edit/<fixture>/<PDB file stem>/<case>.pdb`. A PDB
without a candidate gives a `note:` line. A plan error `UnsupportedEdit` gives a
`refused (<error>)` line and is not a problem. Each output gets checks 1 to 3 above and two
comparisons:

- The `llvm-pdbutil dump --types --ids --symbols --globals` output must be equal to the dump of
  the source PDB after this mapping:
  - In the TPI section (`remove-types`) or the IPI section (`remove-ids`), remove the block
    of each removed record, and change the record count and the width of the index columns.
  - Change each index of the compacted space to `PdbWritePlan::type_index` or
    `PdbWritePlan::id_index`. The type fields are each value in the TPI section, the type
    fields of `LF_FUNC_ID`, `LF_MFUNC_ID`, `LF_UDT_SRC_LINE` and `LF_UDT_MOD_SRC_LINE`, and the
    `type` fields of the symbols. The ID fields are the other values of the IPI section (not
    the text of a string), and the `type` of an `_ID` procedure, `inlinee`, `BuildId` and the
    callee, caller and inlinee lists of the symbols.

  For a difference, the command writes the mapped source dump and the output dump to
  `<case>.source.txt` and `<case>.output.txt` next to the output PDB.
- The masked `DIA2Dump -all` output must be equal to the output for the source PDB.

The last line of the section gives the counts. The command fails for a problem in the section.

## MSFZ writer check

The `verify-msfz` command checks the MSFZ writer and the conversions of pdbrs (#38):

```sh
cargo run -p pdbrs-fixtures --release -- verify-msfz [--fixtures <dir>] [--jobs <n>]
```

The command skips `hlsl-dxc-debug`, as `verify-native-build` does. It builds the `native`
example of pdbrs first. It has three parts.

1. Encode. The command writes each `native-pdb` output as MSFZ with `Container::to_msfz` and
   each of six option sets: `default`, `compress-stream-dir` (zstd directory),
   `one-stream-per-chunk`, `small-chunks` (4 KiB, the smallest size), `pad16k` and `deflate`
   (raw DEFLATE chunks). Stream 0 is empty, as `pdbtool pdz-encode` writes it. The output is
   `target/verify-msfz/encode/<fixture>/<PDB file stem>/<option set>/<file name>`. The name
   keeps the `.pdb` extension for DIA. Each output must pass these checks:
   - A second write gives the same bytes.
   - `pdbtool compare <source> <output>` reports no stream with differences.
   - `DIA2Dump -all` gives the same masked output as for the source.
   - The pdbrs reader gives the same streams from stream 1, and an empty stream 0.
   - The `native` example gives the same output, apart from the lines that start with
     `container:`.
2. Reference. The command writes the source of each `pdz-*` fixture with the matching option
   set and compares the bytes with the `pdz-encode` file. It prints the sizes and the chunk and
   fragment counts of both files, and the reasons for different bytes. Different bytes do not
   make the command fail. The part skips the `msfz-*` fixtures.
3. Decode. The command converts each PDZ fixture (`pdz-*` and `msfz-*`) to MSF 7.0 with
   `Container::to_msf` and the page size of the original. The output is
   `target/verify-msfz/decode/<PDZ fixture>/<source fixture>/<file stem>/<file name>`. Each
   output must pass these checks:
   - A second conversion gives the same bytes.
   - `pdbtool check` shows 0 PDBs with errors.
   - The normalized `llvm-pdbutil dump --all` output and the masked `DIA2Dump -all` output are
     the same as for the original MSF 7.0 fixture.
   - The streams are equal to the streams of the PDZ file, and from stream 1 to the original.
   - `pdbtool compare <original> <PDZ file>` reports no stream with differences.
   - The `native` example gives the same output for the PDZ file and the original, apart from
     the lines that start with `container:`.

Two `DIA2Dump` results are explained and do not make the command fail:

- DIA rejects each file with raw DEFLATE chunks (`loadDataFromPdb failed - HRESULT =
  806D000C`). DIA reads only zstd chunks. `pdbtool` and pdbrs read the DEFLATE files. DIA also
  rejects the `msfz-deflate` and `msfz-deflate-dir` files of Microsoft's writer. The decode part
  runs DIA on the MSF 7.0 output, not on the PDZ file, so these files pass.
- For `msvc-x64-incremental`, DIA prints function names with their signatures for each MSFZ
  file, also for the `pdz-encode` file. The check accepts the output if it is the same as for
  the `pdz-default` fixture of the same source.

A different output writes `<name>.original.txt`, `<name>.written.txt` and `<name>.diff` next to
the output file.

## Remap check

The `verify-remap` command checks the OMAP output and the clean output of the address remap
of pdbrs (#43):

```sh
cargo run -p pdbrs-fixtures --release -- verify-remap [--fixtures <dir>] [--jobs <n>]
```

The command skips `hlsl-dxc-debug`, as `verify-native-build` does. For each other
`native-pdb` output, it reads the procedures with `SymbolLocator::procedures`. It keeps the
procedures that are inside one code section (`IMAGE_SCN_CNT_CODE` or `IMAGE_SCN_MEM_EXECUTE`)
and that do not overlap. The procedures and the gaps between them cut each code section into
pieces. The command makes five range sets:

| Set | New image |
| --- | --- |
| `identity` | Each section maps to itself |
| `section-shift` | Each code section moves by the same amount, to an RVA after the end of the image. The other sections stay |
| `reorder` | The pieces of each code section come in reverse order |
| `split` | Each procedure of 2 or more bytes splits into 2 ranges, at an offset that changes from procedure to procedure. The second parts move to the end of the section |
| `removal` | The procedures that no other record refers to have no range: no `S_PROCREF` or `S_LPROCREF` record of the global index names them, and they have no `S_SEPCODE` chunk |

OMAP cannot map RVA 0, so a range at RVA 0 starts 1 byte later (the HLSL and .NET fixtures
have a section at RVA 0). In the `reorder` set of the HLSL fixtures, the procedure moves to
RVA 0. Its first byte thus has no mapping, and the set line counts it as a removed procedure.

For each set, the command remaps the PDB with `PdbEditor::remap_addresses` and
`RemapMode::Omap` (`IdentityPolicy::IncrementAge`). It writes the output to
`target/verify-remap/<fixture>/<PDB file stem>/<set>.pdb` and does these checks:

1. `PlanReport::changed` lists the Info stream, the DBI stream and each of the 4 OMAP output
   streams (slots 3, 4, 5 and 10) with new bytes. `PlanReport::dropped` is empty.
2. The pdbrs reader finds the address tables of `ImageRemap::address_tables`.
3. The same ranges, added in reverse order, give the same bytes.
4. `pdbtool check` shows 0 PDBs with errors.
5. DIA2Dump on the output gives each procedure, public symbol, data symbol and line the new
   RVA that the ranges give for its source RVA, or RVA 0 without a range:
   - `DIA2Dump -p`, `-g` and `-s` must print the lines of the source PDB, with each RVA
     replaced by the new RVA. The section and the offset do not change.
   - For each start RVA of a line in `DIA2Dump -l` of the source, with a range,
     `DIA2Dump -l <new RVA> 1` on the output must print the answer of `-l <RVA> 1` on the
     source, with the new RVAs.

Each set line gives the counts of the DIA2Dump lines that the check compared. The command
exits with a failure for each problem and for each missing tool. These results are explained:

- `DIA2Dump -g` fails on `msvc-x64-fastlink` (`DumpAllGlobals() returned no symbols`). The
  command prints a note and does not compare `-g` for that PDB.
- In a PDB without type merging (`link /DEBUG:LAZY`, feature `NOTM`), DIA prints a procedure
  without a range with an empty name. The command counts these lines as explained.

### Clean output

After the last line of the OMAP part (`native PDBs: ..., problems: N`), the command prints
the clean part. For each PDB and each set, it remaps the PDB with `RemapMode::Clean`. It
writes the output of `RemovedCode::Drop` to
`target/verify-remap/<fixture>/<PDB file stem>/<set>.clean.pdb` and does these checks:

1. The policies. The plan with `RemovedCode::Fail` must fail with `UnsupportedEdit` exactly
   when the plan with `RemovedCode::Drop` removes records (for example in the `removal` set,
   and for the symbols at RVA 0 of the HLSL and .NET fixtures). The address of the error
   (`Error::remap_address`) must have no range. Without removed records, both policies must
   give the same bytes. A set that removes procedures must remove records.
2. Records without a split form (#43). The plan can fail with `UnsupportedEdit`
   for a record that the ranges split, for example `S_BLOCK32`, an inline site in more than
   one part, or an FPO record. The command then decodes the record at the source location of
   the error (`Error::stream_location`), or finds the procedure that holds the address. A
   range boundary must cut the code of that record. The set line then shows
   `no split form: <kind>` (or `partly removed: <kind>` for a record whose code is only
   partly removed), and the command does no other check of the set.
3. The unwind streams. For a PDB with `.pdata` or `.xdata` (`msvc-x64-debugtype` and
   `msvc-arm64-debugtype`), the call without the streams of the new image must fail with
   `UnsupportedEdit`. The command keeps `.xdata` (its section has an identity range) and
   rewrites each `.pdata` runtime function through the ranges: one entry for each part with a
   range and the same unwind information, sorted by the new start. It gives both streams
   with `ImageRemap::with_unwind_streams`.
4. The report and the reader. The output DBI stream names no OMAP stream and no original
   section headers (slots 3, 4 and 10). The pdbrs reader finds the section headers of the set
   and no OMAP. `PlanReport::dropped` is empty. For the `identity` set of a PDB without OMAP
   and without removed records, `PlanReport::changed` lists only the Info stream and the DBI
   stream. The ranges in reverse order must give the same bytes.
5. `pdbtool check` shows 0 PDBs with errors, and `llvm-pdbutil dump --all` exits with the
   exit code of the source.
6. DIA2Dump on the clean output must give the answers of the OMAP output of the same set. The
   OMAP output keeps the source section and offset, so the command compares the RVA of each
   address only. It also ignores the DIA symbol IDs (`SymIndex` and the properties that end
   with `Id`) and the PDB name in the `Executable (Global)` line of `-sym`. `-sym` prints the
   exception handler of a procedure and the parent of an `S_SEPCODE` scope as a section
   property and an offset property without an RVA. The command replaces each pair with the
   new RVA: for the OMAP output, the new RVA of the source address; for the clean output, the
   RVA in the section headers of the set.
   - `-s` line by line, `-p` and `-g` as sorted lists (their order follows the hash tables).
     An OMAP line with RVA 0 in a section has no clean line: the clean output removed the
     record. In `-s`, the child lines of such a symbol (up to the next empty line or the next
     symbol line with the same or a smaller indent) go with it, and so does the empty line
     that DIA2Dump prints before a procedure.
   - `-l <RVA> 1` at the new RVA of each line start of the OMAP check.
   - `-sym <RVA>` at 3 addresses (the second byte, the middle and the last byte) of up to 100
     procedures, and `-fpo <RVA>` at the addresses of these that an FPO or new FPO record of
     the source holds. DIA2Dump stops at a query without an answer and exits with -1. The
     command counts that query as one without an answer and runs the queries after it again.
7. `SymbolLocator::locate` on both outputs at the same 3 addresses of each procedure must give
   the same procedure name, innermost block, inline call chain (names and call site lines)
   and line (file and line number).

Each set line gives the removed records and the added parts of the report
(`RemapReport::removed_items`, `added_items`), the number of contributions with new CRCs, and
the compared, removed and explained items of checks 6 and 7. The lines
`explained (clean): ...` give the totals of each explained class:

- `no split form: <kind>` and `partly removed: <kind>`: see check 2.
- A `Block` line of the clean output without an OMAP line: DIA shows an `S_SEPCODE` scope as
  a block, and the clean output adds one for each part of a split procedure after the first.
- A line that differs only in a smaller length, for a record that a range boundary cuts: the
  clean record describes the first part.
- A `FuncDebugEnd` line at the end of its procedure. The OMAP output translates the address
  after the procedure through OMAP, so it gets the RVA of the next range (or 0). The clean
  record keeps the offset in the procedure, so DIA gives the new end of the procedure.
- A `FuncDebugStart` or `FuncDebugEnd` line of a split procedure whose old address is outside
  the first part. The clean record limits the debug start and end to the length of the first
  part (#43), so the clean line must give the end of the first part. The OMAP line must give
  the new RVA of an old address in a later part of the procedure.
- `-sym` at an address in a later part of a split procedure. DIA finds the procedure in the
  OMAP output, and the `S_SEPCODE` block of the part in the clean output. The clean block must
  hold the address, the address must be outside the first part, and the parent of the block
  must be the procedure of the OMAP answer with the length of the first part. The procedure
  lines of both answers must agree, with the other explained classes.
- In a PDB without type merging (`NOTM`), DIA stops the `-s` list of a module at the first
  symbol that it cannot read, for example a procedure whose type is in the object file. If the
  clean output removed that symbol, DIA lists more symbols of the module. The extra clean
  lines must end the list of the same module.
- `-sym` at the address of procedures that share their code (identical COMDAT folding, for
  example four `scalar deleting destructor` procedures in `msvc-x64-release`). DIA selects one
  of them, and the selection changes with the record order. The two answers must name two
  procedures that the OMAP `-s` dump lists at that RVA.
- DIA gives the length 0 to the public symbol with the highest RVA. The linker PDBs show the
  same behavior: `-sym` gives the length 0 for `__arm64x_redirection_metadata` in
  `msvc-arm64ec-debug` and for `__dyn_tls_init_callback` in `msvc-x64-lazy`, although their
  section contributions have 16 and 8 bytes. A section shift moves `.text` to the end of the
  image, so in `clang-lld-x64-release` and `clang-flang-lld-x64-primitives` the import thunk
  `terminate` becomes the last public symbol. The section headers, the section map and the
  section contribution (6 bytes) of the clean output are correct. The clean answer must be a
  public symbol (`SymTag` 0xA) at the highest public RVA of the clean `-p` dump.
- A PGO phase line (`Phase Name: <phase>`, and its `-sym` property) of one output only. DIA
  takes the phase from the COFF group `<section>$lp<phase>` that holds the procedure. A COFF
  group is a point (#43): the clean group keeps its start and length, so after a
  reorder it holds other procedures (`msvc-x64-pgo`). The source group must hold the old start
  of the procedure exactly if the OMAP output prints the line. The clean group must hold the
  new start exactly if the clean output prints it.

The line `clean outputs: ...` ends the clean part. The last line is `result: pass`, or
`result: FAIL` with the problems of both parts. The command prints the results at the end, in
fixture order. While it works, it prints a progress line to stderr for each PDB and set:
`verify-remap: <fixture>/<PDB>: <set>: OMAP <seconds> s, clean <seconds> s`.
