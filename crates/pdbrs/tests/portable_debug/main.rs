//! Behavior tests for the typed rows, the blob codecs and the validation of the Portable PDB
//! debug tables.
//!
//! The codec tests use hand-written blobs and round trips through the encoders. The table
//! tests build synthetic Portable PDBs with all eight debug tables and change one row to break
//! one rule.

#[path = "../portable_metadata/builder.rs"]
mod builder;
mod constants;
mod imports;
mod names;
mod rules;
mod sequence;
mod synthetic;

use pdbrs::{Error, ErrorKind};

/// Check the kind of an error and that its context contains `text`.
pub(crate) fn expect(error: &Error, kind: ErrorKind, text: &str) {
    assert_eq!(error.kind(), kind, "{error}");
    let context = error.context().unwrap_or_default();
    assert!(context.contains(text), "context {context:?} of {error}");
}

/// Run an encoder that appends to a buffer, and return the bytes.
pub(crate) fn encoded(
    encode: impl FnOnce(&mut Vec<u8>) -> pdbrs::Result<()>,
) -> pdbrs::Result<Vec<u8>> {
    let mut out = Vec::new();
    encode(&mut out)?;
    Ok(out)
}

/// Check that each strict prefix of `blob` gives an error from `decode` and no panic.
pub(crate) fn every_prefix_fails(blob: &[u8], decode: impl Fn(&[u8]) -> pdbrs::Result<()>) {
    for end in 0..blob.len() {
        assert!(
            decode(&blob[..end]).is_err(),
            "the prefix of {end} bytes of {blob:02x?} decoded"
        );
    }
    decode(blob).unwrap();
}
