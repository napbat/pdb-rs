//! Views of a symbol index and the symbol record stream.
//!
//! [`crate::native::Pdb::globals`] and [`crate::native::Pdb::publics`] return these views. The
//! index and the records stay in the session cache; the views only borrow them.

use core::iter::FusedIterator;

use super::{GlobalSymbolIndex, HashRecords, PublicSymbolIndex};
use crate::Result;
use crate::native::codeview::symbol::Symbol;

/// The global symbol index and the symbol record stream that it indexes.
#[derive(Clone, Copy, Debug)]
pub struct Globals<'a> {
    /// The global symbol index.
    index: &'a GlobalSymbolIndex,
    /// The symbol record stream.
    records: &'a [u8],
}

impl<'a> Globals<'a> {
    /// Make a view of `index` and the symbol record stream `records`.
    pub(crate) const fn new(index: &'a GlobalSymbolIndex, records: &'a [u8]) -> Self {
        Self { index, records }
    }

    /// Return the global symbol index.
    #[must_use]
    pub const fn index(&self) -> &'a GlobalSymbolIndex {
        self.index
    }

    /// Return the symbol record stream.
    ///
    /// The slice is empty if the DBI header names no symbol record stream.
    #[must_use]
    pub const fn records(&self) -> &'a [u8] {
        self.records
    }

    /// Return the symbols that the hash records name, in hash record order.
    #[must_use]
    pub fn symbols(&self) -> IndexedSymbols<'a> {
        IndexedSymbols {
            hash_records: self.index.hash_table().records(),
            records: self.records,
        }
    }
}

/// The public symbol index and the symbol record stream that it indexes.
#[derive(Clone, Copy, Debug)]
pub struct Publics<'a> {
    /// The public symbol index.
    index: &'a PublicSymbolIndex,
    /// The symbol record stream.
    records: &'a [u8],
}

impl<'a> Publics<'a> {
    /// Make a view of `index` and the symbol record stream `records`.
    pub(crate) const fn new(index: &'a PublicSymbolIndex, records: &'a [u8]) -> Self {
        Self { index, records }
    }

    /// Return the public symbol index.
    #[must_use]
    pub const fn index(&self) -> &'a PublicSymbolIndex {
        self.index
    }

    /// Return the symbol record stream.
    ///
    /// The slice is empty if the DBI header names no symbol record stream.
    #[must_use]
    pub const fn records(&self) -> &'a [u8] {
        self.records
    }

    /// Return the symbols that the hash records name, in hash record order.
    #[must_use]
    pub fn symbols(&self) -> IndexedSymbols<'a> {
        IndexedSymbols {
            hash_records: self.index.hash_table().records(),
            records: self.records,
        }
    }
}

/// An iterator over the symbols that the hash records of a symbol index name.
///
/// Each item is the decoded record at [`super::HashRecord::symbol`], or the error of
/// [`super::SymbolOffset::record`] or [`Symbol::decode`]. The iterator continues after an
/// error.
#[derive(Clone, Debug)]
pub struct IndexedSymbols<'a> {
    /// The remaining hash records.
    hash_records: HashRecords<'a>,
    /// The symbol record stream.
    records: &'a [u8],
}

impl<'a> Iterator for IndexedSymbols<'a> {
    type Item = Result<Symbol<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        let hash_record = self.hash_records.next()?;
        Some(
            hash_record
                .symbol
                .record(self.records)
                .and_then(Symbol::decode),
        )
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.hash_records.size_hint()
    }
}

impl ExactSizeIterator for IndexedSymbols<'_> {}

impl FusedIterator for IndexedSymbols<'_> {}
