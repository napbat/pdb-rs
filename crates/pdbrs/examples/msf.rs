//! Copy, repack and compare Big MSF 7.00 containers.
//!
//! Usage:
//!
//! ```text
//! msf copy <input> <output>
//! msf repack [--page-size <bytes>|source] [--old-directory empty|preserve] <input> <output>
//! msf compare <original> <written>
//! ```
//!
//! - `copy` writes a byte-identical copy. It does not check the input.
//! - `repack` writes a new layout with the same streams. The default page size is 4096 and
//!   the default stream 0 policy is `empty`. `--page-size source` keeps the page size of the
//!   input. A preserved stream 0 with bytes needs the page size of the input.
//! - `compare` opens both files as MSF containers. It compares the presence, the size and the
//!   bytes of each stream, and it checks the free page map of the written file. It fails if a
//!   stream other than stream 0 differs or if the written file has allocated pages without a
//!   reference.
//!
//! The output must not be the input file.

use std::error::Error;
use std::path::Path;

use pdbrs::io::FileReader;
use pdbrs::native::msf::{Msf, OldDirectoryPolicy, StreamId, WriteOptions, copy_exact_file};

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

/// The largest stream that `compare` loads at one time (1 GiB).
const MAX_STREAM_BYTES: usize = 1 << 30;

fn main() -> Outcome {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.split_first() {
        Some((command, rest)) if command == "copy" => copy(rest),
        Some((command, rest)) if command == "repack" => repack(rest),
        Some((command, rest)) if command == "compare" => compare(rest),
        _ => Err(usage()),
    }
}

/// Return the usage error.
fn usage() -> Box<dyn Error> {
    "usage: msf copy <input> <output> | msf repack [--page-size <bytes>|source] \
     [--old-directory empty|preserve] <input> <output> | msf compare <original> <written>"
        .into()
}

/// Write a byte-identical copy.
fn copy(arguments: &[String]) -> Outcome {
    let [input, output] = arguments else {
        return Err(usage());
    };
    check_paths(input, output)?;
    let source = FileReader::open(input)?;
    let length = copy_exact_file(&source, output)?;
    println!("copied {length} bytes (exact copy, no validation)");
    Ok(())
}

/// Write a repacked container.
fn repack(arguments: &[String]) -> Outcome {
    let mut options = WriteOptions::default();
    let mut source_page_size = false;
    let mut policy = OldDirectoryPolicy::default();
    let mut paths = Vec::new();
    let mut rest = arguments.iter();
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--page-size" => {
                let value = rest.next().ok_or_else(usage)?;
                source_page_size = value == "source";
                if !source_page_size {
                    options = options.with_page_size(value.parse()?);
                }
            }
            "--old-directory" => {
                policy = match rest.next().map(String::as_str) {
                    Some("empty") => OldDirectoryPolicy::Empty,
                    Some("preserve") => OldDirectoryPolicy::Preserve,
                    _ => return Err(usage()),
                };
            }
            _ => paths.push(argument),
        }
    }
    let [input, output] = paths.as_slice() else {
        return Err(usage());
    };
    check_paths(input, output)?;
    let msf = Msf::open(FileReader::open(input)?)?;
    if source_page_size {
        options = options.with_page_size(msf.block_size());
    }
    let plan = msf.repack(options, policy)?;
    plan.write_file(output)?;
    println!(
        "repacked {} streams: page size {}, {} pages, directory {} bytes, stream 0 {policy:?}",
        plan.stream_count(),
        plan.page_size(),
        plan.page_count(),
        plan.directory_size(),
    );
    Ok(())
}

/// Compare the streams of two containers and check the free page map of the second.
fn compare(arguments: &[String]) -> Outcome {
    let [original, written] = arguments else {
        return Err(usage());
    };
    let first = Msf::open(FileReader::open(original)?)?;
    let second = Msf::open(FileReader::open(written)?)?;
    for (name, msf) in [("original", &first), ("written", &second)] {
        println!(
            "{name}: page size {}, {} pages, {} streams, active FPM {}",
            msf.block_size(),
            msf.page_count(),
            msf.stream_count(),
            msf.active_free_page_map(),
        );
    }
    if first.stream_count() != second.stream_count() {
        return Err("stream counts differ".into());
    }
    let mut nil = 0_u32;
    let mut equal = 0_u32;
    let mut different = Vec::new();
    for id in (0..first.stream_count()).map(StreamId) {
        let left = first.stream(id)?;
        let right = second.stream(id)?;
        let same = match (&left, &right) {
            (None, None) => {
                nil += 1;
                true
            }
            (Some(left), Some(right)) => {
                left.read_all(MAX_STREAM_BYTES)? == right.read_all(MAX_STREAM_BYTES)?
            }
            _ => false,
        };
        if same {
            equal += u32::from(left.is_some());
        } else {
            different.push(id.0);
        }
    }
    println!("streams: {equal} present and equal, {nil} nil in both, different: {different:?}");
    let report = second.check_free_page_map()?;
    println!(
        "written FPM {}: stream 0 pages free {}, allocated {}; unreferenced allocated pages {}",
        report.active(),
        report.old_directory_free_pages(),
        report.old_directory_allocated_pages(),
        report.unreferenced_allocated_count(),
    );
    if different.iter().any(|&id| id != 0) {
        return Err("streams differ".into());
    }
    if report.unreferenced_allocated_count() != 0 {
        return Err("written file has allocated pages without a reference".into());
    }
    Ok(())
}

/// Reject an output path that names the input file.
fn check_paths(input: &str, output: &str) -> Outcome {
    let input = Path::new(input).canonicalize()?;
    if let Ok(output) = Path::new(output).canonicalize()
        && output == input
    {
        return Err("the output must not be the input file".into());
    }
    Ok(())
}
