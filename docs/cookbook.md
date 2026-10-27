# pdb-rs cookbook: common edits

This document gives recipes for common edits of native and Portable PDBs. The
[README](../README.md) gives the installation and a quick start.

The editors have one function for each common edit (#41, #46). Each function checks all its
inputs before it changes the model, so an error leaves the editor unchanged. Plan and write as
for any other edit. The [`edit` example](../crates/pdb-rs/examples/edit.rs) runs the recipes of the
first two sections.

## Source Link, `srcsrv`, named streams and renames (native PDB)

```rust
use pdb_rs::native::{EditOptions, IdentityPolicy, Pdb};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pdb = Pdb::open("app.pdb")?;
    // A rename changes the symbols: give the image the new identity.
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor.set_source_link(br#"{"documents":{"C:\\src\\*":"https://example.com/src/*"}}"#.to_vec())?;
    editor.set_source_server(std::fs::read("srcsrv.txt")?)?; // the `srcsrv` stream
    editor.remove_named_stream(b"/LinkInfo")?;
    // The function sees each name once. A C++ function has a decorated public name and an
    // undecorated procedure name; the procedure and its `S_PROCREF` get the same new name.
    editor.rename_symbols(|name| (name == b"helper").then(|| b"helper_v2".to_vec()))?;
    let plan = editor.plan(EditOptions::default())?;
    plan.write_file("app.edited.pdb")?;
    println!("write into the image: {:?}", plan.identity()?);
    Ok(())
}
```

## Source Link and embedded sources (Portable PDB)

```rust
use pdb_rs::portable::embedded::EmbeddedFormat;
use pdb_rs::portable::metadata::DocumentId;
use pdb_rs::portable::{EditOptions, IdentityPolicy, Pdb};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pdb = Pdb::open("app.pdb")?;
    let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
    editor.set_source_link(br#"{"documents":{"C:\\src\\*":"https://example.com/src/*"}}"#.to_vec())?;
    // Embed the file of the build: a debugger compares it with the document hash.
    editor.embed_source(DocumentId(1), &std::fs::read("Program.cs")?, EmbeddedFormat::Deflate)?;
    editor.plan(EditOptions::default())?.write_file("app.edited.pdb")?;
    Ok(())
}
```

## Identity policy

Use `IdentityPolicy::Preserve` for a change that does not change the symbols, for example
only a Source Link or a `srcsrv` change: the image then keeps its match.

## Source path remapping

`remap_source_paths` gives the source paths new values, for example to remove the build
directory from a PDB (#46). The function gets each distinct path once and returns the new path,
or `None` to keep the path. The native task changes each path field and builds `/names` again.
The portable task changes the document names.

```rust
use pdb_rs::{native, portable};

/// Replace the build directory `C:\build\` with `/_/`.
fn remap(path: &[u8]) -> Option<Vec<u8>> {
    path.strip_prefix(br"C:\build\".as_slice())
        .map(|rest| [b"/_/".as_slice(), rest].concat())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pdb = native::Pdb::open("app.pdb")?;
    // The task does not change symbols or addresses, so the image keeps its match.
    let mut editor = pdb.edit(native::IdentityPolicy::Preserve);
    let changed = editor.remap_source_paths(remap)?;
    println!("{changed} paths get a new value");
    editor.plan(native::EditOptions::default())?.write_file("app.remapped.pdb")?;

    let pdb = portable::Pdb::open("App.pdb")?;
    let mut editor = pdb.edit(portable::IdentityPolicy::ContentDerived);
    editor.remap_source_paths(remap)?;
    editor.plan(portable::EditOptions::default())?.write_file("App.remapped.pdb")?;
    Ok(())
}
```

## Type and ID removal

`TypeList::remove` and `IdList::remove` remove a type or ID record (#47). The removed record
becomes a tombstone: in the model, the other records keep their indices. The plan gives each
record that is not removed the next output index and writes the output index into each
reference. `PdbWritePlan::type_index` and `PdbWritePlan::id_index` give the output index of a
model index.

Caution: the plan cannot see references in other files. An object file that uses a type-server
PDB, a precompiled-header object (`/Yc`) and the object files of a `/DEBUG:FASTLINK` PDB keep
the old indices. Remove records only from a PDB that no other file refers to by index.

```rust
use pdb_rs::native::codeview::index::TypeIndex;
use pdb_rs::native::{EditOptions, IdentityPolicy, Pdb};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pdb = Pdb::open("app.pdb")?;
    // A removal changes the type indices: give the image the new identity.
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    // A reference to the removed record makes the plan fail with `InvalidReference`.
    editor.types_mut()?.remove(TypeIndex(0x1008))?;
    let plan = editor.plan(EditOptions::default())?;
    let moved = plan.type_index(TypeIndex(0x1009))?;
    println!("type 0x1009 is now {:#x}", moved.0);
    plan.write_file("app.compacted.pdb")?;
    println!("write into the image: {:?}", plan.identity()?);
    Ok(())
}
```

The plan refuses the compaction with `UnsupportedEdit` for a PDB without a DBI stream, a
`/DEBUG:FASTLINK` PDB, a PDB without type merging (`NOTM`), a PDB whose modules use type
servers, and data without a decoder that can hold an index. The
[design](design/pdb.md#implemented-native-editing) describes the removal.

## Address remapping

A binary rewriter moves, splits or removes code after the link. `PdbEditor::remap_addresses`
makes the PDB describe the new image (#43). The caller gives the section headers of the new
image and one range for each moved piece: old RVA, length, new RVA. An old address without a
range is removed code.

```rust
use pdb_rs::native::address::Rva;
use pdb_rs::native::section::{SectionHeader, SectionHeaderStream};
use pdb_rs::native::{EditOptions, IdentityPolicy, ImageRemap, Pdb, RemapMode, RemovedCode};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // pdb-rs has no PE parser: read the section headers from the new image.
    let text = SectionHeader::new(*b".text\0\0\0", 0x3000, 0x1000); // name, size, RVA
    let remap = ImageRemap::new(SectionHeaderStream::from_records(&[text])?)
        .with_range(Rva(0x1000), 0x2a0, Rva(0x1400))? // old RVA, length, new RVA
        .with_range(Rva(0x12a0), 0x40, Rva(0x1000))?;
    let pdb = Pdb::open("app.pdb")?;
    // The output describes another image: give it a new identity.
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    // OMAP output: the records keep their addresses, and OMAP tables translate them.
    // Clean output: the plan writes the new addresses into the records.
    editor.remap_addresses(&remap, RemapMode::Clean(RemovedCode::Drop))?;
    let plan = editor.plan(EditOptions::default())?;
    if let Some(report) = plan.report().remap() {
        for (item, count) in report.removed_items() {
            println!("removed {count} records of {item:?}");
        }
    }
    plan.write_file("app.rewritten.pdb")?;
    println!("write into the image: {:?}", plan.identity()?);
    Ok(())
}
```

- `RemapMode::Omap` writes the OMAP tables and the section headers of the new image, and
  copies the other streams. DIA and `AddressMap` translate each address as the ranges say.
- `RemapMode::Clean` writes the new addresses into the records, without OMAP. A record that a
  range splits gets a split form, for example an `S_SEPCODE` scope for a procedure part. A
  kind without a split form, for example `S_BLOCK32` or an FPO record, makes the plan fail.
- `RemovedCode::Fail` (the default) makes the plan fail at the first record with removed
  code. `RemovedCode::Drop` removes these records and the references to them.
- The error of a rejected record names it: `Error::remap_address` gives the kind and the
  address.
- If the PDB has `.pdata` or `.xdata` streams, give the streams of the new image to
  `ImageRemap::with_unwind_streams` for clean output. pdb-rs does not decode `.xdata`.

The [design](design/pdb.md#address-remap) gives all rules of both modes.
