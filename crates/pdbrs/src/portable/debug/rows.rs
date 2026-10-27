//! Shared helpers of the typed row accessors: error locations and row iteration.

use crate::PortableLocation;
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::metadata::{BlobOffset, MetadataStream, TableId};
use crate::{Error, ErrorKind, Result};

/// Add the location of a blob column to a codec error.
pub(super) fn in_blob(
    error: Error,
    table: TableId,
    row: u32,
    column: &'static str,
    offset: BlobOffset,
) -> Error {
    error.with_portable(
        PortableLocation::new()
            .with_stream(MetadataStream::Blob)
            .with_table(table)
            .with_row(row)
            .with_column(column)
            .with_heap_offset(offset.0),
    )
}

/// Make an error for a column of a row.
pub(super) fn row_error(
    kind: ErrorKind,
    context: &'static str,
    table: TableId,
    row: u32,
    column: Option<&'static str>,
) -> Error {
    let mut location = PortableLocation::new()
        .with_stream(MetadataStream::Tables)
        .with_table(table)
        .with_row(row);
    if let Some(column) = column {
        location = location.with_column(column);
    }
    Error::new(kind, None)
        .with_context(context)
        .with_portable(location)
}

/// Call `read` for each row number of `table` in row order.
pub(crate) fn each_row<'a, S: ReadAt, T>(
    pdb: &'a Pdb<S>,
    table: TableId,
    read: fn(&'a Pdb<S>, u32) -> Result<T>,
) -> TypedRows<'a, S, T> {
    TypedRows {
        pdb,
        rows: 0..pdb.tables().row_count(table),
        read,
    }
}

/// An iterator over the typed rows of one debug table of a [`Pdb`], in row order.
///
/// Each item is a typed row, or the error of that row.
#[derive(Debug)]
pub struct TypedRows<'a, S, T> {
    /// The PDB.
    pdb: &'a Pdb<S>,
    /// The 0-based row numbers that are not returned yet.
    rows: core::ops::Range<u32>,
    /// The reader of one row.
    read: fn(&'a Pdb<S>, u32) -> Result<T>,
}

impl<S: ReadAt, T> Iterator for TypedRows<'_, S, T> {
    type Item = Result<T>;

    fn next(&mut self) -> Option<Result<T>> {
        let index = self.rows.next()?;
        Some((self.read)(self.pdb, index + 1))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.rows.size_hint()
    }
}

impl<S: ReadAt, T> ExactSizeIterator for TypedRows<'_, S, T> {}

impl<S: ReadAt, T> core::iter::FusedIterator for TypedRows<'_, S, T> {}
