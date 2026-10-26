//! Behavior tests for the Portable PDB metadata reader with synthetic metadata.
//!
//! The tests cover the metadata root, the stream headers, the `#Pdb` and `#~` streams, heap
//! and table index widths, coded indexes that external row counts widen, row and heap
//! access, truncation and the format facade.

mod builder;
mod dispatch;
mod root;
mod tables;
mod widths;

use pdbrs::PortableLocation;
use pdbrs::portable::Pdb;
use pdbrs::portable::metadata::MetadataStream;
use pdbrs::{Error, ErrorKind};

/// Open bytes and return the error. Panic if the open succeeds.
pub(crate) fn open_error(bytes: &[u8]) -> Error {
    match Pdb::parse(bytes) {
        Ok(_) => panic!("the metadata opened, but an error was expected"),
        Err(error) => error,
    }
}

/// Return the portable location of an error.
pub(crate) fn location(error: &Error) -> PortableLocation {
    *error
        .portable_location()
        .unwrap_or_else(|| panic!("the error has no portable location: {error}"))
}

/// Check the kind and the stream of an error.
pub(crate) fn expect(error: &Error, kind: ErrorKind, stream: MetadataStream) {
    assert_eq!(error.kind(), kind, "{error}");
    assert_eq!(location(error).stream(), Some(stream), "{error}");
}
