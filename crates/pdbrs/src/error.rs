//! Errors for checked byte access and format inspection.

use core::fmt;

use crate::native::msf::StreamId;
use crate::native::{IndexSpace, RemapRange};
use crate::portable::metadata::{MetadataStream, TableId};

/// A result returned by this crate.
pub type Result<T, E = Error> = core::result::Result<T, E>;

/// The reason an operation failed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
    /// An edit asks for a change that the editor does not support.
    UnsupportedEdit,
}

/// The location of an error in Portable PDB metadata.
///
/// Each field is optional. The stream is a metadata stream, for example `#~` or `#Blob`. The
/// table, row and column identify a table cell. The heap offset is an offset into the heap
/// that the stream names, or into the heap that the column refers to.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct PortableLocation {
    stream: Option<MetadataStream>,
    table: Option<TableId>,
    row: Option<u32>,
    column: Option<&'static str>,
    heap_offset: Option<u32>,
}

impl PortableLocation {
    /// Make a location without fields.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stream: None,
            table: None,
            row: None,
            column: None,
            heap_offset: None,
        }
    }

    /// Set the metadata stream.
    #[must_use]
    pub const fn with_stream(mut self, stream: MetadataStream) -> Self {
        self.stream = Some(stream);
        self
    }

    /// Set the table.
    #[must_use]
    pub const fn with_table(mut self, table: TableId) -> Self {
        self.table = Some(table);
        self
    }

    /// Set the 1-based row number in the table.
    #[must_use]
    pub const fn with_row(mut self, row: u32) -> Self {
        self.row = Some(row);
        self
    }

    /// Set the column name, as the Portable PDB specification writes it.
    #[must_use]
    pub const fn with_column(mut self, column: &'static str) -> Self {
        self.column = Some(column);
        self
    }

    /// Set the heap offset. For the `#GUID` heap, the value is the 1-based GUID index.
    #[must_use]
    pub const fn with_heap_offset(mut self, offset: u32) -> Self {
        self.heap_offset = Some(offset);
        self
    }

    /// Return the metadata stream, if known.
    #[must_use]
    pub const fn stream(&self) -> Option<MetadataStream> {
        self.stream
    }

    /// Return the table, if known.
    #[must_use]
    pub const fn table(&self) -> Option<TableId> {
        self.table
    }

    /// Return the 1-based row number, if known.
    #[must_use]
    pub const fn row(&self) -> Option<u32> {
        self.row
    }

    /// Return the column name, if known.
    #[must_use]
    pub const fn column(&self) -> Option<&'static str> {
        self.column
    }

    /// Return the heap offset or GUID index, if known.
    #[must_use]
    pub const fn heap_offset(&self) -> Option<u32> {
        self.heap_offset
    }

    /// Keep the fields of `self` and add the fields of `outer` that `self` does not have.
    const fn merge(self, outer: Self) -> Self {
        Self {
            stream: or(self.stream, outer.stream),
            table: or(self.table, outer.table),
            row: or(self.row, outer.row),
            column: or(self.column, outer.column),
            heap_offset: or(self.heap_offset, outer.heap_offset),
        }
    }
}

/// Return `first` if it has a value, else `second`.
const fn or<T: Copy>(first: Option<T>, second: Option<T>) -> Option<T> {
    match first {
        Some(value) => Some(value),
        None => second,
    }
}

impl fmt::Display for PortableLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut separator = "";
        if let Some(stream) = self.stream {
            write!(formatter, "stream {}", stream.name())?;
            separator = ", ";
        }
        if let Some(table) = self.table {
            write!(formatter, "{separator}table {table}")?;
            separator = ", ";
        }
        if let Some(row) = self.row {
            write!(formatter, "{separator}row {row}")?;
            separator = ", ";
        }
        if let Some(column) = self.column {
            write!(formatter, "{separator}column {column}")?;
            separator = ", ";
        }
        if let Some(offset) = self.heap_offset {
            write!(formatter, "{separator}heap offset {offset:#x}")?;
        }
        Ok(())
    }
}

/// The location of an error in the data of one backend.
///
/// The two kinds never occur in one error. One field holds them, so that [`Error`] stays
/// small.
#[derive(Clone, Copy, Debug)]
enum Location {
    /// A location in Portable PDB metadata.
    Portable(PortableLocation),
    /// A rejected range of an [`ImageRemap`](crate::native::ImageRemap).
    Remap(RemapRange),
    /// The index space of a rejected native edit.
    Space(IndexSpace),
}

/// An error with an optional physical byte offset.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    offset: Option<u64>,
    source: Option<nostdio::IoError>,
    context: Option<&'static str>,
    stream: Option<(StreamId, u64)>,
    location: Option<Location>,
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
            location: None,
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
            location: None,
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
    pub const fn with_stream(mut self, stream: StreamId, offset: u64) -> Self {
        self.stream = Some((stream, offset));
        self
    }

    /// Return the stream ID and logical byte offset, if known.
    #[must_use]
    pub const fn stream_location(&self) -> Option<(StreamId, u64)> {
        self.stream
    }

    /// Add a Portable PDB metadata location.
    ///
    /// The fields that the error already has stay. The function adds only the fields that
    /// are absent, so an outer operation can add the table and row to a heap error.
    #[must_use]
    pub const fn with_portable(mut self, location: PortableLocation) -> Self {
        self.location = Some(Location::Portable(match self.location {
            Some(Location::Portable(current)) => current.merge(location),
            _ => location,
        }));
        self
    }

    /// Return the Portable PDB metadata location, if known.
    #[must_use]
    pub const fn portable_location(&self) -> Option<&PortableLocation> {
        match &self.location {
            Some(Location::Portable(location)) => Some(location),
            _ => None,
        }
    }

    /// Attach the address range of an [`ImageRemap`](crate::native::ImageRemap) that
    /// the operation rejects.
    #[must_use]
    pub const fn with_remap_range(mut self, range: RemapRange) -> Self {
        self.location = Some(Location::Remap(range));
        self
    }

    /// Return the rejected address range of an [`ImageRemap`](crate::native::ImageRemap),
    /// if known.
    #[must_use]
    pub const fn remap_range(&self) -> Option<RemapRange> {
        match self.location {
            Some(Location::Remap(range)) => Some(range),
            _ => None,
        }
    }

    /// Attach the index space of a native edit that the plan rejects.
    ///
    /// For example, a type record that refers to a later type record gives
    /// [`IndexSpace::TypeIndex`].
    #[must_use]
    pub const fn with_index_space(mut self, space: IndexSpace) -> Self {
        self.location = Some(Location::Space(space));
        self
    }

    /// Return the index space of a rejected native edit, if known.
    #[must_use]
    pub const fn index_space(&self) -> Option<IndexSpace> {
        match self.location {
            Some(Location::Space(space)) => Some(space),
            _ => None,
        }
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
            ErrorKind::UnsupportedEdit => "unsupported edit",
        };
        formatter.write_str(message)?;
        if let Some(offset) = self.offset {
            write!(formatter, " at file offset {offset:#x}")?;
        }
        if let Some(context) = self.context {
            write!(formatter, " ({context})")?;
        }
        if let Some((stream, offset)) = self.stream {
            write!(formatter, " in stream {} at {offset:#x}", stream.0)?;
        }
        match &self.location {
            Some(Location::Portable(location)) => {
                write!(formatter, " in Portable PDB metadata: {location}")?;
            }
            Some(Location::Remap(range)) => write!(
                formatter,
                " for the remap range: old RVA {:#x}, length {:#x}, new RVA {:#x}",
                range.old.0, range.length, range.new.0
            )?,
            Some(Location::Space(space)) => write!(formatter, " in the index space {space:?}")?,
            None => {}
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
