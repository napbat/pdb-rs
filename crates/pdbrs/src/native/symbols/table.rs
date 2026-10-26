//! The name hash table that the global and public symbol indexes share.
//!
//! The table has four parts:
//!
//! 1. A 16-byte header (`GSIHashHdr`): signature, version, the size of the
//!    hash records and the size of the bucket data.
//! 2. The hash records (`HRFile`), 8 bytes each: the symbol offset plus 1 and
//!    a reference count. The records are grouped by bucket.
//! 3. A presence bitmap of `bucket count + 1` bits, stored in little-endian
//!    32-bit words. Bit `b % 32` of word `b / 32` is set if bucket `b` has
//!    records. The last bit is the free-list bucket, which is always empty.
//! 4. One 32-bit start offset for each set bit, in bucket order. The offset
//!    is the index of the first hash record of the bucket, multiplied by 12.
//!
//! The factor 12 is the size of the 32-bit in-memory record `HROffsetCalc`
//! (`PDB/dbi/gsi.h`), not the 8-byte file record. A bucket ends where the next
//! non-empty bucket starts, or at the end of the hash records. A writer with
//! no hash records can also omit the bitmap: `GSI1::CompressBuckets` in
//! `PDB/dbi/gsi.cpp` writes no bucket data then. LLVM `GSIStreamBuilder.cpp`
//! writes an empty bitmap instead. This module accepts both forms.

use alloc::vec::Vec;
use core::iter::FusedIterator;
use core::ops::Range;
use core::slice::Iter;

use super::{BucketCount, RecordOffsets, SymbolOffset, index_error, region, u32_at};
use crate::native::msf::StreamId;
use crate::{ErrorKind, Result};

pub(super) mod write;

/// The size of the hash table header.
const HEADER_SIZE: usize = 16;
/// The size of one hash record in the file (`HRFile`).
const RECORD_SIZE: usize = 8;
/// The record size that scales the serialized bucket offsets (`HROffsetCalc`).
const OFFSET_SCALE: u32 = 12;
/// The signature of the hash header with compressed buckets.
const SIGNATURE: u32 = u32::MAX;
/// The version of the hash header with compressed buckets (`GSIHashSCImpvV70`).
const VERSION: u32 = 0xeffe_0000 + 19_990_810;
/// The largest value of a signed 32-bit field.
const MAX_SIGNED: u32 = 0x7fff_ffff;

/// The decoded fields of a hash table header (`GSIHashHdr`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HashHeader {
    /// The header signature. It is always `0xffffffff`.
    pub signature: u32,
    /// The header version. It is always `0xf12f091a` (`0xeffe0000 + 19990810`).
    pub version: u32,
    /// The size of the hash records in bytes.
    pub records_size: u32,
    /// The size of the bucket bitmap and the bucket offsets in bytes.
    pub buckets_size: u32,
}

/// The validated location of a hash table in its stream.
#[derive(Clone, Debug)]
pub(super) struct Layout {
    /// The index stream that holds the table.
    stream: StreamId,
    /// The decoded header.
    header: HashHeader,
    /// The bucket count that the table uses.
    buckets: BucketCount,
    /// The hash records in the stream.
    records: Range<usize>,
    /// The bucket bitmap in the stream. It is empty if the table has no bucket data.
    bitmap: Range<usize>,
    /// The bucket offsets in the stream.
    offsets: Range<usize>,
    /// The number of set bitmap bits before each bitmap word.
    ranks: Vec<u32>,
}

impl Layout {
    /// Decode and check the hash table at `table` in the stream `bytes`.
    ///
    /// The table must fill `table` exactly. Errors identify `stream` and a
    /// logical offset in it.
    pub(super) fn parse(
        bytes: &[u8],
        table: Range<usize>,
        stream: StreamId,
        buckets: BucketCount,
    ) -> Result<Self> {
        let error = |kind, offset, context| index_error(kind, stream, offset, context);
        let start = table.start;
        if table.len() < HEADER_SIZE {
            return Err(error(
                ErrorKind::UnexpectedEof,
                table.end,
                "symbol hash header is truncated",
            ));
        }
        let header = HashHeader {
            signature: u32_at(bytes, start),
            version: u32_at(bytes, start + 4),
            records_size: u32_at(bytes, start + 8),
            buckets_size: u32_at(bytes, start + 12),
        };
        if header.signature != SIGNATURE {
            return Err(error(
                ErrorKind::UnsupportedVersion,
                start,
                "symbol hash header does not have the compressed-bucket signature",
            ));
        }
        if header.version != VERSION {
            return Err(error(
                ErrorKind::UnsupportedVersion,
                start + 4,
                "symbol hash header version is not V70",
            ));
        }
        if !(header.records_size as usize).is_multiple_of(RECORD_SIZE) {
            return Err(error(
                ErrorKind::InvalidFormat,
                start + 8,
                "symbol hash record size is not a multiple of 8",
            ));
        }
        let records_start = start + HEADER_SIZE;
        let records = region(records_start, header.records_size, table.end)
            .map_err(|kind| error(kind, start + 8, "symbol hash records extend past the table"))?;
        let bucket_data = region(records.end, header.buckets_size, table.end).map_err(|kind| {
            error(
                kind,
                start + 12,
                "symbol hash buckets extend past the table",
            )
        })?;
        if bucket_data.end != table.end {
            return Err(error(
                ErrorKind::InvalidFormat,
                bucket_data.end,
                "symbol hash table has bytes after the buckets",
            ));
        }
        check_records(bytes, records.clone(), stream)?;
        let count = records.len() / RECORD_SIZE;
        let (bitmap, offsets, ranks) = parse_buckets(bytes, bucket_data, count, stream, buckets)?;
        Ok(Self {
            stream,
            header,
            buckets,
            records,
            bitmap,
            offsets,
            ranks,
        })
    }

    /// Return a view of the table over the stream `bytes`.
    pub(super) const fn view<'a>(&'a self, bytes: &'a [u8]) -> HashTable<'a> {
        HashTable {
            bytes,
            layout: self,
        }
    }
}

/// Check the symbol offset and the reference count of each hash record.
fn check_records(bytes: &[u8], records: Range<usize>, stream: StreamId) -> Result<()> {
    for position in records.step_by(RECORD_SIZE) {
        let offset = u32_at(bytes, position);
        if offset == 0 || offset > MAX_SIGNED {
            return Err(index_error(
                ErrorKind::InvalidReference,
                stream,
                position,
                "symbol hash record offset is not a positive 32-bit value",
            ));
        }
        let references = u32_at(bytes, position + 4);
        if references == 0 || references > MAX_SIGNED {
            return Err(index_error(
                ErrorKind::InvalidFormat,
                stream,
                position + 4,
                "symbol hash record reference count is not positive",
            ));
        }
    }
    Ok(())
}

/// The bitmap range, the offset range and the bitmap ranks of a table.
type BucketParts = (Range<usize>, Range<usize>, Vec<u32>);

/// Decode and check the bucket bitmap and the bucket offsets.
fn parse_buckets(
    bytes: &[u8],
    data: Range<usize>,
    records: usize,
    stream: StreamId,
    buckets: BucketCount,
) -> Result<BucketParts> {
    let error = |kind, offset, context| index_error(kind, stream, offset, context);
    if data.is_empty() {
        if records != 0 {
            return Err(error(
                ErrorKind::InvalidFormat,
                data.start,
                "symbol hash records have no buckets",
            ));
        }
        return Ok((data.clone(), data, Vec::new()));
    }
    let bucket_count = buckets.get() as usize;
    let words = (bucket_count + 1).div_ceil(32);
    let bitmap = data.start..data.start + words * 4;
    if bitmap.end > data.end {
        return Err(error(
            ErrorKind::UnexpectedEof,
            data.end,
            "symbol hash bucket bitmap is truncated",
        ));
    }
    let mut ranks = Vec::new();
    ranks.try_reserve_exact(words).map_err(|_| {
        error(
            ErrorKind::LimitExceeded,
            data.start,
            "symbol hash bucket ranks cannot be allocated",
        )
    })?;
    let mut nonempty = 0_u32;
    for (first_bit, position) in (0_u32..).step_by(32).zip(bitmap.clone().step_by(4)) {
        let word = u32_at(bytes, position);
        let valid = buckets.get().saturating_sub(first_bit).min(32);
        let mask = u32::MAX.checked_shr(32 - valid).unwrap_or(0);
        if word & !mask != 0 {
            return Err(error(
                ErrorKind::InvalidFormat,
                position,
                "symbol hash bucket bitmap marks a bucket past the last bucket",
            ));
        }
        ranks.push(nonempty);
        nonempty += word.count_ones();
    }
    let expected = bitmap.end + nonempty as usize * 4;
    if expected != data.end {
        let kind = if expected > data.end {
            ErrorKind::UnexpectedEof
        } else {
            ErrorKind::InvalidFormat
        };
        return Err(error(
            kind,
            bitmap.end,
            "symbol hash bucket offset count does not match the bitmap",
        ));
    }
    if records != 0 && nonempty == 0 {
        return Err(error(
            ErrorKind::InvalidFormat,
            bitmap.start,
            "symbol hash records have no buckets",
        ));
    }
    let offsets = bitmap.end..data.end;
    check_offsets(bytes, offsets.clone(), records, stream)?;
    Ok((bitmap, offsets, ranks))
}

/// Check that the bucket offsets start at 0, increase and point to records.
fn check_offsets(
    bytes: &[u8],
    offsets: Range<usize>,
    records: usize,
    stream: StreamId,
) -> Result<()> {
    let error = |offset, context| index_error(ErrorKind::InvalidFormat, stream, offset, context);
    let mut previous = None;
    for position in offsets.step_by(4) {
        let value = u32_at(bytes, position);
        if !value.is_multiple_of(OFFSET_SCALE) {
            return Err(error(
                position,
                "symbol hash bucket offset is not a multiple of 12",
            ));
        }
        let first = (value / OFFSET_SCALE) as usize;
        if first >= records {
            return Err(error(
                position,
                "symbol hash bucket offset is past the last hash record",
            ));
        }
        match previous {
            None if first != 0 => {
                return Err(error(
                    position,
                    "first symbol hash bucket does not start at the first record",
                ));
            }
            Some(last) if first <= last => {
                return Err(error(
                    position,
                    "symbol hash bucket offsets do not increase",
                ));
            }
            _ => {}
        }
        previous = Some(first);
    }
    Ok(())
}

/// A view of a validated name hash table.
///
/// Get a table from [`super::GlobalSymbolIndex::hash_table`] or
/// [`super::PublicSymbolIndex::hash_table`].
#[derive(Clone, Copy, Debug)]
pub struct HashTable<'a> {
    /// The bytes of the index stream.
    bytes: &'a [u8],
    /// The validated location of the table.
    layout: &'a Layout,
}

impl<'a> HashTable<'a> {
    /// Return the decoded header.
    #[must_use]
    pub const fn header(&self) -> HashHeader {
        self.layout.header
    }

    /// Return the bucket count that the table uses.
    #[must_use]
    pub const fn bucket_count(&self) -> BucketCount {
        self.layout.buckets
    }

    /// Return the number of hash records.
    #[must_use]
    pub const fn len(&self) -> usize {
        (self.layout.records.end - self.layout.records.start) / RECORD_SIZE
    }

    /// Return `true` if the table has no hash records.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.layout.records.start == self.layout.records.end
    }

    /// Return all hash records in file order.
    ///
    /// The records are grouped by bucket, in increasing bucket order.
    #[must_use]
    pub fn records(&self) -> HashRecords<'a> {
        HashRecords::new(&self.bytes[self.layout.records.clone()])
    }

    /// Return the non-empty buckets in increasing bucket order.
    #[must_use]
    pub fn buckets(&self) -> Buckets<'a> {
        Buckets {
            table: *self,
            next: 0,
            rank: 0,
        }
    }

    /// Return the number of non-empty buckets.
    #[must_use]
    pub const fn nonempty_bucket_count(&self) -> usize {
        (self.layout.offsets.end - self.layout.offsets.start) / 4
    }

    /// Return the hash records of bucket `index`.
    ///
    /// The result is empty if the bucket is empty or if `index` is not less
    /// than the bucket count.
    #[must_use]
    pub fn bucket(&self, index: u32) -> HashRecords<'a> {
        match self.rank(index) {
            Some(rank) => self.bucket_at(rank).1,
            None => HashRecords::new(&[]),
        }
    }

    /// Return the candidate hash records for `name`.
    ///
    /// The candidates are the records of the bucket of `name`. Decode the
    /// record at each [`HashRecord::symbol`] and compare its name with `name`.
    /// The comparison is the job of the caller.
    #[must_use]
    pub fn lookup(&self, name: &[u8]) -> HashRecords<'a> {
        self.bucket(self.layout.buckets.bucket_of(name))
    }

    /// Make sure that each hash record identifies the start of a record.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ErrorKind::InvalidReference`] for the first hash
    /// record whose offset is not inside the symbol record stream or not at
    /// the start of a record. The error identifies the index stream and the
    /// logical offset of the hash record.
    pub fn check_records(&self, offsets: &RecordOffsets) -> Result<()> {
        let start = self.layout.records.start;
        for (index, record) in self.records().enumerate() {
            offsets.check(
                record.symbol,
                self.layout.stream,
                start + index * RECORD_SIZE,
            )?;
        }
        Ok(())
    }

    /// Return the position in the offset array of non-empty bucket `index`.
    fn rank(&self, index: u32) -> Option<usize> {
        if index >= self.layout.buckets.get() || self.layout.bitmap.is_empty() {
            return None;
        }
        let word_index = (index / 32) as usize;
        let bit = index % 32;
        let word = u32_at(self.bytes, self.layout.bitmap.start + word_index * 4);
        if word & (1 << bit) == 0 {
            return None;
        }
        let below = (word & ((1 << bit) - 1)).count_ones();
        Some((self.layout.ranks[word_index] + below) as usize)
    }

    /// Return the first record index and the records of the non-empty bucket at `rank`.
    fn bucket_at(&self, rank: usize) -> (u32, HashRecords<'a>) {
        let offsets = &self.layout.offsets;
        let first = u32_at(self.bytes, offsets.start + rank * 4) / OFFSET_SCALE;
        let next = offsets.start + (rank + 1) * 4;
        let end = if next < offsets.end {
            (u32_at(self.bytes, next) / OFFSET_SCALE) as usize
        } else {
            self.len()
        };
        let records = self.layout.records.start;
        let bytes =
            &self.bytes[records + first as usize * RECORD_SIZE..records + end * RECORD_SIZE];
        (first, HashRecords::new(bytes))
    }
}

/// One hash record (`HRFile`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HashRecord {
    /// The offset of the symbol record. The file stores this value plus 1.
    pub symbol: SymbolOffset,
    /// The reference count. Writers set it to 1; it is always positive.
    pub reference_count: u32,
}

/// An iterator over hash records.
#[derive(Clone, Debug)]
pub struct HashRecords<'a> {
    /// The remaining 8-byte records.
    chunks: Iter<'a, [u8; RECORD_SIZE]>,
}

impl<'a> HashRecords<'a> {
    /// Create an iterator over validated record bytes.
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            chunks: bytes.as_chunks::<RECORD_SIZE>().0.iter(),
        }
    }

    /// Decode one validated record.
    fn decode(chunk: [u8; RECORD_SIZE]) -> HashRecord {
        let [o0, o1, o2, o3, r0, r1, r2, r3] = chunk;
        HashRecord {
            symbol: SymbolOffset(u32::from_le_bytes([o0, o1, o2, o3]) - 1),
            reference_count: u32::from_le_bytes([r0, r1, r2, r3]),
        }
    }
}

impl Iterator for HashRecords<'_> {
    type Item = HashRecord;

    fn next(&mut self) -> Option<Self::Item> {
        self.chunks.next().copied().map(Self::decode)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.chunks.size_hint()
    }
}

impl DoubleEndedIterator for HashRecords<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.chunks.next_back().copied().map(Self::decode)
    }
}

impl ExactSizeIterator for HashRecords<'_> {}

impl FusedIterator for HashRecords<'_> {}

/// One non-empty bucket of a hash table.
#[derive(Clone, Debug)]
pub struct Bucket<'a> {
    /// The bucket index, less than the bucket count.
    pub index: u32,
    /// The index of the first hash record of the bucket.
    ///
    /// The file stores this value multiplied by 12.
    pub first_record: u32,
    /// The hash records of the bucket. There is at least one record.
    pub records: HashRecords<'a>,
}

/// An iterator over the non-empty buckets of a hash table.
#[derive(Clone, Debug)]
pub struct Buckets<'a> {
    /// The table.
    table: HashTable<'a>,
    /// The first bucket index that is not examined yet.
    next: u32,
    /// The number of non-empty buckets returned so far.
    rank: usize,
}

impl<'a> Iterator for Buckets<'a> {
    type Item = Bucket<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let bitmap = &self.table.layout.bitmap;
        if self.rank >= self.table.nonempty_bucket_count() {
            return None;
        }
        // The rank check above makes sure that a set bit follows `next`.
        let mut word_index = self.next / 32;
        let word_at = |index: u32| u32_at(self.table.bytes, bitmap.start + index as usize * 4);
        let mut word = word_at(word_index) & u32::MAX.checked_shl(self.next % 32).unwrap_or(0);
        while word == 0 {
            word_index += 1;
            word = word_at(word_index);
        }
        let index = word_index * 32 + word.trailing_zeros();
        let (first_record, records) = self.table.bucket_at(self.rank);
        self.rank += 1;
        self.next = index + 1;
        Some(Bucket {
            index,
            first_record,
            records,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.table.nonempty_bucket_count() - self.rank;
        (left, Some(left))
    }
}

impl ExactSizeIterator for Buckets<'_> {}

impl FusedIterator for Buckets<'_> {}
