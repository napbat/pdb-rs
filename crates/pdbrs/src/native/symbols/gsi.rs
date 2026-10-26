//! The global symbol index stream.
//!
//! The stream has no header of its own. It holds only a name hash table. The
//! table indexes the global records of the symbol record stream that are not
//! `S_PUB32` records, for example `S_PROCREF`, `S_UDT` and `S_GDATA32`. See
//! `GSI1::readHash` in `PDB/dbi/gsi.cpp` and LLVM `GlobalsStream.cpp`.

use alloc::vec::Vec;

use super::BucketCount;
use super::table::{HashTable, Layout};
use crate::Result;
use crate::native::msf::StreamId;

mod write;

/// A decoded global symbol index stream with owned serialized bytes.
#[derive(Clone, Debug)]
pub struct GlobalSymbolIndex {
    /// The stream bytes.
    bytes: Vec<u8>,
    /// The validated hash table.
    table: Layout,
}

impl GlobalSymbolIndex {
    /// Decode and check a global symbol index stream.
    ///
    /// Get `stream` from [`crate::native::dbi::DbiHeader::global_stream`]. Get
    /// `buckets` from the PDB Info stream with [`BucketCount::from_features`].
    /// The hash table must fill the stream exactly.
    ///
    /// The function checks the header, the hash records and the buckets. It
    /// does not read the symbol record stream; use
    /// [`HashTable::check_records`] for that.
    ///
    /// # Errors
    ///
    /// - [`crate::ErrorKind::UnexpectedEof`] if a part of the table is truncated.
    /// - [`crate::ErrorKind::UnsupportedVersion`] if the hash header does not
    ///   have the compressed-bucket signature and version.
    /// - [`crate::ErrorKind::InvalidReference`] for a hash record offset that
    ///   is not a positive 32-bit value.
    /// - [`crate::ErrorKind::InvalidFormat`] for a record size that is not a
    ///   multiple of 8, a reference count of zero, an invalid bitmap, invalid
    ///   bucket offsets or bytes after the table.
    /// - [`crate::ErrorKind::LimitExceeded`] or
    ///   [`crate::ErrorKind::OffsetOverflow`] if a size cannot be represented.
    ///
    /// Errors identify `stream` and a logical offset in it.
    pub fn parse(bytes: Vec<u8>, stream: StreamId, buckets: BucketCount) -> Result<Self> {
        let table = Layout::parse(&bytes, 0..bytes.len(), stream, buckets)?;
        Ok(Self { bytes, table })
    }

    /// Return the name hash table.
    #[must_use]
    pub const fn hash_table(&self) -> HashTable<'_> {
        self.table.view(self.bytes.as_slice())
    }

    /// Return the stream bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}
