# Baseline benchmark results

This file records the baseline numbers of `pdbrs` before the first change of the API redesign
(#42). [README.md](README.md) gives the method, the scenarios and the limits of each number.
#42, #48, #49 and #50 compare their results with these numbers.

Status: recorded on 2026-10-26 at commit `7e35f5e`, before the first change of #42. The machine
had no other load.

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
| Commit | 7e35f5e5cdaa8fb42ad113b6411418676a549ede |
| Toolchain | rustc 1.99.0 (b940084d7 2026-09-28) |
| Debug assertions | off |
| Operating system | Microsoft Windows 11 Pro 10.0.26200 |
| Architecture | x86_64 |
| Processor | AMD Ryzen 9 9950X3D 16-Core Processor |
| Logical processors | 32 |
| Physical memory | 61.7 GiB |
| Fixtures | C:\Users\dev\Documents\Git\napbat\pdbrs\target\fixtures |
| Runs | 1 warm-up run and 10 timed runs in one child process for each row |

## Record sizes

| Type | `size_of` in bytes |
| --- | ---: |
| `Symbol` | 128 |
| `SymbolData` | 96 |
| `TypeLeaf` | 176 |
| `IdRecord` | 48 |
| `Public` | 32 |

## Fixtures

| Fixture | File | Bytes |
| --- | --- | ---: |
| msvc-x64-large | `msvc-x64-large/demo.pdb` | 21368832 |
| msvc-x64-debug | `msvc-x64-debug/demo.pdb` | 1273856 |
| dotnet-csharp-embed-sources | `dotnet-csharp-embed-sources/artifacts/bin/CSharpDemo/debug/CSharpDemo.pdb` | 21944 |
| pdz-default/msvc-x64-large | `pdz-default/msvc-x64-large/demo.pdb` | 4412476 |

## Scenarios

| Scenario | Fixture | Count | Min ms | Median ms | Max ms | Peak working set MiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| process/idle | - | 0 | 0.000 | 0.000 | 0.000 | 10.8 |
| native/open | msvc-x64-large | 65 | 0.092 | 0.095 | 0.109 | 10.9 |
| native/full-read | msvc-x64-large | 792745 | 24.207 | 24.347 | 25.593 | 16.6 |
| native/type-table | msvc-x64-large | 224838 | 3.356 | 3.421 | 3.465 | 35.2 |
| native/type-by-index | msvc-x64-large | 224838 | 11.388 | 11.451 | 11.594 | 35.7 |
| native/type-by-name | msvc-x64-large | 28089 | 5.969 | 5.981 | 6.013 | 35.4 |
| native/symbol-locator | msvc-x64-large | 28155 | 9.765 | 9.920 | 10.345 | 39.1 |
| native/symbol-by-name | msvc-x64-large | 987 | 99.979 | 100.261 | 100.624 | 39.7 |
| native/symbol-by-address | msvc-x64-large | 3754 | 0.267 | 0.268 | 0.274 | 38.1 |
| native/line-lookup | msvc-x64-large | 3748 | 1.181 | 1.195 | 1.204 | 38.2 |
| native/edit-named-stream | msvc-x64-large | 21344256 | 3.990 | 4.098 | 4.258 | 45.3 |
| native/edit-rename-public | msvc-x64-large | 21340160 | 26.588 | 26.947 | 27.130 | 48.9 |
| native/edit-append-type | msvc-x64-large | 21340160 | 52.415 | 52.681 | 53.056 | 55.0 |
| native/edit-strip-private | msvc-x64-large | 1781760 | 13.802 | 14.102 | 14.497 | 43.3 |
| native/rebuild | msvc-x64-large | 21340160 | 201.306 | 202.657 | 203.362 | 185.5 |
| native/open | msvc-x64-debug | 74 | 0.029 | 0.030 | 0.046 | 10.9 |
| native/full-read | msvc-x64-debug | 23390 | 1.022 | 1.034 | 1.106 | 11.2 |
| native/type-table | msvc-x64-debug | 4738 | 0.032 | 0.032 | 0.042 | 11.0 |
| native/type-by-index | msvc-x64-debug | 4738 | 0.326 | 0.330 | 0.352 | 11.0 |
| native/type-by-name | msvc-x64-debug | 292 | 0.045 | 0.045 | 0.053 | 11.1 |
| native/symbol-locator | msvc-x64-debug | 580 | 0.348 | 0.381 | 0.423 | 12.1 |
| native/symbol-by-name | msvc-x64-debug | 541 | 0.069 | 0.078 | 0.141 | 11.3 |
| native/symbol-by-address | msvc-x64-debug | 1158 | 0.077 | 0.078 | 0.096 | 11.2 |
| native/line-lookup | msvc-x64-debug | 912 | 0.309 | 0.310 | 0.323 | 11.3 |
| native/edit-named-stream | msvc-x64-debug | 1253376 | 0.186 | 0.257 | 0.315 | 11.1 |
| native/edit-rename-public | msvc-x64-debug | 1249280 | 1.118 | 1.173 | 1.278 | 12.4 |
| native/edit-append-type | msvc-x64-debug | 1249280 | 2.708 | 2.746 | 2.788 | 11.5 |
| native/edit-strip-private | msvc-x64-debug | 217088 | 0.625 | 0.690 | 0.719 | 11.9 |
| native/rebuild | msvc-x64-debug | 1249280 | 6.553 | 6.920 | 7.030 | 12.9 |
| portable/read | dotnet-csharp-embed-sources | 781 | 0.152 | 0.156 | 0.179 | 11.2 |
| portable/rebuild | dotnet-csharp-embed-sources | 21944 | 0.094 | 0.111 | 0.148 | 11.5 |
| portable/edit | dotnet-csharp-embed-sources | 23144 | 0.200 | 0.224 | 0.267 | 11.4 |
| msfz/open | pdz-default/msvc-x64-large | 65 | 0.025 | 0.025 | 0.063 | 11.0 |
| msfz/full-read | pdz-default/msvc-x64-large | 792745 | 73.346 | 74.765 | 76.957 | 40.7 |
| msfz/write-default | msvc-x64-large | 6689764 | 262.627 | 271.180 | 280.499 | 39.9 |
<!-- baseline:end -->
