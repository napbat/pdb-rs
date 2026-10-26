//! Raw DEFLATE and zstd codecs for compressed PDB data.
//!
//! Only this module and its child modules name `miniz_oxide` and `ruzstd`.
//! The module is crate-private. Its functions return plain byte vectors and
//! [`Error`] values, so no dependency type reaches the public API.
//!
//! A decoder gets the declared decoded size and a limit from the caller. It
//! checks the declared size against the limit before it allocates memory. Then
//! it decodes into a buffer of exactly the declared size. Short output, extra
//! output, unused input and corrupt streams are errors.
//!
//! The errors have a static context but no location. The caller adds the stream
//! and the offset of the compressed data.

pub(crate) mod deflate;
pub(crate) mod zstd;

use alloc::vec::Vec;

use crate::{Error, ErrorKind, Result};

/// Allocate a zeroed output buffer of the declared decoded size.
///
/// Returns [`ErrorKind::LimitExceeded`] if `declared` is larger than `limit` or
/// if the allocation fails. The function checks the limit before it allocates.
fn output_buffer(declared: u64, limit: usize) -> Result<Vec<u8>> {
    let size = usize::try_from(declared)
        .ok()
        .filter(|&size| size <= limit)
        .ok_or_else(|| limit_exceeded("declared decoded size is larger than the decode limit"))?;
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(size)
        .map_err(|_| limit_exceeded("decoded buffer allocation failed"))?;
    buffer.resize(size, 0);
    Ok(buffer)
}

/// Return an [`ErrorKind::LimitExceeded`] error with a static context.
fn limit_exceeded(context: &'static str) -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context(context)
}

/// Return an [`ErrorKind::InvalidFormat`] error with a static context.
fn invalid(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, None).with_context(context)
}
