//! Rebuild a native PDB from its decoded records.
//!
//! The example reads every stream of the input, decodes every record and
//! builds a new PDB from the decoded values with `PdbBuilder`. It keeps the
//! identity of the input, so the image of the input matches the output. It
//! plans the output twice and fails if the two plans give different bytes.
//! The input can be an MSF 7.00 or an MSFZ PDB. The output is MSF 7.00.
//!
//! Usage: `cargo run --example native_rebuild -- <input.pdb> <output.pdb>`

use std::error::Error;

use pdbrs::native::IdentityPolicy;
use pdbrs::native::Pdb;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let usage = "usage: cargo run --example native_rebuild -- <input.pdb> <output.pdb>";
    let input = arguments.next().ok_or(usage)?;
    let output = arguments.next().ok_or(usage)?;
    if arguments.next().is_some() {
        return Err(usage.into());
    }
    let pdb = Pdb::open(&input)?;
    let contents = pdb.contents()?;
    let options = contents.build_options();
    let builder = contents.to_builder(IdentityPolicy::Preserve)?;
    let plan = builder.plan(&options)?;
    let bytes = plan.to_vec()?;
    let again = contents
        .to_builder(IdentityPolicy::Preserve)?
        .plan(&options)?
        .to_vec()?;
    if bytes != again {
        return Err("two plans of the same input give different bytes".into());
    }
    std::fs::write(&output, &bytes)?;
    println!("{builder:?}");
    println!(
        "input container: {:?}, streams: {}",
        pdb.container().kind(),
        pdb.container().stream_count()
    );
    println!("output streams: {}", plan.stream_count());
    println!("output bytes: {}", bytes.len());
    println!(
        "unclaimed input streams: {:?}",
        contents.unclaimed_streams()
    );
    let pe = plan.pe_match();
    println!("PE match: guid {:02x?}, age {}", pe.guid, pe.age);
    println!("deterministic: same bytes in two plans");
    Ok(())
}
