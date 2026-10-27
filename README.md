# pdb-rs

pdb-rs is a Rust library for Program Database (PDB) files. It reads, writes, builds and edits
native Windows PDBs and standalone Portable .NET PDBs. It works with the standard library or
with `no_std + alloc`.

## Features

- Native PDBs in MSF 7.0 containers: read, write, build and edit.
- Native PDBs in MSFZ containers (PDZ files): read, write, edit and convert to and from MSF 7.0.
- Portable PDBs: read, write, build and edit.
- One editor for each backend, with tasks for common edits:
  - Source Link and the `srcsrv` source server stream.
  - Symbol renames.
  - Source path remapping.
  - Type and ID removal.
  - Address remapping for a rewritten image, with OMAP tables or clean output.
- `no_std + alloc` support. The default `std` feature adds file access.
- Verification of the outputs against `pdbtool`, DIA, `llvm-pdbutil` and
  `System.Reflection.Metadata`.

## Installation

The crate is not on crates.io yet. Add it from the Git repository:

```toml
[dependencies]
pdb-rs = { git = "https://github.com/napbat/pdb-rs" }
```

## Quick start

`pdb_rs::Pdb` detects the format from the file contents. This program prints the identity of a
PDB, the type names of a native PDB and the document names of a Portable PDB:

```rust
use pdb_rs::Pdb;
use pdb_rs::native::codeview::type_record::TypeLeaf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Pdb::open("app.pdb")? {
        Pdb::Native(pdb) => {
            println!("native PDB, identity {:?}", pdb.info().identity());
            if let Some(types) = pdb.types()? {
                for record in types.records() {
                    let record = record?;
                    if let Some(tag) = TypeLeaf::decode(record.record)?.tag() {
                        println!("{:#x}: {}", record.index, String::from_utf8_lossy(&tag.name));
                    }
                }
            }
        }
        Pdb::Portable(pdb) => {
            println!("Portable PDB, identity {:?}", pdb.identity());
            for document in pdb.documents() {
                let (_, document) = document?;
                println!("{}", String::from_utf8_lossy(&document.name));
            }
        }
        _ => println!("other format"),
    }
    Ok(())
}
```

The library keeps names as bytes (`pdb_rs::Bytes`). It does not decode text or demangle names.
Record kinds without a decoder stay available as raw bytes. A decoded record borrows the PDB
bytes; `into_owned` gives a copy that owns its bytes.

## Edit a PDB

This program removes the build directory from the source paths of a native PDB:

```rust
use pdb_rs::native::{EditOptions, IdentityPolicy, Pdb};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pdb = Pdb::open("app.pdb")?;
    // The task does not change symbols or addresses, so the image keeps its match.
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    let changed = editor.remap_source_paths(|path| {
        path.strip_prefix(br"C:\build\".as_slice())
            .map(|rest| [b"/_/".as_slice(), rest].concat())
    })?;
    println!("{changed} paths get a new value");
    editor.plan(EditOptions::default())?.write_file("app.remapped.pdb")?;
    Ok(())
}
```

The [cookbook](docs/cookbook.md) gives more recipes and their cautions: Source Link, `srcsrv`,
renames, embedded sources, type and ID removal, and address remapping.

## Status

| Format family | Status | Details |
| --- | --- | --- |
| Native PDB (MSF 7.0) | Read, write, build and edit | [Design](docs/design/pdb.md#implemented-native-reading), [coverage](docs/design/native-coverage.md) |
| Native PDB in MSFZ (PDZ) | Read, write, edit and convert | [Design](docs/design/pdb.md#implemented-msfz-reading) |
| Portable PDB | Read, write, build and edit | [Design](docs/design/pdb.md#implemented-portable-pdb-reading) |
| Legacy native PDB (Small MSF 2.00) | Signature identification only | [Design](docs/design/pdb.md#deferred-legacy-native-pdb-support) |

The root epic [#18](https://github.com/napbat/pdb-rs/issues/18) gives the roadmap.

## Features of the crate

- `std` (default): file access through `Pdb::open` and standard I/O error integration.
- Without default features, the library uses `no_std + alloc`. Use `Pdb::parse` for a byte
  slice, or `Pdb::read` with a `ReadAt` source such as `io::SeekReader`.

## Examples

The examples in [crates/pdb-rs/examples/](crates/pdb-rs/examples/) need the `std` feature, except
`streams`. For example:

```sh
# Read every stream and record of a native PDB, encode each again and report undecoded data.
cargo run -p pdb-rs --example native -- path/to/file.pdb

# Apply the editing tasks to a native or a Portable PDB.
cargo run -p pdb-rs --example edit -- native input.pdb output.pdb
cargo run -p pdb-rs --example edit -- portable input.pdb output.pdb

# Convert MSF 7.0 to MSFZ with the options of `pdbtool pdz-encode`.
cargo run -p pdb-rs --example msf -- pdz-encode --max-chunk-size 64K --pad16k input.pdb output.pdz.pdb
```

Other examples: `inspect`, `locate`, `msf`, `native_rebuild`, `native_synthetic`, `portable`,
`streams` and `types`.

## Fixtures and verification tools

The `pdb-rs-fixtures` tool builds the demo programs in [fixtures/](fixtures/) with MSVC,
Clang/LLD, Rust and .NET. It writes the PDBs and a provenance manifest to `target/fixtures/`.
It also installs the consumer tools and compares the output of pdb-rs with them:

```sh
cargo run -p pdb-rs-fixtures
cargo run -p pdb-rs-fixtures -- tools
cargo run -p pdb-rs-fixtures -- detect
```

Automated tests do not read the fixtures; they use synthetic data.
[fixtures/README.md](fixtures/README.md) gives the fixtures, the tools and each `verify-*`
command.

## Development

Read [AGENTS.md](AGENTS.md) before you change the repository. It gives the rules and the
checks. [docs/fuzzing.md](docs/fuzzing.md) gives the fuzz targets, and
[docs/benchmarks/](docs/benchmarks/README.md) gives the benchmark method and results.

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/pdb-rs/` | The library, its examples, its integration tests and the `baseline` benchmark. |
| `crates/pdb-rs/fuzz/` | The `cargo-fuzz` targets. This package is not a workspace member, because it needs the nightly toolchain. |
| `crates/pdb-rs-fixtures/` | The fixture generator and the verification commands. |
| `fixtures/` | The demo program sources for the fixtures and the .NET Portable PDB verifier. |
| `docs/cookbook.md` | Recipes for common edits. |
| `docs/design/` | The design, the implementation evidence and the native coverage. |
| `docs/benchmarks/` | The benchmark method, its limits and the recorded numbers. |
| `docs/fuzzing.md` | The fuzz targets, their commands and limits, and the recorded runs. |
| `docs/reference/` | Pinned LLVM and Microsoft PDB documentation, with provenance and licenses. |

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

The pinned upstream documents in [docs/reference/](docs/reference/README.md) keep their own
licenses.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above,
without any additional terms or conditions.
