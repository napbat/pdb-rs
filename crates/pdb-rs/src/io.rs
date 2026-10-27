//! Sequential I/O and positional byte access.
//!
//! The [`ReadAt`] trait reads bytes at absolute offsets. It does not use a
//! shared cursor. The crate implements it for `[u8]` and for references to
//! other implementations. With the `std` feature on Unix and Windows targets,
//! `FileReader` implements it for files.
//!
//! All offsets are physical byte offsets from the start of the input.
//!
//! Sequential traits and cursors come from `nostdio`. [`SeekReader`] adapts
//! a seekable stream to positional access without requiring `std`.
//!
//! [`copy_exact`] copies every byte of a [`ReadAt`] source. An exact copy does not depend on
//! the format.

use crate::{Error, ErrorKind, Result};

pub use nostdio::{
    BufRead, Cursor, IoError, Read, ReadBe, ReadLe, Seek, SeekFrom, Write, WriteBe, WriteLe,
};

mod copy;
mod seek;
pub use copy::copy_exact;
#[cfg(feature = "std")]
pub use copy::copy_exact_file;
pub use seek::SeekReader;

#[cfg(feature = "std")]
mod replace;
#[cfg(feature = "std")]
pub(crate) use replace::write_new_file;

#[cfg(all(feature = "std", any(unix, windows)))]
mod file;

#[cfg(all(feature = "std", any(unix, windows)))]
pub use file::FileReader;

/// A read-only input that supports reads at absolute byte offsets.
///
/// An implementation must return the same length and the same bytes for the
/// full life of a reader that uses it. A reader can give incorrect results
/// if the input changes while the reader uses it.
pub trait ReadAt {
    /// Return the length of the input in bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the implementation cannot get the length.
    fn len(&self) -> Result<u64>;

    /// Fill `buffer` with the bytes that start at `offset`.
    ///
    /// An empty `buffer` at an `offset` equal to the length is valid.
    /// An empty `buffer` at an `offset` after the length is not valid.
    ///
    /// If the read fails, the contents of `buffer` are unspecified.
    /// The function can change some or all of the bytes before it fails.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::OffsetOverflow`] if the end of the range is larger
    ///   than [`u64::MAX`]. The error offset is `offset`.
    /// - [`ErrorKind::UnexpectedEof`] if the range ends after the end of the
    ///   input. The error offset is the first requested byte that is not
    ///   available.
    /// - [`ErrorKind::Io`] if the underlying input fails.
    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<()>;

    /// Return `true` if the input has a length of zero.
    ///
    /// # Errors
    ///
    /// Returns the error from [`ReadAt::len`].
    fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    /// Return the `length` bytes at `offset` as a slice of the input, if the
    /// input can lend them.
    ///
    /// The default returns `None`: the input cannot lend bytes, and the
    /// caller reads them with [`ReadAt::read_exact_at`]. A byte slice lends
    /// each range inside it. An implementation that lends a range must lend
    /// it for the full life of the input, with the bytes that
    /// [`ReadAt::read_exact_at`] gives. It returns `None` for a range that
    /// is not inside the input.
    fn borrow_at(&self, offset: u64, length: usize) -> Option<&[u8]> {
        let _ = (offset, length);
        None
    }

    /// Read a little-endian `u16` at `offset`.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`ReadAt::read_exact_at`].
    fn read_u16_at(&self, offset: u64) -> Result<u16> {
        let mut bytes = [0; 2];
        self.read_exact_at(offset, &mut bytes)?;
        Ok(u16::from_le_bytes(bytes))
    }

    /// Read a little-endian `u32` at `offset`.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`ReadAt::read_exact_at`].
    fn read_u32_at(&self, offset: u64) -> Result<u32> {
        let mut bytes = [0; 4];
        self.read_exact_at(offset, &mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    /// Read a little-endian `u64` at `offset`.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`ReadAt::read_exact_at`].
    fn read_u64_at(&self, offset: u64) -> Result<u64> {
        let mut bytes = [0; 8];
        self.read_exact_at(offset, &mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }
}

impl ReadAt for [u8] {
    fn len(&self) -> Result<u64> {
        to_u64(<[u8]>::len(self), 0)
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        let total = ReadAt::len(self)?;
        check_range(offset, buffer.len(), total)?;
        // The checked range is inside the slice, so both values fit in `usize`.
        let start = to_usize(offset)?;
        let end = start
            .checked_add(buffer.len())
            .ok_or(Error::new(ErrorKind::OffsetOverflow, Some(offset)))?;
        let source = self
            .get(start..end)
            .ok_or(Error::new(ErrorKind::UnexpectedEof, Some(offset)))?;
        buffer.copy_from_slice(source);
        Ok(())
    }

    fn borrow_at(&self, offset: u64, length: usize) -> Option<&[u8]> {
        let start = usize::try_from(offset).ok()?;
        self.get(start..start.checked_add(length)?)
    }
}

impl<R: ReadAt + ?Sized> ReadAt for &R {
    fn len(&self) -> Result<u64> {
        (**self).len()
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        (**self).read_exact_at(offset, buffer)
    }

    fn borrow_at(&self, offset: u64, length: usize) -> Option<&[u8]> {
        (**self).borrow_at(offset, length)
    }

    fn is_empty(&self) -> Result<bool> {
        (**self).is_empty()
    }

    fn read_u16_at(&self, offset: u64) -> Result<u16> {
        (**self).read_u16_at(offset)
    }

    fn read_u32_at(&self, offset: u64) -> Result<u32> {
        (**self).read_u32_at(offset)
    }

    fn read_u64_at(&self, offset: u64) -> Result<u64> {
        (**self).read_u64_at(offset)
    }
}

/// Make sure that `count` bytes at `offset` are inside an input of `total` bytes.
fn check_range(offset: u64, count: usize, total: u64) -> Result<()> {
    let end = offset
        .checked_add(to_u64(count, offset)?)
        .ok_or(Error::new(ErrorKind::OffsetOverflow, Some(offset)))?;
    if end > total {
        return Err(Error::new(
            ErrorKind::UnexpectedEof,
            Some(offset.max(total)),
        ));
    }
    Ok(())
}

/// Convert a byte count to `u64`. Report a failure at `offset`.
fn to_u64(value: usize, offset: u64) -> Result<u64> {
    u64::try_from(value).map_err(|_| Error::new(ErrorKind::OffsetOverflow, Some(offset)))
}

/// Convert an offset to `usize`. Report a failure at `offset`.
fn to_usize(offset: u64) -> Result<usize> {
    usize::try_from(offset).map_err(|_| Error::new(ErrorKind::OffsetOverflow, Some(offset)))
}
