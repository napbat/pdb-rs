//! Build the PDB of a small synthetic program from scratch.
//!
//! The example builds the records of `program.rs` with `PdbBuilder`, plans
//! the output twice and fails if the two plans give different bytes. It
//! writes the PDB to the output path.
//!
//! Usage: `cargo run --example native_synthetic -- <output.pdb>`

mod program;

use std::error::Error;

use pdbrs::native::write::BuildOptions;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let usage = "usage: cargo run --example native_synthetic -- <output.pdb>";
    let output = arguments.next().ok_or(usage)?;
    if arguments.next().is_some() {
        return Err(usage.into());
    }
    let options = BuildOptions::default();
    let bytes = program::build()?.plan(&options)?.to_vec()?;
    if bytes != program::build()?.plan(&options)?.to_vec()? {
        return Err("two plans of the same records give different bytes".into());
    }
    std::fs::write(&output, &bytes)?;
    println!("output bytes: {}", bytes.len());
    println!("deterministic: same bytes in two plans");
    Ok(())
}
