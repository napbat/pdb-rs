//! Writers of TPI and IPI streams and their hash streams.
//!
//! [`write_type_stream`] takes the complete records of a stream and gives
//! the stream bytes and the hash stream bytes. The hash stream has three
//! buffers:
//!
//! - The hash values: [`super::type_record_hash`] of each record, modulo the
//!   bucket count. The hash function depends on the record kind, as in
//!   Microsoft `tpi.cpp` and LLVM `TpiHashing.cpp`.
//! - The index offsets: pairs of a record index and its offset in the
//!   record bytes, about one pair for each 8 KiB of records.
//! - The hash adjusters: the serialized `mpnitiHead` map of Microsoft
//!   `tpi.cpp`.
//!
//! Microsoft `link.exe` and LLVM `lld-link` order the buffers differently,
//! and they select the records of the index offset pairs with different
//! rules. [`HashLayout`] selects the form of one producer.

use alloc::vec::Vec;

use super::{
    HEADER_SIZE, HashAdjuster, HashBuffer, IndexOffset, TPI_V80, TypeHeader, type_record_hash,
};
use crate::native::codeview::Records;
use crate::native::codeview::index::FIRST_RECORD_INDEX;
use crate::native::msf::StreamId;
use crate::{Error, ErrorKind, Result};

/// The hash bucket count of Microsoft and LLVM writers (`0x3ffff`).
pub const DEFAULT_HASH_BUCKETS: u32 = 0x3_ffff;

/// The size of the record blocks of the index offset buffer.
const INDEX_OFFSET_BLOCK: u32 = 8 * 1024;

/// The serialized value of an absent stream index.
const NIL_STREAM: u16 = 0xffff;

/// The hash key size that version 8.0 requires.
const HASH_KEY_SIZE: u32 = 4;

/// The layout of the hash stream and the rule for index offset pairs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HashLayout {
    /// Microsoft `tpi.cpp`.
    ///
    /// - The buffers are the hash values, the index offsets and the hash
    ///   adjusters, in this order.
    /// - A record gets an index offset pair if it is the first record, or
    ///   if its start offset is in a later 8 KiB block than the offset of
    ///   the last pair (`TPI1::RecordTiOff`).
    /// - Without a hash stream, each buffer has the offset 0 and the length
    ///   `0xffff_ffff`.
    Microsoft,
    /// LLVM `TpiStreamBuilder`.
    ///
    /// - The buffers are the hash values and the index offsets. The empty
    ///   hash adjuster buffer has the offset of the end of the hash values.
    /// - A record gets an index offset pair if it is the first record, or
    ///   if its end is in a later 8 KiB block than its start
    ///   (`updateTypeIndexOffsets`).
    /// - Without a hash stream, each buffer has the offset 0 and the length 0.
    Llvm,
}

impl HashLayout {
    /// Return the layout that a header shows.
    ///
    /// A header with a hash stream and index offsets that start where the
    /// empty hash adjuster buffer starts is [`HashLayout::Llvm`]. A header
    /// without a hash stream and with zero-length buffers is
    /// [`HashLayout::Llvm`] too. All other headers are
    /// [`HashLayout::Microsoft`]. The two layouts give the same bytes when
    /// they cannot be told apart.
    #[must_use]
    pub const fn of(header: &TypeHeader) -> Self {
        let adjusters = header.hash_adjustments;
        let offsets = header.index_offsets;
        let llvm = match header.hash_stream {
            Some(_) => {
                adjusters.length == 0 && offsets.length != 0 && adjusters.offset == offsets.offset
            }
            None => header.hash_values.length == 0,
        };
        if llvm { Self::Llvm } else { Self::Microsoft }
    }
}

/// The parameters of a TPI or IPI stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypeStreamOptions<'a> {
    /// The index of the first record, usually [`FIRST_RECORD_INDEX`].
    pub index_begin: u32,
    /// The number of hash buckets, usually [`DEFAULT_HASH_BUCKETS`].
    pub hash_bucket_count: u32,
    /// The stream of the hash data. `None` writes no hash stream.
    pub hash_stream: Option<StreamId>,
    /// The stream of the auxiliary hash data, if present.
    pub hash_aux_stream: Option<StreamId>,
    /// The hash stream layout and the index offset rule.
    pub layout: HashLayout,
    /// The hash adjusters, in the order that the producer added them.
    pub adjusters: &'a [HashAdjuster],
}

impl<'a> TypeStreamOptions<'a> {
    /// Return the options of a stream with `header`, so that the writer keeps
    /// its hash parameters and its producer layout.
    #[must_use]
    pub const fn from_header(header: &TypeHeader, adjusters: &'a [HashAdjuster]) -> Self {
        Self {
            index_begin: header.index_begin,
            hash_bucket_count: header.hash_bucket_count,
            hash_stream: header.hash_stream,
            hash_aux_stream: header.hash_aux_stream,
            layout: HashLayout::of(header),
            adjusters,
        }
    }
}

/// The bytes of a written TPI or IPI stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypeStreamBytes {
    /// The stream: the header and the records.
    pub stream: Vec<u8>,
    /// The hash stream, present if the options name a hash stream.
    pub hash_stream: Option<Vec<u8>>,
}

/// Return the error for a value that does not fit its field.
fn too_large(context: &'static str) -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context(context)
}

/// Return the error for an allocation that failed.
fn no_memory() -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context("type stream cannot be allocated")
}

/// Append a little-endian `u32`. The caller reserved the space.
fn put(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Return the index offset pairs of records with `sizes` under `layout`.
fn index_offsets(sizes: &[u32], index_begin: u32, layout: HashLayout) -> Result<Vec<IndexOffset>> {
    let mut pairs: Vec<IndexOffset> = Vec::new();
    let mut offset = 0_u32;
    for (index, &size) in (index_begin..).zip(sizes) {
        let end = offset
            .checked_add(size)
            .ok_or_else(|| too_large("type record bytes do not fit in 32 bits"))?;
        let add = match (layout, pairs.last()) {
            (_, None) => true,
            (HashLayout::Microsoft, Some(last)) => {
                last.offset / INDEX_OFFSET_BLOCK < offset / INDEX_OFFSET_BLOCK
            }
            (HashLayout::Llvm, Some(_)) => end / INDEX_OFFSET_BLOCK > offset / INDEX_OFFSET_BLOCK,
        };
        if add {
            pairs.try_reserve(1).map_err(|_| no_memory())?;
            pairs.push(IndexOffset { index, offset });
        }
        offset = end;
    }
    Ok(pairs)
}

/// Serialize hash adjusters as Microsoft `Map::save` does after `Map::add`
/// of each adjuster in order.
///
/// The map starts with one slot. A key goes to the first free slot from
/// the low 16 bits of the key modulo the slot count, with linear probing.
/// After an insertion, a map with `count >= slots * 2 / 3 + 1` entries
/// grows to `2 * (slots * 2 / 3 + 1)` slots, and the entries move in slot
/// order. The serialized map has the count, the slot count, the present
/// bit vector, an empty deleted bit vector and the entries in slot order.
/// A later key that equals an earlier key replaces its value.
///
/// # Errors
///
/// Returns [`ErrorKind::LimitExceeded`] if the map cannot be allocated.
pub fn serialize_hash_adjusters(adjusters: &[HashAdjuster]) -> Result<Vec<u8>> {
    let mut slots: Vec<Option<HashAdjuster>> = Vec::new();
    slots.try_reserve(1).map_err(|_| no_memory())?;
    slots.push(None);
    let mut count = 0_usize;
    for &adjuster in adjusters {
        if insert(&mut slots, adjuster) {
            count += 1;
            let load = slots.len() * 2 / 3 + 1;
            if count >= load {
                let mut grown = Vec::new();
                grown.try_reserve_exact(2 * load).map_err(|_| no_memory())?;
                grown.resize(2 * load, None);
                for entry in slots.iter().flatten() {
                    insert(&mut grown, *entry);
                }
                slots = grown;
            }
        }
    }
    let used = slots
        .iter()
        .rposition(Option::is_some)
        .map_or(0, |last| last / 32);
    let words = used + 1;
    // The count, the slot count, the two bit vector lengths and the words.
    let size = 4 * (4 + words) + 8 * count;
    let mut out = Vec::new();
    out.try_reserve_exact(size).map_err(|_| no_memory())?;
    let to_u32 = |value: usize| {
        u32::try_from(value).map_err(|_| too_large("hash adjuster map is too large"))
    };
    put(&mut out, to_u32(count)?);
    put(&mut out, to_u32(slots.len())?);
    put(&mut out, to_u32(words)?);
    for word in 0..words {
        let bits = slots
            .iter()
            .skip(word * 32)
            .take(32)
            .enumerate()
            .filter(|(_, slot)| slot.is_some())
            .fold(0_u32, |bits, (bit, _)| bits | 1 << bit);
        put(&mut out, bits);
    }
    put(&mut out, 0);
    for entry in slots.iter().flatten() {
        put(&mut out, entry.name_offset);
        put(&mut out, entry.index);
    }
    Ok(out)
}

/// Insert or replace an adjuster. Return `true` for a new key.
fn insert(slots: &mut [Option<HashAdjuster>], adjuster: HashAdjuster) -> bool {
    let count = slots.len();
    let start = usize::from(adjuster.name_offset.to_le_bytes()[0])
        | usize::from(adjuster.name_offset.to_le_bytes()[1]) << 8;
    let mut position = start % count;
    loop {
        match &mut slots[position] {
            Some(entry) if entry.name_offset == adjuster.name_offset => {
                entry.index = adjuster.index;
                return false;
            }
            Some(_) => position = (position + 1) % count,
            empty @ None => {
                *empty = Some(adjuster);
                return true;
            }
        }
    }
}

/// Write a TPI or IPI stream and its hash stream.
///
/// `records` holds the complete records in index order: each record with
/// its length field, its kind, its payload and its padding. The function
/// frames the records, computes their hash values, and writes the header,
/// the records and the hash stream of `options`.
///
/// # Errors
///
/// - [`ErrorKind::InvalidFormat`] if `records` is not a sequence of complete
///   records, if `options.index_begin` is below [`FIRST_RECORD_INDEX`], if
///   the bucket count is 0, or if the LLVM layout has hash adjusters.
/// - [`ErrorKind::LimitExceeded`] if a count or a size does not fit its
///   field, or if the output cannot be allocated.
/// - The errors of [`type_record_hash`].
pub fn write_type_stream(
    records: &[u8],
    options: &TypeStreamOptions<'_>,
) -> Result<TypeStreamBytes> {
    if options.index_begin < FIRST_RECORD_INDEX || options.hash_bucket_count == 0 {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("type stream options are not valid"));
    }
    if options.layout == HashLayout::Llvm && !options.adjusters.is_empty() {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("the LLVM hash layout has no hash adjusters"));
    }
    let record_bytes = u32::try_from(records.len())
        .map_err(|_| too_large("type record bytes do not fit in 32 bits"))?;
    let mut sizes = Vec::new();
    let mut hashes = Vec::new();
    let mut framing = Records::new(records);
    let mut start = 0;
    while let Some(record) = framing.next() {
        record?;
        let end = framing.position();
        sizes.try_reserve(1).map_err(|_| no_memory())?;
        hashes.try_reserve(1).map_err(|_| no_memory())?;
        // `end - start` is at most 65537.
        sizes.push(u32::try_from(end - start).unwrap_or(u32::MAX));
        hashes.push(type_record_hash(&records[start..end])? % options.hash_bucket_count);
        start = end;
    }
    let count = u32::try_from(sizes.len())
        .map_err(|_| too_large("type record count does not fit in 32 bits"))?;
    let index_end = options
        .index_begin
        .checked_add(count)
        .ok_or_else(|| too_large("type index end does not fit in 32 bits"))?;
    let hash_stream = match options.hash_stream {
        Some(_) => Some(hash_stream(&sizes, &hashes, options)?),
        None => None,
    };
    let buffers = match (&hash_stream, options.layout) {
        (Some((_, buffers)), _) => *buffers,
        (None, HashLayout::Microsoft) => {
            [HashBuffer {
                offset: 0,
                length: u32::MAX,
            }; 3]
        }
        (None, HashLayout::Llvm) => {
            [HashBuffer {
                offset: 0,
                length: 0,
            }; 3]
        }
    };
    let stream_id = |id: Option<StreamId>| -> Result<u16> {
        id.map_or(Ok(NIL_STREAM), |id| {
            u16::try_from(id.0)
                .ok()
                .filter(|&value| value != NIL_STREAM)
                .ok_or_else(|| too_large("hash stream index does not fit in 16 bits"))
        })
    };
    let mut stream = Vec::new();
    stream
        .try_reserve_exact(HEADER_SIZE + records.len())
        .map_err(|_| no_memory())?;
    for value in [
        TPI_V80,
        // `HEADER_SIZE` is 56.
        u32::try_from(HEADER_SIZE).unwrap_or(56),
        options.index_begin,
        index_end,
        record_bytes,
    ] {
        put(&mut stream, value);
    }
    stream.extend_from_slice(&stream_id(options.hash_stream)?.to_le_bytes());
    stream.extend_from_slice(&stream_id(options.hash_aux_stream)?.to_le_bytes());
    put(&mut stream, HASH_KEY_SIZE);
    put(&mut stream, options.hash_bucket_count);
    for buffer in buffers {
        stream.extend_from_slice(&buffer.offset.to_le_bytes());
        put(&mut stream, buffer.length);
    }
    stream.extend_from_slice(records);
    Ok(TypeStreamBytes {
        stream,
        hash_stream: hash_stream.map(|(bytes, _)| bytes),
    })
}

/// Build the hash stream and the header buffers: values, index offsets and
/// adjusters, in header order.
fn hash_stream(
    sizes: &[u32],
    hashes: &[u32],
    options: &TypeStreamOptions<'_>,
) -> Result<(Vec<u8>, [HashBuffer; 3])> {
    let pairs = index_offsets(sizes, options.index_begin, options.layout)?;
    let adjusters = if options.adjusters.is_empty() {
        Vec::new()
    } else {
        serialize_hash_adjusters(options.adjusters)?
    };
    let length = |bytes: usize| {
        u32::try_from(bytes).map_err(|_| too_large("type hash buffer does not fit in 32 bits"))
    };
    let values_length = length(hashes.len() * 4)?;
    let pairs_length = length(pairs.len() * 8)?;
    let adjusters_length = length(adjusters.len())?;
    let offset = |value: u32| {
        i32::try_from(value)
            .map_err(|_| too_large("type hash buffer offset does not fit in 31 bits"))
    };
    let values = HashBuffer {
        offset: 0,
        length: values_length,
    };
    let (index_offsets, hash_adjustments) = match options.layout {
        HashLayout::Microsoft => {
            let pairs_end = values_length
                .checked_add(pairs_length)
                .ok_or_else(|| too_large("type hash stream does not fit in 32 bits"))?;
            (
                HashBuffer {
                    offset: offset(values_length)?,
                    length: pairs_length,
                },
                HashBuffer {
                    offset: offset(pairs_end)?,
                    length: adjusters_length,
                },
            )
        }
        HashLayout::Llvm => (
            HashBuffer {
                offset: offset(values_length)?,
                length: pairs_length,
            },
            HashBuffer {
                offset: offset(values_length)?,
                length: 0,
            },
        ),
    };
    let total = hashes.len() * 4 + pairs.len() * 8 + adjusters.len();
    length(total)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(total).map_err(|_| no_memory())?;
    for &hash in hashes {
        put(&mut bytes, hash);
    }
    for pair in &pairs {
        put(&mut bytes, pair.index);
        put(&mut bytes, pair.offset);
    }
    bytes.extend_from_slice(&adjusters);
    Ok((bytes, [values, index_offsets, hash_adjustments]))
}
