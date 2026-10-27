//! The heap builders: deduplication, suffix strings, offsets and value checks.

use pdb_rs::ErrorKind;
use pdb_rs::portable::metadata::MetadataStream;
use pdb_rs::portable::write::{BlobHeap, GuidHeap, StringHeapBuilder, UserStringHeap};

use crate::location;

#[test]
fn string_heap_puts_a_suffix_inside_the_longer_string() {
    let mut builder = StringHeapBuilder::new();
    for value in [&b"alue"[..], b"value", b"x", b"value", b""] {
        builder.add(value).unwrap();
    }
    let heap = builder.finish().unwrap();
    // Sorted by reversed bytes: "value" before its suffix "alue", then "x".
    assert_eq!(heap.bytes(), b"\0value\0x\0");
    assert_eq!(heap.size(), 9);
    assert_eq!(heap.offset(b"value").unwrap().0, 1);
    assert_eq!(heap.offset(b"alue").unwrap().0, 2);
    assert_eq!(heap.offset(b"x").unwrap().0, 7);
    assert_eq!(heap.offset(b"").unwrap().0, 0);
    assert_eq!(heap.offset(b"lue"), None);
}

#[test]
fn string_heap_rejects_a_zero_byte() {
    let error = StringHeapBuilder::new().add(b"a\0b").unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(location(&error).stream(), Some(MetadataStream::Strings));
}

#[test]
fn blob_heap_removes_duplicates_in_insertion_order() {
    let mut heap = BlobHeap::new();
    assert_eq!(heap.add(b"").unwrap().0, 0);
    let first = heap.add(b"abc").unwrap();
    let second = heap.add(&[0x55; 0x80]).unwrap();
    assert_eq!(heap.add(b"abc").unwrap(), first);
    assert_eq!((first.0, second.0), (1, 5));
    // A 0x80-byte blob has a 2-byte length prefix.
    assert_eq!(heap.size(), 5 + 2 + 0x80);
    assert_eq!(&heap.bytes()[..5], &[0, 3, b'a', b'b', b'c']);
    assert_eq!(&heap.bytes()[5..7], &[0x80, 0x80]);
    assert!(heap.contains(first) && heap.contains(second));
    assert!(!heap.contains(pdb_rs::portable::metadata::BlobOffset(2)));
}

#[test]
fn user_string_heap_needs_an_odd_entry_length() {
    let mut heap = UserStringHeap::new();
    assert_eq!(heap.add(&[b'a', 0, 0]).unwrap().0, 1);
    let error = heap.add(&[b'a', 0]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(location(&error).stream(), Some(MetadataStream::UserStrings));
    assert_eq!(heap.bytes(), &[0, 3, b'a', 0, 0]);
}

#[test]
fn guid_heap_uses_one_based_indexes() {
    let mut heap = GuidHeap::new();
    assert_eq!(heap.add([1; 16]).unwrap().0, 1);
    assert_eq!(heap.add([0; 16]).unwrap().0, 2);
    assert_eq!(heap.add([1; 16]).unwrap().0, 1);
    assert_eq!((heap.count(), heap.size()), (2, 32));
}
