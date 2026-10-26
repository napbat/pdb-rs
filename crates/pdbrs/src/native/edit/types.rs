//! The type and ID edits: records appended to the TPI and IPI streams.
//!
//! The plan keeps the source header, records and hash buffers. It appends
//! the new records, their hash values and their index offset pairs, and it
//! moves each hash buffer after the buffers before it. The hash adjusters
//! keep their bytes.

use alloc::vec::Vec;

use super::plan::{Changes, Context, missing};
use super::{AddedRecords, INDEX_END_OFFSET, IPI_STREAM, TPI_STREAM, unsupported};
use crate::io::ReadAt;
use crate::native::Pdb;
use crate::native::codeview::Records;
use crate::native::codeview::id_record::IdTable;
use crate::native::codeview::index::{FIRST_RECORD_INDEX, IdIndex, TypeIndex};
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::native::types::{
    HashBuffer, HashLayout, IndexOffset, ReferenceLimits, TypeHashes, TypeHeader, TypeStream,
    TypeTable, type_record_hash, validate_type_references,
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

/// The index ends of the TPI and IPI streams after the edit.
#[derive(Clone, Copy, Debug)]
pub(super) struct Ends {
    /// The index after the last TPI record.
    pub(super) types: TypeIndex,
    /// The index after the last IPI record.
    pub(super) ids: IdIndex,
}

/// A TPI or IPI stream with appended records, and its hash stream.
struct Appended {
    /// The stream.
    stream: TypeStream,
    /// The hash stream, if the header names one.
    hash: Option<Vec<u8>>,
}

/// Return the error for a value that does not fit its field.
fn too_large(context: &'static str) -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context(context)
}

/// Append the added TPI and IPI records, check the references and return
/// the index ends.
pub(super) fn plan<S: ReadAt>(
    pdb: &Pdb<S>,
    context: &Context,
    types: &AddedRecords,
    ids: &AddedRecords,
    changes: &mut Changes,
) -> Result<Ends> {
    let mut type_end = TypeIndex(context.tpi.header().index_end);
    if types.count != 0 {
        let appended = append(pdb, &context.tpi, types)?;
        validate_type_references(
            &TypeTable::new(&appended.stream)?,
            ReferenceLimits::default(),
        )?;
        type_end = TypeIndex(appended.stream.header().index_end);
        store(appended, TPI_STREAM, changes)?;
    }
    let id_end = if ids.count == 0 {
        match pdb.msf().stream(IPI_STREAM)? {
            Some(stream) if stream.len()? != 0 => IdIndex(stream.read_u32_at(INDEX_END_OFFSET)?),
            _ => IdIndex(FIRST_RECORD_INDEX),
        }
    } else {
        let source = pdb.ids()?.ok_or_else(|| missing("PDB has no IPI stream"))?;
        let appended = append(pdb, &source, ids)?;
        IdTable::new(&appended.stream)?.validate(type_end)?;
        let end = IdIndex(appended.stream.header().index_end);
        store(appended, IPI_STREAM, changes)?;
        end
    };
    Ok(Ends {
        types: type_end,
        ids: id_end,
    })
}

/// Give the stream and the hash stream of `appended` to `changes`.
fn store(appended: Appended, stream: StreamId, changes: &mut Changes) -> Result<()> {
    if let (Some(hash), Some(id)) = (appended.hash, appended.stream.header().hash_stream) {
        changes.replace(id, hash)?;
    }
    changes.replace(stream, appended.stream.into_bytes())
}

/// Append the records of `added` to `source` and build the hash stream.
fn append<S: ReadAt>(pdb: &Pdb<S>, source: &TypeStream, added: &AddedRecords) -> Result<Appended> {
    let header = source.header();
    if added.first != Some(header.index_end) {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("type stream changed after the editor read its header"));
    }
    let old_bytes = header.record_bytes;
    let record_bytes = u32::try_from(added.records.len())
        .ok()
        .and_then(|length| old_bytes.checked_add(length))
        .ok_or_else(|| too_large("type record bytes do not fit in 32 bits"))?;
    let index_end = header
        .index_end
        .checked_add(added.count)
        .ok_or_else(|| too_large("type index end does not fit in 32 bits"))?;
    // The size and the hash value of each added record.
    let mut sizes = Vec::new();
    let mut hashes = Vec::new();
    sizes
        .try_reserve_exact(added.count as usize)
        .map_err(|_| no_memory())?;
    hashes
        .try_reserve_exact(added.count as usize)
        .map_err(|_| no_memory())?;
    let mut framing = Records::new(&added.records);
    let mut start = 0;
    while let Some(record) = framing.next() {
        record?;
        let end = framing.position();
        // A record has at most 65537 bytes.
        sizes.push(u32::try_from(end - start).unwrap_or(u32::MAX));
        hashes.push(type_record_hash(&added.records[start..end])? % header.hash_bucket_count);
        start = end;
    }
    let mut new_header = *header;
    new_header.index_end = index_end;
    new_header.record_bytes = record_bytes;
    let hash = match header.hash_stream {
        Some(id) => {
            let bytes = pdb.stream_bytes(id)?;
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
    stream.extend_from_slice(&added.records);
    let stream = TypeStream::parse(stream)?;
    if let Some(bytes) = &hash {
        TypeHashes::parse(stream.header(), bytes)?.verify(&stream)?;
    }
    Ok(Appended { stream, hash })
}

/// Append the 56-byte header to `out`.
fn encode_header(header: &TypeHeader, out: &mut Vec<u8>) {
    let stream = |id: Option<crate::native::msf::StreamId>| {
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
/// The rule continues the rule of the source producer (see [`HashLayout`]).
fn added_pairs(
    header: &TypeHeader,
    last: Option<IndexOffset>,
    sizes: &[u32],
) -> Result<Vec<IndexOffset>> {
    let layout = HashLayout::of(header);
    let mut last = last;
    let mut pairs = Vec::new();
    let mut offset = header.record_bytes;
    for (index, &size) in (header.index_end..).zip(sizes) {
        let end = offset
            .checked_add(size)
            .ok_or_else(|| too_large("type record bytes do not fit in 32 bits"))?;
        let add = match (layout, last) {
            (_, None) => true,
            (HashLayout::Microsoft, Some(last)) => {
                last.offset / INDEX_OFFSET_BLOCK < offset / INDEX_OFFSET_BLOCK
            }
            (HashLayout::Llvm, Some(_)) => end / INDEX_OFFSET_BLOCK > offset / INDEX_OFFSET_BLOCK,
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
        return Err(unsupported(
            "type hash stream does not have one hash value for each record",
        ));
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
