//! Random access to the records of a TPI or IPI stream.
//!
//! [`TypeTable`] frames all records once and keeps the offset of each record
//! in one `Vec<u32>`. This costs 4 bytes for each record and one pass over
//! the record bytes. After that, each lookup is one bounds check and the
//! framing of one record. The index offset buffer of the hash stream needs
//! no allocation, but each lookup must then frame up to about 8 KiB of
//! records ([`super::TypeHashes::locate`]).

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::{TypeRecord, TypeStream};
use crate::native::codeview::LeafKind;
use crate::native::codeview::field_list::{FieldList, Member, Members};
use crate::native::codeview::index::{IdIndex, TypeIndex};
use crate::native::codeview::type_record::{LF_FIELDLIST, TypeLeaf};
use crate::{Error, ErrorKind, Result};

/// Return an invalid-reference error with `context`.
fn invalid_reference(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidReference, None).with_context(context)
}

/// An index from record indices to record offsets for one type stream.
#[derive(Clone, Debug)]
pub struct TypeTable<'a> {
    /// The stream.
    stream: &'a TypeStream,
    /// The offset of each record in the record bytes, in index order.
    offsets: Vec<u32>,
}

impl<'a> TypeTable<'a> {
    /// Frame all records of `stream` and keep their offsets.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`] if the offset table cannot be allocated.
    /// - The framing and record count errors of [`TypeStream::records`].
    pub fn new(stream: &'a TypeStream) -> Result<Self> {
        let count = usize::try_from(stream.header().record_count()).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("type record count does not fit in memory")
        })?;
        let mut offsets = Vec::new();
        offsets.try_reserve_exact(count).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("type offset table cannot be allocated")
        })?;
        for record in stream.records() {
            offsets.push(record?.record.offset);
        }
        Ok(Self { stream, offsets })
    }

    /// Return the stream.
    #[must_use]
    pub const fn stream(&self) -> &'a TypeStream {
        self.stream
    }

    /// Return the number of records.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.offsets.len()
    }

    /// Return `true` if the stream has no records.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Return the record with a raw record index.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if no record has the index.
    pub fn record_at_index(&self, index: u32) -> Result<TypeRecord<'a>> {
        let position = index
            .checked_sub(self.stream.header().index_begin)
            .and_then(|position| usize::try_from(position).ok())
            .and_then(|position| self.offsets.get(position));
        let Some(&offset) = position else {
            return Err(invalid_reference("record index is outside the type stream"));
        };
        self.stream.record_at(offset, index)
    }

    /// Return the TPI record of a type index.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] for a primitive type index or
    /// an index after the last record.
    pub fn record(&self, index: TypeIndex) -> Result<TypeRecord<'a>> {
        if index.is_primitive() {
            return Err(invalid_reference("primitive type index has no record"));
        }
        self.record_at_index(index.0)
    }

    /// Return the IPI record of an ID index.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if no record has the index.
    pub fn id_record(&self, index: IdIndex) -> Result<TypeRecord<'a>> {
        self.record_at_index(index.0)
    }

    /// Decode the TPI record of a type index.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`TypeTable::record`] and [`TypeLeaf::decode`].
    pub fn leaf(&self, index: TypeIndex) -> Result<TypeLeaf<'a>> {
        TypeLeaf::decode(self.record(index)?.record)
    }

    /// Decode the `LF_FIELDLIST` record of a type index.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if the record is not a field
    /// list, and the errors of [`TypeTable::leaf`].
    pub fn field_list(&self, index: TypeIndex) -> Result<FieldList<'a>> {
        match self.leaf(index)? {
            TypeLeaf::FieldList(list) => Ok(list),
            _ => Err(invalid_reference(
                "type index does not refer to a field list",
            )),
        }
    }

    /// Return an iterator over the members of a field list and its continuations.
    ///
    /// The iterator follows each `LF_INDEX` member to the next `LF_FIELDLIST`
    /// record. It does not return the `LF_INDEX` members.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`TypeTable::field_list`] for the first record.
    /// The iterator returns the errors for the continuation records, and an
    /// [`ErrorKind::InvalidFormat`] error for a continuation cycle.
    pub fn members(&self, index: TypeIndex) -> Result<FieldMembers<'_, 'a>> {
        Ok(FieldMembers {
            table: self,
            members: self.field_members(index)?,
            tortoise: index,
            power: 1,
            steps: 1,
            done: false,
        })
    }

    /// Return an iterator over the members of the `LF_FIELDLIST` record of a
    /// type index. The iterator borrows the stream bytes.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`TypeTable::field_list`].
    fn field_members(&self, index: TypeIndex) -> Result<Members<'a>> {
        let record = self.record(index)?.record;
        if LeafKind(record.kind) != LF_FIELDLIST {
            return Err(invalid_reference(
                "type index does not refer to a field list",
            ));
        }
        FieldList::parse(record.data)?;
        Ok(Members::new(record.data))
    }
}

/// An iterator over the members of a field list and its continuation records.
///
/// The iterator detects a continuation cycle with Brent's algorithm. It does
/// not allocate, and it detects a cycle in a number of steps that is linear in
/// the number of records in the chain.
#[derive(Clone, Debug)]
pub struct FieldMembers<'t, 'a> {
    /// The table that holds the records.
    table: &'t TypeTable<'a>,
    /// The members of the current record.
    members: Members<'a>,
    /// The saved record index for cycle detection.
    tortoise: TypeIndex,
    /// The current power of two for cycle detection.
    power: u32,
    /// The number of steps since the last save of `tortoise`.
    steps: u32,
    /// `true` after the end or after an error.
    done: bool,
}

impl<'a> FieldMembers<'_, 'a> {
    /// Move to the continuation record `next`.
    fn follow(&mut self, next: TypeIndex) -> Result<()> {
        if next == self.tortoise {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("field list continuations form a cycle"));
        }
        if self.power == self.steps {
            self.tortoise = next;
            self.power = self.power.saturating_mul(2);
            self.steps = 0;
        }
        self.steps = self.steps.saturating_add(1);
        self.members = self.table.field_members(next)?;
        Ok(())
    }

    /// Return the next member that is not a continuation.
    fn read(&mut self) -> Option<Result<Member<'a>>> {
        loop {
            match self.members.next()? {
                Ok(Member::Continuation(next)) => {
                    if let Err(error) = self.follow(next) {
                        return Some(Err(error));
                    }
                }
                other => return Some(other),
            }
        }
    }
}

impl<'a> Iterator for FieldMembers<'_, 'a> {
    type Item = Result<Member<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let result = self.read();
        self.done = !matches!(result, Some(Ok(_)));
        result
    }
}

impl FusedIterator for FieldMembers<'_, '_> {}
