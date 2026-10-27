//! The transfer of new field values into source bytes.
//!
//! A rewriter encodes a record twice: with its model values and with its
//! output values. The two encodings have the same layout and differ only in
//! the index fields. The bytes where they differ go into the source bytes of
//! the record. Thus the source bytes keep everything else, also padding that
//! an encoder does not write again.

use crate::{Error, ErrorKind, Result};

/// Write each byte of `after` that differs from `before` into `target` at the
/// same position.
///
/// # Errors
///
/// Returns [`ErrorKind::InvalidFormat`] if `before` and `after` have
/// different lengths, or if `target` is shorter than `before`: a rewritten
/// record must keep its size.
pub(in crate::native::editor::plan) fn apply_difference(
    target: &mut [u8],
    before: &[u8],
    after: &[u8],
) -> Result<()> {
    if before.len() != after.len() || target.len() < before.len() {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("a record with new indices does not keep its size"));
    }
    for ((target, before), after) in target.iter_mut().zip(before).zip(after) {
        if before != after {
            *target = *after;
        }
    }
    Ok(())
}
