//! The overlap check of the file ranges of an MSFZ container.

use alloc::vec::Vec;

use super::directory::{Chunk, Fragment, Location};
use super::header::{HEADER_SIZE, Header};
use super::no_memory;
use crate::{Error, ErrorKind, Result};

/// The kind of structure that a file range holds.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Region {
    /// The file header.
    Header,
    /// The stream directory, with its size in the file.
    Directory,
    /// The Chunk Table.
    ChunkTable,
    /// The stored bytes of a chunk.
    Chunk,
    /// The bytes of an uncompressed fragment.
    Fragment,
}

/// A non-empty file range.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Range {
    start: u64,
    end: u64,
    region: Region,
}

/// Check that the header, the stream directory and the Chunk Table do not overlap.
///
/// The reader calls this function before it reads the directory or the Chunk Table.
pub(super) fn check_fixed(header: &Header) -> Result<()> {
    check(header, &[], &[])
}

/// Check that no two structures of the file overlap.
///
/// The structures are the header, the stream directory, the Chunk Table, the chunks and the
/// uncompressed fragments. Gaps between the structures are allowed. The function does not
/// check that the ranges are inside the file.
pub(super) fn check(header: &Header, chunks: &[Chunk], fragments: &[Fragment]) -> Result<()> {
    let files = fragments
        .iter()
        .filter_map(|fragment| match fragment.location {
            Location::File(offset) => Some((offset, fragment.size)),
            Location::Chunks(_) => None,
        });
    let count = files.clone().count() + chunks.len() + 3;
    let mut ranges = Vec::new();
    ranges.try_reserve_exact(count).map_err(|_| no_memory())?;
    let mut add = |start: u64, size: u64, region| {
        if size != 0 {
            ranges.push(Range {
                start,
                end: start.saturating_add(size),
                region,
            });
        }
    };
    add(0, HEADER_SIZE, Region::Header);
    add(
        header.directory_offset,
        u64::from(header.directory_size_compressed),
        Region::Directory,
    );
    if header.chunk_count != 0 {
        add(
            header.chunk_table_offset,
            u64::from(header.chunk_table_size),
            Region::ChunkTable,
        );
    }
    for chunk in chunks {
        add(chunk.offset, u64::from(chunk.stored_size), Region::Chunk);
    }
    for (offset, size) in files {
        add(offset, u64::from(size), Region::Fragment);
    }
    ranges.sort_unstable();
    let mut ranges = ranges.into_iter();
    let Some(mut reach) = ranges.next() else {
        return Ok(());
    };
    for range in ranges {
        if range.start < reach.end {
            return Err(Error::new(ErrorKind::InvalidFormat, Some(range.start))
                .with_context(overlap(reach.region, range.region)));
        }
        if range.end > reach.end {
            reach = range;
        }
    }
    Ok(())
}

/// Return the error context for an overlap of two regions.
fn overlap(first: Region, second: Region) -> &'static str {
    match (first.min(second), first.max(second)) {
        (Region::Header, Region::Directory) => "MSFZ stream directory overlaps the header",
        (Region::Header, Region::ChunkTable) => "MSFZ Chunk Table overlaps the header",
        (Region::Header, Region::Chunk) => "MSFZ chunk overlaps the header",
        (Region::Header, Region::Fragment) => "uncompressed MSFZ fragment overlaps the header",
        (Region::Directory, Region::ChunkTable) => "MSFZ Chunk Table overlaps the stream directory",
        (Region::Directory, Region::Chunk) => "MSFZ chunk overlaps the stream directory",
        (Region::Directory, Region::Fragment) => {
            "uncompressed MSFZ fragment overlaps the stream directory"
        }
        (Region::ChunkTable, Region::Chunk) => "MSFZ chunk overlaps the Chunk Table",
        (Region::ChunkTable, Region::Fragment) => {
            "uncompressed MSFZ fragment overlaps the Chunk Table"
        }
        (Region::Chunk, Region::Chunk) => "two MSFZ chunks overlap",
        (Region::Chunk, Region::Fragment) => "uncompressed MSFZ fragment overlaps a chunk",
        (Region::Fragment, Region::Fragment) => "two uncompressed MSFZ fragments overlap",
        // The header, the directory and the Chunk Table occur only once.
        _ => "MSFZ structures overlap",
    }
}
