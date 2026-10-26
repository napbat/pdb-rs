//! Builders of the `#Strings`, `#US`, `#Blob` and `#GUID` heaps.
//!
//! Each builder removes duplicate values. The `#Blob`, `#US` and `#GUID` builders give the
//! offset or index of a value when it is added, in insertion order. The `#Strings` builder
//! gives the offsets only after [`StringHeapBuilder::finish`], because it puts a string that
//! is a suffix of another string inside that string.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::portable::compressed::{self, MAX_UNSIGNED};
use crate::portable::metadata::{
    BlobOffset, GuidIndex, MetadataStream, StringOffset, UserStringOffset,
};
use crate::{Error, ErrorKind, Result};

mod seed;

pub(super) use seed::walk_entries;

/// Make an [`ErrorKind::LimitExceeded`] error in a heap.
fn limit(stream: MetadataStream, context: &'static str) -> Error {
    Error::new(ErrorKind::LimitExceeded, None)
        .with_context(context)
        .with_portable(stream.location())
}

/// Return the size of a heap as `u32`, or a [`ErrorKind::LimitExceeded`] error.
fn heap_size(bytes: &[u8], stream: MetadataStream) -> Result<u32> {
    u32::try_from(bytes.len()).map_err(|_| limit(stream, "heap is larger than 4 GiB"))
}

/// Append `value` to `bytes` with a checked allocation.
fn append(bytes: &mut Vec<u8>, value: &[u8], stream: MetadataStream) -> Result<()> {
    bytes
        .try_reserve(value.len())
        .map_err(|_| limit(stream, "heap allocation failed"))?;
    bytes.extend_from_slice(value);
    Ok(())
}

/// Collects the strings of a `#Strings` heap.
///
/// [`StringHeapBuilder::finish`] lays out the heap. It sorts the strings by their bytes from
/// the end to the start, with a longer string first if one string is a suffix of the other.
/// A string that is a suffix of the string before it uses the end of that string. This is the
/// rule of `System.Reflection.Metadata`, applied to bytes. For ASCII strings, both rules give
/// the same heap.
///
/// A builder with a base heap (see [`Pdb::edit`](crate::portable::Pdb::edit)) keeps the base
/// bytes and their offsets. It adds the strings that the base does not have after the base.
#[derive(Clone, Debug, Default)]
pub struct StringHeapBuilder {
    base: Option<StringHeap>,
    strings: BTreeSet<Vec<u8>>,
}

impl StringHeapBuilder {
    /// Make a builder without strings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a string without its terminator. The empty string always has offset 0.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `value` contains a zero byte.
    /// - [`ErrorKind::LimitExceeded`] if the allocation fails.
    pub fn add(&mut self, value: &[u8]) -> Result<()> {
        if value.contains(&0) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("a #Strings value contains a zero byte")
                .with_portable(MetadataStream::Strings.location()));
        }
        let in_base = self
            .base
            .as_ref()
            .is_some_and(|base| base.offsets.contains_key(value));
        if value.is_empty() || in_base || self.strings.contains(value) {
            return Ok(());
        }
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(value.len())
            .map_err(|_| limit(MetadataStream::Strings, "heap allocation failed"))?;
        owned.extend_from_slice(value);
        self.strings.insert(owned);
        Ok(())
    }

    /// Lay out the heap and return it with the offset of each string.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::LimitExceeded`] if the heap is larger than 4 GiB or an allocation fails.
    pub fn finish(self) -> Result<StringHeap> {
        let mut sorted: Vec<Vec<u8>> = self.strings.into_iter().collect();
        sorted.sort_unstable_by(|left, right| suffix_order(left, right));
        let StringHeap {
            mut bytes,
            offsets: mut map,
        } = self.base.unwrap_or_default();
        let mut offsets = Vec::new();
        offsets
            .try_reserve_exact(sorted.len())
            .map_err(|_| limit(MetadataStream::Strings, "heap allocation failed"))?;
        let mut previous: &[u8] = &[];
        for value in &sorted {
            let offset = if previous.ends_with(value) {
                // The previous string ends just before the last terminator.
                bytes.len() - 1 - value.len()
            } else {
                let start = bytes.len();
                append(&mut bytes, value, MetadataStream::Strings)?;
                append(&mut bytes, &[0], MetadataStream::Strings)?;
                start
            };
            offsets.push(
                u32::try_from(offset)
                    .map_err(|_| limit(MetadataStream::Strings, "heap is larger than 4 GiB"))?,
            );
            previous = value;
        }
        heap_size(&bytes, MetadataStream::Strings)?;
        map.extend(sorted.into_iter().zip(offsets));
        Ok(StringHeap {
            bytes,
            offsets: map,
        })
    }

    /// Make a builder that keeps the bytes and the offsets of `base`.
    pub(crate) fn with_base(base: StringHeap) -> Self {
        Self {
            base: Some(base),
            strings: BTreeSet::new(),
        }
    }
}

/// Order strings so that each string comes just before the strings that are its suffixes.
fn suffix_order(left: &[u8], right: &[u8]) -> Ordering {
    for (left, right) in left.iter().rev().zip(right.iter().rev()) {
        match left.cmp(right) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    right.len().cmp(&left.len())
}

/// A laid-out `#Strings` heap.
#[derive(Clone, Debug)]
pub struct StringHeap {
    bytes: Vec<u8>,
    offsets: BTreeMap<Vec<u8>, u32>,
}

impl Default for StringHeap {
    fn default() -> Self {
        Self {
            bytes: Vec::from([0]),
            offsets: BTreeMap::new(),
        }
    }
}

impl StringHeap {
    /// Return the offset of a string that the builder added, or offset 0 for the empty
    /// string. Return `None` for another string.
    #[must_use]
    pub fn offset(&self, value: &[u8]) -> Option<StringOffset> {
        if value.is_empty() {
            return Some(StringOffset::new(0));
        }
        self.offsets.get(value).copied().map(StringOffset::new)
    }

    /// Return the heap bytes without padding.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return the heap size in bytes without padding.
    #[must_use]
    pub fn size(&self) -> u32 {
        // `finish` checks that the size fits in `u32`.
        u32::try_from(self.bytes.len()).unwrap_or(u32::MAX)
    }

    /// Return `true` if a row can refer to `offset`: 0, or an offset inside the heap.
    ///
    /// Any offset inside the heap is the start of a string that ends at the next zero byte.
    pub(crate) fn contains(&self, offset: StringOffset) -> bool {
        offset.get() == 0 || offset.get() < self.size()
    }

    /// Move the heap bytes out of the heap.
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// A heap of length-prefixed entries: `#Blob` or `#US`.
#[derive(Clone, Debug)]
struct PrefixedHeap {
    stream: MetadataStream,
    bytes: Vec<u8>,
    index: BTreeMap<Vec<u8>, u32>,
    /// The start of each entry after the first, in ascending order.
    starts: Vec<u32>,
}

impl PrefixedHeap {
    /// Make a heap with the empty entry at offset 0.
    fn new(stream: MetadataStream) -> Self {
        Self {
            stream,
            bytes: Vec::from([0]),
            index: BTreeMap::new(),
            starts: Vec::new(),
        }
    }

    /// Add an entry and return its offset.
    fn add(&mut self, value: &[u8]) -> Result<u32> {
        if value.is_empty() {
            return Ok(0);
        }
        if let Some(&offset) = self.index.get(value) {
            return Ok(offset);
        }
        let length = u32::try_from(value.len())
            .ok()
            .filter(|&length| length <= MAX_UNSIGNED)
            .ok_or_else(|| limit(self.stream, "heap entry is larger than 0x1FFFFFFF bytes"))?;
        let prefix = compressed::encode_unsigned(length)?;
        let offset = heap_size(&self.bytes, self.stream)?;
        let end = u64::from(offset) + u64::from(length) + 4;
        if end > u64::from(u32::MAX) {
            return Err(limit(self.stream, "heap is larger than 4 GiB"));
        }
        let mut key = Vec::new();
        key.try_reserve_exact(value.len())
            .map_err(|_| limit(self.stream, "heap allocation failed"))?;
        key.extend_from_slice(value);
        self.starts
            .try_reserve(1)
            .map_err(|_| limit(self.stream, "heap allocation failed"))?;
        append(&mut self.bytes, prefix.as_bytes(), self.stream)?;
        append(&mut self.bytes, value, self.stream)?;
        self.index.insert(key, offset);
        self.starts.push(offset);
        Ok(offset)
    }

    /// Return `true` if `offset` is 0 or the start of an entry.
    fn contains(&self, offset: u32) -> bool {
        offset == 0 || self.starts.binary_search(&offset).is_ok()
    }

    /// Return the heap size in bytes without padding.
    fn size(&self) -> u32 {
        // `add` checks that the size fits in `u32`.
        u32::try_from(self.bytes.len()).unwrap_or(u32::MAX)
    }
}

/// A `#Blob` heap builder.
///
/// The heap starts with the empty blob at offset 0. Each new blob gets the next offset.
#[derive(Clone, Debug)]
pub struct BlobHeap(PrefixedHeap);

impl Default for BlobHeap {
    fn default() -> Self {
        Self(PrefixedHeap::new(MetadataStream::Blob))
    }
}

impl BlobHeap {
    /// Make a heap with only the empty blob.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a blob without its length prefix and return its offset. The empty blob has
    /// offset 0. A blob that the heap has already gets its first offset.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::LimitExceeded`] if the blob is larger than `0x1FFFFFFF` bytes, the heap
    /// is larger than 4 GiB, or an allocation fails.
    pub fn add(&mut self, value: &[u8]) -> Result<BlobOffset> {
        self.0.add(value).map(BlobOffset::new)
    }

    /// Return `true` if `offset` is 0 or the start of a blob.
    #[must_use]
    pub fn contains(&self, offset: BlobOffset) -> bool {
        self.0.contains(offset.get())
    }

    /// Return the heap bytes without padding.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.0.bytes
    }

    /// Return the heap size in bytes without padding.
    #[must_use]
    pub fn size(&self) -> u32 {
        self.0.size()
    }

    /// Move the heap bytes out of the heap.
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.0.bytes
    }
}

/// A `#US` (user string) heap builder.
///
/// An entry has UTF-16LE code units and a final flag byte, so its length is odd. A Portable
/// PDB does not refer to user strings, but the heap can have entries.
#[derive(Clone, Debug)]
pub struct UserStringHeap(PrefixedHeap);

impl Default for UserStringHeap {
    fn default() -> Self {
        Self(PrefixedHeap::new(MetadataStream::UserStrings))
    }
}

impl UserStringHeap {
    /// Make a heap with only the empty entry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an entry without its length prefix and return its offset.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `entry` is not empty and its length is even.
    /// - The errors of [`BlobHeap::add`].
    pub fn add(&mut self, entry: &[u8]) -> Result<UserStringOffset> {
        if entry.len().is_multiple_of(2) && !entry.is_empty() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("a #US entry needs UTF-16 code units and a flag byte")
                .with_portable(MetadataStream::UserStrings.location()));
        }
        self.0.add(entry).map(UserStringOffset::new)
    }

    /// Return the heap bytes without padding.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.0.bytes
    }

    /// Return the heap size in bytes without padding.
    #[must_use]
    pub fn size(&self) -> u32 {
        self.0.size()
    }

    /// Move the heap bytes out of the heap.
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.0.bytes
    }
}

/// A `#GUID` heap builder. Index 0 is the nil GUID; the first GUID has index 1.
#[derive(Clone, Debug, Default)]
pub struct GuidHeap {
    bytes: Vec<u8>,
    index: BTreeMap<[u8; 16], u32>,
}

impl GuidHeap {
    /// Make a heap without GUIDs.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a GUID in its serialized byte order and return its 1-based index. A GUID that the
    /// heap has already gets its first index. A GUID of zero bytes is a heap entry too; use
    /// the index 0 for "no GUID".
    ///
    /// # Errors
    ///
    /// [`ErrorKind::LimitExceeded`] if the heap is larger than 4 GiB or an allocation fails.
    pub fn add(&mut self, guid: [u8; 16]) -> Result<GuidIndex> {
        if let Some(&index) = self.index.get(&guid) {
            return Ok(GuidIndex::new(index));
        }
        let index = heap_size(&self.bytes, MetadataStream::Guid)? / 16 + 1;
        if u64::from(index) * 16 > u64::from(u32::MAX) {
            return Err(limit(MetadataStream::Guid, "heap is larger than 4 GiB"));
        }
        append(&mut self.bytes, &guid, MetadataStream::Guid)?;
        self.index.insert(guid, index);
        Ok(GuidIndex::new(index))
    }

    /// Return the number of GUIDs.
    #[must_use]
    pub fn count(&self) -> u32 {
        self.size() / 16
    }

    /// Return the heap bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return the heap size in bytes, a multiple of 16.
    #[must_use]
    pub fn size(&self) -> u32 {
        // `add` checks that the size fits in `u32`.
        u32::try_from(self.bytes.len()).unwrap_or(u32::MAX)
    }

    /// Move the heap bytes out of the heap.
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}
