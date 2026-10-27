# pdb-rs fuzz targets

This package holds the `cargo-fuzz` targets of `pdb-rs` (#35). The targets use only the public
API of `pdb-rs`. `docs/fuzzing.md` lists the targets, the commands, the limits and the recorded
runs.

## Why this package is outside the workspace

The package has its own `[workspace]` table, and the root workspace does not list it. The
checks in `AGENTS.md` use the stable toolchain. `libfuzzer-sys` and the sanitizers need a
nightly toolchain, so the stable checks must not build this package.

The package does not use `[lints] workspace = true`, because it is not a workspace member.
Its manifest denies `missing_docs`, `unsafe_code`, `unused` and Clippy `pedantic`. This
configuration does not change the lints of the workspace.

## Requirements

- The nightly toolchain: `rustup toolchain install nightly`.
- `cargo-fuzz` 0.13.2: `cargo install cargo-fuzz --version 0.13.2`.
- On Windows, the MSVC AddressSanitizer runtime. Copy
  `clang_rt.asan_dynamic-x86_64.dll` from
  `<Visual Studio>/VC/Tools/MSVC/<version>/bin/Hostx64/x64/` into
  `target/x86_64-pc-windows-msvc/release/` of this package after the first build. Without
  it, a target exits with `STATUS_DLL_NOT_FOUND` (`0xc0000135`).

## Commands

Run the commands in `crates/pdb-rs/fuzz/`.

1. Write the seed corpora. Add fixture paths to add seeds from local PDB files:

   ```sh
   cargo +nightly run --release --example seeds -- [<fixture file or directory>...]
   ```

2. Build all targets:

   ```sh
   cargo +nightly fuzz build
   ```

3. Run one target:

   ```sh
   cargo +nightly fuzz run <target> -- -max_total_time=900 -rss_limit_mb=2048 -timeout=10
   ```

4. Run the checks of the package:

   ```sh
   cargo +nightly fmt --check
   cargo +nightly clippy --all-targets
   ```

Caution: do not commit `corpus/`, `artifacts/` or the fixtures. `.gitignore` excludes the
corpora and the artifacts.

For a finding, `cargo +nightly fuzz run <target> artifacts/<target>/<file>` runs the input
again. Add a regression test in `crates/pdb-rs/tests/` before the fix.
