//! The reads of a named stream change and of its output (S6c-3 of #42).
//!
//! A counting input records each read of the source PDB. The stream
//! directory of `layout.rs` maps each read to the stream pages.

use core::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use pdbrs::io::ReadAt;
use pdbrs::native::msf::StreamId;
use pdbrs::native::{EditOptions, IdentityPolicy, Pdb, StreamChange};

use crate::layout::Layout;
use crate::{build, stream};

/// The stream ID of the Info stream.
const INFO_STREAM: u32 = 1;
/// The stream ID of the TPI stream.
const TPI_STREAM: u32 = 2;
/// The size of the TPI stream header.
const TPI_HEADER_SIZE: usize = 56;
/// The size of the `raw` stream: more than three pages of 4096 bytes.
const RAW_SIZE: usize = 3 * 4096 + 100;

/// An input that records each read: the file offset and the length.
pub struct Counting<'a> {
    /// The bytes of the input.
    bytes: &'a [u8],
    /// The reads since the last call of [`Counting::take`].
    reads: RefCell<Vec<(u64, usize)>>,
}

impl<'a> Counting<'a> {
    /// Make a counting input of `bytes`.
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            reads: RefCell::new(Vec::new()),
        }
    }

    /// Return the recorded reads and reset the record.
    pub fn take(&self) -> Vec<(u64, usize)> {
        core::mem::take(&mut *self.reads.borrow_mut())
    }
}

impl ReadAt for Counting<'_> {
    fn len(&self) -> pdbrs::Result<u64> {
        ReadAt::len(self.bytes)
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> pdbrs::Result<()> {
        self.reads.borrow_mut().push((offset, buffer.len()));
        self.bytes.read_exact_at(offset, buffer)
    }
}

/// Return `RAW_SIZE` bytes of a pattern that depends on `seed`.
fn pattern(seed: usize) -> Vec<u8> {
    (0..RAW_SIZE)
        .map(|index| u8::try_from((index * seed + 3) % 251).expect("byte"))
        .collect()
}

#[test]
fn named_stream_change_reads_only_its_streams() {
    let source = build(|editor| editor.named_streams_mut()?.insert(b"raw", pattern(1)));
    let layout = Layout::parse(&source);
    let raw = Pdb::parse(source.as_slice())
        .expect("source")
        .info()
        .named_stream(b"raw")
        .expect("raw stream");
    assert!(layout.stream(raw.0).expect("raw pages").pages.len() > 3);

    // Step 1: open the PDB, then reset the counts.
    let counting = Counting::new(&source);
    let pdb = Pdb::read(&counting).expect("source through the counting input");
    counting.take();

    // Step 2: change the named stream and plan.
    let replaced = pattern(7);
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor
        .named_streams_mut()
        .expect("named streams")
        .insert(b"raw", replaced.clone())
        .expect("replace");
    let plan = editor.plan(EditOptions::default()).expect("plan");
    let reads = counting.take();
    for &(offset, length) in &reads {
        layout.split(offset, length, |owner, range| {
            let (id, _) =
                owner.unwrap_or_else(|| panic!("read at {offset} is outside each stream"));
            assert!(
                id == INFO_STREAM
                    || id == raw.0
                    || (id == TPI_STREAM && range.end <= TPI_HEADER_SIZE),
                "the plan read bytes {range:?} of stream {id}"
            );
        });
    }
    assert_eq!(
        plan.report().output(raw).map(|entry| entry.change),
        Some(StreamChange::Edited)
    );

    // Step 3: the output reads each page of each copied stream once.
    let mut output = Vec::new();
    plan.write_to(&mut output).expect("output");
    let mut counts: BTreeMap<(u32, usize), usize> = BTreeMap::new();
    for (offset, length) in counting.take() {
        let mut pages = BTreeSet::new();
        layout.split(offset, length, |owner, _| {
            pages
                .insert(owner.unwrap_or_else(|| panic!("read at {offset} is outside each stream")));
        });
        for page in pages {
            *counts.entry(page).or_default() += 1;
        }
    }
    let copied: BTreeSet<u32> = plan
        .report()
        .streams()
        .iter()
        .filter(|entry| entry.change == StreamChange::Copied && entry.output != Some(StreamId(0)))
        .filter_map(|entry| entry.source)
        .map(|id| id.0)
        .collect();
    assert!(copied.contains(&TPI_STREAM));
    assert!(!copied.contains(&raw.0));
    let expected: BTreeMap<(u32, usize), usize> = copied
        .iter()
        .flat_map(|&id| {
            let pages = layout.stream(id).map_or(0, |stream| stream.pages.len());
            (0..pages).map(move |position| ((id, position), 1))
        })
        .collect();
    assert_eq!(counts, expected);

    // The output has the new bytes of `raw`.
    let edited = Pdb::parse(output.as_slice()).expect("output parses");
    let id = edited.info().named_stream(b"raw").expect("raw stream");
    assert_eq!(id, raw);
    assert_eq!(stream(&edited, id.0), Some(replaced));
}
