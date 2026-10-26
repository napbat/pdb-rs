//! Copy all bytes of a positional source to a sequential output.

use alloc::vec::Vec;

use super::{ReadAt, Write};
use crate::{Error, ErrorKind, Result};

/// The size of the copy buffer and of the file output buffer (64 KiB).
pub(crate) const COPY_BUFFER_SIZE: usize = 64 << 10;

/// Copy all bytes of `source` to `output` through one buffer of at most 64 KiB.
///
/// Return the number of bytes. An output error has the file offset of the buffer.
pub(crate) fn copy_all<S: ReadAt + ?Sized, W: Write + ?Sized>(
    source: &S,
    output: &mut W,
) -> Result<u64> {
    let length = source.len()?;
    let capacity =
        usize::try_from(length).map_or(COPY_BUFFER_SIZE, |length| length.min(COPY_BUFFER_SIZE));
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(capacity).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None).with_context("copy buffer allocation failed")
    })?;
    buffer.resize(capacity, 0);
    let mut offset = 0_u64;
    while offset < length {
        // The count is at most the buffer length, so it fits in `usize`.
        let count = usize::try_from(length - offset).map_or(capacity, |rest| rest.min(capacity));
        let chunk = &mut buffer[..count];
        source.read_exact_at(offset, chunk)?;
        output
            .write_all(chunk)
            .map_err(|error| Error::from_io(error, Some(offset)))?;
        // `count` is at most 64 KiB.
        offset += u64::try_from(count).unwrap_or(0);
    }
    Ok(length)
}
