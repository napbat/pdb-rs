//! An owned serialized hash table that changes with the insertion rules of a producer.
//!
//! Microsoft `PDB/include/map.h` (`Map::add`, `Map::remove`, `Map::find`, `Map::grow`) and LLVM
//! `HashTable.h` (`set_as_internal`, `find_as`, `grow`) use the same rules:
//!
//! 1. A lookup starts at `hash % capacity` and walks forward with linear probing. It stops at
//!    the key, or at the first bucket that is neither present nor deleted.
//! 2. An insertion uses the first bucket on that walk that is not present. Then, if the entry
//!    count is at least `capacity * 2 / 3 + 1`, the table grows to two times that value. The
//!    growth inserts the present entries again in bucket order into an empty table.
//! 3. A removal clears the present bit and sets the deleted bit of the bucket.
//!
//! The producers differ in some points. [`OwnedHashTable`] lists them.
//!
//! A table decoded from a file keeps its bit vector words exactly, so that
//! [`OwnedHashTable::encode`] gives the same bytes again.

use alloc::vec::Vec;

use super::{HashTable, HashTableEntry, HashTableValue};
use crate::native::Producer;
use crate::native::output::{Output, no_memory, to_u32};
use crate::{Error, ErrorKind, Result};

/// Return the bucket count of a table from the default constructor of `producer`.
pub(crate) const fn initial_capacity(producer: Producer) -> u32 {
    match producer {
        Producer::Microsoft => 1,
        Producer::Llvm => 8,
    }
}

/// The hash function and the key equality of a table.
pub trait HashTableKeys {
    /// Return the hash of `key` before the reduction modulo the capacity.
    ///
    /// # Errors
    ///
    /// Returns an error if the key does not refer to a valid item, for example a name offset
    /// outside a string buffer.
    fn hash(&self, key: u32) -> Result<u32>;

    /// Return `true` if `left` and `right` name the same item.
    ///
    /// The default compares the keys. A table with name offsets as keys can compare names.
    ///
    /// # Errors
    ///
    /// Returns an error if a key does not refer to a valid item.
    fn same(&self, left: u32, right: u32) -> Result<bool> {
        Ok(left == right)
    }
}

/// The result of a lookup.
enum Slot {
    /// The key is in the entry with this index.
    Found(usize),
    /// The key is absent. An insertion uses this bucket, if any.
    Absent(Option<u32>),
}

/// An owned hash table with `u32` keys.
///
/// The table stores only the present entries, so a large capacity does not allocate buckets.
/// The entries are in bucket order.
///
/// The [`Producer`] of the table selects these rules:
///
/// | Rule | [`Producer::Microsoft`] | [`Producer::Llvm`] |
/// | --- | --- | --- |
/// | Capacity of a table from the default constructor (`Map()`, `HashTable()`) | 1 | 8 |
/// | Present bit vector words | At least 1; extended to the highest bit set since the last growth | Up to the highest present bit |
/// | Deleted bit vector words | Extended to the highest bit set since the last growth | Up to the highest deleted bit |
///
/// Some tables use another first capacity: the LLVM named-stream map starts with 1 bucket
/// (`OffsetIndexMap(1)`). Use [`OwnedHashTable::with_capacity`] for them.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct OwnedHashTable<V> {
    /// The insertion and serialization rules.
    producer: Producer,
    /// The number of buckets.
    capacity: u32,
    /// The present entries in bucket order.
    entries: Vec<HashTableEntry<V>>,
    /// The words of the present bit vector, with their serialized count.
    present: Vec<u32>,
    /// The words of the deleted bit vector, with their serialized count.
    deleted: Vec<u32>,
}

impl<V: Clone> OwnedHashTable<V> {
    /// Create an empty table with the default capacity of `producer`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the allocation fails.
    pub fn new(producer: Producer) -> Result<Self> {
        Self::with_capacity(producer, initial_capacity(producer))
    }

    /// Create an empty table with `capacity` buckets.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `capacity` is zero.
    /// - [`ErrorKind::LimitExceeded`] if the allocation fails.
    pub fn with_capacity(producer: Producer, capacity: u32) -> Result<Self> {
        if capacity == 0 {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("hash table capacity is zero"));
        }
        let mut present = Vec::new();
        if producer == Producer::Microsoft {
            // `ISet(1)` of a new `Map` has one word.
            present.try_reserve_exact(1).map_err(|_| no_memory())?;
            present.push(0);
        }
        Ok(Self {
            producer,
            capacity,
            entries: Vec::new(),
            present,
            deleted: Vec::new(),
        })
    }

    /// Copy a decoded table. The copy keeps the bucket of each entry and the bit vectors.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the allocation fails.
    pub fn from_table(table: &HashTable<'_, V>, producer: Producer) -> Result<Self> {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(table.len())
            .map_err(|_| no_memory())?;
        entries.extend_from_slice(table.entries());
        Ok(Self {
            producer,
            capacity: table.capacity(),
            entries,
            present: words_of(table.present)?,
            deleted: words_of(table.deleted)?,
        })
    }

    /// Return the insertion and serialization rules.
    #[must_use]
    pub const fn producer(&self) -> Producer {
        self.producer
    }

    /// Return the number of buckets.
    #[must_use]
    pub const fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Return the number of present entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Return `true` if the table has no present entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return the present entries in bucket order.
    #[must_use]
    pub fn entries(&self) -> OwnedHashTableEntries<'_, V> {
        OwnedHashTableEntries(self.entries.iter())
    }

    /// Find the value of `key`.
    ///
    /// # Errors
    ///
    /// Returns an error from `keys`.
    pub fn get(&self, key: u32, keys: &impl HashTableKeys) -> Result<Option<&V>> {
        Ok(match self.find(key, keys)? {
            Slot::Found(index) => self.entries.get(index).map(|entry| &entry.value),
            Slot::Absent(_) => None,
        })
    }

    /// Insert `key` with `value`, or replace the value of an existing key.
    ///
    /// Return the previous value of the key. An insertion can grow the table.
    ///
    /// # Errors
    ///
    /// - An error from `keys`.
    /// - [`ErrorKind::LimitExceeded`] if no bucket is free, the capacity overflows `u32` or an
    ///   allocation fails.
    pub fn insert(&mut self, key: u32, value: V, keys: &impl HashTableKeys) -> Result<Option<V>> {
        match self.find(key, keys)? {
            Slot::Found(index) => Ok(self
                .entries
                .get_mut(index)
                .map(|entry| core::mem::replace(&mut entry.value, value))),
            Slot::Absent(None) => Err(Error::new(ErrorKind::LimitExceeded, None)
                .with_context("hash table has no free bucket")),
            Slot::Absent(Some(bucket)) => {
                self.entries.try_reserve(1).map_err(|_| no_memory())?;
                self.set_bit(Bits::Present, bucket, true)?;
                self.set_bit(Bits::Deleted, bucket, false)?;
                let index = self.entries.partition_point(|entry| entry.bucket < bucket);
                self.entries
                    .insert(index, HashTableEntry { bucket, key, value });
                self.normalize();
                self.grow(keys)?;
                Ok(None)
            }
        }
    }

    /// Remove `key` and return its value. The bucket becomes deleted.
    ///
    /// # Errors
    ///
    /// Returns an error from `keys`, or [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn remove(&mut self, key: u32, keys: &impl HashTableKeys) -> Result<Option<V>> {
        let Slot::Found(index) = self.find(key, keys)? else {
            return Ok(None);
        };
        let bucket = self.entries[index].bucket;
        self.set_bit(Bits::Present, bucket, false)?;
        self.set_bit(Bits::Deleted, bucket, true)?;
        self.normalize();
        Ok(Some(self.entries.remove(index).value))
    }

    /// Walk the probe sequence of `key`.
    fn find(&self, key: u32, keys: &impl HashTableKeys) -> Result<Slot> {
        let home = keys.hash(key)? % self.capacity;
        let mut bucket = home;
        let mut unused = None;
        loop {
            if let Ok(index) = self
                .entries
                .binary_search_by_key(&bucket, |entry| entry.bucket)
            {
                if keys.same(self.entries[index].key, key)? {
                    return Ok(Slot::Found(index));
                }
            } else {
                unused.get_or_insert(bucket);
                if !bit(&self.deleted, bucket) {
                    break;
                }
            }
            bucket = if bucket + 1 < self.capacity {
                bucket + 1
            } else {
                0
            };
            if bucket == home {
                break;
            }
        }
        Ok(Slot::Absent(unused))
    }

    /// Grow the table if the entry count reached the load limit.
    fn grow(&mut self, keys: &impl HashTableKeys) -> Result<()> {
        let limit = u64::from(self.capacity) * 2 / 3 + 1;
        if u64::try_from(self.entries.len()).unwrap_or(u64::MAX) < limit {
            return Ok(());
        }
        let capacity = u32::try_from(limit * 2).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("hash table capacity overflows u32")
        })?;
        let mut grown = Self::with_capacity(self.producer, capacity)?;
        for entry in &self.entries {
            grown.insert(entry.key, entry.value.clone(), keys)?;
        }
        *self = grown;
        Ok(())
    }

    /// Set or clear one bit of a bit vector.
    fn set_bit(&mut self, bits: Bits, bucket: u32, value: bool) -> Result<()> {
        let words = match bits {
            Bits::Present => &mut self.present,
            Bits::Deleted => &mut self.deleted,
        };
        let index = usize::try_from(bucket / 32).map_err(|_| no_memory())?;
        if index >= words.len() {
            if !value {
                return Ok(());
            }
            words
                .try_reserve(index + 1 - words.len())
                .map_err(|_| no_memory())?;
            words.resize(index + 1, 0);
        }
        let mask = 1_u32 << (bucket % 32);
        if value {
            words[index] |= mask;
        } else {
            words[index] &= !mask;
        }
        Ok(())
    }

    /// Apply the bit vector rule of the producer after a change.
    fn normalize(&mut self) {
        if self.producer == Producer::Llvm {
            for words in [&mut self.present, &mut self.deleted] {
                let used = words
                    .iter()
                    .rposition(|&word| word != 0)
                    .map_or(0, |at| at + 1);
                words.truncate(used);
            }
        }
    }
}

impl<'a, V: HashTableValue<'a>> OwnedHashTable<V> {
    /// Append the serialized table to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if a count does not fit in `u32` or an allocation
    /// fails, or an error from [`HashTableValue::encode`].
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut output = Output::new(out);
        output.u32(to_u32(self.entries.len(), "hash table size")?)?;
        output.u32(self.capacity)?;
        for words in [&self.present, &self.deleted] {
            output.u32(to_u32(words.len(), "hash table bit vector")?)?;
            for word in words {
                output.u32(*word)?;
            }
        }
        for entry in &self.entries {
            out.try_reserve(4 + V::SIZE).map_err(|_| no_memory())?;
            out.extend_from_slice(&entry.key.to_le_bytes());
            entry.value.encode(out)?;
        }
        Ok(())
    }
}

/// A bit vector of the table.
#[derive(Clone, Copy)]
enum Bits {
    Present,
    Deleted,
}

/// Return bit `index` of a word vector.
fn bit(words: &[u32], index: u32) -> bool {
    usize::try_from(index / 32)
        .ok()
        .and_then(|word| words.get(word))
        .is_some_and(|word| word >> (index % 32) & 1 != 0)
}

/// Copy the words of a serialized bit vector.
fn words_of(bytes: &[u8]) -> Result<Vec<u32>> {
    let chunks = bytes.as_chunks::<4>().0;
    let mut words = Vec::new();
    words
        .try_reserve_exact(chunks.len())
        .map_err(|_| no_memory())?;
    words.extend(chunks.iter().map(|word| u32::from_le_bytes(*word)));
    Ok(words)
}

/// An iterator over the present entries of an [`OwnedHashTable`] in bucket order.
#[derive(Debug)]
pub struct OwnedHashTableEntries<'a, V>(core::slice::Iter<'a, HashTableEntry<V>>);

impl<V> Clone for OwnedHashTableEntries<'_, V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<'a, V> Iterator for OwnedHashTableEntries<'a, V> {
    type Item = HashTableEntry<&'a V>;

    fn next(&mut self) -> Option<HashTableEntry<&'a V>> {
        self.0.next().map(|entry| HashTableEntry {
            bucket: entry.bucket,
            key: entry.key,
            value: &entry.value,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<V> ExactSizeIterator for OwnedHashTableEntries<'_, V> {}

impl<V> core::iter::FusedIterator for OwnedHashTableEntries<'_, V> {}
