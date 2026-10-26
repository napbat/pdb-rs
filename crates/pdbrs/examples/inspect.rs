//! Inspect a container signature without claiming that its contents form a valid PDB.

use pdbrs::io::{FileReader, ReadAt};
use pdbrs::probe;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let path = arguments
        .next()
        .ok_or("usage: cargo run -p pdbrs --example inspect -- <path>")?;
    if arguments.next().is_some() {
        return Err("expected exactly one input path".into());
    }

    let source = FileReader::open(&path)?;
    let kind = probe(&source)?;
    println!("Container: {kind:?}");
    println!("Bytes: {}", source.len()?);
    println!("Signature identification only; PDB contents have not been validated.");
    Ok(())
}
