//! Positional access to a cursor-based input.

use core::cell::RefCell;

use super::{Read, ReadAt, Seek, SeekFrom, check_range, to_u64};
use crate::{Error, ErrorKind, Result};

/// A positional adapter for a `nostdio` reader with seek support.
///
/// The adapter captures the input length at construction. Keep the input bytes
/// stable while it is in use. Each nonempty read seeks to its absolute offset.
/// Reads change the inner cursor; they do not restore its previous position.
/// This adapter uses interior mutability and is not `Sync`. Use native
/// positional I/O when callers need concurrent reads.
#[derive(Debug)]
pub struct SeekReader<R> {
    inner: RefCell<R>,
    len: u64,
}

impl<R: Read + Seek> SeekReader<R> {
    /// Capture the input length and restore its original cursor position.
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O error if a position query or seek fails.
    /// On failure, the inner cursor position is unspecified.
    pub fn new(mut inner: R) -> Result<Self> {
        let original = inner
            .stream_position()
            .map_err(|error| Error::from_io(error, None))?;
        let len = inner
            .seek(SeekFrom::End(0))
            .map_err(|error| Error::from_io(error, None))?;
        let restored = inner
            .seek(SeekFrom::Start(original))
            .map_err(|error| Error::from_io(error, Some(original)))?;
        if restored != original {
            return Err(Error::from_io(nostdio::invalid_seek(), Some(original)));
        }
        Ok(Self {
            inner: RefCell::new(inner),
            len,
        })
    }
}

impl<R> SeekReader<R> {
    /// Return the input at its current cursor position.
    #[must_use]
    pub fn into_inner(self) -> R {
        self.inner.into_inner()
    }
}

impl<R: Read + Seek> ReadAt for SeekReader<R> {
    fn len(&self) -> Result<u64> {
        Ok(self.len)
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        check_range(offset, buffer.len(), self.len)?;
        if buffer.is_empty() {
            return Ok(());
        }
        let mut inner = self
            .inner
            .try_borrow_mut()
            .map_err(|_| Error::new(ErrorKind::InputBorrowed, Some(offset)))?;
        let actual = inner
            .seek(SeekFrom::Start(offset))
            .map_err(|error| Error::from_io(error, Some(offset)))?;
        if actual != offset {
            return Err(Error::from_io(nostdio::invalid_seek(), Some(offset)));
        }
        let mut remaining = buffer;
        let mut position = offset;
        while !remaining.is_empty() {
            match inner.read(remaining) {
                Ok(0) => return Err(Error::from_io(nostdio::unexpected_eof(), Some(position))),
                Ok(count) => {
                    let next = core::mem::take(&mut remaining)
                        .get_mut(count..)
                        .ok_or(Error::new(ErrorKind::Io, Some(position)))?;
                    position = position
                        .checked_add(to_u64(count, position)?)
                        .ok_or(Error::new(ErrorKind::OffsetOverflow, Some(position)))?;
                    remaining = next;
                }
                Err(error) if nostdio::is_interrupted(&error) => {}
                Err(error) => return Err(Error::from_io(error, Some(position))),
            }
        }
        Ok(())
    }
}
