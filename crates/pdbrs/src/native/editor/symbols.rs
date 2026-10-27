//! The records of the symbol record stream and the global and public symbol
//! indexes of the editor model.

use alloc::vec::Vec;

use super::record::SymbolRecord;
use crate::native::codeview::index::SymbolHandle;
use crate::native::codeview::symbol::{Public, S_PUB32, Symbol, SymbolData};
use crate::native::codeview::{Records, SymbolKind};
use crate::native::output::no_memory;
use crate::native::symbols::{
    GlobalSymbolIndex, PublicSymbolIndex, SectionStart, SymbolOffset, ThunkTable,
};
use crate::{Error, ErrorKind, Result};

mod index;

use index::{NameIndex, View};

/// An entry of the global or the public symbol index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct IndexEntry {
    /// The record.
    pub(super) symbol: SymbolHandle,
    /// The reference count.
    pub(super) reference_count: u32,
    /// The bucket of a source hash record, or `None` for the bucket of the
    /// name.
    pub(super) bucket: Option<u32>,
}

/// The incremental thunk table of the public symbol index.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct Thunks {
    /// The size of one thunk.
    pub(super) thunk_size: u32,
    /// The section of the thunk table.
    pub(super) section: u16,
    /// The offset of the thunk table in its section.
    pub(super) offset: u32,
    /// The thunk targets.
    pub(super) targets: Vec<u32>,
}

impl Thunks {
    /// Return the table for the encoder.
    pub(super) fn table(&self) -> ThunkTable<'_> {
        ThunkTable {
            thunk_size: self.thunk_size,
            section: self.section,
            offset: self.offset,
            targets: &self.targets,
        }
    }
}

/// The source of the model of the symbol record stream.
#[derive(Clone, Debug)]
pub(super) struct SharedSource {
    /// The source entries of the global index.
    pub(super) globals: Vec<IndexEntry>,
    /// The source entries of the public index.
    pub(super) publics: Vec<IndexEntry>,
    /// The number of source records.
    pub(super) count: usize,
}

/// The model of the symbol record stream and its two indexes.
#[derive(Clone, Debug, Default)]
pub(super) struct SharedSymbols {
    /// The records in stream order. A removed record leaves `None`.
    pub(super) records: Vec<Option<SymbolRecord>>,
    /// The entries of the global index.
    pub(super) globals: Vec<IndexEntry>,
    /// The entries of the public index.
    pub(super) publics: Vec<IndexEntry>,
    /// The thunk table of the public index.
    pub(super) thunks: Thunks,
    /// The section starts of the public index.
    pub(super) section_starts: Vec<SectionStart>,
    /// The source, or `None` for a model without a source.
    pub(super) source: Option<SharedSource>,
    /// The index of the names (#48).
    name_index: NameIndex,
}

/// Return the handle of the record at `offset`.
fn handle_of(offsets: &[u32], offset: SymbolOffset) -> Result<SymbolHandle> {
    offsets
        .binary_search(&offset.0)
        .map(|position| SymbolHandle(u32::try_from(position).unwrap_or(u32::MAX)))
        .map_err(|_| {
            Error::new(ErrorKind::InvalidReference, None).with_context(
                "symbol offset does not identify a record of the symbol record stream",
            )
        })
}

impl SharedSymbols {
    /// Decode the records of a symbol record stream and the entries of its
    /// indexes. Each source hash record keeps its bucket and its reference
    /// count.
    pub(super) fn decode(
        records: &[u8],
        globals: Option<&GlobalSymbolIndex>,
        publics: Option<&PublicSymbolIndex>,
    ) -> Result<Self> {
        let mut model = Self::default();
        let mut offsets = Vec::new();
        for record in Records::new(records) {
            let record = record?;
            let symbol = Symbol::decode(record)?.into_owned();
            push(&mut offsets, record.offset)?;
            push(&mut model.records, Some(SymbolRecord::from_source(symbol)))?;
        }
        if let Some(globals) = globals {
            for bucket in globals.hash_table().buckets() {
                for record in bucket.records {
                    let entry = IndexEntry {
                        symbol: handle_of(&offsets, record.symbol)?,
                        reference_count: record.reference_count,
                        bucket: Some(bucket.index),
                    };
                    push(&mut model.globals, entry)?;
                }
            }
        }
        if let Some(publics) = publics {
            for bucket in publics.hash_table().buckets() {
                for record in bucket.records {
                    let entry = IndexEntry {
                        symbol: handle_of(&offsets, record.symbol)?,
                        reference_count: record.reference_count,
                        bucket: Some(bucket.index),
                    };
                    push(&mut model.publics, entry)?;
                }
            }
            let header = publics.header();
            model.thunks = Thunks {
                thunk_size: header.thunk_size,
                section: header.thunk_table_section,
                offset: header.thunk_table_offset,
                targets: publics.thunk_map().collect(),
            };
            model.section_starts = publics.section_starts().collect();
        }
        model.source = Some(SharedSource {
            globals: model.globals.clone(),
            publics: model.publics.clone(),
            count: model.records.len(),
        });
        model.name_index = NameIndex::from_source(
            model.records.len(),
            globals.map(|index| (index.hash_table().bucket_count(), &model.globals[..])),
            publics.map(|index| (index.hash_table().bucket_count(), &model.publics[..])),
        )?;
        Ok(model)
    }

    /// Return the record of `handle`.
    pub(super) fn record(&self, handle: SymbolHandle) -> Option<&SymbolRecord> {
        self.records.get(handle.0 as usize).and_then(Option::as_ref)
    }

    /// Add a record of the index `view` and return its handle.
    fn add(&mut self, record: SymbolRecord, view: View) -> Result<SymbolHandle> {
        let position = u32::try_from(self.records.len())
            .ok()
            .filter(|&position| position != u32::MAX)
            .ok_or_else(|| {
                Error::new(ErrorKind::LimitExceeded, None)
                    .with_context("symbol record stream has too many records")
            })?;
        push(&mut self.records, Some(record))?;
        let handle = SymbolHandle(position);
        let name = self.records[position as usize]
            .as_ref()
            .and_then(SymbolRecord::name);
        self.name_index.add(&self.records, view, handle, name)?;
        Ok(handle)
    }

    /// Remove a record and its index entries.
    fn remove(&mut self, handle: SymbolHandle) -> Result<()> {
        let slot = self
            .records
            .get_mut(handle.0 as usize)
            .filter(|slot| slot.is_some())
            .ok_or_else(missing_record)?;
        *slot = None;
        self.globals.retain(|entry| entry.symbol != handle);
        self.publics.retain(|entry| entry.symbol != handle);
        self.name_index.remove(&self.records, handle);
        Ok(())
    }

    /// Return the record of `handle` for a change. Its name can change.
    fn lend(&mut self, handle: SymbolHandle) -> Option<&mut SymbolRecord> {
        self.name_index.lend(&self.records, handle);
        self.records
            .get_mut(handle.0 as usize)
            .and_then(Option::as_mut)
    }

    /// Return the only record of the index `view` with the name `name`.
    ///
    /// The function compares the names of the candidates of the name index
    /// only: it does not scan the records (#48).
    fn find(&self, view: View, name: &[u8]) -> Result<Option<SymbolHandle>> {
        let mut found = None;
        for symbol in self.name_index.candidates(view, name) {
            if self.record(symbol).and_then(SymbolRecord::name) == Some(name) {
                if found.is_some_and(|handle| handle != symbol) {
                    return Err(Error::new(ErrorKind::InvalidReference, None)
                        .with_context("the name identifies more than one symbol record"));
                }
                found = Some(symbol);
            }
        }
        Ok(found)
    }
}

/// Return the error for a handle without a record.
fn missing_record() -> Error {
    Error::new(ErrorKind::InvalidReference, None)
        .with_context("symbol handle has no record in the editor")
}

/// Push a value with a checked allocation.
fn push<T>(list: &mut Vec<T>, value: T) -> Result<()> {
    list.try_reserve(1).map_err(|_| no_memory())?;
    list.push(value);
    Ok(())
}

/// Return an error if `data` cannot be a record of the symbol record stream.
fn check_shared(data: &SymbolData<'_>) -> Result<()> {
    if matches!(data, SymbolData::Unsupported(_)) {
        return Err(Error::new(ErrorKind::UnsupportedEdit, None)
            .with_context("added symbol record has no decoder"));
    }
    if data.starts_scope() {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("a record that starts a scope belongs in a module stream"));
    }
    if data.name().is_some_and(|name| name.as_bytes().contains(&0)) {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("symbol name contains a NUL byte"));
    }
    Ok(())
}

macro_rules! shared_view {
    ($(#[$doc:meta])* $name:ident, $entries:ident, $view:ident, $what:literal) => {
        $(#[$doc])*
        #[derive(Debug)]
        pub struct $name<'e> {
            /// The model of the symbol record stream.
            pub(super) model: &'e mut SharedSymbols,
        }

        impl $name<'_> {
            #[doc = concat!("Return the handles of the records of the ", $what, ", in index order.")]
            pub fn handles(&self) -> impl Iterator<Item = SymbolHandle> + '_ {
                self.model.$entries.iter().map(|entry| entry.symbol)
            }

            #[doc = concat!("Return the number of entries of the ", $what, ".")]
            #[must_use]
            pub fn len(&self) -> usize {
                self.model.$entries.len()
            }

            #[doc = concat!("Return `true` if the ", $what, " has no entries.")]
            #[must_use]
            pub fn is_empty(&self) -> bool {
                self.model.$entries.is_empty()
            }

            #[doc = concat!("Return the record of the ", $what, " with the name `name`.")]
            ///
            /// The function looks in the hash bucket of the name in the source
            /// index and in the records with a changed name. It does not scan
            /// the records.
            ///
            /// # Errors
            ///
            /// Returns [`ErrorKind::InvalidReference`] if more than one record
            /// has the name.
            pub fn find(&self, name: &[u8]) -> Result<Option<SymbolHandle>> {
                self.model.find(View::$view, name)
            }

            /// Return the record of `handle`.
            #[must_use]
            pub fn get(&self, handle: SymbolHandle) -> Option<&SymbolRecord> {
                self.model.record(handle)
            }

            /// Return the record of `handle` for a change.
            ///
            /// A record with a new name goes into the hash bucket of its new
            /// name. The plan rejects a new name of a reference record such as
            /// `S_PROCREF` with [`ErrorKind::UnsupportedEdit`]: the module
            /// record holds the same name.
            pub fn get_mut(&mut self, handle: SymbolHandle) -> Option<&mut SymbolRecord> {
                self.model.lend(handle)
            }

            /// Remove the record of `handle` from the symbol record stream and
            /// from both indexes.
            ///
            /// The handles of the other records stay valid. The plan removes
            /// the global references of the modules to the record.
            ///
            /// # Errors
            ///
            /// Returns [`ErrorKind::InvalidReference`] if the editor has no
            /// record with `handle`.
            pub fn remove(&mut self, handle: SymbolHandle) -> Result<()> {
                self.model.remove(handle)
            }
        }
    };
}

shared_view!(
    /// The public symbols: the `S_PUB32` records of the public symbol index.
    ///
    /// [`PdbEditor::publics_mut`](super::PdbEditor::publics_mut) gives the
    /// view. The public and the global symbols share the symbol record
    /// stream.
    PublicSymbols,
    publics,
    Publics,
    "public symbol index"
);

shared_view!(
    /// The global symbols: the records of the global symbol index.
    ///
    /// [`PdbEditor::globals_mut`](super::PdbEditor::globals_mut) gives the
    /// view. The public and the global symbols share the symbol record
    /// stream.
    GlobalSymbols,
    globals,
    Globals,
    "global symbol index"
);

impl PublicSymbols<'_> {
    /// Add an `S_PUB32` record to the end of the symbol record stream and to
    /// the public index. Return its handle.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the name contains a NUL byte.
    /// - [`ErrorKind::LimitExceeded`] if the record count does not fit in 32
    ///   bits or an allocation fails.
    pub fn push(&mut self, public: Public<'_>) -> Result<SymbolHandle> {
        let data = SymbolData::Public(public);
        check_shared(&data)?;
        self.model.publics.try_reserve(1).map_err(|_| no_memory())?;
        let handle = self
            .model
            .add(SymbolRecord::new(S_PUB32, data), View::Publics)?;
        self.model.publics.push(IndexEntry {
            symbol: handle,
            reference_count: 1,
            bucket: None,
        });
        Ok(handle)
    }

    /// Set the incremental thunk table: the size of one thunk, the section
    /// and the offset of the table, and the flat target address of each
    /// thunk.
    pub fn set_thunk_table(
        &mut self,
        thunk_size: u32,
        section: u16,
        offset: u32,
        targets: Vec<u32>,
    ) {
        self.model.thunks = Thunks {
            thunk_size,
            section,
            offset,
            targets,
        };
    }

    /// Set the section start table.
    pub fn set_section_starts(&mut self, starts: Vec<SectionStart>) {
        self.model.section_starts = starts;
    }
}

impl GlobalSymbols<'_> {
    /// Add a record of `kind` with `data` to the end of the symbol record
    /// stream and to the global index with `reference_count`. Return its
    /// handle.
    ///
    /// The reference count is the number of modules that use the record.
    /// LLVM writes 1. A reference such as `S_PROCREF` can have a
    /// [`ModuleSymbolTarget::Handle`](crate::native::codeview::symbol::ModuleSymbolTarget::Handle);
    /// the plan writes the module index and the offset of that record.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedEdit`] for [`SymbolData::Unsupported`].
    /// - [`ErrorKind::InvalidFormat`] for a record that starts a scope or a
    ///   name with a NUL byte.
    /// - [`ErrorKind::LimitExceeded`] if the record count does not fit in 32
    ///   bits or an allocation fails.
    pub fn push(
        &mut self,
        kind: SymbolKind,
        data: SymbolData<'_>,
        reference_count: u32,
    ) -> Result<SymbolHandle> {
        check_shared(&data)?;
        self.model.globals.try_reserve(1).map_err(|_| no_memory())?;
        let handle = self
            .model
            .add(SymbolRecord::new(kind, data), View::Globals)?;
        self.model.globals.push(IndexEntry {
            symbol: handle,
            reference_count,
            bucket: None,
        });
        Ok(handle)
    }
}
