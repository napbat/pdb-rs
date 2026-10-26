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
name as the area (for example `pdbrs`).

## Project

The repository is a Cargo workspace. Its `pdbrs` package uses edition 2024 and lives in
`crates/pdbrs/`. The default executable is in `crates/pdbrs/src/main.rs`.

- Keep the standard Cargo package layout.
- Do not assume that this repository has docstore's crates, layers or storage engine.
- Check the manifest and source before you describe project behavior or dependencies.

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

## Working copies

- Use one git worktree for each branch when agents work in parallel.
- Use absolute paths for all edits in a worktree. Do not edit the primary checkout from a
  worktree task.
- Preserve unrelated user changes.

## Checks

Run these checks after code changes before you report the change as complete:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --workspace --no-deps
cargo test --workspace
```

Exercise the changed behavior as well as the automated checks. Report which checks ran and
any checks that could not run. For documentation-only changes, check paths, commands and
claims against the repository; a Rust build is not required.
