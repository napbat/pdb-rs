# Baseline benchmark results

This file records the baseline numbers of `pdbrs` before the first change of the API redesign
(#42). [README.md](README.md) gives the method, the scenarios and the limits of each number.
#42, #48, #49 and #50 compare their results with these numbers.

Status: not recorded yet. The sections between the markers below have the structure of the
report, but no numbers.

## Record the baseline

Caution: other processes change the times. Record the baseline on a quiet machine only.

1. Merge the bench into `main`. Use a commit before the first change of #42.
2. Make sure that the fixtures exist (`cargo run -p pdbrs-fixtures`).
3. Stop builds, tests, agents and other loads on the machine.
4. Connect the machine to mains power, and use the same power plan for later comparisons.
5. Run this command from the workspace root:

   ```sh
   cargo bench -p pdbrs --bench baseline -- --runs 10 > target/baseline.md
   ```

   Use a POSIX shell or `pwsh` 7. The `>` of Windows PowerShell 5.1 writes UTF-16.

6. Make sure that the `Commit` row of `target/baseline.md` has no `(with uncommitted changes)`
   note, and that no row shows `missing` or `failed`.
7. Replace the text between the `baseline:start` and `baseline:end` markers with the contents
   of `target/baseline.md`.
8. Change the status line above to the date of the run.
9. Commit the change with the message `pdbrs: record the baseline benchmark numbers (#44)`.

For a comparison, run the same command on the same machine at the later commit. Compare only
rows with the same count.

<!-- baseline:start -->
## Environment

| Item | Value |
| --- | --- |
| Commit | |
| Toolchain | |
| Debug assertions | |
| Operating system | |
| Architecture | |
| Processor | |
| Logical processors | |
| Physical memory | |
| Fixtures | |
| Runs | |

## Record sizes

| Type | `size_of` in bytes |
| --- | ---: |
| `Symbol` | |
| `SymbolData` | |
| `TypeLeaf` | |
| `IdRecord` | |
| `Public` | |

## Fixtures

| Fixture | File | Bytes |
| --- | --- | ---: |
| msvc-x64-large | `msvc-x64-large/demo.pdb` | |
| msvc-x64-debug | `msvc-x64-debug/demo.pdb` | |
| dotnet-csharp-embed-sources | `dotnet-csharp-embed-sources/artifacts/bin/CSharpDemo/debug/CSharpDemo.pdb` | |
| pdz-default/msvc-x64-large | `pdz-default/msvc-x64-large/demo.pdb` | |

## Scenarios

| Scenario | Fixture | Count | Min ms | Median ms | Max ms | Peak working set MiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
<!-- baseline:end -->
