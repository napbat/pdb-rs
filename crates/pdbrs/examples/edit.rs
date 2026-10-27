//! Recipes for the editing tasks of the native and the portable editor (#41).
//!
//! The `native` command applies each native task to a native PDB:
//!
//! 1. `set_source_link`: map the local source paths to URLs (Source Link).
//! 2. `set_source_server`: give the source server commands (`srcsrv`).
//! 3. `remove_named_stream`: remove `/LinkInfo`, if the PDB has it.
//! 4. `rename_symbols`: give the function `main` the name `program_main`: the
//!    public symbol, the procedure and its `S_PROCREF` record.
//!
//! The `portable` command applies each portable task to a standalone Portable
//! PDB:
//!
//! 1. `set_source_link`: map the local source paths to URLs (Source Link).
//! 2. `embed_source`: embed the file of each document without embedded
//!    source, if the file exists, as Roslyn does for `EmbedAllSources`.
//!
//! Each command plans the edit, writes the output and prints the plan report.
//! The rename changes the symbols, so the native command uses
//! `IdentityPolicy::IncrementAge`: write the printed identity into the image.
//!
//! Usage:
//!
//! ```text
//! cargo run --example edit -- native <input.pdb> <output.pdb>
//! cargo run --example edit -- portable <input.pdb> <output.pdb>
//! ```

use std::error::Error;
use std::ffi::OsString;
use std::path::Path;

use pdbrs::native::{self, Identity};
use pdbrs::portable::{self, custom::CustomDebugKind, custom::CustomDebugParent};
use pdbrs::portable::{embedded::EmbeddedFormat, metadata::DocumentId};

/// The Source Link file of both recipes.
const SOURCE_LINK: &str = r#"{"documents":{"C:\\src\\*":"https://example.com/src/*"}}"#;

/// The `srcsrv` stream of the native recipe: `SRCSRVTRG` gives the local path
/// of each source file, and the source files section lists the files.
const SOURCE_SERVER: &str = "SRCSRV: ini ------------------------------------------------\r\n\
VERSION=2\r\n\
VERCTRL=https\r\n\
SRCSRV: variables ------------------------------------------\r\n\
SRCSRVTRG=%targ%\\%var2%\r\n\
SRCSRV: source files ---------------------------------------\r\n\
C:\\src\\main.c*main.c\r\n\
SRCSRV: end ------------------------------------------------\r\n";

/// The usage text.
const USAGE: &str = "usage: cargo run --example edit -- native|portable <input.pdb> <output.pdb>";

/// Apply the native recipes to `input` and write `output`.
fn native(input: &Path, output: &Path) -> Result<(), Box<dyn Error>> {
    let pdb = native::Pdb::open(input)?;
    let mut editor = pdb.edit(native::IdentityPolicy::IncrementAge);

    // Recipe: set the Source Link file. The task checks the JSON shape.
    editor.set_source_link(SOURCE_LINK.as_bytes().to_vec())?;
    println!("set_source_link: sourcelink");

    // Recipe: set the source server commands. The task checks the sections.
    editor.set_source_server(SOURCE_SERVER.as_bytes().to_vec())?;
    println!("set_source_server: srcsrv");

    // Recipe: remove a named stream and its Info entry.
    if pdb.info().named_stream(b"/LinkInfo").is_some() {
        editor.remove_named_stream(b"/LinkInfo")?;
        println!("remove_named_stream: /LinkInfo");
    } else {
        println!("remove_named_stream: the PDB has no /LinkInfo");
    }

    // Recipe: rename a function. The function sees each name once. A C
    // function has the same name in its public symbol and its procedure; a
    // C++ function has a decorated public name, so return a name for both.
    let renamed =
        editor.rename_symbols(|name| (name == b"main").then(|| b"program_main".to_vec()))?;
    println!("rename_symbols: {renamed} records");

    let plan = editor.plan(native::EditOptions::default())?;
    plan.write_file(output)?;
    for stream in plan.report().streams() {
        if let (Some(id), change) = (stream.output, stream.change)
            && change != native::StreamChange::Copied
        {
            println!("stream {}: {change:?}", id.0);
        }
    }
    if let Identity::Modern { guid, age } = plan.identity()? {
        println!("write into the image: guid {guid:02x?}, age {age}");
    }
    Ok(())
}

/// Apply the portable recipes to `input` and write `output`.
fn portable(input: &Path, output: &Path) -> Result<(), Box<dyn Error>> {
    let pdb = portable::Pdb::open(input)?;
    let mut editor = pdb.edit(portable::IdentityPolicy::ContentDerived);

    // Recipe: set the Source Link file of the module.
    editor.set_source_link(SOURCE_LINK.as_bytes().to_vec())?;
    println!("set_source_link: module");

    // Recipe: embed each source file that is on disk and not embedded yet.
    let embedded: Vec<DocumentId> = pdb
        .custom_debug_informations()
        .filter_map(|information| {
            let (_, information) = information.ok()?;
            match information.parent {
                CustomDebugParent::Document(id)
                    if information.kind == CustomDebugKind::EMBEDDED_SOURCE =>
                {
                    Some(id)
                }
                _ => None,
            }
        })
        .collect();
    for document in pdb.documents() {
        let (id, document) = document?;
        let name = String::from_utf8_lossy(&document.name).into_owned();
        if embedded.contains(&id) {
            continue;
        }
        // A debugger compares the source with the hash of the document, so
        // embed only the file of the build.
        let Ok(source) = std::fs::read(&name) else {
            println!("embed_source: document {}: no file {name}", id.0);
            continue;
        };
        editor.embed_source(id, &source, EmbeddedFormat::Deflate)?;
        println!("embed_source: document {}: {} bytes", id.0, source.len());
    }

    let plan = editor.plan(portable::EditOptions::default())?;
    plan.write_file(output)?;
    for (stream, change) in plan.report().streams() {
        println!("stream {}: {change:?}", stream.name());
    }
    println!("PDB ID: {:02x?}", plan.identity().to_bytes());
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    let [command, input, output] = arguments.as_slice() else {
        return Err(USAGE.into());
    };
    match command.to_str() {
        Some("native") => native(Path::new(input), Path::new(output)),
        Some("portable") => portable(Path::new(input), Path::new(output)),
        _ => Err(USAGE.into()),
    }
}
