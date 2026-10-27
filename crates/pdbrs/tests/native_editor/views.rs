//! Borrowed and copied stream views.
//!
//! A byte slice lends the bytes of a stream whose pages are adjacent. A
//! source that cannot lend bytes gives copies. Both give the same records.

use pdbrs::io::ReadAt;
use pdbrs::native::Pdb;
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::symbol::Symbol;
use pdbrs::native::msf::{OldDirectoryPolicy, StreamId};
use pdbrs::native::msfz::MsfzWriteOptions;

use crate::build;

/// The size of the `raw` stream: more than three pages of 4096 bytes.
const RAW_SIZE: usize = 3 * 4096 + 100;

/// An input that cannot lend bytes: it keeps the default
/// [`ReadAt::borrow_at`].
struct NoLend<'a>(&'a [u8]);

impl ReadAt for NoLend<'_> {
    fn len(&self) -> pdbrs::Result<u64> {
        ReadAt::len(self.0)
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> pdbrs::Result<()> {
        self.0.read_exact_at(offset, buffer)
    }
}

/// Decode the symbol records of `bytes`.
fn records(bytes: &[u8]) -> Vec<Symbol<'static>> {
    Records::new(bytes)
        .map(|record| Symbol::decode(record.expect("framed record")).map(Symbol::into_owned))
        .collect::<pdbrs::Result<_>>()
        .expect("decoded records")
}

/// Return the stream ID of the symbol record stream of `pdb`.
fn symbol_stream<S: ReadAt>(pdb: &Pdb<S>) -> StreamId {
    pdb.debug_info()
        .expect("DBI")
        .expect("DBI present")
        .header()
        .symbol_stream
        .expect("symbol record stream")
}

#[test]
fn a_slice_lends_only_ranges_inside_it() {
    let bytes = [1_u8, 2, 3, 4];
    let slice = bytes.as_slice();
    assert_eq!(slice.borrow_at(1, 2), Some(&bytes[1..3]));
    assert_eq!(slice.borrow_at(4, 0), Some(&bytes[4..]));
    assert_eq!(slice.borrow_at(3, 2), None);
    assert_eq!(slice.borrow_at(u64::MAX, 1), None);
    assert_eq!((&slice).borrow_at(0, 4), Some(slice));
    assert_eq!(NoLend(slice).borrow_at(0, 1), None);
}

#[test]
fn borrowed_and_copied_views_give_the_same_records() {
    let raw: Vec<u8> = (0..RAW_SIZE)
        .map(|index| u8::try_from(index % 251).expect("byte"))
        .collect();
    let source = build(|editor| editor.named_streams_mut()?.insert(b"raw", raw.clone()));
    let lent = Pdb::parse(source.as_slice()).expect("slice source");
    let copied = Pdb::read(NoLend(&source)).expect("source without lent bytes");
    let inside = |bytes: &[u8]| source.as_ptr_range().contains(&bytes.as_ptr());

    // The symbol record stream: the same records from both views.
    let id = symbol_stream(&lent);
    let borrowed = lent.stream_view(id).expect("borrowed view");
    let copy = copied.stream_view(id).expect("copied view");
    assert!(borrowed.is_borrowed());
    assert!(inside(&borrowed));
    assert!(!copy.is_borrowed());
    let decoded = records(&borrowed);
    assert!(decoded.len() > 1, "the program has more than one record");
    assert_eq!(decoded, records(&copy));

    // A stream of more than one page.
    let raw_id = lent.info().named_stream(b"raw").expect("raw stream");
    let borrowed = lent.stream_view(raw_id).expect("borrowed view");
    assert!(borrowed.is_borrowed());
    assert_eq!(borrowed.as_bytes(), raw.as_slice());
    assert_eq!(
        copied.stream_view(raw_id).expect("copied view").as_bytes(),
        raw.as_slice()
    );

    // The session cells lend the bytes of a slice source and copy the others.
    let lent_records = lent.symbol_records().expect("records").expect("present");
    let copied_records = copied.symbol_records().expect("records").expect("present");
    assert!(inside(lent_records));
    assert!(!inside(copied_records));
    assert_eq!(records(lent_records), records(copied_records));
    let lent_names = lent.names().expect("names").expect("present");
    let copied_names = copied.names().expect("names").expect("present");
    assert!(inside(lent_names.strings()));
    assert_eq!(lent_names.strings(), copied_names.strings());
    let globals = lent.globals().expect("globals").expect("present");
    assert_eq!(records(globals.records()), decoded);

    // An MSFZ stream does not lend: its bytes are compressed.
    let msfz = lent
        .container()
        .to_msfz(MsfzWriteOptions::default(), OldDirectoryPolicy::Empty)
        .and_then(|plan| plan.to_vec())
        .expect("MSFZ output");
    let msfz = Pdb::parse(msfz.as_slice()).expect("MSFZ source");
    let view = msfz.stream_view(symbol_stream(&msfz)).expect("MSFZ view");
    assert!(!view.is_borrowed());
    assert_eq!(records(&view), decoded);
}
