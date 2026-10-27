//! Stream views: the bytes of one stream, borrowed from the source or
//! copied.

use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::fmt;
use core::ops::Deref;

use super::Stream;
use crate::io::ReadAt;
use crate::native::msf::StreamId;
use crate::{Error, ErrorKind, Result};

/// The bytes of one stream: borrowed from the source, or an owned copy.
///
/// [`Stream::view`] and [`crate::native::Pdb::stream_view`] borrow the
/// bytes if the source lends the whole stream. That is an MSF 7.00 stream
/// whose pages are adjacent in a source that lends bytes, for example a byte
/// slice. Otherwise they copy the bytes, for example for a file source or an
/// MSFZ stream (MSFZ stream bytes are compressed).
///
/// # Example
///
/// ```
/// use pdb_rs::native::container::StreamBytes;
///
/// let copied = StreamBytes::from(vec![1, 2, 3]);
/// assert!(!copied.is_borrowed());
/// assert_eq!(copied.as_bytes(), [1, 2, 3]);
/// ```
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct StreamBytes<'a>(Cow<'a, [u8]>);

impl<'a> StreamBytes<'a> {
    /// Make a value that borrows `bytes`.
    #[must_use]
    pub const fn borrowed(bytes: &'a [u8]) -> Self {
        Self(Cow::Borrowed(bytes))
    }

    /// Return the bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Return `true` if the value borrows its bytes from the source.
    #[must_use]
    pub const fn is_borrowed(&self) -> bool {
        matches!(self.0, Cow::Borrowed(_))
    }

    /// Return the bytes as a vector. Copy borrowed bytes.
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        self.0.into_owned()
    }
}

impl From<Vec<u8>> for StreamBytes<'static> {
    fn from(bytes: Vec<u8>) -> Self {
        Self(Cow::Owned(bytes))
    }
}

impl Deref for StreamBytes<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for StreamBytes<'_> {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for StreamBytes<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StreamBytes")
            .field("len", &self.0.len())
            .field("borrowed", &self.is_borrowed())
            .finish()
    }
}

impl<'a, S: ReadAt> Stream<'a, S> {
    /// Return the bytes of the stream: borrowed from the source if the
    /// source lends the whole stream, else a copy.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`] if the stream is larger than
    ///   `max_bytes` or the allocation of a copy fails. The function checks
    ///   the size before it borrows or allocates.
    /// - The errors of [`Stream::read_all`].
    pub fn view(&self, max_bytes: usize) -> Result<StreamBytes<'a>> {
        let length = usize::try_from(self.len()?)
            .ok()
            .filter(|&length| length <= max_bytes)
            .ok_or_else(|| too_large(self.id()))?;
        match self.lend(0, length) {
            Some(bytes) => Ok(StreamBytes::borrowed(bytes)),
            None => self.read_all(max_bytes).map(StreamBytes::from),
        }
    }

    /// Return the `length` bytes at the logical `offset` as a slice of the
    /// source, if the source lends them. An MSFZ stream does not lend.
    pub(crate) fn lend(&self, offset: u64, length: usize) -> Option<&'a [u8]> {
        match self {
            Self::Msf(stream) => stream.lend(offset, length),
            Self::Msfz(_) => None,
        }
    }

    /// Return the source offset of the whole stream, if the source lends
    /// it.
    pub(crate) fn lent_offset(&self) -> Option<u64> {
        let Self::Msf(stream) = self else {
            return None;
        };
        let length = usize::try_from(stream.len().ok()?).ok()?;
        stream.lend(0, length)?;
        stream.source_offset(0, length)
    }
}

/// Return the error for a stream that is larger than the read limit.
fn too_large(id: StreamId) -> Error {
    Error::new(ErrorKind::LimitExceeded, None)
        .with_context("stream is larger than the read limit")
        .with_stream(id, 0)
}
