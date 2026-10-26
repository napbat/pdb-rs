//! The builder of the global symbol index stream.

use alloc::vec::Vec;

use super::super::BucketCount;
use super::super::table::write::{HashEntry, append_table};
use super::GlobalSymbolIndex;
use crate::Result;
use crate::native::Producer;

impl GlobalSymbolIndex {
    /// Append the bytes of a global symbol index stream to `out`.
    ///
    /// `symbol_records` is the symbol record stream. `entries` gives the
    /// records to index, in any order. Select `buckets` as for
    /// [`GlobalSymbolIndex::parse`]. The stream holds only the name hash
    /// table. [`HashEntry`] gives the order of the records in a bucket.
    ///
    /// `producer` selects the layout of a table without records:
    ///
    /// - [`Producer::Microsoft`]: the table has no bucket bitmap
    ///   (`GSI1::CompressBuckets` in `PDB/dbi/gsi.cpp`).
    /// - [`Producer::Llvm`]: the table has a bucket bitmap with no bits set
    ///   (LLVM `GSIStreamBuilder.cpp`).
    ///
    /// `GlobalSymbolIndex::parse` of the result gives a table with the
    /// entries in their buckets, with their reference counts.
    ///
    /// # Errors
    ///
    /// - [`crate::ErrorKind::LimitExceeded`] if a symbol offset, a reference
    ///   count or a size does not fit its field, or if an allocation fails.
    /// - [`crate::ErrorKind::InvalidFormat`] for a reference count of 0, a
    ///   bucket that is not less than the bucket count, or a record without
    ///   a name.
    /// - [`crate::ErrorKind::InvalidReference`] or the errors of the symbol
    ///   decoder for an entry that does not identify a valid record.
    ///
    /// The errors do not have a location. If the function fails, `out` does
    /// not change.
    pub fn encode_parts(
        symbol_records: &[u8],
        entries: &[HashEntry],
        buckets: BucketCount,
        producer: Producer,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        append_table(symbol_records, entries, buckets, producer, out)
    }
}
