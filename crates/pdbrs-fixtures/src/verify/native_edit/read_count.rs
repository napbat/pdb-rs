//! The read count of a named stream change (S6c-3 of #42).
//!
//! The step reads the PDB of [`FIXTURE`] through a counting source. The source records each
//! read. The step maps each read to the pages of the MSF stream directory, which it parses
//! itself. Then it does these steps:
//!
//! 1. Open the PDB with `Pdb::read`. Then reset the counts.
//! 2. Insert a new `srcsrv` named stream and plan. The reads must be in the Info stream, in
//!    the source stream of the name (if any) and in bytes 0 to 56 of the TPI stream.
//! 3. Write the plan with `write_to`. Each page of each stream that the report gives as
//!    `Copied` must be read exactly once. The step must read no other page.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs;

use pdbrs::io::ReadAt;
use pdbrs::native::{EditOptions, IdentityPolicy, Pdb, StreamChange};

use super::super::native::NativeFixture;
use super::edits::SOURCE_SERVER;

/// The fixture of the step.
const FIXTURE: &str = "msvc-x64-large";

/// The size of the TPI stream header. The plan reads only these bytes of the TPI stream.
const TPI_HEADER_SIZE: u64 = 56;

/// The Info stream.
const INFO_STREAM: u32 = 1;

/// The TPI stream.
const TPI_STREAM: u32 = 2;

/// A source over the bytes of a file that records each read: its offset and its size.
#[derive(Debug)]
struct Counting {
    /// The bytes of the file.
    bytes: Vec<u8>,
    /// The reads since the last reset.
    reads: RefCell<Vec<(u64, u64)>>,
}

impl Counting {
    /// Return the reads since the last reset, and reset them.
    fn take(&self) -> Vec<(u64, u64)> {
        self.reads.take()
    }
}

impl ReadAt for Counting {
    fn len(&self) -> pdbrs::Result<u64> {
        ReadAt::len(self.bytes.as_slice())
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> pdbrs::Result<()> {
        self.reads
            .borrow_mut()
            .push((offset, u64::try_from(buffer.len()).unwrap_or(u64::MAX)));
        self.bytes.as_slice().read_exact_at(offset, buffer)
    }
}

/// The pages of the MSF container.
#[derive(Debug)]
struct Layout {
    /// The page size.
    page_size: u64,
    /// For each page: the stream and the position of the page in the stream, or `None`
    /// for a page of no stream.
    owners: Vec<Option<(u32, u64)>>,
    /// The pages of each stream, in stream order.
    streams: Vec<Vec<u32>>,
    /// The size of each stream. A nil stream has the size 0.
    sizes: Vec<u64>,
}

/// Read a `u32` at `offset` of `bytes`.
fn u32_at(bytes: &[u8], offset: u64) -> Result<u32, String> {
    usize::try_from(offset)
        .ok()
        .and_then(|start| bytes.get(start..start.checked_add(4)?))
        .and_then(|value| <[u8; 4]>::try_from(value).ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| format!("the MSF data at offset {offset} is truncated"))
}

/// Parse the MSF header and the stream directory of `bytes`.
fn layout(bytes: &[u8]) -> Result<Layout, String> {
    let page_size = u64::from(u32_at(bytes, 32)?);
    let page_count = u32_at(bytes, 40)?;
    let directory_size = u64::from(u32_at(bytes, 44)?);
    let page_map = u64::from(u32_at(bytes, 52)?);
    if page_size == 0 {
        return Err("the MSF page size is 0".into());
    }
    let mut directory = Vec::new();
    for position in 0..directory_size.div_ceil(page_size) {
        let page = u64::from(u32_at(bytes, page_map * page_size + position * 4)?);
        for offset in (0..page_size).step_by(4) {
            directory.extend(u32_at(bytes, page * page_size + offset)?.to_le_bytes());
        }
    }
    directory.truncate(usize::try_from(directory_size).map_err(|error| error.to_string())?);
    let count = u32_at(&directory, 0)?;
    let mut sizes = Vec::new();
    for stream in 0..u64::from(count) {
        let size = u32_at(&directory, 4 + stream * 4)?;
        sizes.push(if size == u32::MAX { 0 } else { u64::from(size) });
    }
    let mut owners = vec![None; usize::try_from(page_count).map_err(|error| error.to_string())?];
    let mut streams = Vec::new();
    let mut next = 4 + u64::from(count) * 4;
    for (stream, size) in (0..count).zip(&sizes) {
        let mut pages = Vec::new();
        for position in 0..size.div_ceil(page_size) {
            let page = u32_at(&directory, next)?;
            next += 4;
            let owner = usize::try_from(page)
                .ok()
                .and_then(|page| owners.get_mut(page))
                .ok_or_else(|| format!("stream {stream} names the page {page} after the end"))?;
            *owner = Some((stream, position));
            pages.push(page);
        }
        streams.push(pages);
    }
    Ok(Layout {
        page_size,
        owners,
        streams,
        sizes,
    })
}

/// Call `visit` for each page that the read `(offset, size)` touches: the page and the
/// range of the read in the page.
fn pages(layout: &Layout, (offset, size): (u64, u64), mut visit: impl FnMut(u64, u64, u64)) {
    let end = offset.saturating_add(size);
    let mut start = offset;
    while start < end {
        let page = start / layout.page_size;
        let page_end = ((page + 1) * layout.page_size).min(end);
        visit(
            page,
            start % layout.page_size,
            page_end - page * layout.page_size,
        );
        start = page_end;
    }
}

/// Return the owner of page `page`.
fn owner(layout: &Layout, page: u64) -> Option<(u32, u64)> {
    usize::try_from(page)
        .ok()
        .and_then(|page| layout.owners.get(page).copied().flatten())
}

/// The reads of the plan step, by kind.
#[derive(Debug, Default)]
struct PlanReads {
    /// The number of reads.
    reads: usize,
    /// The bytes in the Info stream.
    info: u64,
    /// The bytes in the TPI header.
    tpi: u64,
    /// The bytes in the source stream of the changed name.
    source: u64,
    /// The bytes outside the accepted set.
    other: u64,
    /// The streams of the bytes outside the accepted set. `None` is a page of no stream.
    other_streams: BTreeSet<Option<u32>>,
}

/// Sort the reads of the plan step.
fn plan_reads(layout: &Layout, reads: &[(u64, u64)], source: Option<u32>) -> PlanReads {
    let mut result = PlanReads {
        reads: reads.len(),
        ..PlanReads::default()
    };
    for &read in reads {
        pages(layout, read, |page, start, end| {
            let size = end - start;
            match owner(layout, page) {
                Some((INFO_STREAM, _)) => result.info += size,
                Some((stream, _)) if Some(stream) == source => result.source += size,
                Some((TPI_STREAM, position))
                    if position * layout.page_size + end <= TPI_HEADER_SIZE =>
                {
                    result.tpi += size;
                }
                other => {
                    result.other += size;
                    result.other_streams.insert(other.map(|(stream, _)| stream));
                }
            }
        });
    }
    result
}

/// The reads of the write step.
#[derive(Debug, Default)]
struct WriteReads {
    /// The number of pages of the copied streams.
    pages: usize,
    /// The number of copied streams.
    streams: usize,
    /// The number of copied pages that the write did not read exactly once.
    not_once: usize,
    /// The number of reads of pages of other streams or of no stream.
    other: usize,
    /// The streams of these reads. `None` is a page of no stream.
    other_streams: BTreeSet<Option<u32>>,
}

/// Check the reads of the write step against the copied streams `copied`.
fn write_reads(layout: &Layout, reads: &[(u64, u64)], copied: &BTreeSet<u32>) -> WriteReads {
    let mut counts = vec![0_usize; layout.owners.len()];
    let mut result = WriteReads::default();
    for &read in reads {
        pages(layout, read, |page, _, _| {
            if let Some(count) = usize::try_from(page)
                .ok()
                .and_then(|page| counts.get_mut(page))
            {
                *count += 1;
            }
            let stream = owner(layout, page).map(|(stream, _)| stream);
            if !stream.is_some_and(|stream| copied.contains(&stream)) {
                result.other += 1;
                result.other_streams.insert(stream);
            }
        });
    }
    for &stream in copied {
        let Some(pages) = usize::try_from(stream)
            .ok()
            .and_then(|stream| layout.streams.get(stream))
        else {
            continue;
        };
        result.streams += 1;
        for &page in pages {
            result.pages += 1;
            let count = usize::try_from(page)
                .ok()
                .and_then(|page| counts.get(page).copied());
            if count != Some(1) {
                result.not_once += 1;
            }
        }
    }
    result
}

/// Run the steps on the PDB at `fixture` and return the line and the problems.
fn check(fixture: &NativeFixture) -> Result<(String, Vec<String>), String> {
    let bytes = fs::read(&fixture.pdb)
        .map_err(|error| format!("cannot read {}: {error}", fixture.pdb.display()))?;
    let layout = layout(&bytes)?;
    let counting = Counting {
        bytes,
        reads: RefCell::new(Vec::new()),
    };
    let pdb = Pdb::read(&counting).map_err(|error| format!("cannot read the PDB: {error}"))?;
    let source = pdb.info().named_stream(b"srcsrv").map(|id| id.0);
    counting.take();
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    let plan = editor
        .named_streams_mut()
        .and_then(|named| named.insert(b"srcsrv", SOURCE_SERVER.as_bytes().to_vec()))
        .and_then(|()| editor.plan(EditOptions::default()))
        .map_err(|error| format!("cannot plan the named stream change: {error}"))?;
    let planned = plan_reads(&layout, &counting.take(), source);
    let copied: BTreeSet<u32> = plan
        .report()
        .streams()
        .iter()
        .filter(|entry| entry.change == StreamChange::Copied)
        .filter_map(|entry| entry.source.map(|id| id.0))
        .collect();
    let mut output = Vec::new();
    plan.write_to(&mut output)
        .map_err(|error| format!("cannot write the plan: {error}"))?;
    let written = write_reads(&layout, &counting.take(), &copied);
    let mut problems = Vec::new();
    if planned.other != 0 {
        problems.push(format!(
            "the open and the plan read {} bytes outside the Info stream, the source stream and the TPI header, in the streams {:?}",
            planned.other, planned.other_streams
        ));
    }
    if written.not_once != 0 {
        problems.push(format!(
            "the write reads {} pages of the copied streams not exactly once",
            written.not_once
        ));
    }
    if written.other != 0 {
        problems.push(format!(
            "the write reads {} pages of other streams {:?}",
            written.other, written.other_streams
        ));
    }
    let copied_bytes: u64 = copied
        .iter()
        .filter_map(|&stream| layout.sizes.get(usize::try_from(stream).ok()?))
        .sum();
    let status = if problems.is_empty() { "pass" } else { "FAIL" };
    let line = format!(
        "read count {}: {status}, the plan of a new srcsrv stream reads {} bytes in {} reads (Info stream {}, TPI header {}, source stream {}, other {}); the write reads {} pages of {} copied streams ({copied_bytes} bytes), {} pages not exactly once, {} other pages",
        fixture.label(),
        planned.info + planned.tpi + planned.source + planned.other,
        planned.reads,
        planned.info,
        planned.tpi,
        planned.source,
        planned.other,
        written.pages,
        written.streams,
        written.not_once,
        written.other
    );
    Ok((line, problems))
}

/// Run the read count on the first PDB of [`FIXTURE`] in `list`. Print one line and return
/// the number of problems.
pub(super) fn run(list: &[NativeFixture]) -> usize {
    let Some(fixture) = list.iter().find(|fixture| fixture.fixture == FIXTURE) else {
        println!("read count: FAIL, the manifest has no {FIXTURE} PDB");
        return 1;
    };
    match check(fixture) {
        Ok((line, problems)) => {
            println!("{line}");
            for problem in &problems {
                println!("  problem: {problem}");
            }
            problems.len()
        }
        Err(error) => {
            println!("read count {}: FAIL, {error}", fixture.label());
            1
        }
    }
}
