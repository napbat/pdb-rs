//! Errors for checked byte access and format inspection.

use core::fmt;

/// A result returned by this crate.
pub type Result<T, E = Error> = core::result::Result<T, E>;

/// The reason an operation failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The input ends before the requested bytes are available.
    UnexpectedEof,
    /// The input does not have a recognized container signature.
    InvalidSignature,
    /// An offset calculation cannot be represented.
    OffsetOverflow,
    /// An underlying I/O operation failed.
    Io,
    /// A seekable input is already in use by another read.
    InputBorrowed,
}

/// An error with an optional physical byte offset.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    offset: Option<u64>,
    source: Option<nostdio::IoError>,
}

impl Error {
    /// Create an error at an optional physical byte offset.
    #[must_use]
    pub const fn new(kind: ErrorKind, offset: Option<u64>) -> Self {
        Self {
            kind,
            offset,
            source: None,
        }
    }

    /// Retain an I/O error and its optional physical byte offset.
    ///
    /// Map an underlying EOF to [`ErrorKind::UnexpectedEof`]. Other I/O
    /// failures use [`ErrorKind::Io`]. Retain the typed source in both feature modes.
    #[must_use]
    pub fn from_io(source: nostdio::IoError, offset: Option<u64>) -> Self {
        let kind = if nostdio::is_unexpected_eof(&source) {
            ErrorKind::UnexpectedEof
        } else {
            ErrorKind::Io
        };
        Self {
            kind,
            offset,
            source: Some(source),
        }
    }

    /// Return the reason the operation failed.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Return the physical byte offset, if known.
    #[must_use]
    pub const fn offset(&self) -> Option<u64> {
        self.offset
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self.kind {
            ErrorKind::UnexpectedEof => "unexpected end of input",
            ErrorKind::InvalidSignature => "unrecognized container signature",
            ErrorKind::OffsetOverflow => "byte offset overflow",
            ErrorKind::Io => "I/O operation failed",
            ErrorKind::InputBorrowed => "seekable input is already borrowed",
        };
        formatter.write_str(message)?;
        if let Some(offset) = self.offset {
            write!(formatter, " at file offset {offset:#x}")?;
        }
        if let Some(source) = &self.source {
            write!(formatter, ": {source}")?;
        }
        Ok(())
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        if let Some(source) = &self.source {
            return Some(source);
        }
        None
    }
}
