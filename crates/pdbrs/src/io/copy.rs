//! Exact copies of a positional source to a sequential output or a file.

use alloc::vec::Vec;

use super::{ReadAt, Write};
use crate::{Error, ErrorKind, Result};

/// The size of the copy buffer and of the file output buffer (64 KiB).
pub(crate) const COPY_BUFFER_SIZE: usize = 64 << 10;

/// Copy all bytes of `source` to `output` without a change.
///
/// An exact copy does not depend on the format. The function does not check that `source`
/// is a valid container or PDB. It copies through one buffer of at most 64 KiB and returns
/// the number of bytes.
///
/// # Errors
///
/// - An error from `source` if a read fails.
/// - [`ErrorKind::Io`] if `output` fails. The error offset is the file offset of the buffer.
/// - [`ErrorKind::LimitExceeded`] if the buffer allocation fails.
///
/// After an error, `output` can hold a part of the source.
pub fn copy_exact<S: ReadAt + ?Sized, W: Write + ?Sized>(
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

/// Copy all bytes of `source` to the file `path` without a change.
///
/// This function needs the `std` feature.
///
/// The function uses [`copy_exact`] to write a temporary file in the same directory. Then it
/// renames the temporary file to `path`. The rename replaces an existing file. The function
/// returns the number of bytes.
///
/// Caution: Do not give the path of the source file. This operation is not supported.
///
/// # Errors
///
/// - [`ErrorKind::Io`] if the temporary file does not open, a write fails, or the rename
///   fails.
/// - The errors of [`copy_exact`].
///
/// After an error, the function deletes the temporary file. The file at `path` does not change.
#[cfg(feature = "std")]
pub fn copy_exact_file<S: ReadAt + ?Sized>(
    source: &S,
    path: impl AsRef<std::path::Path>,
) -> Result<u64> {
    let mut length = 0;
    super::write_new_file(path.as_ref(), |output| {
        length = copy_exact(source, output)?;
        Ok(())
    })?;
    Ok(length)
}
