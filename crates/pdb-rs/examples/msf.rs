//! Copy, repack, convert and compare native PDB containers.
//!
//! Usage:
//!
//! ```text
//! msf copy <input> <output>
//! msf repack [--page-size <bytes>|source] [--old-directory empty|preserve] <input> <output>
//! msf pdz-encode [--max-chunk-size <bytes>[K|M]] [--one-stream-per-chunk]
//!     [--compress-stream-dir] [--pad16k] [--deflate] [--old-directory empty|preserve]
//!     <input> <output>
//! msf compare <original> <written>
//! ```
//!
//! - `copy` writes a byte-identical copy. It does not check the input.
//! - `repack` writes an MSF 7.00 container with the same streams. The input can be MSF 7.00
//!   or MSFZ, so `repack` also converts MSFZ to MSF 7.00. The default page size is 4096 and
//!   the default stream 0 policy is `empty`. `--page-size source` keeps the page size of an
//!   MSF 7.00 input. A preserved stream 0 with bytes needs the page size of the input.
//! - `pdz-encode` writes an MSFZ container with the same streams. The input can be MSF 7.00 or
//!   MSFZ. The options are those of `pdbtool pdz-encode`: the maximum chunk size (default
//!   1M, from 4K to 1024M), one stream for each chunk, a zstd stream directory and padding to
//!   16 KiB. `--deflate` writes raw DEFLATE chunks instead of zstd chunks. The default stream
//!   0 policy is `empty`, as `pdbtool pdz-encode` writes it.
//! - `compare` opens both files as MSF 7.00 or MSFZ containers. It compares the presence, the
//!   size and the bytes of each stream. For an MSF 7.00 written file, it also checks the free
//!   page map; MSFZ has no free page map. It fails if a stream other than stream 0 differs or
//!   if an MSF 7.00 written file has allocated pages without a reference. Stream 0 holds the
//!   previous MSF stream directory, so it can differ, for example in an MSFZ copy.
//!
//! The output must not be the input file.

use std::error::Error;
use std::path::Path;

use pdb_rs::io::FileReader;
use pdb_rs::io::copy_exact_file;
use pdb_rs::native::container::Container;
use pdb_rs::native::msf::{MsfWriteOptions, OldDirectoryPolicy, StreamId};
use pdb_rs::native::msfz::{self, Codec};

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

/// The largest stream that `compare` loads at one time (1 GiB).
const MAX_STREAM_BYTES: usize = 1 << 30;

fn main() -> Outcome {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.split_first() {
        Some((command, rest)) if command == "copy" => copy(rest),
        Some((command, rest)) if command == "repack" => repack(rest),
        Some((command, rest)) if command == "pdz-encode" => pdz_encode(rest),
        Some((command, rest)) if command == "compare" => compare(rest),
        _ => Err(usage()),
    }
}

/// Return the usage error.
fn usage() -> Box<dyn Error> {
    "usage: msf copy <input> <output> | msf repack [--page-size <bytes>|source] \
     [--old-directory empty|preserve] <input> <output> | msf pdz-encode \
     [--max-chunk-size <bytes>[K|M]] [--one-stream-per-chunk] [--compress-stream-dir] \
     [--pad16k] [--deflate] [--old-directory empty|preserve] <input> <output> | \
     msf compare <original> <written>"
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
    let mut options = MsfWriteOptions::default();
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
            "--old-directory" => policy = old_directory(rest.next())?,
            _ => paths.push(argument),
        }
    }
    let [input, output] = paths.as_slice() else {
        return Err(usage());
    };
    check_paths(input, output)?;
    let container = Container::read(FileReader::open(input)?)?;
    if source_page_size {
        let msf = container
            .msf()
            .ok_or("--page-size source needs an MSF 7.00 input")?;
        options = options.with_page_size(msf.block_size());
    }
    let plan = container.to_msf(options, policy)?;
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

/// Write an MSFZ container with the options of `pdbtool pdz-encode`.
fn pdz_encode(arguments: &[String]) -> Outcome {
    let mut options = msfz::MsfzWriteOptions::default();
    let mut policy = OldDirectoryPolicy::default();
    let mut paths = Vec::new();
    let mut rest = arguments.iter();
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--max-chunk-size" => {
                let value = rest.next().ok_or_else(usage)?;
                options = options.with_max_chunk_size(parse_bytes(value)?);
            }
            "--one-stream-per-chunk" => options = options.with_one_stream_per_chunk(true),
            "--compress-stream-dir" => options = options.with_directory_codec(Some(Codec::Zstd)),
            "--pad16k" => options = options.with_pad_16k(true),
            "--deflate" => options = options.with_codec(Codec::Deflate),
            "--old-directory" => policy = old_directory(rest.next())?,
            _ => paths.push(argument),
        }
    }
    let [input, output] = paths.as_slice() else {
        return Err(usage());
    };
    check_paths(input, output)?;
    let container = Container::read(FileReader::open(input)?)?;
    let plan = container.to_msfz(options, policy)?;
    plan.write_file(output)?;
    println!(
        "encoded {} streams: {} chunks of at most {} bytes, {} fragments, directory {} bytes \
         ({} stored), {} bytes, stream 0 {policy:?}",
        plan.stream_count(),
        plan.chunk_count(),
        options.max_chunk_size(),
        plan.fragment_count(),
        plan.directory_size(),
        plan.stored_directory_size(),
        plan.file_size(),
    );
    Ok(())
}

/// Parse a stream 0 policy.
fn old_directory(value: Option<&String>) -> Outcome<OldDirectoryPolicy> {
    match value.map(String::as_str) {
        Some("empty") => Ok(OldDirectoryPolicy::Empty),
        Some("preserve") => Ok(OldDirectoryPolicy::Preserve),
        _ => Err(usage()),
    }
}

/// Parse a byte count with an optional `K` (1024) or `M` (1048576) suffix, as
/// `pdbtool pdz-encode` does.
fn parse_bytes(text: &str) -> Outcome<u32> {
    let (digits, unit) = if let Some(digits) = text.strip_suffix(['k', 'K']) {
        (digits, 1 << 10)
    } else if let Some(digits) = text.strip_suffix(['m', 'M']) {
        (digits, 1 << 20)
    } else {
        (text, 1)
    };
    digits
        .parse::<u32>()?
        .checked_mul(unit)
        .ok_or_else(|| "size is too large".into())
}

/// Print the facts of a container.
fn describe(name: &str, container: &Container<FileReader>) {
    match container {
        Container::Msf(msf) => println!(
            "{name}: MSF 7.00, page size {}, {} pages, {} streams, active FPM {}",
            msf.block_size(),
            msf.page_count(),
            msf.stream_count(),
            msf.active_free_page_map(),
        ),
        Container::Msfz(msfz) => println!(
            "{name}: MSFZ, {} chunks, {} fragments, {} streams, compressed directory {}",
            msfz.chunk_count(),
            msfz.fragment_count(),
            msfz.stream_count(),
            msfz.is_directory_compressed(),
        ),
        other => println!("{name}: {:?}", other.kind()),
    }
}

/// Compare the streams of two containers and check the free page map of the second.
fn compare(arguments: &[String]) -> Outcome {
    let [original, written] = arguments else {
        return Err(usage());
    };
    let first = Container::read(FileReader::open(original)?)?;
    let second = Container::read(FileReader::open(written)?)?;
    describe("original", &first);
    describe("written", &second);
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
    let Some(written) = second.msf() else {
        println!("written free page map: not applicable (MSFZ)");
        return if different.iter().any(|&id| id != 0) {
            Err("streams differ".into())
        } else {
            Ok(())
        };
    };
    let report = written.validate_free_page_map()?;
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
