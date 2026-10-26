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
}

/// An error with an optional physical byte offset.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    offset: Option<u64>,
    #[cfg(feature = "std")]
    source: Option<std::io::Error>,
}

impl Error {
    /// Create an error at an optional physical byte offset.
    #[must_use]
    pub const fn new(kind: ErrorKind, offset: Option<u64>) -> Self {
        Self {
            kind,
            offset,
            #[cfg(feature = "std")]
            source: None,
        }
    }

    /// Retain an I/O error and its optional physical byte offset.
    #[cfg(feature = "std")]
    #[must_use]
    pub fn from_io(source: std::io::Error, offset: Option<u64>) -> Self {
        Self {
            kind: ErrorKind::Io,
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
        };
        formatter.write_str(message)?;
        if let Some(offset) = self.offset {
            write!(formatter, " at file offset {offset:#x}")?;
        }
        #[cfg(feature = "std")]
        if let Some(source) = &self.source {
            write!(formatter, ": {source}")?;
        }
        Ok(())
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        #[cfg(feature = "std")]
        if let Some(source) = &self.source {
            return Some(source);
        }
        None
    }
}
