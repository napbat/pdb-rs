# Benchmarks

This document records the benchmark of `pdbrs`: the command, the method, the scenarios and the
limits of each number (#44). [baseline.md](baseline.md) records the machine, the toolchain, the
commit, the fixtures, the run count and the baseline numbers. The API redesign (#42) and #48,
#49 and #50 compare their results with this baseline. [msfz-write.md](msfz-write.md) records
the result of #50.

## Command

Run all scenarios from the workspace root:

```sh
cargo bench -p pdbrs --bench baseline
```

The command prints one Markdown report to standard output. Cargo prints its build messages to
standard error. The report has four sections in a fixed order: the environment, the record
sizes, the fixtures with their sizes and one table row for each scenario. The order of the rows
is the order of the scenario table below. [baseline.md](baseline.md) tells how to record the
report.

Give options after `--`:

| Option | Effect |
| --- | --- |
| `--runs <n>` | The number of timed runs of each scenario. The default is 10. |
| `--filter <text>` | Run only the scenarios whose `<scenario>@<fixture>` contains the text, for example `--filter msvc-x64-large`. |
| `--fixtures <directory>` | Read the fixtures from this directory. The default is `target/fixtures` of the workspace. |

The bench target is `crates/pdbrs/benches/baseline/`. It needs the `std` feature, so the checks
with `--no-default-features` do not build it. It uses `std::time::Instant` and has no dependency
other than `pdbrs`.

## Fixtures

The bench reads the fixtures from `target/fixtures`. `cargo run -p pdbrs-fixtures` makes them.
If a fixture file is missing, the row of each scenario on that fixture shows `missing` and no
number. The report then names each missing file.

| Fixture | File below `target/fixtures` | Use |
| --- | --- | --- |
| `msvc-x64-large` | `msvc-x64-large/demo.pdb` | The large native PDB (MSVC 14.51, `cl /Zi /Od`, `link /DEBUG:FULL`, about 21 MB). |
| `msvc-x64-debug` | `msvc-x64-debug/demo.pdb` | The small native PDB (the same compiler and linker options, but no `/MP`; about 1.3 MB). |
| `dotnet-csharp-embed-sources` | `dotnet-csharp-embed-sources/artifacts/bin/CSharpDemo/debug/CSharpDemo.pdb` | The largest standalone Portable PDB (about 22 KB). The `dotnet-*-full` fixtures are native PDBs. |
| `pdz-default/msvc-x64-large` | `pdz-default/msvc-x64-large/demo.pdb` | The MSFZ form of the large native PDB that `pdbtool pdz-encode` writes (about 4.4 MB). |

## Method

### Processes

The driver runs each pair of scenario and fixture in a child process. The child is the same
executable with the internal option `--child <scenario>@<fixture>`. Each child does these
steps:

1. Load the input and build the structures that the scenario needs. This step has no timer.
2. Do 1 warm-up run without a timer.
3. Do the timed runs. Each run has its own `Instant::now()` and `elapsed()` interval.
4. Check that each run gives the same item count as the warm-up run.
5. Get the peak working set of the child process (see below).
6. Print one result line for the driver.

The driver prints the minimum, the median and the maximum of the timed runs. The runs of one
row use one thread. The rows run one after the other.

### Peak memory

The workspace denies `unsafe_code`. Thus the bench cannot call `GetProcessMemoryInfo`, and it
cannot install a counting global allocator. .NET gives no `Process.PeakWorkingSet64` value for a
process that has exited (tested with `pwsh` on Windows 11). Thus the child gets the value before
it exits:

- On Windows, the child starts `pwsh -NoProfile -NonInteractive -Command
  "(Get-Process -Id <pid>).PeakWorkingSet64"` with its own process ID. If `pwsh` is not
  available, it uses `powershell`.
- On Linux, the child reads `VmHWM` from `/proc/self/status`.
- On other systems, and if the query fails, the row shows `-`.

The scenario `process/idle` does no work. Its value is the base value of a child process.
Subtract it to get the memory that a scenario adds.

### Record sizes

The report gives `core::mem::size_of` of `Symbol`, `SymbolData`, `TypeLeaf`, `IdRecord` and
`Public`, as the baseline of the `Bytes` change of #42.

## Scenarios

The count column gives the number of items that one run processed. The count shows that a run
did the expected work. If a later commit gives another count, the scenario did other work and
its times are not comparable.

| Scenario | Fixtures | Timed run | Count |
| --- | --- | --- | --- |
| `process/idle` | none | Nothing. | 0 |
| `native/open` | native | `Pdb::open`: the container directory and the Info stream. | Streams |
| `native/full-read` | native | `Pdb::open` and the decode calls of the `native` example (see below). | Decoded records and streams |
| `native/type-table` | native | `Pdb::types`, the TPI hash stream, `TypeHashes::parse`, `TypeTable::new` and `ForwardRefs::new`. | TPI records |
| `native/type-by-index` | native | `TypeTable::leaf` for each TPI index, in a fixed pseudo-random order. | Lookups |
| `native/type-by-name` | native | `ForwardRefs::resolve` for each UDT forward reference: the name lookup in the TPI hash buckets. | Forward references with a definition |
| `native/symbol-locator` | native | `SymbolLocator::new`. | Procedures |
| `native/symbol-by-name` | native | `SymbolLocator::find_name` for up to 1000 names of procedures and of the public symbols at their starts. | Matches |
| `native/symbol-by-address` | native | `SymbolLocator::public_at` for the start and the middle byte of up to 2000 procedures. | Addresses with a public symbol |
| `native/line-lookup` | native | `SymbolLocator::locate` for the same addresses: module, procedure, inline frames and line. | Addresses with a line |
| `native/edit-named-stream` | native | `Pdb::edit(IdentityPolicy::Preserve)`, `named_streams_mut()?.insert(b"srcsrv", ...)`, `plan(EditOptions::default())`, `to_vec`. | Output bytes |
| `native/edit-rename-public` | native | `Pdb::edit(IdentityPolicy::IncrementAge)`, `publics_mut`, `find` and `get_mut` rename the first public symbol with a unique name, plan, `to_vec`. | Output bytes |
| `native/edit-append-type` | native | `Pdb::edit(IdentityPolicy::IncrementAge)`, `types_mut()?.push` of a field list and a structure, plan, `to_vec`. | Output bytes |
| `native/edit-strip-private` | native | `Pdb::edit(IdentityPolicy::Preserve)`, `plan(EditOptions::default().with_strip_private(true))`, `to_vec`. | Output bytes |
| `native/rebuild` | native | `Pdb::edit(IdentityPolicy::Preserve)`, `decode_all`, `plan(EditOptions::default())`, `to_vec`. | Output bytes |
| `portable/read` | Portable | `portable::Pdb::parse`, every row of the eight debug tables with its heap values, the custom debug information values, the document sources, the sequence points, the local variables and constants, the imports, the state machine methods and `validate_debug`. | Rows and decoded values |
| `portable/rebuild` | Portable | `Pdb::to_parts`, `PdbParts::plan(IdentityPolicy::ContentDerived)`, `to_vec`. | Output bytes |
| `portable/edit` | Portable | `Pdb::edit(IdentityPolicy::ContentDerived)`, a line shift of all sequence points and a rename of document 1, `plan(EditOptions::default())`, `to_vec`. Before #42 stage 7: `Pdb::edit()` and `plan(IdentityPolicy::ContentDerived)`. | Output bytes |
| `msfz/open` | MSFZ | `Pdb::open` of the MSFZ file. | Streams |
| `msfz/full-read` | MSFZ | The full read of `native/full-read` on the MSFZ file. | Decoded records and streams |
| `msfz/write-default` | `msvc-x64-large` | `Container::to_msfz(MsfzWriteOptions::default(), OldDirectoryPolicy::Empty)` and `to_vec`. `verify-msfz` and `pdbtool pdz-encode` use the same stream 0 policy. | Output bytes |

The native scenarios run on `msvc-x64-large` first, then on `msvc-x64-debug`.

`native/open`, `native/full-read`, `msfz/open` and `msfz/full-read` open the file in each run,
as the `native` example does. The other scenarios read the file into memory before the timed
runs and use `Pdb::parse` on the bytes. Their times do not include file reads.

### Full read

The full read makes the decode calls of the `native` example. It decodes these items:

- the `/names` table and each named stream with the decoder of its kind (`/LinkInfo`,
  `/TMCache` with its type map streams, `/UDTSRCLINEUNDONE`, `sourcelink`, `srcsrv` and
  `/src/headerblock` with the content stream of each injected source);
- the TPI and IPI hash streams and each TPI and IPI record (`TypeLeaf::decode` and
  `IdRecord::decode`);
- the DBI modules, section contributions, section map, source files, EC names and type server
  map, the FPO and frame data streams and the address tables;
- each module symbol (`ModuleSymbols`) and each C13 subsection (`ModuleLines`) with its line
  blocks, file checksums and inlinee lines;
- each record of the symbol record stream and each record of the global and public indexes.

The full read does not do the producer checks and the re-encode comparisons of the example.
These test the encoders and the producers, not the reader. It does not decode the module type
streams of a PDB without type merging (`NOTM`).

## Limits

- **Times.** Each time is the wall-clock time of one run on one thread. It includes the
  allocator. The scheduler, the processor frequency and other processes change the times. Use
  a quiet machine, and compare numbers from the same machine only. The minimum is the most
  stable value for a comparison. The median shows the usual value. Times below 1 ms have a
  large relative variation. After the warm-up run, the operating system has the fixture file
  in its file cache. Thus the `open` and `full-read` rows measure cached reads, not disk reads.
- **Setup.** The rows do not include the setup step of their scenario. For example,
  `native/type-by-index` does not include `TypeTable::new`; `native/type-table` measures it.
- **Counts.** A count shows the work of one run. It is not a measure of speed.
- **Peak working set.** The value is the peak of the whole child process, not of one run. It
  includes the executable, the runtime, the input bytes, the structures of the setup step, the
  warm-up run and all timed runs. It counts resident pages, so it includes freed memory that the
  allocator keeps and the code pages. It does not include memory that the system pages out. Its
  resolution is one page. Compare a row with `process/idle` and with the same row of another
  commit.
- **Record sizes.** The sizes are exact for the compiler and the target of the run. Another
  toolchain or target can give other sizes.
- **No bounds.** The bench has no wall-clock bound and no memory bound. A slow machine cannot
  make it fail. It fails only if a scenario returns an error or gives two different counts.
