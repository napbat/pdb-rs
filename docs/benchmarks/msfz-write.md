# MSFZ write: compress each chunk once (#50)

This file records the effect of #50 on the MSFZ writer. Before #50, the plan compressed each
chunk to get its size, and `MsfzWritePlan::write_to` compressed each chunk again (#38). Now the
plan keeps the compressed chunks up to `MsfzWriteOptions::max_kept_chunk_bytes` (256 MiB by
default), and `write_to` compresses only the chunks after the kept chunks again.

Status: recorded on 2026-10-26 on a loaded machine. Compare the ratios, not the absolute times.
Run the bench of [README.md](README.md) again on a quiet machine to replace the times.

## Method

- Before: `main` at `a2162d8`. After: commit `b4154eb`. Both are release builds with the same
  toolchain as [baseline.md](baseline.md), on the same machine.
- Bench: `cargo bench -p pdbrs --bench baseline -- --filter msfz/write-default --runs 10`. The
  row `msfz/write-default` writes `msvc-x64-large` with the default options (1 warm-up run and
  10 timed runs in one child process).
- Passes: a throwaway program (not in the repository) timed `Container::to_msfz` (the plan) and
  `MsfzWritePlan::to_vec` in one process, with 1 warm-up run and 5 timed runs. It ran the
  `verify-msfz` option sets `small-chunks` (the largest output, 7,341,476 bytes) and `deflate`
  (the longest write) on `msvc-x64-large`. Its peak working set is the peak of the whole
  process, as in the bench.
- Order: 7 rounds. Each round ran the before build and the after build one after the other. The
  order changed in each round (before first in rounds 1, 3, 5 and 7). The tables give the median
  of the 7 rounds.
- Load: other agents built and tested in other worktrees at the same time. The processor load
  at the start of a round was from 2 % to 91 %, with 1 to 26 `rustc`, `cargo` and `link`
  processes.

## Results

All outputs have the same size before and after (6,689,764 bytes for the default options).

| Write of `msvc-x64-large` | Before | After | After / before |
| --- | ---: | ---: | ---: |
| Default options, bench median ms | 294.3 | 150.7 | 0.51 |
| Default options, bench minimum ms | 268.3 | 147.5 | 0.55 |
| Default options, bench peak working set MiB | 39.5 | 41.1 | 1.04 |
| `small-chunks`, plan ms | 215.3 | 218.2 | 1.01 |
| `small-chunks`, `to_vec` ms | 209.6 | 0.9 | 0.004 |
| `small-chunks`, total ms | 430.2 | 219.2 | 0.51 |
| `small-chunks`, peak working set MiB | 33.4 | 40.0 | 1.20 |
| `deflate`, plan ms | 440.9 | 442.4 | 1.00 |
| `deflate`, `to_vec` ms | 447.4 | 0.5 | 0.001 |
| `deflate`, total ms | 891.2 | 442.9 | 0.50 |
| `deflate`, peak working set MiB | 31.3 | 33.1 | 1.06 |

The quiet baseline of #44 for the default options is a median of 271.2 ms and a peak working
set of 39.9 MiB.

- A write now takes about half of the time. The plan takes about the same time as before.
  `to_vec` only copies the kept chunks, the stream directory and the Chunk Table.
- The plan keeps all chunks of these writes: 6.4 MiB for the default options, 6.8 MiB for
  `small-chunks` and 4.2 MiB for `deflate`. The peak working set grew by less than these
  values: 1.6 MiB, 6.6 MiB and 1.8 MiB.
- The ratios of the 7 rounds for the default bench median were from 0.46 to 0.80. The highest
  value comes from the round with the most load (91 %).
- A full bench run of the after build (all rows, `--runs 10`) gave no `failed` or `missing` row,
  and the counts were the same as in [baseline.md](baseline.md).

## Memory bound

`MsfzWriteOptions::with_max_kept_chunk_bytes` sets the bound. The plan holds at most this number
of bytes of compressed chunks. It keeps the chunks in Chunk Table order and stops at the first
chunk that does not fit. If an allocation for the kept chunks fails, the plan keeps no more
chunks. A value of 0 gives the two compression passes of #38. The limit does not change the
output bytes.
