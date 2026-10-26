//! The builder of the global symbol index stream.

use alloc::vec::Vec;

use super::super::BucketCount;
use super::super::table::write::{HashEntry, IndexProducer, append_table};
use super::GlobalSymbolIndex;
use crate::Result;

impl GlobalSymbolIndex {
    /// Build the bytes of a global symbol index stream.
    ///
    /// `symbol_records` is the symbol record stream. `entries` gives the
    /// records to index, in any order. Select `buckets` as for
    /// [`GlobalSymbolIndex::parse`]. The stream holds only the name hash
    /// table. [`HashEntry`] gives the order of the records in a bucket.
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
    /// The errors do not have a location.
    pub fn build(
        symbol_records: &[u8],
        entries: &[HashEntry],
        buckets: BucketCount,
        producer: IndexProducer,
    ) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        append_table(symbol_records, entries, buckets, producer, &mut bytes)?;
        Ok(bytes)
    }
}
