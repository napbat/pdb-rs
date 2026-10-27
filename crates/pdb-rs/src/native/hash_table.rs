//! The serialized PDB hash table.
//!
//! The PDB Info named-stream map and the `/src/headerblock` stream store a hash table with
//! `u32` keys and fixed-size values. The layout follows the LLVM document `HashTable.md`, LLVM
//! `HashTable.h` and Microsoft `PDB/include/map.h` (`Map::reload`):
//!
//! 1. `size: u32`, the number of present buckets.
//! 2. `capacity: u32`, the number of buckets.
//! 3. The present bit vector: a word count and that many `u32` words.
//! 4. The deleted bit vector, in the same format.
//! 5. One key and one value for each present bucket, in bucket order.
//!
//! # Validation
//!
//! [`HashTable::parse`] checks these rules:
//!
//! - The capacity is not zero, and the size is at most `capacity * 2 / 3 + 1` (LLVM
//!   `HashTable::load`).
//! - A bit vector has at most `ceil(capacity / 32)` words and no bit at or after `capacity`.
//! - The number of present bits equals the size. No bucket is both present and deleted.
//! - Each value decodes with [`HashTableValue::decode`].
//!
//! [`HashTable::check_placement`] checks that a lookup finds each entry. Producers insert with
//! linear probing from `hash % capacity` (`Map::find` in `map.h`). A lookup stops at the first
//! bucket that is neither present nor deleted.

mod owned;

pub use owned::{HashTableKeys, OwnedHashTable, OwnedHashTableEntries};

use alloc::vec::Vec;

use super::msf::StreamId;
use super::output::{Output, to_u32};
use crate::{Error, ErrorKind, Result};

/// The size of a key or a bit vector word in bytes.
const WORD: usize = 4;

/// A fixed-size value in a serialized hash table.
pub trait HashTableValue<'a>: Sized {
    /// The size of one serialized value in bytes.
    const SIZE: usize;

    /// Decode one value from exactly [`Self::SIZE`] bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes do not hold a valid value. The table adds the stream ID
    /// and the logical offset of the value.
    fn decode(bytes: &'a [u8]) -> Result<Self>;

    /// Append exactly [`Self::SIZE`] bytes for the value to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow, or an error if a field
    /// does not fit its serialized width.
    fn encode(&self, out: &mut Vec<u8>) -> Result<()>;
}

impl HashTableValue<'_> for u32 {
    const SIZE: usize = 4;

    fn decode(bytes: &[u8]) -> Result<Self> {
        let word = <[u8; 4]>::try_from(bytes).map_err(|_| {
            Error::new(ErrorKind::InvalidFormat, None).with_context("hash table value size")
        })?;
        Ok(Self::from_le_bytes(word))
    }

    fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        Output::new(out).u32(*self)
    }
}

/// One present bucket of a hash table.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct HashTableEntry<V> {
    /// The bucket index, less than the capacity.
    pub bucket: u32,
    /// The serialized key.
    pub key: u32,
    /// The decoded value.
    pub value: V,
}

/// A decoded serialized hash table.
///
/// The entries are in bucket order. The bit vectors stay borrowed from the stream bytes.
#[derive(Debug)]
pub struct HashTable<'a, V> {
    capacity: u32,
    present: &'a [u8],
    deleted: &'a [u8],
    entries: Vec<HashTableEntry<V>>,
    stream: StreamId,
    entries_offset: usize,
    entry_size: usize,
}

impl<'a, V: HashTableValue<'a>> HashTable<'a, V> {
    /// Decode the hash table that starts at `offset` in `bytes`.
    ///
    /// `bytes` holds the full stream `stream`. Errors have this stream ID and a logical offset
    /// in the stream. They have no physical offset.
    ///
    /// Returns the table and the offset of the first byte after the table.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] if the table ends before its declared contents.
    /// - [`ErrorKind::InvalidFormat`] if a rule in the module documentation fails.
    /// - [`ErrorKind::LimitExceeded`] if the entry allocation fails.
    /// - An error from [`HashTableValue::decode`].
    pub fn parse(bytes: &'a [u8], offset: usize, stream: StreamId) -> Result<(Self, usize)> {
        let mut cursor = Cursor::new(bytes, stream, "PDB hash table");
        cursor.skip(offset)?;
        let table = Self::read(&mut cursor)?;
        Ok((table, cursor.position()))
    }

    /// Decode a hash table at the position of `cursor`.
    pub(crate) fn read(cursor: &mut Cursor<'a>) -> Result<Self> {
        let header = cursor.position();
        let size = cursor.u32()?;
        let capacity = cursor.u32()?;
        if capacity == 0 {
            return Err(cursor.fail(
                ErrorKind::InvalidFormat,
                header + WORD,
                "hash table capacity",
            ));
        }
        if u64::from(size) > u64::from(capacity) * 2 / 3 + 1 {
            return Err(cursor.fail(ErrorKind::InvalidFormat, header, "hash table load factor"));
        }
        let present = read_bit_vector(cursor, capacity)?;
        let deleted_at = cursor.position();
        let deleted = read_bit_vector(cursor, capacity)?;
        let mut population = 0_u64;
        for (index, word) in words(present).enumerate() {
            population += u64::from(word.count_ones());
            if word & word_at(deleted, index) != 0 {
                return Err(cursor.fail(
                    ErrorKind::InvalidFormat,
                    deleted_at,
                    "hash table bucket is both present and deleted",
                ));
            }
        }
        if population != u64::from(size) {
            return Err(cursor.fail(
                ErrorKind::InvalidFormat,
                header,
                "hash table size differs from the present buckets",
            ));
        }
        let entries_offset = cursor.position();
        let count = usize::try_from(size)
            .map_err(|_| cursor.fail(ErrorKind::LimitExceeded, header, "hash table size"))?;
        let entry_size = WORD + V::SIZE;
        if count > cursor.remaining() / entry_size {
            return Err(cursor.fail(
                ErrorKind::UnexpectedEof,
                entries_offset,
                "hash table entries extend past the stream",
            ));
        }
        let mut entries = Vec::new();
        entries.try_reserve_exact(count).map_err(|_| {
            cursor.fail(
                ErrorKind::LimitExceeded,
                entries_offset,
                "hash table allocation failed",
            )
        })?;
        for bucket in set_bits(present) {
            let key = cursor.u32()?;
            let at = cursor.position();
            let value = V::decode(cursor.take(V::SIZE)?).map_err(|error| {
                error.with_stream(StreamId(stream_number(cursor.stream())), offset(at))
            })?;
            entries.push(HashTableEntry { bucket, key, value });
        }
        Ok(Self {
            capacity,
            present,
            deleted,
            entries,
            stream: cursor.stream(),
            entries_offset,
            entry_size,
        })
    }

    /// Append the serialized table to `out`.
    ///
    /// The output keeps the capacity, the bit vector words and the bucket of each entry, so a
    /// decoded table gives the same bytes again.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if a count does not fit in `u32` or the output
    /// cannot grow, or an error from [`HashTableValue::encode`].
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut output = Output::new(out);
        output.u32(to_u32(self.entries.len(), "hash table size")?)?;
        output.u32(self.capacity)?;
        for words in [self.present, self.deleted] {
            output.u32(to_u32(words.len() / WORD, "hash table bit vector")?)?;
            output.bytes(words)?;
        }
        for entry in &self.entries {
            Output::new(out).u32(entry.key)?;
            entry.value.encode(out)?;
        }
        Ok(())
    }
}

impl<V> HashTable<'_, V> {
    /// Return the number of buckets.
    #[must_use]
    pub const fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Return the number of present entries.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return `true` if the table has no present entries.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return the present entries in bucket order.
    #[must_use]
    pub fn entries(&self) -> &[HashTableEntry<V>] {
        &self.entries
    }

    /// Return `true` if the deleted bit vector marks `bucket`.
    #[must_use]
    pub fn is_deleted(&self, bucket: u32) -> bool {
        bit(self.deleted, bucket)
    }

    /// Return the number of deleted buckets.
    #[must_use]
    pub fn deleted_count(&self) -> u32 {
        words(self.deleted).map(u32::count_ones).sum()
    }

    /// Make sure that a linear-probing lookup finds each entry.
    ///
    /// `hash` returns the hash of an entry key, before the reduction modulo the capacity.
    /// Each bucket from `hash % capacity` up to the bucket of the entry must be present or
    /// deleted. The walk wraps at the capacity. The cost of each check is bounded by the
    /// number of bit vector words, not by the capacity.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if a lookup cannot find an entry. The logical offset is
    ///   the offset of the entry key.
    /// - An error from `hash`.
    pub fn check_placement(
        &self,
        mut hash: impl FnMut(&HashTableEntry<V>) -> Result<u32>,
    ) -> Result<()> {
        for (index, entry) in self.entries.iter().enumerate() {
            let home = hash(entry)? % self.capacity;
            let reachable = if home <= entry.bucket {
                self.occupied(home, entry.bucket)
            } else {
                self.occupied(home, self.capacity) && self.occupied(0, entry.bucket)
            };
            if !reachable {
                return Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("hash table entry is not in its probe sequence")
                    .with_stream(
                        StreamId(stream_number(self.stream)),
                        offset(self.key_offset(index)),
                    ));
            }
        }
        Ok(())
    }

    /// Return the logical offset of the key of entry `index`.
    pub(crate) fn key_offset(&self, index: usize) -> usize {
        index
            .checked_mul(self.entry_size)
            .and_then(|delta| delta.checked_add(self.entries_offset))
            .unwrap_or(usize::MAX)
    }

    /// Return `true` if each bucket in `start..end` is present or deleted.
    fn occupied(&self, start: u32, end: u32) -> bool {
        let mut bucket = start;
        while bucket < end {
            let index = usize::try_from(bucket / 32).unwrap_or(usize::MAX);
            let word = word_at(self.present, index) | word_at(self.deleted, index);
            let low = bucket % 32;
            let high = (end - (bucket - low)).min(32);
            let upper = if high == 32 {
                u32::MAX
            } else {
                (1_u32 << high) - 1
            };
            let mask = upper & !((1_u32 << low) - 1);
            if word & mask != mask {
                return false;
            }
            bucket = bucket - low + high;
        }
        true
    }
}

/// Read a bit vector and check it against `capacity`.
fn read_bit_vector<'a>(cursor: &mut Cursor<'a>, capacity: u32) -> Result<&'a [u8]> {
    let at = cursor.position();
    let count = cursor.u32()?;
    let limit = capacity.div_ceil(32);
    if count > limit {
        return Err(cursor.fail(
            ErrorKind::InvalidFormat,
            at,
            "hash table bit vector is too long",
        ));
    }
    let length = usize::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(WORD))
        .ok_or_else(|| cursor.fail(ErrorKind::LimitExceeded, at, "hash table bit vector"))?;
    let bytes = cursor.take(length)?;
    if count == limit && !capacity.is_multiple_of(32) {
        let last = words(bytes).last().unwrap_or(0);
        if last >> (capacity % 32) != 0 {
            return Err(cursor.fail(
                ErrorKind::InvalidFormat,
                at,
                "hash table bit vector marks a bucket after the capacity",
            ));
        }
    }
    Ok(bytes)
}

/// Iterate the little-endian words of a bit vector.
fn words(bytes: &[u8]) -> impl Iterator<Item = u32> + '_ {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_le_bytes(*word))
}

/// Return word `index` of a bit vector. Words after the stored words are zero.
fn word_at(bytes: &[u8], index: usize) -> u32 {
    bytes
        .as_chunks::<4>()
        .0
        .get(index)
        .map_or(0, |word| u32::from_le_bytes(*word))
}

/// Return bit `index` of a bit vector.
fn bit(bytes: &[u8], index: u32) -> bool {
    let word = usize::try_from(index / 32).map_or(0, |word| word_at(bytes, word));
    word >> (index % 32) & 1 != 0
}

/// Iterate the indices of the set bits of a bit vector in increasing order.
fn set_bits(bytes: &[u8]) -> impl Iterator<Item = u32> + '_ {
    (0_u32..).zip(words(bytes)).flat_map(|(index, word)| {
        (0..32)
            .filter(move |bit| word >> bit & 1 != 0)
            .map(move |bit| index * 32 + bit)
    })
}

/// Return the raw stream number for an error location.
const fn stream_number(stream: StreamId) -> u32 {
    stream.0
}

/// Convert a logical offset to the error representation.
fn offset(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// A reader of little-endian fields in a stream payload.
///
/// Errors have the stream ID and the logical offset in the stream, and no physical offset.
pub(crate) struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
    stream: StreamId,
    context: &'static str,
}

impl<'a> Cursor<'a> {
    /// Create a reader at offset 0 of the stream bytes `bytes`.
    ///
    /// `context` describes the structure in errors.
    pub(crate) const fn new(bytes: &'a [u8], stream: StreamId, context: &'static str) -> Self {
        Self {
            bytes,
            position: 0,
            stream,
            context,
        }
    }

    /// Return the stream ID.
    pub(crate) const fn stream(&self) -> StreamId {
        self.stream
    }

    /// Return the logical offset of the next byte.
    pub(crate) const fn position(&self) -> usize {
        self.position
    }

    /// Return the number of bytes after the position.
    pub(crate) const fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    /// Return an error of `kind` at the logical offset `at`.
    pub(crate) fn fail(&self, kind: ErrorKind, at: usize, context: &'static str) -> Error {
        Error::new(kind, None)
            .with_context(context)
            .with_stream(StreamId(stream_number(self.stream)), offset(at))
    }

    /// Read `count` bytes.
    pub(crate) fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let bytes = self
            .position
            .checked_add(count)
            .and_then(|end| self.bytes.get(self.position..end))
            .ok_or_else(|| self.fail(ErrorKind::UnexpectedEof, self.position, self.context))?;
        self.position += count;
        Ok(bytes)
    }

    /// Skip `count` bytes.
    pub(crate) fn skip(&mut self, count: usize) -> Result<()> {
        self.take(count).map(|_| ())
    }

    /// Read a fixed-size array.
    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let bytes = self.take(N)?;
        <[u8; N]>::try_from(bytes)
            .map_err(|_| self.fail(ErrorKind::UnexpectedEof, self.position, self.context))
    }

    /// Read a little-endian `u32`.
    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    /// Read a little-endian `u64`.
    pub(crate) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    /// Read all bytes after the position.
    pub(crate) fn rest(&mut self) -> &'a [u8] {
        let bytes = self.bytes.get(self.position..).unwrap_or_default();
        self.position = self.bytes.len();
        bytes
    }

    /// Make sure that no bytes follow the position.
    pub(crate) fn finish(&self) -> Result<()> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(self.fail(
                ErrorKind::InvalidFormat,
                self.position,
                "unexpected bytes after the end of the structure",
            ))
        }
    }
}
