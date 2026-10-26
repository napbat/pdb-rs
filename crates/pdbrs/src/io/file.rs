//! Positional reads from a file.

use std::fs::File;
use std::io;
use std::path::Path;

use super::{ReadAt, check_range, to_u64};
use crate::{Error, ErrorKind, Result};

/// A [`ReadAt`] implementation for a [`File`].
///
/// The reader keeps the file length from the time of construction.
/// [`ReadAt::len`] returns this captured length. Reads after the captured
/// length fail, also if the file becomes longer.
///
/// Each read uses an absolute offset. On Unix, a read does not move the file
/// cursor. On Windows, a read moves the file cursor to the end of the read
/// bytes. The reader does not use the cursor, so the reader owns the file.
///
/// Caution: Handles that you make with [`File::try_clone`] share the cursor
/// on Windows. Do not use the cursor of such a handle while the reader reads.
///
/// The reader does not lock the file. If the file becomes shorter, a read can
/// fail with [`ErrorKind::UnexpectedEof`]. If the file contents change, a
/// read can return the new bytes.
#[derive(Debug)]
pub struct FileReader {
    file: File,
    len: u64,
}

impl FileReader {
    /// Open the file at `path` for reading.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Io`] if the file does not open or if its
    /// metadata is not available.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path).map_err(|error| Error::from_io(error, None))?;
        Self::new(file)
    }

    /// Make a reader that owns `file`.
    ///
    /// The `file` must allow reads. The current cursor position has no
    /// effect on reads.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Io`] if the metadata of `file` is not available.
    pub fn new(file: File) -> Result<Self> {
        let len = file
            .metadata()
            .map_err(|error| Error::from_io(error, None))?
            .len();
        Ok(Self { file, len })
    }

    /// Return the file.
    ///
    /// On Windows, the cursor position is unspecified after reads.
    /// On Unix, reads do not change the cursor position.
    #[must_use]
    pub fn into_inner(self) -> File {
        self.file
    }
}

impl ReadAt for FileReader {
    fn len(&self) -> Result<u64> {
        Ok(self.len)
    }

    /// Fill `buffer` with the bytes that start at `offset`.
    ///
    /// The function repeats short reads and reads that fail with
    /// [`io::ErrorKind::Interrupted`]. An [`ErrorKind::Io`] error keeps the
    /// source error. Its offset is the position of the failed read.
    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        check_range(offset, buffer.len(), self.len)?;
        let mut remaining = buffer;
        let mut position = offset;
        while !remaining.is_empty() {
            match read_once(&self.file, remaining, position) {
                Ok(0) => return Err(Error::new(ErrorKind::UnexpectedEof, Some(position))),
                Ok(count) => {
                    let advance = to_u64(count, position)?;
                    position = position
                        .checked_add(advance)
                        .ok_or(Error::new(ErrorKind::OffsetOverflow, Some(position)))?;
                    // A count larger than the request is a defect of the system read.
                    remaining = core::mem::take(&mut remaining)
                        .get_mut(count..)
                        .ok_or(Error::new(ErrorKind::Io, Some(position)))?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(Error::from_io(error, Some(position))),
            }
        }
        Ok(())
    }
}

/// Read bytes at `offset` without the shared cursor.
#[cfg(unix)]
fn read_once(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buffer, offset)
}

/// Read bytes at `offset`. This moves the file cursor to the end of the read.
#[cfg(windows)]
fn read_once(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buffer, offset)
}
