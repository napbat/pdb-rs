//! The `#~` table stream header and the table layout (ECMA-335 II.24.2.6).
//!
//! The header has a reserved `u32`, the version (2.0), the `HeapSizes` flags, a reserved
//! byte, the `Valid` and `Sorted` table masks and one `u32` row count for each valid table.
//! The rows of each valid table follow in table number order.

use super::coded::CodedIndex;
use super::ids::{MAX_ROW, TableId};
use super::pdb_stream::PdbStream;
use super::root::MetadataStream;
use crate::PortableLocation;
use crate::io::ReadAt;
use crate::{Error, ErrorKind, Result};

/// The size of the fixed header fields.
const FIXED_LEN: u32 = 24;

/// The `HeapSizes` flag for 4-byte `#Strings` indexes.
const LARGE_STRINGS: u8 = 0x01;
/// The `HeapSizes` flag for 4-byte `#GUID` indexes.
const LARGE_GUIDS: u8 = 0x02;
/// The `HeapSizes` flag for 4-byte `#Blob` indexes.
const LARGE_BLOBS: u8 = 0x04;

/// A simple table index uses 4 bytes when the table has this many rows or more.
const LARGE_TABLE: u32 = 1 << 16;

/// The validated `#~` stream header with the computed table layout.
///
/// The layout uses the local row counts of the debug tables and the external row counts of
/// the `#Pdb` stream for the type-system tables.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableStream {
    major: u8,
    minor: u8,
    heap_sizes: u8,
    valid: u64,
    sorted: u64,
    local_rows: [u32; 64],
    row_counts: [u32; 64],
    row_sizes: [u8; 8],
    table_offsets: [u32; 8],
}

impl TableStream {
    /// Read and validate the `#~` stream of `size` bytes at `offset`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedVersion`] for a table stream version other than 2.0 or for
    ///   `HeapSizes` flags other than the three heap index flags.
    /// - [`ErrorKind::InvalidFormat`] if the header or the rows do not fit in the stream, a
    ///   row count does not fit in a metadata token, or a table outside the debug schema has
    ///   rows. The row width of such a table is unknown.
    /// - Errors of `source`.
    pub(crate) fn read(
        source: &(impl ReadAt + ?Sized),
        offset: u32,
        size: u32,
        pdb: &PdbStream,
    ) -> Result<Self> {
        let at = |relative: u32| u64::from(offset) + u64::from(relative);
        if size < FIXED_LEN {
            return Err(invalid(at(0), "#~ stream is shorter than its header"));
        }
        let mut header = [0; FIXED_LEN as usize];
        source.read_exact_at(at(0), &mut header)?;
        let (major, minor, heap_sizes) = (header[4], header[5], header[6]);
        if (major, minor) != (2, 0) {
            return Err(unsupported(at(4), "#~ stream version is not 2.0"));
        }
        if heap_sizes & !(LARGE_STRINGS | LARGE_GUIDS | LARGE_BLOBS) != 0 {
            return Err(unsupported(
                at(6),
                "#~ HeapSizes has flags for delta metadata",
            ));
        }
        let mask = |start: usize| {
            let mut bytes = [0; 8];
            bytes.copy_from_slice(&header[start..start + 8]);
            u64::from_le_bytes(bytes)
        };
        let (valid, sorted) = (mask(8), mask(16));
        let header_len = FIXED_LEN + 4 * valid.count_ones();
        if header_len > size {
            return Err(invalid(at(FIXED_LEN), "#~ row counts end after the stream"));
        }

        let mut local_rows = [0; 64];
        let mut position = FIXED_LEN;
        for table in (0_u8..64).filter_map(TableId::new) {
            if valid & table.bit() == 0 {
                continue;
            }
            let count = source.read_u32_at(at(position))?;
            let location = PortableLocation::new()
                .with_stream(MetadataStream::Tables)
                .with_table(table);
            if count > MAX_ROW {
                return Err(
                    invalid(at(position), "row count does not fit in a metadata token")
                        .with_portable(location),
                );
            }
            if count > 0 && !table.is_debug() {
                return Err(invalid(
                    at(position),
                    "a table outside the Portable PDB debug schema has rows; its row width is unknown",
                )
                .with_portable(location));
            }
            local_rows[table.index()] = count;
            position += 4;
        }

        let mut row_counts = *pdb.row_counts();
        for table in TableId::DEBUG_TABLES {
            row_counts[table.index()] = local_rows[table.index()];
        }
        let mut stream = Self {
            major,
            minor,
            heap_sizes,
            valid,
            sorted,
            local_rows,
            row_counts,
            row_sizes: [0; 8],
            table_offsets: [0; 8],
        };
        stream.lay_out(offset, header_len, size)?;
        Ok(stream)
    }

    /// Compute the row size and offset of each debug table, and check that the rows fit in
    /// the stream.
    fn lay_out(&mut self, offset: u32, header_len: u32, size: u32) -> Result<()> {
        let mut position = u64::from(header_len);
        for (slot, table) in TableId::DEBUG_TABLES.into_iter().enumerate() {
            let row_size = self.compute_row_size(table);
            let start = position;
            position += u64::from(self.local_rows[table.index()]) * u64::from(row_size);
            let location = PortableLocation::new()
                .with_stream(MetadataStream::Tables)
                .with_table(table);
            if position > u64::from(size) {
                return Err(Error::new(
                    ErrorKind::InvalidFormat,
                    Some(u64::from(offset) + u64::from(size)),
                )
                .with_context("table rows end after the #~ stream")
                .with_portable(location));
            }
            // `start` is at most `size`, so it fits in `u32`.
            let table_offset = u32::try_from(start)
                .ok()
                .and_then(|start| offset.checked_add(start))
                .ok_or_else(|| {
                    Error::new(ErrorKind::OffsetOverflow, Some(u64::from(offset) + start))
                        .with_portable(location)
                })?;
            self.row_sizes[slot] = row_size;
            self.table_offsets[slot] = table_offset;
        }
        Ok(())
    }

    /// Compute the row size of a debug table from the column widths.
    fn compute_row_size(&self, table: TableId) -> u8 {
        let string = self.string_index_width();
        let guid = self.guid_index_width();
        let blob = self.blob_index_width();
        let index = |table| self.table_index_width(table);
        match table {
            TableId::DOCUMENT => blob + guid + blob + guid,
            TableId::METHOD_DEBUG_INFORMATION => index(TableId::DOCUMENT) + blob,
            TableId::LOCAL_SCOPE => {
                index(TableId::METHOD_DEF)
                    + index(TableId::IMPORT_SCOPE)
                    + index(TableId::LOCAL_VARIABLE)
                    + index(TableId::LOCAL_CONSTANT)
                    + 8
            }
            TableId::LOCAL_VARIABLE => 4 + string,
            TableId::LOCAL_CONSTANT => string + blob,
            TableId::IMPORT_SCOPE => index(TableId::IMPORT_SCOPE) + blob,
            TableId::STATE_MACHINE_METHOD => 2 * index(TableId::METHOD_DEF),
            _ => self.coded_index_width(CodedIndex::HasCustomDebugInformation) + guid + blob,
        }
    }

    /// Return the table stream version as (major, minor). The reader accepts only 2.0.
    #[must_use]
    pub const fn version(&self) -> (u8, u8) {
        (self.major, self.minor)
    }

    /// Return the `HeapSizes` flags.
    #[must_use]
    pub const fn heap_sizes(&self) -> u8 {
        self.heap_sizes
    }

    /// Return the `Valid` mask: a bit for each table that has a row count.
    #[must_use]
    pub const fn valid_mask(&self) -> u64 {
        self.valid
    }

    /// Return the `Sorted` mask as serialized.
    #[must_use]
    pub const fn sorted_mask(&self) -> u64 {
        self.sorted
    }

    /// Return the local row count of a table in this stream.
    #[must_use]
    pub const fn row_count(&self, table: TableId) -> u32 {
        self.local_rows[table.index()]
    }

    /// Return the row count that sets index widths: the local count of a debug table, or the
    /// external `#Pdb` count of a type-system table.
    #[must_use]
    pub const fn width_row_count(&self, table: TableId) -> u32 {
        self.row_counts[table.index()]
    }

    /// Return the width in bytes (2 or 4) of a `#Strings` index.
    #[must_use]
    pub const fn string_index_width(&self) -> u8 {
        heap_width(self.heap_sizes, LARGE_STRINGS)
    }

    /// Return the width in bytes (2 or 4) of a `#GUID` index.
    #[must_use]
    pub const fn guid_index_width(&self) -> u8 {
        heap_width(self.heap_sizes, LARGE_GUIDS)
    }

    /// Return the width in bytes (2 or 4) of a `#Blob` index.
    #[must_use]
    pub const fn blob_index_width(&self) -> u8 {
        heap_width(self.heap_sizes, LARGE_BLOBS)
    }

    /// Return the width in bytes (2 or 4) of a simple index into `table`.
    #[must_use]
    pub const fn table_index_width(&self, table: TableId) -> u8 {
        if self.row_counts[table.index()] < LARGE_TABLE {
            2
        } else {
            4
        }
    }

    /// Return the width in bytes (2 or 4) of a coded index.
    #[must_use]
    pub fn coded_index_width(&self, index: CodedIndex) -> u8 {
        index.width(&self.row_counts)
    }

    /// Return the row size in bytes of a debug table, or `None` for another table.
    #[must_use]
    pub fn row_size(&self, table: TableId) -> Option<u8> {
        debug_slot(table).map(|slot| self.row_sizes[slot])
    }

    /// Return the offset of the first row of a debug table from the start of the metadata
    /// root, or `None` for another table.
    #[must_use]
    pub fn table_offset(&self, table: TableId) -> Option<u32> {
        debug_slot(table).map(|slot| self.table_offsets[slot])
    }
}

/// Return the position of a debug table in [`TableId::DEBUG_TABLES`].
fn debug_slot(table: TableId) -> Option<usize> {
    table
        .is_debug()
        .then(|| usize::from(table.number() - TableId::DOCUMENT.number()))
}

/// Return the heap index width for a `HeapSizes` flag.
const fn heap_width(heap_sizes: u8, flag: u8) -> u8 {
    if heap_sizes & flag == 0 { 2 } else { 4 }
}

/// Make an [`ErrorKind::InvalidFormat`] error in the `#~` stream.
fn invalid(offset: u64, context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, Some(offset))
        .with_context(context)
        .with_portable(MetadataStream::Tables.location())
}

/// Make an [`ErrorKind::UnsupportedVersion`] error in the `#~` stream.
fn unsupported(offset: u64, context: &'static str) -> Error {
    Error::new(ErrorKind::UnsupportedVersion, Some(offset))
        .with_context(context)
        .with_portable(MetadataStream::Tables.location())
}
