# Agent rules

These rules apply to all agents that change this repository. Obey all rules in all changes.

## Language: ASD-STE100 style

Write all new and changed text in a style like ASD-STE100 Simplified Technical English. This
includes doc comments, code comments, Markdown files and commit messages. Follow the writing
rules below. The STE dictionary does not apply: use any clear English word.

- Keep sentences short: about 20 words for an instruction, about 25 words for a description.
- Write one instruction in each sentence.
- Use the imperative form for instructions.
- Use the active voice.
- Prefer simple tenses.
- Give each word one meaning, and use the same term for the same item each time.
- Use the names from the code for code items.
- Use articles ("the", "a") where they make the text clearer.
- Avoid long noun clusters. Break up clusters of more than three words.
- Write short paragraphs with one topic each.
- Use a vertical list for a sequence of steps or for many items.
- Put a warning or a caution before the instruction it applies to.

Write a commit message as `area: summary`, with a lowercase summary. Use the crate or module
name as the area (for example `pdb-rs`).

## Project

The repository is a Cargo workspace. Its `pdb-rs` library uses edition 2024 and lives in
`crates/pdb-rs/`. The library root is `crates/pdb-rs/src/lib.rs`.

The default `std` feature enables `nostdio/std` and file access on Unix and Windows.
Use `nostdio` for sequential I/O. Keep `ReadAt` for positional access.
Keep the shared primitives usable with `--no-default-features` (`no_std + alloc`).
The `inspect` example identifies container signatures; it does not validate PDB contents.
The `native` example reads every stream and record of a native PDB. It fails if data has no typed decoder.
Run it on real PDBs after reader changes, and compare decoded fields with `llvm-pdbutil`.

- Keep the standard Cargo package layout.
- Do not assume that this repository has docstore's crates, layers or storage engine.
- Check the manifest and source before you describe project behavior or dependencies.
- For a new dependency, choose a portable crate that supports `no_std + alloc`. Use it in only
  one crate-private module, and do not expose its types, traits or errors in the public API.
  Record the choice in the issue. A later change can replace the crate.
- Only `compression.rs` and its child modules name `miniz_oxide` and `ruzstd`. Only
  `crypto.rs` and its child modules name `sha2` and `digest`.
- Keep the `compression` and `crypto` modules crate-private. Do not put their dependency types,
  traits or errors in the public API.
- Check the declared decoded size against a limit before you allocate. Then decode into a buffer
  of the declared size.
- Compare reader and writer output with the independent consumers of #21 (`pdbtool`, DIA2Dump)
  as well as `llvm-pdbutil`.

## Lints: Clippy pedantic

The workspace denies warnings, Clippy `pedantic` lints, `missing_docs` and `unsafe_code`.
The root `Cargo.toml` sets these levels in `[workspace.lints]`.

- Keep `[lints] workspace = true` in each crate manifest.
- Do not lower a lint level in the manifest or in the code.
- Do not use `#[allow(...)]`.
- Use `#[expect(lint, reason = "...")]` only if no change to the code can remove the lint.
- Do not add lint configuration to hide a lint.
- Do not add `unsafe` code.
- Document public items.

## File size: 1000 lines

- Keep each source file at a maximum of 1000 lines. This includes integration tests.
- If a change makes a file longer than 1000 lines, split the file into submodules.
- If you change a file that is already longer than 1000 lines, split it as part of the change.
- Put each submodule in a new file below the directory of its parent module.
- Split by topic. Do not split at a random line.
- Keep each function at a maximum of 200 lines.

## Layout: Cargo package layout

Follow the Cargo package layout:
<https://doc.rust-lang.org/cargo/guide/project-layout.html>.

Put each crate in `crates/<crate-name>/`. The paths below are relative to each crate.

- Put the library root in `src/lib.rs` when a library is needed.
- Put the default executable in `src/main.rs`.
- Put other executables in `src/bin/`.
- Put integration tests in `tests/`.
- Put examples in `examples/`.
- Put benchmarks in `benches/`.
- For a test, example, benchmark or executable with many files, make a directory with a `main.rs`
  file and the modules.
- Use snake_case names for modules.
- Put a module in `foo.rs` and its child modules in `foo/`. Do not use `mod.rs` files.

## Tests

- Add a regression test for each defect. Make sure that the test fails before the fix.
- Test observable behavior, boundaries and error cases rather than implementation details.
- Do not use tight wall-clock bounds. Use explicit signals or controlled time.
- Make sure that tests do not depend on execution order or shared mutable state.
- Record benchmark measurements with their method and limits in `docs/benchmarks/` when benchmarks are added.

## Tracking

- Track all work in GitHub issues in `napbat/pdb-rs`. Do not add ticket or tracking files to the
  repository.
- Put the scope and acceptance criteria of each work area in its issue. Link a sub-issue to its
  parent issue.
- Put each work area under the epic of its format family: #1 (native MSF 7.0), #20 (native
  MSFZ), #17 (Portable PDB) or #16 (legacy MSF 2.0). These epics are children of the root epic
  #18. Put shared work, such as the `compression` and `crypto` modules, directly under #18. Give
  an epic the `epic` label.
- Record verification evidence as an issue comment. Close an issue only when all its acceptance
  criteria pass.
- If a check needs a file, tool or service that is not available, do not use a weaker check.
  Record what is missing in an issue comment, add the `external-input` label and leave the
  criterion open.
- Follow the goal brief in #18 for the milestone order of an autonomous run.
- Refer to issues by number, for example `#1`, in documentation and commit messages.

## Working copies

- Use one git worktree for each branch when agents work in parallel.
- Use absolute paths for all edits in a worktree. Do not edit the primary checkout from a
  worktree task.
- Preserve unrelated user changes.
- Commit completed work on `main` after the checks pass, then push `main` to `origin`. Do not
  force-push or rewrite pushed history.
- Merge a worktree branch into `main` after its checks pass. Then delete the branch and the
  worktree.

## Checks

Run these checks after code changes before you report the change as complete:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --no-default-features -- -D warnings
cargo doc --workspace --no-deps
cargo doc --workspace --no-deps --no-default-features
cargo test --workspace
cargo test --workspace --no-default-features
```

Exercise the changed behavior as well as the automated checks. Report which checks ran and
any checks that could not run. For documentation-only changes, check paths, commands and
claims against the repository; a Rust build is not required.
