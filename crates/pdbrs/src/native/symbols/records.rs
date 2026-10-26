//! A writer of the symbol record stream.
//!
//! [`SymbolRecordWriter`] encodes the shared symbol records one after the
//! other, each with its zero padding. It returns the [`SymbolOffset`] of each
//! record. Give the offsets to [`HashEntry::new`](super::HashEntry::new) for
//! [`GlobalSymbolIndex::build`](super::GlobalSymbolIndex::build) and
//! [`PublicSymbolIndex::build`](super::PublicSymbolIndex::build), and to the
//! global references of the module streams.

use alloc::vec::Vec;

use super::SymbolOffset;
use crate::native::codeview::symbol::SymbolData;
use crate::{Error, ErrorKind, Result};

/// A writer of the symbol record stream.
#[derive(Clone, Debug, Default)]
pub struct SymbolRecordWriter {
    /// The encoded records.
    bytes: Vec<u8>,
    /// The offset of each record, in stream order.
    offsets: Vec<SymbolOffset>,
}

/// The output of [`SymbolRecordWriter::finish`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct SymbolRecordStream {
    /// The stream bytes.
    pub bytes: Vec<u8>,
    /// The offset of each record, in stream order.
    pub offsets: Vec<SymbolOffset>,
}

impl SymbolRecordWriter {
    /// Create an empty writer.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bytes: Vec::new(),
            offsets: Vec::new(),
        }
    }

    /// Encode a record of `kind` and return its offset in the stream.
    ///
    /// # Errors
    ///
    /// - The errors of [`SymbolData::encode`].
    /// - [`ErrorKind::OffsetOverflow`] if the offset does not fit in 32 bits.
    /// - [`ErrorKind::LimitExceeded`] if the offset list cannot grow.
    ///
    /// After an error, the writer is unchanged.
    pub fn push(&mut self, kind: u16, data: &SymbolData<'_>) -> Result<SymbolOffset> {
        let offset = u32::try_from(self.bytes.len()).map_err(|_| {
            Error::new(ErrorKind::OffsetOverflow, None)
                .with_context("symbol record offset does not fit in 32 bits")
        })?;
        self.offsets.try_reserve(1).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("symbol record offsets cannot be allocated")
        })?;
        let start = self.bytes.len();
        if let Err(error) = data.encode(kind, &mut self.bytes) {
            self.bytes.truncate(start);
            return Err(error);
        }
        let offset = SymbolOffset(offset);
        self.offsets.push(offset);
        Ok(offset)
    }

    /// Return the stream bytes and the record offsets.
    #[must_use]
    pub fn finish(self) -> SymbolRecordStream {
        SymbolRecordStream {
            bytes: self.bytes,
            offsets: self.offsets,
        }
    }
}
