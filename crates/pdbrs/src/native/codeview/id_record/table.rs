//! Lookup, string resolution and reference validation for IPI records.

use alloc::vec::Vec;

use super::{IdRecord, LF_STRING_ID, LF_SUBSTR_LIST};
use crate::native::codeview::Record;
use crate::native::codeview::index::{IdIndex, TypeIndex};
use crate::native::types::{HEADER_SIZE, TypeStream, TypeTable};
use crate::{Error, ErrorKind, Result};

/// The fixed stream index of the IPI stream.
const IPI_STREAM: u32 = 4;

/// The maximum nesting depth of substring lists in one string.
///
/// A string at depth 0 can refer to a substring list. Each string in that
/// list is at depth 1, and so on. Microsoft documents only depth 1.
pub const MAX_STRING_DEPTH: usize = 8;

/// The framed records of an IPI stream, by ID index.
///
/// The table borrows the records from a [`TypeStream`] through a
/// [`TypeTable`]. It does not decode payloads in advance.
#[derive(Clone, Debug)]
pub struct IdTable<'a> {
    /// The record offsets in index order.
    table: TypeTable<'a>,
}

/// One step of string resolution.
#[derive(Clone, Copy)]
enum Pending<'a> {
    /// Resolve a string ID at a nesting depth.
    Id(IdIndex, usize),
    /// Append string bytes.
    Bytes(&'a [u8]),
}

impl<'a> IdTable<'a> {
    /// Frame all records of the IPI stream.
    ///
    /// Give the parsed IPI stream (stream 4), not the TPI stream.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`TypeTable::new`].
    pub fn new(stream: &'a TypeStream) -> Result<Self> {
        Ok(Self {
            table: TypeTable::new(stream)?,
        })
    }

    /// Return the index of the first record.
    #[must_use]
    pub const fn index_begin(&self) -> IdIndex {
        IdIndex(self.table.stream().header().index_begin)
    }

    /// Return the index after the last record.
    #[must_use]
    pub const fn index_end(&self) -> IdIndex {
        IdIndex(self.table.stream().header().index_end)
    }

    /// Return the number of records.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.table.len()
    }

    /// Return `true` if the stream has no records.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Return the framed record of `id`, or `None` if `id` is out of range.
    #[must_use]
    pub fn get(&self, id: IdIndex) -> Option<Record<'a>> {
        self.table.id_record(id).ok().map(|record| record.record)
    }

    /// Decode the record of `id`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if `id` is out of range.
    /// Returns the errors of [`IdRecord::decode`] with the IPI stream
    /// location of the record.
    pub fn decode(&self, id: IdIndex) -> Result<IdRecord<'a>> {
        let record = self.get(id).ok_or_else(|| {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("IPI index is outside the IPI stream")
        })?;
        decode_at(record)
    }

    /// Append the full string of an `LF_STRING_ID` record to `out`.
    ///
    /// The full string is the strings of its `LF_SUBSTR_LIST` record, in
    /// list order, and then its own string. The `LF_SUBSTR_LIST` example in
    /// the Microsoft `pdb-rs` documentation shows this order: the parts
    /// join without gaps only in this order. A string in a substring list
    /// can have a substring list too, up to [`MAX_STRING_DEPTH`].
    ///
    /// On error, `out` can contain a part of the string.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if an index is out of range, if a
    ///   string reference is not an `LF_STRING_ID` record, or if a substring
    ///   reference is not an `LF_SUBSTR_LIST` record.
    /// - [`ErrorKind::LimitExceeded`] if the nesting is deeper than
    ///   [`MAX_STRING_DEPTH`], if the resolution visits more records than the
    ///   table holds, or if `out` cannot grow.
    /// - The errors of [`IdTable::decode`].
    pub fn resolve_string(&self, id: IdIndex, out: &mut Vec<u8>) -> Result<()> {
        let mut stack = Vec::new();
        push(&mut stack, Pending::Id(id, 0))?;
        let mut visits = 0usize;
        while let Some(pending) = stack.pop() {
            let (id, depth) = match pending {
                Pending::Bytes(bytes) => {
                    out.try_reserve(bytes.len()).map_err(|_| {
                        Error::new(ErrorKind::LimitExceeded, None)
                            .with_context("resolved IPI string cannot be allocated")
                    })?;
                    out.extend_from_slice(bytes);
                    continue;
                }
                Pending::Id(id, depth) => (id, depth),
            };
            visits += 1;
            if visits > self.len() {
                return Err(Error::new(ErrorKind::LimitExceeded, None)
                    .with_context("IPI string resolution visits too many records"));
            }
            let IdRecord::String(string) = self.decode(id)? else {
                return Err(Error::new(ErrorKind::InvalidReference, None)
                    .with_context("IPI string reference is not an LF_STRING_ID record"));
            };
            push(&mut stack, Pending::Bytes(string.string))?;
            let Some(list_id) = string.substrings else {
                continue;
            };
            if depth >= MAX_STRING_DEPTH {
                return Err(Error::new(ErrorKind::LimitExceeded, None)
                    .with_context("IPI substring lists are nested too deeply"));
            }
            let IdRecord::SubstringList(list) = self.decode(list_id)? else {
                return Err(Error::new(ErrorKind::InvalidReference, None)
                    .with_context("IPI substring reference is not an LF_SUBSTR_LIST record"));
            };
            stack.try_reserve(list.len()).map_err(|_| stack_error())?;
            stack.extend(list.iter().rev().map(|part| Pending::Id(part, depth + 1)));
        }
        Ok(())
    }

    /// Decode all records and validate their references.
    ///
    /// `type_index_end` is the index after the last TPI record
    /// ([`TypeHeader::index_end`](crate::native::types::TypeHeader::index_end)
    /// of the TPI stream). These rules apply:
    ///
    /// - A type index is a known primitive type or is below `type_index_end`.
    /// - An ID index is in the range of this table.
    /// - A function scope, a UDT source file and a substring list entry
    ///   refer to `LF_STRING_ID` records.
    /// - A string substring reference refers to an `LF_SUBSTR_LIST` record.
    /// - A build argument is 0 or refers to an `LF_STRING_ID` or an
    ///   `LF_SUBSTR_LIST` record.
    ///
    /// The function does not check `/names` offsets.
    ///
    /// # Errors
    ///
    /// All errors have the IPI stream location of the record.
    ///
    /// - [`ErrorKind::InvalidReference`] if a reference breaks a rule.
    /// - [`ErrorKind::UnsupportedVersion`] if a record has a kind that
    ///   [`IdRecord::decode`] does not support.
    /// - The errors of [`IdRecord::decode`].
    pub fn validate(&self, type_index_end: TypeIndex) -> Result<()> {
        for record in self.table.stream().records() {
            let record = record?.record;
            self.validate_record(record, type_index_end)
                .map_err(|error| error.with_stream(IPI_STREAM, record_offset(record)))?;
        }
        Ok(())
    }

    /// Validate the references of one record.
    fn validate_record(&self, record: Record<'a>, type_index_end: TypeIndex) -> Result<()> {
        let check_type = |index| check_type(index, type_index_end);
        match IdRecord::decode(record)? {
            IdRecord::Function(function) => {
                if let Some(scope) = function.scope {
                    self.check_kind(scope, &[LF_STRING_ID])?;
                }
                check_type(function.function_type)
            }
            IdRecord::MemberFunction(function) => {
                check_type(function.parent_type)?;
                check_type(function.function_type)
            }
            IdRecord::BuildInfo(info) => info
                .arguments
                .iter()
                .filter(|&argument| argument != IdIndex::NONE)
                .try_for_each(|argument| {
                    self.check_kind(argument, &[LF_STRING_ID, LF_SUBSTR_LIST])
                }),
            IdRecord::SubstringList(list) => list
                .iter()
                .try_for_each(|part| self.check_kind(part, &[LF_STRING_ID])),
            IdRecord::String(string) => string
                .substrings
                .map_or(Ok(()), |list| self.check_kind(list, &[LF_SUBSTR_LIST])),
            IdRecord::UdtSourceLine(line) => {
                check_type(line.udt)?;
                self.check_kind(line.source_file, &[LF_STRING_ID])
            }
            IdRecord::UdtModuleSourceLine(line) => check_type(line.udt),
            IdRecord::Unsupported(_) => Err(Error::new(ErrorKind::UnsupportedVersion, None)
                .with_context("IPI record kind is not supported")),
        }
    }

    /// Make sure that `id` refers to a record of one of `kinds`.
    fn check_kind(&self, id: IdIndex, kinds: &[u16]) -> Result<()> {
        let record = self.get(id).ok_or_else(|| {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("IPI index is outside the IPI stream")
        })?;
        if kinds.contains(&record.kind) {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("IPI index refers to a record of the wrong kind"))
        }
    }
}

/// Make sure that a type index is a known primitive or a TPI record index.
fn check_type(index: TypeIndex, type_index_end: TypeIndex) -> Result<()> {
    let valid = if index.is_primitive() {
        index.primitive().is_ok()
    } else {
        index < type_index_end
    };
    if valid {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::InvalidReference, None)
            .with_context("IPI record refers to a type index outside the TPI stream"))
    }
}

/// Decode a record and add its IPI stream location to an error.
fn decode_at(record: Record<'_>) -> Result<IdRecord<'_>> {
    IdRecord::decode(record).map_err(|error| error.with_stream(IPI_STREAM, record_offset(record)))
}

/// Return the IPI stream offset of a record.
fn record_offset(record: Record<'_>) -> u64 {
    // A `usize` value always fits in `u64` on supported targets.
    logical(HEADER_SIZE).saturating_add(u64::from(record.offset))
}

/// Convert a logical offset to the error representation.
fn logical(offset: usize) -> u64 {
    u64::try_from(offset).unwrap_or(u64::MAX)
}

/// Push one step with fallible allocation.
fn push<'a>(stack: &mut Vec<Pending<'a>>, pending: Pending<'a>) -> Result<()> {
    stack.try_reserve(1).map_err(|_| stack_error())?;
    stack.push(pending);
    Ok(())
}

/// Return the error for a resolution stack that cannot grow.
fn stack_error() -> Error {
    Error::new(ErrorKind::LimitExceeded, None)
        .with_context("IPI string resolution stack cannot be allocated")
}
