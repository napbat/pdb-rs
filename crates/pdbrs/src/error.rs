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
    /// A serialized structure violates its format invariants.
    InvalidFormat,
    /// A recognized format version is not supported.
    UnsupportedVersion,
    /// A stream index lies outside the stream directory.
    InvalidStreamIndex,
    /// A required stream is absent.
    MissingStream,
    /// A serialized reference does not identify valid data.
    InvalidReference,
    /// A configured resource limit or allocation capacity was exceeded.
    LimitExceeded,
}

/// An error with an optional physical byte offset.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    offset: Option<u64>,
    source: Option<nostdio::IoError>,
    context: Option<&'static str>,
    stream: Option<(u32, u64)>,
}

impl Error {
    /// Create an error at an optional physical byte offset.
    #[must_use]
    pub const fn new(kind: ErrorKind, offset: Option<u64>) -> Self {
        Self {
            kind,
            offset,
            source: None,
            context: None,
            stream: None,
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
            context: None,
            stream: None,
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

    /// Attach a static description of the structure or operation.
    #[must_use]
    pub const fn with_context(mut self, context: &'static str) -> Self {
        self.context = Some(context);
        self
    }

    /// Attach a stream ID and logical byte offset without replacing the physical offset.
    #[must_use]
    pub const fn with_stream(mut self, stream: u32, offset: u64) -> Self {
        self.stream = Some((stream, offset));
        self
    }

    /// Return the stream ID and logical byte offset, if known.
    #[must_use]
    pub const fn stream_location(&self) -> Option<(u32, u64)> {
        self.stream
    }

    /// Return the static structure or operation description, if known.
    #[must_use]
    pub const fn context(&self) -> Option<&'static str> {
        self.context
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
            ErrorKind::InvalidFormat => "invalid serialized structure",
            ErrorKind::UnsupportedVersion => "unsupported format version",
            ErrorKind::InvalidStreamIndex => "stream index out of range",
            ErrorKind::MissingStream => "required stream is absent",
            ErrorKind::InvalidReference => "invalid serialized reference",
            ErrorKind::LimitExceeded => "resource limit exceeded",
        };
        formatter.write_str(message)?;
        if let Some(offset) = self.offset {
            write!(formatter, " at file offset {offset:#x}")?;
        }
        if let Some(context) = self.context {
            write!(formatter, " ({context})")?;
        }
        if let Some((stream, offset)) = self.stream {
            write!(formatter, " in stream {stream} at {offset:#x}")?;
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
