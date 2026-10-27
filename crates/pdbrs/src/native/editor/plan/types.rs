//! The TPI and IPI streams of a plan.
//!
//! The new layout encodes every record and writes the stream and its hash
//! stream with [`encode_type_stream`].
//!
//! The copy-on-write layout keeps the source stream if the records did not
//! change. If the caller only appended records, the plan keeps the source
//! header, records and hash buffers. It appends the new records, their hash
//! values and their index offset pairs, and it moves each hash buffer after
//! the buffers before it. The hash adjusters keep their bytes. If a source
//! record or a hash adjuster changed, the plan writes all records again with
//! the header options of the source.

use alloc::vec::Vec;

use super::compact::map::{IndexMaps, SpaceMap};
use super::compact::tables::{CompactRecord, compact};
use crate::native::Producer;
use crate::native::codeview::Records;
use crate::native::editor::types::{ListRecord, RecordList};
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::native::types::{
    HashBuffer, IndexOffset, TypeHashes, TypeHeader, TypeStream, TypeStreamOptions,
    encode_type_stream, type_record_hash,
};
use crate::{Error, ErrorKind, Result};

/// The size of the TPI and IPI header.
const HEADER_SIZE: usize = 56;
/// The size of the record blocks of the index offset buffer.
const INDEX_OFFSET_BLOCK: u32 = 8 * 1024;
/// The size of one hash value.
const VALUE_SIZE: usize = 4;
/// The size of one index offset pair.
const PAIR_SIZE: usize = 8;

/// A planned TPI or IPI stream.
pub(super) struct PlannedTypes {
    /// The parsed stream.
    pub(super) stream: TypeStream,
    /// The hash stream bytes, if the stream has a hash stream.
    pub(super) hash: Option<Vec<u8>>,
}

impl PlannedTypes {
    /// Return the index after the last record.
    pub(super) const fn index_end(&self) -> u32 {
        self.stream.header().index_end
    }
}

/// Return the error for a value that does not fit its field.
fn too_large(context: &'static str) -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context(context)
}

/// The options of a stream of the new layout.
#[derive(Clone, Copy, Debug)]
pub(super) struct NewTypeOptions {
    /// The hash bucket count.
    pub(super) hash_buckets: u32,
    /// The producer of the hash stream layout.
    pub(super) producer: Producer,
}

/// Encode every record of `list` in index order.
fn encode_all<T: ListRecord>(list: &RecordList<T>) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for entry in &list.entries {
        entry.value.encode_record(&mut bytes)?;
    }
    Ok(bytes)
}

/// Write a stream of the new layout and its hash stream.
pub(super) fn write_new<T: ListRecord>(
    list: &RecordList<T>,
    options: NewTypeOptions,
    hash_stream: Option<StreamId>,
) -> Result<PlannedTypes> {
    let records = encode_all(list)?;
    let stream_options = TypeStreamOptions::default()
        .with_index_begin(crate::native::codeview::index::FIRST_RECORD_INDEX)
        .with_hash_bucket_count(options.hash_buckets)
        .with_hash_stream(hash_stream)
        .with_hash_aux_stream(None)
        .with_producer(options.producer)
        .with_adjusters(&list.adjusters);
    let written = encode_type_stream(&records, &stream_options)?;
    Ok(PlannedTypes {
        stream: TypeStream::parse(written.stream)?,
        hash: written.hash_stream,
    })
}

/// Plan a stream of the copy-on-write layout. Return `None` if the plan
/// keeps the source stream.
///
/// `hash` gives the bytes of the source hash stream.
pub(super) fn write_cow<T: ListRecord>(
    list: &RecordList<T>,
    hash: impl FnOnce(StreamId) -> Result<Vec<u8>>,
) -> Result<Option<PlannedTypes>> {
    let source = list.source.as_ref().ok_or_else(|| {
        Error::new(ErrorKind::InvalidFormat, None)
            .with_context("type list of the copy-on-write layout has no source stream")
    })?;
    let mut changed = list.adjusters_changed;
    let mut added = Vec::new();
    let mut count = 0_u32;
    for entry in &list.entries {
        if entry.source.is_none() {
            entry.value.encode_record(&mut added)?;
            count = count
                .checked_add(1)
                .ok_or_else(|| too_large("type index end does not fit in 32 bits"))?;
        } else if !list.unchanged(entry)? {
            changed = true;
        }
    }
    if !changed && count == 0 {
        return Ok(None);
    }
    if !changed {
        return append(source, &added, count, hash).map(Some);
    }
    let mut records = Vec::new();
    for entry in &list.entries {
        if list.unchanged(entry)? {
            let bytes = list.source_bytes(entry)?.ok_or_else(|| {
                Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("source record has no bytes")
            })?;
            records.try_reserve(bytes.len()).map_err(|_| no_memory())?;
            records.extend_from_slice(bytes);
        } else {
            entry.value.encode_record(&mut records)?;
        }
    }
    let options = TypeStreamOptions::from_header(source.header(), &list.adjusters);
    let written = encode_type_stream(&records, &options)?;
    Ok(Some(PlannedTypes {
        stream: TypeStream::parse(written.stream)?,
        hash: written.hash_stream,
    }))
}

/// Plan a stream of the copy-on-write layout with output indices (#47).
///
/// `own` is the map of the space of `list`. The plan writes all records
/// again with the header options of the source and computes all hash values
/// again.
pub(super) fn write_compacted<T: CompactRecord>(
    list: &RecordList<T>,
    own: Option<&SpaceMap>,
    maps: &IndexMaps,
) -> Result<PlannedTypes> {
    let source = list.source.as_ref().ok_or_else(|| {
        Error::new(ErrorKind::InvalidFormat, None)
            .with_context("type list of the copy-on-write layout has no source stream")
    })?;
    let compacted = compact(list, own, maps)?;
    let options = TypeStreamOptions::from_header(source.header(), &compacted.adjusters);
    let written = encode_type_stream(&compacted.records, &options)?;
    Ok(PlannedTypes {
        stream: TypeStream::parse(written.stream)?,
        hash: written.hash_stream,
    })
}

/// Write a stream of the new layout with output indices, and its hash
/// stream (#47).
pub(super) fn write_new_compacted<T: CompactRecord>(
    list: &RecordList<T>,
    own: Option<&SpaceMap>,
    maps: &IndexMaps,
    options: NewTypeOptions,
    hash_stream: Option<StreamId>,
) -> Result<PlannedTypes> {
    let compacted = compact(list, own, maps)?;
    let stream_options = TypeStreamOptions::default()
        .with_index_begin(crate::native::codeview::index::FIRST_RECORD_INDEX)
        .with_hash_bucket_count(options.hash_buckets)
        .with_hash_stream(hash_stream)
        .with_hash_aux_stream(None)
        .with_producer(options.producer)
        .with_adjusters(&compacted.adjusters);
    let written = encode_type_stream(&compacted.records, &stream_options)?;
    Ok(PlannedTypes {
        stream: TypeStream::parse(written.stream)?,
        hash: written.hash_stream,
    })
}

/// Append the records `added` to `source` and build the hash stream.
fn append(
    source: &TypeStream,
    added: &[u8],
    count: u32,
    hash: impl FnOnce(StreamId) -> Result<Vec<u8>>,
) -> Result<PlannedTypes> {
    let header = source.header();
    let old_bytes = header.record_bytes;
    let record_bytes = u32::try_from(added.len())
        .ok()
        .and_then(|length| old_bytes.checked_add(length))
        .ok_or_else(|| too_large("type record bytes do not fit in 32 bits"))?;
    let index_end = header
        .index_end
        .checked_add(count)
        .ok_or_else(|| too_large("type index end does not fit in 32 bits"))?;
    // The size and the hash value of each added record.
    let mut sizes = Vec::new();
    let mut hashes = Vec::new();
    sizes
        .try_reserve_exact(count as usize)
        .map_err(|_| no_memory())?;
    hashes
        .try_reserve_exact(count as usize)
        .map_err(|_| no_memory())?;
    let mut framing = Records::new(added);
    let mut start = 0;
    while let Some(record) = framing.next() {
        record?;
        let end = framing.position();
        // A record has at most 65537 bytes.
        sizes.push(u32::try_from(end - start).unwrap_or(u32::MAX));
        hashes.push(type_record_hash(&added[start..end])? % header.hash_bucket_count);
        start = end;
    }
    let mut new_header = *header;
    new_header.index_end = index_end;
    new_header.record_bytes = record_bytes;
    let hash = match header.hash_stream {
        Some(id) => {
            let bytes = hash(id)?;
            Some(append_hashes(
                &mut new_header,
                header,
                &bytes,
                &sizes,
                &hashes,
            )?)
        }
        None => None,
    };
    let mut stream = Vec::new();
    stream
        .try_reserve_exact(HEADER_SIZE + record_bytes as usize)
        .map_err(|_| no_memory())?;
    encode_header(&new_header, &mut stream);
    stream.extend_from_slice(source.record_bytes());
    stream.extend_from_slice(added);
    let stream = TypeStream::parse(stream)?;
    if let Some(bytes) = &hash {
        TypeHashes::parse(stream.header(), bytes)?.check(&stream)?;
    }
    Ok(PlannedTypes { stream, hash })
}

/// Append the 56-byte header to `out`.
fn encode_header(header: &TypeHeader, out: &mut Vec<u8>) {
    let stream = |id: Option<StreamId>| {
        // The source header stored these stream IDs in 16 bits.
        id.map_or(0xffff_u16, |id| u16::try_from(id.0).unwrap_or(u16::MAX))
    };
    for value in [
        header.version,
        header.header_size,
        header.index_begin,
        header.index_end,
        header.record_bytes,
    ] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&stream(header.hash_stream).to_le_bytes());
    out.extend_from_slice(&stream(header.hash_aux_stream).to_le_bytes());
    out.extend_from_slice(&header.hash_key_size.to_le_bytes());
    out.extend_from_slice(&header.hash_bucket_count.to_le_bytes());
    for buffer in [
        header.hash_values,
        header.index_offsets,
        header.hash_adjustments,
    ] {
        out.extend_from_slice(&buffer.offset.to_le_bytes());
        out.extend_from_slice(&buffer.length.to_le_bytes());
    }
}

/// Return the bytes of a hash buffer of the source hash stream.
fn buffer_bytes(bytes: &[u8], buffer: HashBuffer) -> Result<&[u8]> {
    if buffer.length == 0 {
        return Ok(&[]);
    }
    usize::try_from(buffer.offset)
        .ok()
        .and_then(|start| bytes.get(start..start.checked_add(buffer.length as usize)?))
        .ok_or_else(|| {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("type hash buffer is outside the hash stream")
        })
}

/// Return the index offset pairs of the added records.
///
/// The rule continues the rule of the source producer (see
/// [`TypeHeader::producer`]).
fn added_pairs(
    header: &TypeHeader,
    last: Option<IndexOffset>,
    sizes: &[u32],
) -> Result<Vec<IndexOffset>> {
    let producer = header.producer();
    let mut last = last;
    let mut pairs = Vec::new();
    let mut offset = header.record_bytes;
    for (index, &size) in (header.index_end..).zip(sizes) {
        let end = offset
            .checked_add(size)
            .ok_or_else(|| too_large("type record bytes do not fit in 32 bits"))?;
        let add = match (producer, last) {
            (_, None) => true,
            (Producer::Microsoft, Some(last)) => {
                last.offset / INDEX_OFFSET_BLOCK < offset / INDEX_OFFSET_BLOCK
            }
            (Producer::Llvm, Some(_)) => end / INDEX_OFFSET_BLOCK > offset / INDEX_OFFSET_BLOCK,
        };
        if add {
            let pair = IndexOffset { index, offset };
            pairs.try_reserve(1).map_err(|_| no_memory())?;
            pairs.push(pair);
            last = Some(pair);
        }
        offset = end;
    }
    Ok(pairs)
}

/// Build the hash stream with the hash values and index offset pairs of the
/// added records, and set the buffers of `header`.
///
/// Each buffer keeps its position relative to the other buffers. An empty
/// buffer comes before a buffer with bytes at the same source offset, as
/// the LLVM layout puts the empty hash adjuster buffer.
fn append_hashes(
    header: &mut TypeHeader,
    source: &TypeHeader,
    bytes: &[u8],
    sizes: &[u32],
    hashes: &[u32],
) -> Result<Vec<u8>> {
    let old = [
        source.hash_values,
        source.index_offsets,
        source.hash_adjustments,
    ];
    let values = buffer_bytes(bytes, old[0])?;
    let pairs = buffer_bytes(bytes, old[1])?;
    let adjusters = buffer_bytes(bytes, old[2])?;
    if values.len() != source.record_count() as usize * VALUE_SIZE {
        return Err(Error::new(ErrorKind::UnsupportedEdit, None)
            .with_context("type hash stream does not have one hash value for each record"));
    }
    let last = pairs
        .as_chunks::<PAIR_SIZE>()
        .0
        .last()
        .map(|pair| IndexOffset {
            index: u32::from_le_bytes([pair[0], pair[1], pair[2], pair[3]]),
            offset: u32::from_le_bytes([pair[4], pair[5], pair[6], pair[7]]),
        });
    let new_pairs = added_pairs(source, last, sizes)?;
    let mut parts: [Vec<u8>; 3] = Default::default();
    parts[0]
        .try_reserve_exact(values.len() + hashes.len() * VALUE_SIZE)
        .map_err(|_| no_memory())?;
    parts[0].extend_from_slice(values);
    for hash in hashes {
        parts[0].extend_from_slice(&hash.to_le_bytes());
    }
    parts[1]
        .try_reserve_exact(pairs.len() + new_pairs.len() * PAIR_SIZE)
        .map_err(|_| no_memory())?;
    parts[1].extend_from_slice(pairs);
    for pair in &new_pairs {
        parts[1].extend_from_slice(&pair.index.to_le_bytes());
        parts[1].extend_from_slice(&pair.offset.to_le_bytes());
    }
    parts[2]
        .try_reserve_exact(adjusters.len())
        .map_err(|_| no_memory())?;
    parts[2].extend_from_slice(adjusters);
    let mut order = [0_usize, 1, 2];
    order.sort_by_key(|&slot| (old[slot].offset, old[slot].length != 0, slot));
    let total: usize = parts.iter().map(Vec::len).sum();
    let mut out = Vec::new();
    out.try_reserve_exact(total).map_err(|_| no_memory())?;
    let mut new = old;
    for slot in order {
        let offset = i32::try_from(out.len())
            .map_err(|_| too_large("type hash buffer offset does not fit in 31 bits"))?;
        let length = u32::try_from(parts[slot].len())
            .map_err(|_| too_large("type hash buffer does not fit in 32 bits"))?;
        new[slot] = HashBuffer { offset, length };
        out.extend_from_slice(&parts[slot]);
    }
    [
        header.hash_values,
        header.index_offsets,
        header.hash_adjustments,
    ] = new;
    Ok(out)
}
