//! The hash stream of a TPI or IPI stream.
//!
//! [`TypeHeader`] gives three buffers in the hash stream:
//!
//! - The hash values: one `u32` for each record, already reduced modulo
//!   [`TypeHeader::hash_bucket_count`].
//! - The index offsets: sorted pairs of a record index and the offset of that
//!   record in the record bytes. Microsoft writes about one pair for each
//!   8 KiB of records.
//! - The hash adjusters: a serialized hash table (`HashTable.md` in LLVM)
//!   whose keys are `/names` offsets of UDT names and whose values are record
//!   indices. Microsoft `tpi.cpp` (`mpnitiHead`) uses it to put one UDT at
//!   the head of a hash chain. LLVM `TpiStream.md` calls the keys hash
//!   values, but `tpi.cpp` and LLVM `DumpOutputStyle.cpp` read them as
//!   string table offsets.
//!
//! [`type_record_hash`] computes the record hash of LLVM `hashTypeRecord` in
//! `TpiHashing.cpp`.

use alloc::vec::Vec;

use super::{TypeHeader, TypeRecord, TypeStream};
use crate::native::codeview::id_record::{LF_UDT_MOD_SRC_LINE, LF_UDT_SRC_LINE};
use crate::native::codeview::index::NameOffset;
use crate::native::codeview::type_record::{
    LF_CLASS, LF_CLASS2, LF_ENUM, LF_INTERFACE, LF_INTERFACE2, LF_STRUCTURE, LF_STRUCTURE2,
    LF_UNION, LF_UNION2, Tag, TypeLeaf,
};
use crate::native::codeview::{Fields, LeafKind, Records};
use crate::native::hash::{hash_buffer_v8, hash_string_v1};
use crate::native::msf::StreamId;
use crate::{Error, ErrorKind, Result};

/// The size of one hash value.
const VALUE_SIZE: usize = 4;
/// The size of one index offset pair.
const PAIR_SIZE: usize = 8;

/// One entry of the index offset buffer.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct IndexOffset {
    /// The record index: a type index in TPI, an ID index in IPI.
    pub index: u32,
    /// The byte offset of the record in the record bytes after the header.
    pub offset: u32,
}

/// One entry of the hash adjuster table.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct HashAdjuster {
    /// The offset of the UDT name in the `/names` string table.
    pub name_offset: NameOffset,
    /// The record index of the UDT that is at the head of its hash chain.
    pub index: u32,
}

/// The decoded hash stream of one TPI or IPI stream.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TypeHashes<'a> {
    /// The hash stream, for error locations.
    stream: StreamId,
    /// The number of hash buckets.
    bucket_count: u32,
    /// The index of the first record.
    index_begin: u32,
    /// The index after the last record.
    index_end: u32,
    /// The offset of the hash value buffer in the hash stream.
    values_offset: u32,
    /// The offset of the index offset buffer in the hash stream.
    index_offsets_offset: u32,
    /// The hash value buffer.
    values: &'a [u8],
    /// The index offset buffer.
    index_offsets: &'a [u8],
    /// The decoded hash adjusters, in bucket order.
    adjusters: Vec<HashAdjuster>,
}

/// Return an error of `kind` with `context` at `offset` in the hash stream.
fn error(kind: ErrorKind, context: &'static str, stream: StreamId, offset: u32) -> Error {
    Error::new(kind, None)
        .with_context(context)
        .with_stream(stream, u64::from(offset))
}

/// Return the value of the `u32` at `position` in `bytes`, if present.
fn word(bytes: &[u8], position: usize) -> Option<u32> {
    let end = position.checked_add(4)?;
    let chunk = bytes.get(position..end)?;
    Some(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
}

impl<'a> TypeHashes<'a> {
    /// Decode the hash stream `bytes` of a type stream with `header`.
    ///
    /// This function checks the structure of the buffers. Call
    /// [`TypeHashes::check`] to check the buffers against the records.
    ///
    /// # Errors
    ///
    /// The errors have the hash stream and a hash stream offset.
    ///
    /// - [`ErrorKind::MissingStream`] if the header has no hash stream.
    /// - [`ErrorKind::InvalidReference`] if a buffer is outside the stream,
    ///   or if an index offset pair is outside the records.
    /// - [`ErrorKind::InvalidFormat`] if the buffers overlap or leave gaps,
    ///   if a hash value is not below the bucket count, if the index offset
    ///   pairs are not sorted, or if the hash adjuster table is malformed.
    /// - [`ErrorKind::LimitExceeded`] if the adjusters cannot be allocated.
    pub fn parse(header: &TypeHeader, bytes: &'a [u8]) -> Result<Self> {
        let stream = header.hash_stream.ok_or_else(|| {
            Error::new(ErrorKind::MissingStream, None)
                .with_context("type stream header has no hash stream")
        })?;
        let buffers = [
            header.hash_values,
            header.index_offsets,
            header.hash_adjustments,
        ]
        .map(|buffer| {
            // `TypeStream::parse` validated the offset of a non-empty buffer.
            let start = u32::try_from(buffer.offset).unwrap_or(0);
            (start, buffer.length)
        });
        check_layout(stream, bytes, buffers)?;
        let slice = |(start, length): (u32, u32)| -> &'a [u8] {
            if length == 0 {
                return &[];
            }
            // `check_layout` made sure that the range is inside `bytes`.
            &bytes[start as usize..start as usize + length as usize]
        };
        let hashes = Self {
            stream,
            bucket_count: header.hash_bucket_count,
            index_begin: header.index_begin,
            index_end: header.index_end,
            values_offset: buffers[0].0,
            index_offsets_offset: buffers[1].0,
            values: slice(buffers[0]),
            index_offsets: slice(buffers[1]),
            adjusters: parse_adjusters(stream, buffers[2].0, slice(buffers[2]))?,
        };
        hashes.check_values()?;
        hashes.check_index_offsets(header, buffers[1].0)?;
        Ok(hashes)
    }

    /// Return the number of hash buckets.
    #[must_use]
    pub const fn bucket_count(&self) -> u32 {
        self.bucket_count
    }

    /// Return the stored hash values in record order.
    ///
    /// The iterator is empty if the stream stores no hash values.
    #[must_use]
    pub fn hash_values(&self) -> HashValues<'a> {
        HashValues(self.values.as_chunks::<VALUE_SIZE>().0.iter())
    }

    /// Return the stored hash value of the record with a raw record index.
    #[must_use]
    pub fn hash_value(&self, index: u32) -> Option<u32> {
        let position = usize::try_from(index.checked_sub(self.index_begin)?).ok()?;
        word(self.values, position.checked_mul(VALUE_SIZE)?)
    }

    /// Return the index offset pairs in index order.
    #[must_use]
    pub fn index_offsets(&self) -> IndexOffsets<'a> {
        IndexOffsets(self.index_offsets.as_chunks::<PAIR_SIZE>().0.iter())
    }

    /// Decode an index offset pair.
    fn index_offset(chunk: [u8; PAIR_SIZE]) -> IndexOffset {
        {
            IndexOffset {
                index: u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
                offset: u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]),
            }
        }
    }

    /// Return the hash adjusters in bucket order.
    #[must_use]
    pub fn adjusters(&self) -> &[HashAdjuster] {
        &self.adjusters
    }

    /// Make sure that each stored hash value is below the bucket count.
    fn check_values(&self) -> Result<()> {
        let mut offset = self.values_offset;
        for value in self.hash_values() {
            if value >= self.bucket_count {
                return Err(error(
                    ErrorKind::InvalidFormat,
                    "type hash value is not below the bucket count",
                    self.stream,
                    offset,
                ));
            }
            offset = offset.wrapping_add(4);
        }
        Ok(())
    }

    /// Make sure that the pairs are sorted and inside the records.
    fn check_index_offsets(&self, header: &TypeHeader, start: u32) -> Result<()> {
        check_index_offsets(header, self.index_offsets(), self.stream, start)
    }

    /// Check the buffers against the records of `types`.
    ///
    /// The function makes sure of these conditions:
    ///
    /// - `types` has the index range and the bucket count of this hash stream.
    /// - Each stored hash value is the [`type_record_hash`] of its record,
    ///   modulo the bucket count.
    /// - Each index offset pair gives the offset of the record with its index.
    /// - Each hash adjuster refers to a record of the stream.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if `types` does not match, or if a
    ///   hash adjuster refers to no record.
    /// - [`ErrorKind::InvalidFormat`] if a hash value or an index offset pair
    ///   does not match its record.
    /// - The errors of [`TypeStream::records`] and [`type_record_hash`].
    pub fn check(&self, types: &TypeStream) -> Result<()> {
        let header = types.header();
        if (
            header.index_begin,
            header.index_end,
            header.hash_bucket_count,
        ) != (self.index_begin, self.index_end, self.bucket_count)
        {
            return Err(error(
                ErrorKind::InvalidReference,
                "type hash stream belongs to another type stream",
                self.stream,
                0,
            ));
        }
        let mut values = self.hash_values();
        let mut pairs = (0_u32..).zip(self.index_offsets()).peekable();
        for (position, record) in (0_u32..).zip(types.records()) {
            let record = record?;
            if let Some(stored) = values.next() {
                let computed = type_record_hash(types.full_record(&record.record)?)?;
                if computed % self.bucket_count != stored {
                    return Err(error(
                        ErrorKind::InvalidFormat,
                        "type hash value does not match its record",
                        self.stream,
                        self.values_offset.wrapping_add(position.wrapping_mul(4)),
                    ));
                }
            }
            if let Some((pair_position, pair)) =
                pairs.next_if(|(_, pair)| pair.index == record.index)
                && pair.offset != record.record.offset
            {
                return Err(error(
                    ErrorKind::InvalidFormat,
                    "type index offset pair does not point to its record",
                    self.stream,
                    self.index_offsets_offset
                        .wrapping_add(pair_position.wrapping_mul(8)),
                ));
            }
        }
        for adjuster in &self.adjusters {
            if !(self.index_begin..self.index_end).contains(&adjuster.index) {
                return Err(Error::new(ErrorKind::InvalidReference, None)
                    .with_context("type hash adjuster refers to no record"));
            }
        }
        Ok(())
    }

    /// Find a record with the index offset buffer.
    ///
    /// The function starts at the last pair with an index that is not
    /// greater than `index`, and frames records from there. Without pairs,
    /// it starts at the first record. The result is correct only for the
    /// type stream of this hash stream; [`TypeHashes::check`] checks that.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if `index` is outside the records.
    /// - The framing errors of [`TypeStream::records`].
    pub fn locate<'s>(&self, types: &'s TypeStream, index: u32) -> Result<TypeRecord<'s>> {
        if !(self.index_begin..self.index_end).contains(&index) {
            return Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("record index is outside the type stream"));
        }
        let pairs = self.index_offsets.as_chunks::<PAIR_SIZE>().0;
        let after =
            pairs.partition_point(|chunk| word(chunk, 0).is_some_and(|first| first <= index));
        let start = after
            .checked_sub(1)
            .and_then(|position| pairs.get(position))
            .map_or(
                IndexOffset {
                    index: self.index_begin,
                    offset: 0,
                },
                |chunk| IndexOffset {
                    index: word(chunk, 0).unwrap_or(self.index_begin),
                    offset: word(chunk, 4).unwrap_or(0),
                },
            );
        let rest = types
            .record_bytes()
            .get(start.offset as usize..)
            .unwrap_or_default();
        let skip = (index - start.index) as usize;
        let record = Records::new(rest).nth(skip).ok_or_else(|| {
            Error::new(ErrorKind::InvalidFormat, None)
                .with_context("type index offset pair is past the record")
        })??;
        let offset = start.offset.checked_add(record.offset).ok_or_else(|| {
            Error::new(ErrorKind::OffsetOverflow, None)
                .with_context("type record offset does not fit in 32 bits")
        })?;
        types.record_at(offset, index)
    }
}

/// Make sure that the index offset `pairs` of `header` are sorted and inside the records.
///
/// `start` is the offset of the first pair in the hash `stream`, for the error location.
pub(super) fn check_index_offsets(
    header: &TypeHeader,
    pairs: impl IntoIterator<Item = IndexOffset>,
    stream: StreamId,
    start: u32,
) -> Result<()> {
    let mut previous: Option<IndexOffset> = None;
    let mut offset = start;
    for pair in pairs {
        if !(header.index_begin..header.index_end).contains(&pair.index)
            || pair.offset >= header.record_bytes
        {
            return Err(error(
                ErrorKind::InvalidReference,
                "type index offset pair is outside the records",
                stream,
                offset,
            ));
        }
        if previous.is_some_and(|last| last.index >= pair.index || last.offset >= pair.offset) {
            return Err(error(
                ErrorKind::InvalidFormat,
                "type index offset pairs are not sorted",
                stream,
                offset,
            ));
        }
        previous = Some(pair);
        offset = offset.wrapping_add(8);
    }
    Ok(())
}

/// Return the index offset pairs of an index offset buffer. A partial pair at the end is
/// ignored.
pub(super) fn index_offset_pairs(bytes: &[u8]) -> IndexOffsets<'_> {
    IndexOffsets(bytes.as_chunks::<PAIR_SIZE>().0.iter())
}

/// Make sure that the non-empty buffers cover `bytes` without gaps or overlaps.
fn check_layout(stream: StreamId, bytes: &[u8], buffers: [(u32, u32); 3]) -> Result<()> {
    let mut ranges = buffers.map(|(start, length)| (u64::from(start), u64::from(length)));
    ranges.sort_unstable();
    let mut end = 0_u64;
    for (start, length) in ranges {
        if length == 0 {
            continue;
        }
        if start != end {
            return Err(error(
                ErrorKind::InvalidFormat,
                "type hash buffers overlap or leave a gap",
                stream,
                u32::try_from(start).unwrap_or(u32::MAX),
            ));
        }
        end = start + length;
    }
    match u64::try_from(bytes.len()) {
        Ok(size) if size == end => Ok(()),
        Ok(size) if size > end => Err(error(
            ErrorKind::InvalidFormat,
            "type hash stream has bytes after its buffers",
            stream,
            u32::try_from(end).unwrap_or(u32::MAX),
        )),
        _ => Err(error(
            ErrorKind::InvalidReference,
            "type hash buffer is outside the hash stream",
            stream,
            u32::try_from(bytes.len()).unwrap_or(u32::MAX),
        )),
    }
}

/// A reader of `u32` values in the hash adjuster buffer.
struct Words<'a> {
    /// The buffer.
    bytes: &'a [u8],
    /// The position of the next value.
    position: usize,
    /// The hash stream, for error locations.
    stream: StreamId,
    /// The offset of the buffer in the hash stream.
    base: u32,
}

impl<'a> Words<'a> {
    /// Return an error at the current position.
    fn error(&self, kind: ErrorKind, context: &'static str) -> Error {
        let offset = u32::try_from(self.position)
            .ok()
            .and_then(|position| self.base.checked_add(position))
            .unwrap_or(u32::MAX);
        error(kind, context, self.stream, offset)
    }

    /// Read the next `u32`.
    fn next(&mut self) -> Result<u32> {
        let value = word(self.bytes, self.position).ok_or_else(|| {
            self.error(
                ErrorKind::UnexpectedEof,
                "type hash adjuster table is truncated",
            )
        })?;
        self.position += 4;
        Ok(value)
    }

    /// Read a bit vector: a `u32` word count and the words.
    fn bit_vector(&mut self) -> Result<&'a [u8]> {
        let count = usize::try_from(self.next()?).unwrap_or(usize::MAX);
        let length = count.saturating_mul(4);
        let end = self.position.saturating_add(length);
        let words = self.bytes.get(self.position..end).ok_or_else(|| {
            self.error(
                ErrorKind::UnexpectedEof,
                "type hash adjuster table is truncated",
            )
        })?;
        self.position = end;
        Ok(words)
    }
}

/// Return the positions of the set bits in a bit vector.
fn set_bits(words: &[u8]) -> impl Iterator<Item = u64> + '_ {
    (0_u64..)
        .zip(words.as_chunks::<4>().0)
        .flat_map(|(word, chunk)| {
            let value = u32::from_le_bytes(*chunk);
            (0..32_u64)
                .filter(move |bit| value >> bit & 1 != 0)
                .map(move |bit| word * 32 + bit)
        })
}

/// Decode the serialized hash adjuster table (`HashTable::load` in LLVM).
fn parse_adjusters(stream: StreamId, base: u32, bytes: &[u8]) -> Result<Vec<HashAdjuster>> {
    let mut adjusters = Vec::new();
    if bytes.is_empty() {
        return Ok(adjusters);
    }
    let mut words = Words {
        bytes,
        position: 0,
        stream,
        base,
    };
    let size = words.next()?;
    let capacity = words.next()?;
    if capacity == 0 || u64::from(size) > u64::from(capacity) * 2 / 3 + 1 {
        return Err(words.error(
            ErrorKind::InvalidFormat,
            "type hash adjuster table size or capacity is not valid",
        ));
    }
    let present = words.bit_vector()?;
    let deleted = words.bit_vector()?;
    let mut count = 0_u64;
    for bit in set_bits(present) {
        count += 1;
        if bit >= u64::from(capacity) {
            return Err(words.error(
                ErrorKind::InvalidFormat,
                "type hash adjuster bucket is past the capacity",
            ));
        }
    }
    let overlap = present
        .as_chunks::<4>()
        .0
        .iter()
        .zip(deleted.as_chunks::<4>().0)
        .any(|(left, right)| u32::from_le_bytes(*left) & u32::from_le_bytes(*right) != 0);
    if count != u64::from(size) || overlap {
        return Err(words.error(
            ErrorKind::InvalidFormat,
            "type hash adjuster bit vectors do not match the size",
        ));
    }
    // `count` equals `size`, and the buffer holds 8 bytes for each entry.
    let entries = usize::try_from(size).unwrap_or(usize::MAX);
    if entries > (bytes.len() - words.position) / 8 {
        return Err(words.error(
            ErrorKind::UnexpectedEof,
            "type hash adjuster table is truncated",
        ));
    }
    adjusters.try_reserve_exact(entries).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("type hash adjusters cannot be allocated")
    })?;
    for _ in 0..entries {
        let name_offset = NameOffset(words.next()?);
        let index = words.next()?;
        adjusters.push(HashAdjuster { name_offset, index });
    }
    if words.position != bytes.len() {
        return Err(words.error(
            ErrorKind::InvalidFormat,
            "type hash adjuster table has bytes after its entries",
        ));
    }
    Ok(adjusters)
}

/// Return `true` for an anonymous UDT name (`fUDTAnon` in Microsoft `tpi.cpp`).
fn is_anonymous(name: &[u8]) -> bool {
    [b"<unnamed-tag>".as_slice(), b"__unnamed"]
        .iter()
        .any(|anonymous| {
            name == *anonymous
                || name
                    .strip_suffix(*anonymous)
                    .is_some_and(|scope| scope.ends_with(b"::"))
        })
}

/// Return the record hash of a UDT (`getHashForUdt` in LLVM `TpiHashing.cpp`).
///
/// A complete type that is not scoped and not anonymous hashes its name. A
/// scoped complete type with a unique name hashes the unique name. Forward
/// references and anonymous types hash the full record bytes.
pub(super) fn udt_hash(tag: &Tag<'_>, full: &[u8]) -> u32 {
    let forward = tag.options.is_forward_reference();
    let anonymous = tag.unique_name.is_some() && is_anonymous(tag.name);
    if !forward && !tag.options.is_scoped() && !anonymous {
        return hash_string_v1(tag.name);
    }
    match tag.unique_name {
        Some(unique_name) if !forward && !anonymous => hash_string_v1(unique_name),
        _ => hash_buffer_v8(full),
    }
}

/// Return the name hash of a UDT (`hashTagRecord` in LLVM `TpiHashing.cpp`).
///
/// A forward reference hashes the unique name if it is scoped, otherwise the
/// name. A complete type gives its [`udt_hash`]. A forward reference and its
/// definition then have the same value.
pub(super) fn tag_hash(tag: &Tag<'_>, full: &[u8]) -> u32 {
    if !tag.options.is_forward_reference() {
        return udt_hash(tag, full);
    }
    if tag.options.is_scoped() {
        hash_string_v1(tag.unique_name.unwrap_or_default())
    } else {
        hash_string_v1(tag.name)
    }
}

/// Return `true` for the UDT kinds: the normal and the wide forms of
/// classes, structures, interfaces and unions, and enumerations.
pub(super) const fn is_udt_kind(kind: LeafKind) -> bool {
    matches!(
        kind,
        LF_CLASS
            | LF_STRUCTURE
            | LF_INTERFACE
            | LF_UNION
            | LF_ENUM
            | LF_CLASS2
            | LF_STRUCTURE2
            | LF_INTERFACE2
            | LF_UNION2
    )
}

/// Return the hash of a UDT source-line record (`getSourceLineHash` in LLVM).
///
/// The payload is a UDT index, a source file ID and a line number, and for
/// `LF_UDT_MOD_SRC_LINE` a `u16` module index (`TypeRecordMapping.cpp`).
/// This function checks only the size of the payload and its padding. It
/// hashes the 4 bytes of the UDT index with [`hash_string_v1`].
fn source_line_hash(kind: LeafKind, data: &[u8]) -> Result<u32> {
    let mut fields = Fields::new(
        data,
        ErrorKind::InvalidFormat,
        "UDT source-line record is truncated",
    );
    let udt = fields.bytes(4)?;
    fields.bytes(if kind == LF_UDT_MOD_SRC_LINE { 10 } else { 8 })?;
    fields.finish_type_padding()?;
    Ok(hash_string_v1(udt))
}

/// Return the record hash of one complete TPI or IPI record.
///
/// `bytes` holds the length field, the kind field and the payload with its
/// padding, for example from [`TypeStream::full_record`]. The value is not
/// reduced modulo the bucket count. The rules follow LLVM `hashTypeRecord`:
///
/// - A UDT gives its name hash, its unique name hash, or the CRC of `bytes`.
///   The wide forms `LF_CLASS2`, `LF_STRUCTURE2`, `LF_INTERFACE2` and
///   `LF_UNION2` are UDTs too.
/// - `LF_UDT_SRC_LINE` and `LF_UDT_MOD_SRC_LINE` hash the 4 bytes of their
///   UDT index with [`hash_string_v1`].
/// - All other records give [`hash_buffer_v8`] of `bytes`.
///
/// # Errors
///
/// - [`ErrorKind::InvalidFormat`] if `bytes` is not exactly one record, or if
///   the payload of a UDT or a source-line record is malformed.
/// - [`ErrorKind::UnsupportedVersion`] if a UDT has a numeric leaf with an
///   unknown tag.
pub fn type_record_hash(bytes: &[u8]) -> Result<u32> {
    let mut records = Records::new(bytes);
    let record = match (records.next(), records.position() == bytes.len()) {
        (Some(Ok(record)), true) => record,
        (Some(Err(error)), _) => return Err(error),
        _ => {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("type hash input is not one complete record"));
        }
    };
    match LeafKind(record.kind) {
        kind if is_udt_kind(kind) => {
            match TypeLeaf::decode(record)?.tag() {
                Some(tag) => Ok(udt_hash(&tag, bytes)),
                None => Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("UDT record has no name")),
            }
        }
        kind @ (LF_UDT_SRC_LINE | LF_UDT_MOD_SRC_LINE) => source_line_hash(kind, record.data),
        _ => Ok(hash_buffer_v8(bytes)),
    }
}

/// An iterator over the stored hash values of a [`TypeHashes`] value.
#[derive(Clone, Debug)]
pub struct HashValues<'a>(core::slice::Iter<'a, [u8; VALUE_SIZE]>);

impl Iterator for HashValues<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        self.0.next().map(|x| u32::from_le_bytes(*x))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl ExactSizeIterator for HashValues<'_> {}

impl core::iter::FusedIterator for HashValues<'_> {}

/// An iterator over the index offset pairs of a [`TypeHashes`] value, in index order.
#[derive(Clone, Debug)]
pub struct IndexOffsets<'a>(core::slice::Iter<'a, [u8; PAIR_SIZE]>);

impl Iterator for IndexOffsets<'_> {
    type Item = IndexOffset;

    fn next(&mut self) -> Option<IndexOffset> {
        self.0.next().map(|x| TypeHashes::index_offset(*x))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl ExactSizeIterator for IndexOffsets<'_> {}

impl core::iter::FusedIterator for IndexOffsets<'_> {}
