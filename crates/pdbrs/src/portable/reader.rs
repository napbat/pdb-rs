//! The standalone Portable PDB reader.

use alloc::vec::Vec;
use core::marker::PhantomData;

use super::Identity;
use super::metadata::heaps::Heap;
use super::metadata::{
    BlobOffset, GuidIndex, MetadataRoot, MetadataStream, PdbStream, RowReader, StringOffset,
    TableRow, TableStream, UserStringOffset, decode_row,
};
use crate::error::PortableLocation;
use crate::io::ReadAt;
use crate::{Error, ErrorKind, Result};

/// The largest row size of a debug table: `LocalScope` with four 4-byte indexes and two
/// `u32` columns.
const MAX_ROW_SIZE: usize = 24;

/// The error context for CLI metadata without a `#Pdb` stream.
const ASSEMBLY_METADATA: &str =
    "CLI metadata without a #Pdb stream is ordinary assembly metadata, not a Portable PDB";

/// Resource limits for Portable PDB metadata and heap values.
#[derive(Clone, Copy, Debug)]
pub struct ReadOptions {
    /// Maximum number of stream headers in the metadata root.
    pub max_streams: u16,
    /// Maximum bytes of one heap value: a string, a blob or a user string.
    pub max_value_bytes: usize,
    /// Maximum bytes of a stream that [`Pdb::stream_bytes`] loads.
    pub max_stream_bytes: usize,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            max_streams: 64,
            max_value_bytes: 64 * 1024 * 1024,
            max_stream_bytes: 64 * 1024 * 1024,
        }
    }
}

/// A standalone Portable PDB with validated metadata headers.
///
/// Opening validates the metadata root, the stream ranges, the `#Pdb` stream, the `#~`
/// header, the table schemas and the row extents. It does not read rows or heap values.
/// Row and heap accessors read and check only the requested data.
#[derive(Debug)]
pub struct Pdb<S> {
    source: S,
    options: ReadOptions,
    root: MetadataRoot,
    debug_header: PdbStream,
    tables: TableStream,
    strings: Heap,
    user_strings: Heap,
    blob: Heap,
    guid: Heap,
}

impl<S: ReadAt> Pdb<S> {
    /// Read a stable positional source with default resource limits.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::with_options`].
    pub fn read(source: S) -> Result<Self> {
        Self::with_options(source, ReadOptions::default())
    }

    /// Read a source with explicit resource limits. The metadata root starts at offset 0.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidSignature`] if the source does not start with `BSJB`.
    /// - [`ErrorKind::MissingStream`] if the `#Pdb` stream is absent (ordinary assembly
    ///   metadata) or the `#~` stream is absent.
    /// - [`ErrorKind::UnsupportedVersion`] for the uncompressed `#-` table stream and other
    ///   recognized but unsupported versions.
    /// - [`ErrorKind::InvalidFormat`], [`ErrorKind::InvalidReference`],
    ///   [`ErrorKind::UnexpectedEof`] or [`ErrorKind::LimitExceeded`] for malformed or
    ///   truncated metadata.
    /// - Errors of `source`.
    pub fn with_options(source: S, options: ReadOptions) -> Result<Self> {
        let root = MetadataRoot::read(&source, options.max_streams)?;
        let Some(pdb_header) = root.stream(MetadataStream::Pdb) else {
            return Err(Error::new(ErrorKind::MissingStream, None)
                .with_context(ASSEMBLY_METADATA)
                .with_portable(PortableLocation::new().with_stream(MetadataStream::Pdb)));
        };
        if let Some(header) = root.stream(MetadataStream::UncompressedTables) {
            return Err(Error::new(
                ErrorKind::UnsupportedVersion,
                Some(u64::from(header.offset())),
            )
            .with_context("the uncompressed #- table stream is not supported")
            .with_portable(MetadataStream::UncompressedTables.location()));
        }
        let Some(tables_header) = root.stream(MetadataStream::Tables) else {
            return Err(Error::new(ErrorKind::MissingStream, None)
                .with_context("a Portable PDB needs the #~ stream")
                .with_portable(MetadataStream::Tables.location()));
        };
        let debug_header = PdbStream::read(&source, pdb_header.offset(), pdb_header.size())?;
        let tables = TableStream::read(
            &source,
            tables_header.offset(),
            tables_header.size(),
            &debug_header,
        )?;
        let strings = Heap::read(&source, &root, MetadataStream::Strings)?;
        let user_strings = Heap::read(&source, &root, MetadataStream::UserStrings)?;
        let blob = Heap::read(&source, &root, MetadataStream::Blob)?;
        let guid = Heap::read(&source, &root, MetadataStream::Guid)?;
        Ok(Self {
            source,
            options,
            root,
            debug_header,
            tables,
            strings,
            user_strings,
            blob,
            guid,
        })
    }

    /// Return the positional source.
    #[must_use]
    pub const fn source(&self) -> &S {
        &self.source
    }

    /// Return the resource limits.
    #[must_use]
    pub const fn options(&self) -> &ReadOptions {
        &self.options
    }

    /// Return the metadata root with the version string and the stream headers.
    #[must_use]
    pub const fn metadata(&self) -> &MetadataRoot {
        &self.root
    }

    /// Return the `#Pdb` stream: PDB ID, entry point and external row counts.
    #[must_use]
    pub const fn pdb_stream(&self) -> &PdbStream {
        &self.debug_header
    }

    /// Return the `#~` header, the row counts, the index widths and the table layout.
    #[must_use]
    pub const fn tables(&self) -> &TableStream {
        &self.tables
    }

    /// Return the PDB ID as a GUID and a stamp.
    #[must_use]
    pub fn identity(&self) -> Identity {
        self.debug_header.identity()
    }

    /// Return the size in bytes of a heap, or 0 if its stream is absent.
    ///
    /// Return `None` for a stream that is not a heap.
    #[must_use]
    pub const fn heap_size(&self, heap: MetadataStream) -> Option<u32> {
        match heap {
            MetadataStream::Strings => Some(self.strings.size()),
            MetadataStream::UserStrings => Some(self.user_strings.size()),
            MetadataStream::Blob => Some(self.blob.size()),
            MetadataStream::Guid => Some(self.guid.size()),
            _ => None,
        }
    }

    /// Read one row of a debug table.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the row ID is nil or after the last row, or if a
    ///   column refers outside its heap or table.
    /// - Errors of the source.
    pub fn row<T: TableRow>(&self, id: T::Id) -> Result<T> {
        let row = T::row_number(id);
        let location = PortableLocation::new()
            .with_stream(MetadataStream::Tables)
            .with_table(T::TABLE)
            .with_row(row);
        if row == 0 || row > self.tables.row_count(T::TABLE) {
            return Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("row ID is nil or after the last row")
                .with_portable(location));
        }
        let (Some(size), Some(start)) = (
            self.tables.row_size(T::TABLE),
            self.tables.table_offset(T::TABLE),
        ) else {
            return Err(Error::new(ErrorKind::InvalidReference, None).with_portable(location));
        };
        let size = usize::from(size);
        let offset = u64::from(start) + u64::from(row - 1) * u64::try_from(size).unwrap_or(0);
        let mut buffer = [0; MAX_ROW_SIZE];
        let bytes = &mut buffer[..size];
        self.source
            .read_exact_at(offset, bytes)
            .map_err(|error| error.with_portable(location))?;
        let heaps = [&self.strings, &self.blob, &self.guid];
        let mut reader = RowReader::new(bytes, &self.tables, heaps, T::TABLE, row);
        decode_row(&mut reader)
    }

    /// Iterate over all rows of a debug table in row order.
    #[must_use]
    pub fn rows<T: TableRow>(&self) -> Rows<'_, S, T> {
        Rows {
            pdb: self,
            next: 1,
            count: self.tables.row_count(T::TABLE),
            row: PhantomData,
        }
    }

    /// Read the string at `offset` in the `#Strings` heap, without its terminator.
    ///
    /// The bytes are UTF-8 in valid metadata. This function does not check the encoding.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the offset is outside the heap.
    /// - [`ErrorKind::InvalidFormat`] if the string has no terminator in the heap.
    /// - [`ErrorKind::LimitExceeded`] if the string is longer than
    ///   [`ReadOptions::max_value_bytes`].
    pub fn string(&self, offset: StringOffset) -> Result<Vec<u8>> {
        self.strings
            .string(&self.source, offset.get(), self.options.max_value_bytes)
    }

    /// Read the blob at `offset` in the `#Blob` heap, without its length prefix.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the offset is outside the heap.
    /// - [`ErrorKind::InvalidFormat`] if the length prefix is malformed or the blob ends
    ///   after the heap.
    /// - [`ErrorKind::LimitExceeded`] if the blob is longer than
    ///   [`ReadOptions::max_value_bytes`].
    pub fn blob(&self, offset: BlobOffset) -> Result<Vec<u8>> {
        self.blob
            .blob(&self.source, offset.get(), self.options.max_value_bytes)
    }

    /// Read the entry at `offset` in the `#US` heap, without its length prefix.
    ///
    /// The entry has UTF-16LE code units and a final flag byte.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Pdb::blob`].
    pub fn user_string(&self, offset: UserStringOffset) -> Result<Vec<u8>> {
        self.user_strings
            .blob(&self.source, offset.get(), self.options.max_value_bytes)
    }

    /// Read the GUID at `index` in the `#GUID` heap. Return `None` for the nil index 0.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the index is after the heap.
    /// - Errors of the source.
    pub fn guid(&self, index: GuidIndex) -> Result<Option<[u8; 16]>> {
        self.guid.guid(&self.source, index.get())
    }

    /// Load all bytes of a recognized stream within [`ReadOptions::max_stream_bytes`].
    ///
    /// Return `None` if the stream is absent.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`] if the stream is larger than the limit.
    /// - Errors of the source.
    pub fn stream_bytes(&self, stream: MetadataStream) -> Result<Option<Vec<u8>>> {
        let Some(header) = self.root.stream(stream) else {
            return Ok(None);
        };
        let size = usize::try_from(header.size()).unwrap_or(usize::MAX);
        let limit = || {
            Error::new(ErrorKind::LimitExceeded, Some(u64::from(header.offset())))
                .with_context("metadata stream is larger than the limit")
                .with_portable(stream.location())
        };
        if size > self.options.max_stream_bytes {
            return Err(limit());
        }
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(size).map_err(|_| limit())?;
        bytes.resize(size, 0);
        self.source
            .read_exact_at(u64::from(header.offset()), &mut bytes)?;
        Ok(Some(bytes))
    }
}

impl<'a> Pdb<&'a [u8]> {
    /// Parse a standalone Portable PDB from borrowed bytes.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Pdb::read`].
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::read(bytes)
    }
}

#[cfg(all(feature = "std", any(unix, windows)))]
impl Pdb<crate::io::FileReader> {
    /// Open a standalone Portable PDB without loading all file bytes into memory.
    ///
    /// # Errors
    ///
    /// Returns file I/O errors and the errors of [`Pdb::read`].
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Self::read(crate::io::FileReader::open(path)?)
    }
}

/// An iterator over the rows of one debug table.
///
/// Each item is the row ID and the row, or the error of that row.
#[derive(Debug)]
pub struct Rows<'a, S, T> {
    pdb: &'a Pdb<S>,
    next: u32,
    count: u32,
    row: PhantomData<fn() -> T>,
}

impl<S: ReadAt, T: TableRow> Iterator for Rows<'_, S, T> {
    type Item = Result<(T::Id, T)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next > self.count {
            return None;
        }
        let id = T::row_id(self.next);
        self.next += 1;
        Some(self.pdb.row::<T>(id).map(|row| (id, row)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = usize::try_from(self.count + 1 - self.next).unwrap_or(usize::MAX);
        (remaining, Some(remaining))
    }
}

impl<S: ReadAt, T: TableRow> ExactSizeIterator for Rows<'_, S, T> {}
