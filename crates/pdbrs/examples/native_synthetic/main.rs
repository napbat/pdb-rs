//! Build the PDB of a small synthetic program from scratch.
//!
//! The example adds the records of `program.rs` to a new `PdbEditor`, plans
//! the output twice and fails if the two plans give different bytes. It
//! writes the PDB to the output path.
//!
//! Usage: `cargo run --example native_synthetic -- <output.pdb>`

mod program;

use std::error::Error;

use pdbrs::native::EditOptions;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let usage = "usage: cargo run --example native_synthetic -- <output.pdb>";
    let output = arguments.next().ok_or(usage)?;
    if arguments.next().is_some() {
        return Err(usage.into());
    }
    let options = EditOptions::default();
    let editor = program::build()?;
    let bytes = editor.plan(options)?.to_vec()?;
    if bytes != editor.plan(options)?.to_vec()? {
        return Err("two plans of the same records give different bytes".into());
    }
    std::fs::write(&output, &bytes)?;
    println!("output bytes: {}", bytes.len());
    println!("deterministic: same bytes in two plans");
    Ok(())
}
