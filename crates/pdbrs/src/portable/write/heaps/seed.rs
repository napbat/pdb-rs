//! Heaps that start with the bytes of an existing heap.
//!
//! An editor keeps every entry of the source heaps at its offset, so the rows and the opaque
//! blobs of the source keep valid heap references. The heaps index the existing entries, so a
//! value that the source has gets its existing offset.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::{BlobHeap, GuidHeap, PrefixedHeap, StringHeap, UserStringHeap, heap_size, limit};
use crate::portable::compressed;
use crate::portable::metadata::{MetadataStream, StringOffset};
use crate::{Error, ErrorKind, Result};

/// Make an [`ErrorKind::InvalidFormat`] error at an offset of a heap.
fn invalid(stream: MetadataStream, offset: usize, context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, None)
        .with_context(context)
        .with_portable(
            stream
                .location()
                .with_heap_offset(u32::try_from(offset).unwrap_or(u32::MAX)),
        )
}

/// Call `entry` with the offset and the bytes of each length-prefixed entry of a `#Blob` or
/// `#US` heap, in heap order.
///
/// # Errors
///
/// [`ErrorKind::InvalidFormat`] if the heap is not a sequence of length-prefixed entries, and
/// the errors of `entry`.
pub(in crate::portable::write) fn walk_entries(
    bytes: &[u8],
    stream: MetadataStream,
    mut entry: impl FnMut(u32, &[u8]) -> Result<()>,
) -> Result<()> {
    let context = "heap is not a sequence of length-prefixed entries";
    let mut position = 0;
    while position < bytes.len() {
        let rest = &bytes[position..];
        let (length, after) =
            compressed::decode_unsigned(rest).map_err(|_| invalid(stream, position, context))?;
        let start = rest.len() - after.len();
        let value = usize::try_from(length)
            .ok()
            .and_then(|length| after.get(..length))
            .ok_or_else(|| invalid(stream, position, context))?;
        let offset =
            u32::try_from(position).map_err(|_| limit(stream, "heap is larger than 4 GiB"))?;
        entry(offset, value)?;
        position += start + value.len();
    }
    Ok(())
}

impl PrefixedHeap {
    /// Make a heap with the bytes of an existing heap.
    fn from_bytes(stream: MetadataStream, bytes: Vec<u8>) -> Result<Self> {
        if bytes.is_empty() {
            return Ok(Self::new(stream));
        }
        heap_size(&bytes, stream)?;
        if bytes[0] != 0 {
            return Err(invalid(
                stream,
                0,
                "heap does not start with an empty entry",
            ));
        }
        let mut index = BTreeMap::new();
        let mut starts = Vec::new();
        walk_entries(&bytes, stream, |offset, value| {
            if offset != 0 {
                starts.push(offset);
            }
            if !value.is_empty() && !index.contains_key(value) {
                index.insert(value.to_vec(), offset);
            }
            Ok(())
        })?;
        Ok(Self {
            stream,
            bytes,
            index,
            starts,
        })
    }
}

impl BlobHeap {
    /// Make a heap with the bytes of an existing `#Blob` heap.
    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        PrefixedHeap::from_bytes(MetadataStream::Blob, bytes).map(Self)
    }
}

impl UserStringHeap {
    /// Make a heap with the bytes of an existing `#US` heap.
    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        PrefixedHeap::from_bytes(MetadataStream::UserStrings, bytes).map(Self)
    }
}

impl GuidHeap {
    /// Make a heap with the bytes of an existing `#GUID` heap.
    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        heap_size(&bytes, MetadataStream::Guid)?;
        let (guids, rest) = bytes.as_chunks::<16>();
        if !rest.is_empty() {
            return Err(invalid(
                MetadataStream::Guid,
                bytes.len(),
                "#GUID heap size is not a multiple of 16",
            ));
        }
        let mut index = BTreeMap::new();
        for (position, guid) in (1..).zip(guids) {
            index.entry(*guid).or_insert(position);
        }
        Ok(Self { bytes, index })
    }
}

impl StringHeap {
    /// Make a heap with the bytes of an existing `#Strings` heap. The heap knows the string
    /// that starts after each zero byte; [`StringHeap::register`] adds other offsets.
    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        if bytes.is_empty() {
            return Ok(Self::default());
        }
        heap_size(&bytes, MetadataStream::Strings)?;
        if bytes[0] != 0 {
            return Err(invalid(
                MetadataStream::Strings,
                0,
                "heap does not start with an empty entry",
            ));
        }
        let mut offsets = BTreeMap::new();
        let mut position = 1;
        while let Some(length) = bytes[position..].iter().position(|&byte| byte == 0) {
            if length > 0 {
                let value = &bytes[position..position + length];
                // `heap_size` checks that each offset fits in `u32`.
                let offset = u32::try_from(position).unwrap_or(u32::MAX);
                offsets.entry(value.to_vec()).or_insert(offset);
            }
            position += length + 1;
        }
        Ok(Self { bytes, offsets })
    }

    /// Record that the string `value` is at `offset`, if the heap has no offset for it.
    pub(crate) fn register(&mut self, value: &[u8], offset: StringOffset) {
        if !value.is_empty() && !self.offsets.contains_key(value) {
            self.offsets.insert(value.to_vec(), offset.get());
        }
    }
}
