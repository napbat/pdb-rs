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
pub(super) fn each_row<'a, S: ReadAt, T>(
    pdb: &'a Pdb<S>,
    table: TableId,
    read: impl Fn(&'a Pdb<S>, u32) -> Result<T> + 'a,
) -> impl ExactSizeIterator<Item = Result<T>> + 'a {
    let rows = pdb.tables().row_count(table);
    (0..rows).map(move |index| read(pdb, index + 1))
}
