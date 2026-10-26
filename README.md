# pdbrs

pdbrs is a Rust library for Program Database (PDB) files. It reads native Windows PDB files
(MSF 7.0), writes MSF 7.0 containers and reads the metadata of standalone Portable .NET PDB
files. `pdbrs::Pdb` detects the format from the file contents. The library supports `std` and
`no_std + alloc`.

## Status

| Area | Status |
| --- | --- |
| Native PDB reading (MSF 7.0) | Reads every stream, record and C13 subsection in Rust-produced test PDBs from MSVC `link.exe` and LLD. Other producers use record kinds that pdbrs keeps raw ([#13](https://github.com/napbat/pdbrs/issues/13), [#14](https://github.com/napbat/pdbrs/issues/14)). |
| Native MSF 7.0 container writing | Exact copies, stream-preserving repacks and new layouts with page sizes from 512 to 32768 ([#15](https://github.com/napbat/pdbrs/issues/15)). The writer rejects 65536-byte pages because DIA, LLVM and `link.exe` reject them; the reader accepts them. The writer makes no PDB semantic claim about the streams. |
| Native PDB semantic building and editing | Not implemented. |
| Portable PDB | Reads the metadata root, the heaps, the `#Pdb` stream, the `#~` tables and the raw rows of the eight debug tables. Results match `System.Reflection.Metadata` for the four standalone fixtures ([#22](https://github.com/napbat/pdbrs/issues/22)). Blob decoding is partial and writing is not implemented ([#17](https://github.com/napbat/pdbrs/issues/17)). |
| Native PDB in MSFZ (PDZ files) | Not implemented. `format::probe` does not recognize the signature yet ([#20](https://github.com/napbat/pdbrs/issues/20)). |
| Legacy native PDB (Small MSF 2.00) | Signature identification only ([#16](https://github.com/napbat/pdbrs/issues/16)). |

The root epic [#18](https://github.com/napbat/pdbrs/issues/18) has one epic for each format family:
[#1](https://github.com/napbat/pdbrs/issues/1) (native MSF 7.0),
[#20](https://github.com/napbat/pdbrs/issues/20) (native MSFZ),
[#17](https://github.com/napbat/pdbrs/issues/17) (Portable PDB) and
[#16](https://github.com/napbat/pdbrs/issues/16) (legacy MSF 2.0, deferred).
The [design](docs/design/pdb.md) lists each implemented area and its limits.

## Use

Add the library from this repository:

```toml
[dependencies]
pdbrs = { git = "https://github.com/napbat/pdbrs" }
```

Read the identity, the type records and the module symbols of a PDB:

```rust
use pdbrs::native::Pdb;
use pdbrs::native::codeview::symbol::ModuleSymbols;
use pdbrs::native::codeview::type_record::TypeLeaf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pdb = Pdb::open("app.pdb")?;
    println!("identity: {:?}", pdb.info().identity());

    if let Some(types) = pdb.types()? {
        for record in types.records() {
            let record = record?;
            if let Some(tag) = TypeLeaf::decode(record.record)?.tag() {
                println!("{:#x}: {}", record.index, String::from_utf8_lossy(tag.name));
            }
        }
    }

    if let Some(dbi) = pdb.debug_info()? {
        for module in dbi.modules() {
            let Some(stream) = pdb.module_stream(&module?)? else { continue };
            let base = u32::try_from(stream.symbol_offset())?;
            for symbol in ModuleSymbols::new(stream.symbol_data(), stream.stream(), base, 256) {
                if let Some(name) = symbol?.symbol.name() {
                    println!("{}", String::from_utf8_lossy(name));
                }
            }
        }
    }
    Ok(())
}
```

The library keeps names as bytes. It does not decode text or demangle names.
Record kinds without a decoder stay available as raw bytes.

## Features

- `std` (default): file access through `Pdb::open` and standard I/O error integration.
- Without default features, the library uses `no_std + alloc`. Use `Pdb::parse` for a byte
  slice, or `Pdb::read` with a `ReadAt` source such as `io::SeekReader`.

## Examples

```sh
# Read every stream and record of a native PDB and report undecoded data.
cargo run -p pdbrs --example native -- path/to/file.pdb

# Read every table row and heap value of a standalone Portable PDB.
cargo run -p pdbrs --example portable -- path/to/file.pdb

# Identify a container signature.
cargo run -p pdbrs --example inspect -- path/to/file

# Exercise sequential and positional I/O, also without the standard library.
cargo run -p pdbrs --example streams --no-default-features
```

```sh
# Copy a native PDB byte for byte, or repack its streams into a new MSF layout.
cargo run -p pdbrs --example msf -- copy input.pdb copy.pdb
cargo run -p pdbrs --example msf -- repack --page-size 8192 input.pdb repacked.pdb

# Compare the streams of two containers and check the free page map of the second.
cargo run -p pdbrs --example msf -- compare input.pdb repacked.pdb
```

The `native` example fails if a record kind, a subsection kind or a stream has no typed decoder.
It also reports producer defects that do not stop reading. The `msf` example writes through a
temporary file and does not change the destination if an error occurs. Do not give the input
file as the output.

## Test fixtures

[fixtures/](fixtures/README.md) holds demo programs in C, C++, Rust, C#, Visual Basic and F#.
The `pdbrs-fixtures` tool builds them with MSVC, Clang/LLD, Rust and .NET and writes PDBs
and a provenance manifest to `target/fixtures/`:

```sh
cargo run -p pdbrs-fixtures
```

The tool finds MSVC and Clang through the Visual Studio installation. Generated files are not
committed. Automated tests do not read them; they use synthetic data.

## Consumer tools

Compare the output of pdbrs with three independent consumers: Microsoft's `pdbtool`,
`DIA2Dump` from the Visual Studio DIA SDK and a .NET verifier that uses
`System.Reflection.Metadata`. This command installs `pdbtool` 0.1.22 and builds `DIA2Dump`
and the verifier into `target/tools/bin/`:

```sh
cargo run -p pdbrs-fixtures -- tools
```

The command finds Visual Studio through `vswhere` and builds a changed copy of the DIA2Dump
sample. It writes the version, source and path of each tool to `target/tools/manifest.json`.
The fixture manifest copies these records. [fixtures/README.md](fixtures/README.md#consumer-tools)
gives the steps and the use of the tools.

Compare the Portable PDB fixtures with the verifier, and check the format detection of all
fixture outputs:

```sh
cargo run -p pdbrs-fixtures -- verify-portable
cargo run -p pdbrs-fixtures -- detect
```

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/pdbrs/` | The library, its examples and its integration tests. |
| `crates/pdbrs-fixtures/` | The fixture generator, the `tools` command for the consumer tools and the `verify-portable` and `detect` commands. |
| `fixtures/` | Demo program sources for the fixtures and the .NET Portable PDB verifier. |
| `docs/design/` | The reader and writer design and the implementation evidence. |
| `docs/reference/` | Pinned LLVM and Microsoft PDB documentation, with provenance and licenses. |

## Development

Read [AGENTS.md](AGENTS.md) before you change the repository. Run these checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --no-default-features -- -D warnings
cargo doc --workspace --no-deps
cargo doc --workspace --no-deps --no-default-features
cargo test --workspace
cargo test --workspace --no-default-features
```

The repository stores text files with LF line endings (`.gitattributes`).
