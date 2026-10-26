//! The reference part of `verify-msfz`: pdbrs output and `pdbtool pdz-encode` output.
//!
//! Byte equality is not required. The part prints the facts of both files and the reasons
//! for the differences.

use std::fs;

use pdbrs::native::container::Container;

use super::super::native::consumers::parallel_map;
use super::decode::PdzFixture;
use super::encode::encode;
use super::{OPTION_SETS, read_streams, stream_differences};

/// The reasons why the bytes of pdbrs and of `pdbtool pdz-encode` differ.
const REASONS: &str = "\
  - pdz-encode stores the Info stream and the DBI stream header without compression; pdbrs \
puts all stream bytes in chunks.
  - pdz-encode ends chunks at DBI substreams, type records, symbol records and hash stream \
regions, and it uses chunks of at least 128 KiB for the TPI, IPI and global symbol streams; \
pdbrs fills each chunk to the maximum chunk size.
  - pdz-encode compresses with the zstd C library (level 0, the default level); pdbrs uses \
ruzstd (the \"Fastest\" level), so the pdbrs files are larger.
  - The different chunks give a different stream directory and Chunk Table.";

/// Write the source of each `pdz-*` fixture with the matching pdbrs options and compare the
/// bytes. The part skips the other PDZ fixtures, because `pdz-encode` did not write them.
pub(super) fn run(all: &[PdzFixture]) {
    let list: Vec<PdzFixture> = all
        .iter()
        .filter(|fixture| fixture.fixture.starts_with("pdz-"))
        .cloned()
        .collect();
    println!(
        "== reference: pdbrs and pdbtool pdz-encode for {} PDZ fixtures",
        list.len()
    );
    let results = parallel_map(&list, compare);
    let mut identical = 0;
    let mut compared = 0;
    for (fixture, result) in list.iter().zip(&results) {
        match result {
            Ok(Some(text)) => {
                compared += 1;
                identical += usize::from(text.starts_with("identical"));
                println!("{}: {text}", fixture.label());
            }
            Ok(None) => println!("{}: no matching option set", fixture.label()),
            Err(error) => println!("{}: error: {error}", fixture.label()),
        }
    }
    println!(
        "reference: {identical} of {compared} pdbrs files are byte-identical to pdz-encode; \
         this is not required. The reasons for different bytes:\n{REASONS}"
    );
}

/// Compare one PDZ fixture with the pdbrs output of the same options.
///
/// Return `None` if no option set matches the fixture.
fn compare(fixture: &PdzFixture) -> Result<Option<String>, String> {
    let Some((name, options)) = OPTION_SETS
        .iter()
        .find(|(name, _)| fixture.fixture.strip_prefix("pdz-") == Some(name))
    else {
        return Ok(None);
    };
    let read = |path: &std::path::Path| {
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
    };
    let theirs = read(&fixture.pdz)?;
    let (ours, _) = encode(&read(&fixture.original)?, options())?;
    if ours == theirs {
        return Ok(Some(format!("identical ({name})")));
    }
    let streams = match stream_differences(&read_streams(&theirs)?, &read_streams(&ours)?, 0) {
        None => "the same streams".to_owned(),
        Some(difference) => format!("STREAMS DIFFER: {difference}"),
    };
    Ok(Some(format!(
        "different bytes ({name}): pdz-encode {}, pdbrs {}; {streams}",
        facts(&theirs)?,
        facts(&ours)?
    )))
}

/// Describe the size, the chunks and the fragments of an MSFZ file.
fn facts(bytes: &[u8]) -> Result<String, String> {
    let container = Container::open(bytes).map_err(|error| format!("pdbrs: {error}"))?;
    let msfz = container
        .msfz()
        .ok_or_else(|| "not an MSFZ container".to_owned())?;
    Ok(format!(
        "{} bytes, {} chunks, {} fragments",
        bytes.len(),
        msfz.chunk_count(),
        msfz.fragment_count()
    ))
}
