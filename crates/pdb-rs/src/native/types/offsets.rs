//! The header of a TPI or IPI stream with the index offset pairs of its hash stream.
//!
//! [`crate::native::Pdb::type_offsets`] and [`crate::native::Pdb::id_offsets`] read only these
//! parts. They do not read the records. A writer that adds records needs the header (the next
//! record index and the record size) and the index offset pairs.

use alloc::vec::Vec;

use super::hash::{check_index_offsets, index_offset_pairs};
use super::{IndexOffset, TypeHeader};
use crate::native::output::no_memory;
use crate::{Error, ErrorKind, Result};

/// The validated header of a TPI or IPI stream and the index offset pairs of its hash stream.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TypeOffsets {
    /// The validated header.
    header: TypeHeader,
    /// The index offset pairs in index order.
    index_offsets: Vec<IndexOffset>,
}

impl TypeOffsets {
    /// Make the value from a validated `header` and the index offset buffer of its hash stream.
    ///
    /// `buffer` holds the bytes at [`TypeHeader::index_offsets`] in the hash stream. It is empty
    /// if the header has no hash stream.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the pairs are not sorted.
    /// - [`ErrorKind::InvalidReference`] if a pair is outside the records.
    /// - [`ErrorKind::LimitExceeded`] if the pairs cannot be allocated.
    ///
    /// The errors have the hash stream and a hash stream offset.
    pub(crate) fn from_parts(header: TypeHeader, buffer: &[u8]) -> Result<Self> {
        let pairs = index_offset_pairs(buffer);
        if let Some(stream) = header.hash_stream {
            // `parse_header_prefix` validated the offset of a non-empty buffer.
            let start = u32::try_from(header.index_offsets.offset).unwrap_or(0);
            check_index_offsets(&header, pairs.clone(), stream, start)?;
        } else if !buffer.is_empty() {
            return Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("type stream header has no hash stream"));
        }
        let mut index_offsets = Vec::new();
        index_offsets
            .try_reserve_exact(pairs.len())
            .map_err(|_| no_memory())?;
        index_offsets.extend(pairs);
        Ok(Self {
            header,
            index_offsets,
        })
    }

    /// Return the validated header.
    #[must_use]
    pub const fn header(&self) -> &TypeHeader {
        &self.header
    }

    /// Return the index offset pairs in index order.
    ///
    /// The slice is empty if the stream has no hash stream or no pairs.
    #[must_use]
    pub fn index_offsets(&self) -> &[IndexOffset] {
        &self.index_offsets
    }
}
