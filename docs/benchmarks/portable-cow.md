# Portable PDB edit: copy-on-write (#49)

This file records the effect of #49 on the Portable PDB editor. Before #49, `PdbEditor::plan`
decoded each table that no `*_mut` call decoded into a temporary model, and encoded all tables
from the records again. It also made the value index of each source heap. Now the plan copies the
raw rows of each table that no `*_mut` call decoded, and makes the value index of a source heap
only on the first addition to that heap.

Status: recorded on 2026-10-27 on a loaded machine. Compare the ratios, not the absolute times.
Run the bench of [README.md](README.md) again on a quiet machine to replace the times.

## Method

- Before: commit `074c364` (`main` at `d1d3fdd` with the two new bench rows). After: commit
  `13fd082`. Both are release builds with the same toolchain as [baseline.md](baseline.md), on
  the same machine.
- Bench: `cargo bench -p pdb-rs --bench baseline -- --filter portable/ --runs 10`. The rows
  `portable/edit-document` (the name of document 1 gets the suffix `.edited`) and
  `portable/edit-method` (the lines of the first method with sequence points + 1000) are new
  for #49. All rows use `dotnet-csharp-embed-sources` (21,944 bytes), the largest portable
  fixture.
- Order: 7 rounds. Each round ran the before build and the after build one after the other. The
  order changed in each round (before first in rounds 1, 3, 5 and 7). The tables give the median
  of the 7 rounds.
- Larger inputs: a throwaway program (not in the repository) ran the same edits in one process
  on two real Portable PDBs that are not fixtures. It used a counting allocator for the peak
  live heap and the allocated bytes. Each run timed all scenarios one after the other in each
  round (10 rounds for the 2.9 MB input, 5 rounds for the 30 MB input). 3 runs of each build,
  in alternating order; the tables give the median of the 3 runs.
  - `Microsoft.CodeAnalysis.CSharp.pdb` from the NuGet package `microsoft.codeanalysis.csharp`
    5.0.0 (`lib/net9.0`): 2,895,116 bytes, 1,145 documents, 40,758 methods with sequence
    points.
  - `Microsoft.iOS.pdb` from the .NET pack `Microsoft.iOS.Runtime.ios.net10.0_26.5` 26.5.10301:
    30,247,348 bytes, 9,767 documents, 138,375 methods with sequence points, a `#Blob` heap of
    25,426,904 bytes.
- Load: another project ran load tests on the machine at the same time. The processor load at
  the start of each round was 100 %. The rows that #49 does not change (`portable/read`,
  `portable/rebuild`, `decode_all`) show the noise: their ratios are from 0.80 to 1.17.

## Results: bench

The outputs have the same size before and after (the `Count` column).

| Row | Before median ms | After median ms | After / before | Ratios of the 7 rounds | Before minimum ms | After minimum ms | Peak working set MiB before / after |
| --- | ---: | ---: | ---: | --- | ---: | ---: | --- |
| `portable/read` | 0.253 | 0.272 | 1.08 | 0.50 to 1.18 | 0.231 | 0.241 | 11.2 / 11.3 |
| `portable/rebuild` | 0.212 | 0.244 | 1.15 | 0.52 to 1.32 | 0.161 | 0.174 | 11.4 / 11.6 |
| `portable/edit` | 0.319 | 0.220 | 0.69 | 0.55 to 1.14 | 0.275 | 0.188 | 11.5 / 11.4 |
| `portable/edit-document` | 0.287 | 0.152 | 0.53 | 0.25 to 0.65 | 0.254 | 0.129 | 11.5 / 11.4 |
| `portable/edit-method` | 0.290 | 0.162 | 0.56 | 0.50 to 0.73 | 0.250 | 0.126 | 11.5 / 11.4 |

The quiet baseline of #44 for `portable/edit` is a median of 0.224 ms and a peak working set of
11.4 MiB. On this fixture the whole edit uses less than 0.2 MiB of heap, so the peak working
set does not change.

## Results: larger inputs

| Input | Edit | Before median ms | After median ms | After / before | Peak live heap before | Peak live heap after | After / before |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| fixture (21,944 B) | no change | 0.234 | 0.040 | 0.17 | 102,612 | 43,908 | 0.43 |
| | one document | 0.215 | 0.124 | 0.58 | 121,034 | 82,226 | 0.68 |
| | one method | 0.221 | 0.105 | 0.48 | 121,065 | 82,933 | 0.69 |
| CodeAnalysis (2.9 MB) | no change | 257.7 | 24.9 | 0.10 | 23,130,515 | 6,088,182 | 0.26 |
| | one document | 229.0 | 68.0 | 0.30 | 24,552,433 | 11,326,664 | 0.46 |
| | one method | 290.6 | 111.8 | 0.38 | 24,552,392 | 14,810,767 | 0.60 |
| iOS (30.2 MB) | no change | 989.6 | 178.4 | 0.18 | 143,811,937 | 60,494,716 | 0.42 |
| | one document | 1,067.5 | 439.9 | 0.41 | 169,238,884 | 102,161,252 | 0.60 |
| | one method | 1,003.6 | 590.5 | 0.59 | 169,238,850 | 112,063,647 | 0.66 |

- An edit of one document or of one method now takes 0.3 to 0.6 of the time, and its peak live
  heap is 0.46 to 0.69 of the value before.
- An edit of one method still decodes all methods, because the editor decodes a whole table on
  the first change of the table (#49). The edit also adds each sequence point blob to `#Blob`
  again.
- An edit that adds a value to `#Blob` still makes the value index of the source heap: a copy of
  each distinct entry. This is the largest remaining cost of these edits on the iOS input.
- The allocation counts do not depend on the load. For the iOS input, the edit of one document
  allocated 164,558,898 bytes (before: 235,292,715).
