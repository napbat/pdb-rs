//! The TPI and IPI records of the editor model.

use alloc::vec::Vec;

use crate::native::codeview::Record;
use crate::native::codeview::id_record::IdRecord;
use crate::native::codeview::index::{FIRST_RECORD_INDEX, IdIndex, NameOffset, TypeIndex};
use crate::native::codeview::type_record::TypeLeaf;
use crate::native::output::no_memory;
use crate::native::types::{HashAdjuster, TypeStream};
use crate::{Error, ErrorKind, Result};

/// A record type of a TPI or IPI stream.
pub(super) trait ListRecord: Clone + PartialEq + Sized {
    /// Decode a source record into a value that owns its bytes.
    fn decode_record(record: Record<'_>) -> Result<Self>;
    /// Append the encoded record to `out`.
    fn encode_record(&self, out: &mut Vec<u8>) -> Result<()>;
    /// Return `true` for a record of a kind without a decoder.
    fn unsupported(&self) -> bool;
}

impl ListRecord for TypeLeaf<'static> {
    fn decode_record(record: Record<'_>) -> Result<Self> {
        TypeLeaf::decode(record).map(TypeLeaf::into_owned)
    }

    fn encode_record(&self, out: &mut Vec<u8>) -> Result<()> {
        self.encode(out)
    }

    fn unsupported(&self) -> bool {
        matches!(self, TypeLeaf::Unsupported(_))
    }
}

impl ListRecord for IdRecord<'static> {
    fn decode_record(record: Record<'_>) -> Result<Self> {
        IdRecord::decode(record).map(IdRecord::into_owned)
    }

    fn encode_record(&self, out: &mut Vec<u8>) -> Result<()> {
        self.encode(out)
    }

    fn unsupported(&self) -> bool {
        matches!(self, IdRecord::Unsupported(_))
    }
}

/// One record of a list.
#[derive(Clone, Debug)]
pub(super) struct Entry<T> {
    /// The record.
    pub(super) value: T,
    /// The offset of the source record in the record bytes of the source
    /// stream, or `None` for a new record.
    pub(super) source: Option<u32>,
    /// `true` if the caller had mutable access to the record.
    pub(super) touched: bool,
    /// `true` if the caller removed the record. The model keeps the value,
    /// so that the plan can name it.
    pub(super) removed: bool,
}

/// The records of a TPI or IPI stream in index order.
#[derive(Clone, Debug)]
pub(super) struct RecordList<T> {
    /// The index of the first record.
    pub(super) begin: u32,
    /// The records.
    pub(super) entries: Vec<Entry<T>>,
    /// The hash adjusters.
    pub(super) adjusters: Vec<HashAdjuster>,
    /// `true` if the caller added a hash adjuster.
    pub(super) adjusters_changed: bool,
    /// The source stream, or `None` for a list without a source.
    pub(super) source: Option<TypeStream>,
}

impl<T: ListRecord> RecordList<T> {
    /// Create an empty list.
    pub(super) const fn new() -> Self {
        Self {
            begin: FIRST_RECORD_INDEX,
            entries: Vec::new(),
            adjusters: Vec::new(),
            adjusters_changed: false,
            source: None,
        }
    }

    /// Decode every record of a source stream.
    pub(super) fn decode(stream: TypeStream, adjusters: Vec<HashAdjuster>) -> Result<Self> {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(stream.header().record_count() as usize)
            .map_err(|_| no_memory())?;
        for record in stream.records() {
            let record = record?.record;
            entries.push(Entry {
                value: T::decode_record(record)?,
                source: Some(record.offset),
                touched: false,
                removed: false,
            });
        }
        Ok(Self {
            begin: stream.header().index_begin,
            entries,
            adjusters,
            adjusters_changed: false,
            source: Some(stream),
        })
    }

    /// Return the index after the last record.
    pub(super) fn end(&self) -> Result<u32> {
        u32::try_from(self.entries.len())
            .ok()
            .and_then(|count| self.begin.checked_add(count))
            .ok_or_else(|| {
                Error::new(ErrorKind::LimitExceeded, None)
                    .with_context("record index does not fit in 32 bits")
            })
    }

    /// Return the position of the record with `index`, removed records
    /// included.
    fn position(&self, index: u32) -> Option<usize> {
        index
            .checked_sub(self.begin)
            .and_then(|position| usize::try_from(position).ok())
            .filter(|&position| position < self.entries.len())
    }

    /// Return the position of the record with `index` if it is not removed.
    fn live_position(&self, index: u32) -> Option<usize> {
        self.position(index)
            .filter(|&position| !self.entries[position].removed)
    }

    /// Return `true` if a record of the list is removed.
    pub(super) fn has_removals(&self) -> bool {
        self.entries.iter().any(|entry| entry.removed)
    }

    /// Return the number of records that are not removed.
    pub(super) fn live_count(&self) -> usize {
        self.entries.iter().filter(|entry| !entry.removed).count()
    }

    /// Remove the record with `index`: the record becomes a tombstone.
    pub(super) fn remove(&mut self, index: u32) -> Result<()> {
        let position = self.live_position(index).ok_or_else(|| {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("the list has no record with this index, or it is removed")
        })?;
        self.entries[position].removed = true;
        Ok(())
    }

    /// Append a record and return its index.
    pub(super) fn push(&mut self, value: T) -> Result<u32> {
        if value.unsupported() {
            return Err(Error::new(ErrorKind::UnsupportedEdit, None)
                .with_context("added record has no decoder"));
        }
        let index = self.end()?;
        index.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("record index does not fit in 32 bits")
        })?;
        self.entries.try_reserve(1).map_err(|_| no_memory())?;
        self.entries.push(Entry {
            value,
            source: None,
            touched: false,
            removed: false,
        });
        Ok(index)
    }

    /// Return the record with `index`, or `None` if it is removed.
    pub(super) fn get(&self, index: u32) -> Option<&T> {
        self.live_position(index)
            .map(|position| &self.entries[position].value)
    }

    /// Return the record with `index` for a change, or `None` if it is
    /// removed.
    pub(super) fn get_mut(&mut self, index: u32) -> Option<&mut T> {
        let position = self.live_position(index)?;
        let entry = &mut self.entries[position];
        entry.touched = true;
        Some(&mut entry.value)
    }

    /// Return the record with `index`, also a removed record.
    pub(super) fn model_value(&self, index: u32) -> Option<&T> {
        self.position(index)
            .map(|position| &self.entries[position].value)
    }

    /// Add a hash adjuster for the record with `index`.
    pub(super) fn add_adjuster(&mut self, index: u32, name_offset: NameOffset) -> Result<()> {
        if self.live_position(index).is_none() {
            return Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("hash adjuster index has no record or names a removed record"));
        }
        self.adjusters.try_reserve(1).map_err(|_| no_memory())?;
        self.adjusters.push(HashAdjuster { name_offset, index });
        self.adjusters_changed = true;
        Ok(())
    }

    /// Return the source record of `entry`, decoded again, or `None` for a
    /// new record.
    fn source_value(&self, entry: &Entry<T>) -> Result<Option<T>> {
        let (Some(offset), Some(stream)) = (entry.source, &self.source) else {
            return Ok(None);
        };
        let record = stream.record_at(offset, 0)?.record;
        T::decode_record(record).map(Some)
    }

    /// Return `true` if `entry` is a source record with its source value.
    pub(super) fn unchanged(&self, entry: &Entry<T>) -> Result<bool> {
        if entry.source.is_none() {
            return Ok(false);
        }
        if !entry.touched {
            return Ok(true);
        }
        Ok(self.source_value(entry)?.as_ref() == Some(&entry.value))
    }

    /// Return the complete bytes of the source record of `entry`.
    pub(super) fn source_bytes(&self, entry: &Entry<T>) -> Result<Option<&[u8]>> {
        let (Some(offset), Some(stream)) = (entry.source, &self.source) else {
            return Ok(None);
        };
        let record = stream.record_at(offset, 0)?.record;
        stream.full_record(&record).map(Some)
    }

    /// Forget the source: the plan encodes every record.
    pub(super) fn forget_source(&mut self) {
        self.source = None;
        for entry in &mut self.entries {
            entry.source = None;
        }
    }
}

macro_rules! record_list {
    ($(#[$doc:meta])* $name:ident, $record:ident, $index:ident, $what:literal) => {
        $(#[$doc])*
        #[derive(Clone, Debug)]
        pub struct $name {
            /// The records.
            pub(super) list: RecordList<$record<'static>>,
        }

        impl $name {
            /// Make a list from its records.
            pub(super) const fn from_list(list: RecordList<$record<'static>>) -> Self {
                Self { list }
            }

            #[doc = concat!("Append ", $what, " and return its index in the model.")]
            ///
            /// The index is the next index after the last record, removed
            /// records included: in the model, the index space only grows.
            /// While the list has no removed record, the
            /// index is also the index in the output. After a removal, the
            /// index is a handle: other records refer to the new record with
            /// it, and the plan gives the output index (see
            /// [`PdbWritePlan::type_index`](crate::native::PdbWritePlan::type_index)
            /// and [`PdbWritePlan::id_index`](crate::native::PdbWritePlan::id_index)).
            ///
            /// The record can refer only to earlier records: the plan rejects
            /// a forward reference with [`ErrorKind::UnsupportedEdit`].
            ///
            /// # Errors
            ///
            /// - [`ErrorKind::UnsupportedEdit`] for a record without a decoder
            ///   (`Unsupported`).
            /// - [`ErrorKind::LimitExceeded`] if the index does not fit in 32
            ///   bits or an allocation fails.
            pub fn push(&mut self, record: $record<'_>) -> Result<$index> {
                self.list.push(record.into_owned()).map($index)
            }

            #[doc = concat!("Remove ", $what, " with `index`.")]
            ///
            /// Caution: the plan cannot see references in other files. An
            /// object file that uses a type-server PDB, a precompiled-header
            /// object (`/Yc`) and the object files of a `/DEBUG:FASTLINK` PDB
            /// keep the old indices. Remove records only from a PDB that no
            /// other file refers to by index.
            ///
            /// The record becomes a tombstone: the other records keep their
            /// indices in the model, and [`Self::get`] gives `None` for it.
            /// The plan gives each record that is not removed the next output
            /// index, in index order, and writes the output index into each
            /// reference of each stream that it decodes. A reference to a
            /// removed record fails the plan with
            /// [`ErrorKind::InvalidReference`]. This function does not look
            /// for references, because other streams hold them and later
            /// changes can add or remove them. See
            /// [`PdbEditor`](crate::native::PdbEditor#removal-of-type-and-id-records)
            /// for the streams and the PDBs that the plan refuses.
            ///
            /// # Errors
            ///
            /// Returns [`ErrorKind::InvalidReference`] if the list has no
            /// record with `index`, or if the record is already removed.
            pub fn remove(&mut self, index: $index) -> Result<()> {
                self.list.remove(index.0)
            }

            #[doc = concat!("Return ", $what, " with `index`, or `None` if it is removed.")]
            #[must_use]
            pub fn get(&self, index: $index) -> Option<&$record<'static>> {
                self.list.get(index.0)
            }

            #[doc = concat!("Return ", $what, " with `index` for a change, or `None` if it is removed.")]
            ///
            /// The plan encodes a changed source record and writes the stream
            /// and its hash stream again.
            pub fn get_mut(&mut self, index: $index) -> Option<&mut $record<'static>> {
                self.list.get_mut(index.0)
            }

            /// Return the records that are not removed, with their indices in
            /// the model, in index order.
            pub fn iter(&self) -> impl Iterator<Item = ($index, &$record<'static>)> + '_ {
                (self.list.begin..)
                    .zip(&self.list.entries)
                    .filter(|(_, entry)| !entry.removed)
                    .map(|(index, entry)| ($index(index), &entry.value))
            }

            /// Return the number of records that are not removed.
            #[must_use]
            pub fn len(&self) -> usize {
                self.list.live_count()
            }

            /// Return `true` if the list has no records that are not removed.
            #[must_use]
            pub fn is_empty(&self) -> bool {
                self.len() == 0
            }

            /// Return the index after the last record of the model, removed
            /// records included. The next [`Self::push`] gives this index.
            ///
            /// # Errors
            ///
            /// Returns [`ErrorKind::LimitExceeded`] if the index does not fit
            /// in 32 bits.
            pub fn end(&self) -> Result<$index> {
                self.list.end().map($index)
            }

            /// Add a hash adjuster to the hash stream: the record `index` is at
            /// the head of the hash chain of the name at `name_offset` in
            /// `/names`.
            ///
            /// # Errors
            ///
            /// - [`ErrorKind::InvalidReference`] if the list has no record
            ///   with `index`, or if the record is removed.
            /// - [`ErrorKind::LimitExceeded`] if an allocation fails.
            pub fn add_hash_adjuster(&mut self, index: $index, name_offset: NameOffset) -> Result<()> {
                self.list.add_adjuster(index.0, name_offset)
            }
        }
    };
}

record_list!(
    /// The type records of the TPI stream, in index order.
    ///
    /// [`PdbEditor::types_mut`](super::PdbEditor::types_mut) gives the list.
    TypeList,
    TypeLeaf,
    TypeIndex,
    "a type record"
);

record_list!(
    /// The ID records of the IPI stream, in index order.
    ///
    /// [`PdbEditor::ids_mut`](super::PdbEditor::ids_mut) gives the list.
    IdList,
    IdRecord,
    IdIndex,
    "an ID record"
);
