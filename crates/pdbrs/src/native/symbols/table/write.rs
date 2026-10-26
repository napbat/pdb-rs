//! The builder of the name hash table that the symbol indexes share.
//!
//! The builder puts each record in a bucket and sorts the records of each
//! bucket. Then it writes the layout that [`super::Layout::parse`] reads.
//!
//! The order of the records in a bucket comes from the producers:
//!
//! - A shorter name comes first.
//! - Names of the same length compare without case if both are ASCII, else
//!   byte by byte (`caseInsensitiveComparePchPchCchCch` in `PDB/dbi/gsi.cpp`,
//!   `gsiRecordCmp` in LLVM `GSIStreamBuilder.cpp`).
//! - Names that compare equal keep the order of their symbol offsets.
//!
//! LLVM sorts by the symbol offset for the last rule. `GSI1::packSym` of
//! `microsoft/microsoft-pdb` puts a new global record before the records with
//! an equal name. But in the global indexes of the MSVC `LINK` 14.51 fixtures,
//! all records with equal names are in increasing offset order. Thus the
//! rule is the same for both producers.

use alloc::vec::Vec;
use core::cmp::Ordering;

use super::super::{BucketCount, SymbolOffset};
use super::{HEADER_SIZE, HashRecord, MAX_SIGNED, OFFSET_SCALE, RECORD_SIZE, SIGNATURE, VERSION};
use crate::{Error, ErrorKind, Result};

/// The kind of a local managed data record (`S_LMANDATA`).
const S_LMANDATA: u16 = 0x111c;
/// The kind of a global managed data record (`S_GMANDATA`).
const S_GMANDATA: u16 = 0x111d;
/// The kind of a data reference record (`S_DATAREF`).
const S_DATAREF: u16 = 0x1126;
/// The kind of an annotation reference record (`S_ANNOTATIONREF`).
const S_ANNOTATIONREF: u16 = 0x1128;
/// The size of the fields before the name in `DATASYM32` and `REFSYM2`.
const NAMED_FIELDS_SIZE: usize = 10;

/// The producer conventions that a symbol index builder follows.
///
/// The producers write the same bytes for most inputs. They differ only in
/// the cases that each variant describes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum IndexProducer {
    /// The conventions of `mspdbcore.dll`, which MSVC `LINK` uses.
    ///
    /// - A hash table without records has no bucket bitmap
    ///   (`GSI1::CompressBuckets` in `PDB/dbi/gsi.cpp`).
    /// - The address map compares sections as signed 16-bit values and
    ///   offsets as signed 32-bit values (`cmpAddrMapByAddr`).
    Microsoft,
    /// The conventions of LLVM `GSIStreamBuilder.cpp`, which LLD uses.
    ///
    /// - A hash table without records has a bucket bitmap with no bits set.
    /// - The address map compares sections and offsets as unsigned values
    ///   (`computeAddrMap`).
    Llvm,
}

/// One record to put in a symbol hash table.
///
/// The builder sorts the records of each bucket by name:
///
/// - A shorter name comes first.
/// - Two ASCII names of the same length compare without case.
/// - Other names of the same length compare byte by byte.
/// - Names that compare equal keep the order of their symbol offsets.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HashEntry {
    /// The offset of the record in the symbol record stream.
    ///
    /// The record must have a name. The offset plus 1 must be a positive
    /// signed 32-bit value.
    pub symbol: SymbolOffset,
    /// The reference count, from 1 to `0x7fffffff`.
    ///
    /// LLVM writes 1. MSVC counts the modules that share a record, for
    /// example an `S_UDT` record.
    pub reference_count: u32,
    /// The bucket of the record, or `None` for the bucket of the record name.
    ///
    /// `None` uses [`BucketCount::bucket_of`]. Use a value only to keep the
    /// bucket of an existing index. For example, LLD 22.1.3 hashes a public
    /// name of exactly 8 bytes together with the bytes after the short name
    /// in the COFF symbol table, up to a NUL byte. Thus these records are not
    /// in the bucket of their name.
    pub bucket: Option<u32>,
}

impl HashEntry {
    /// Create an entry with a reference count of 1 in the bucket of its name.
    #[must_use]
    pub const fn new(symbol: SymbolOffset) -> Self {
        Self {
            symbol,
            reference_count: 1,
            bucket: None,
        }
    }
}

impl From<HashRecord> for HashEntry {
    /// Keep the symbol offset and the reference count. Select the bucket of the name.
    fn from(record: HashRecord) -> Self {
        Self {
            symbol: record.symbol,
            reference_count: record.reference_count,
            bucket: None,
        }
    }
}

/// One entry with its bucket and name.
#[derive(Clone, Copy, Debug)]
struct Placed<'a> {
    /// The bucket.
    bucket: u32,
    /// The record name without the NUL terminator.
    name: &'a [u8],
    /// The symbol offset.
    symbol: u32,
    /// The reference count.
    references: u32,
}

/// Append a hash table of `entries` to `output`.
///
/// `symbol_records` is the symbol record stream. The function decodes the
/// record of each entry to get its name.
///
/// # Errors
///
/// - [`ErrorKind::LimitExceeded`] if a symbol offset, a reference count or a
///   size does not fit its field, or if an allocation fails.
/// - [`ErrorKind::InvalidFormat`] for a reference count of 0, a bucket that
///   is not less than the bucket count, or a record without a name.
/// - The errors of [`SymbolOffset::record`] and of the symbol decoder for an
///   entry that does not identify a valid record.
pub(in crate::native::symbols) fn append_table(
    symbol_records: &[u8],
    entries: &[HashEntry],
    buckets: BucketCount,
    producer: IndexProducer,
    output: &mut Vec<u8>,
) -> Result<()> {
    let mut placed = place(symbol_records, entries, buckets)?;
    merge_sort(&mut placed, |left, right| {
        left.bucket
            .cmp(&right.bucket)
            .then_with(|| compare_names(left.name, right.name))
            .then(left.symbol.cmp(&right.symbol))
    })?;
    let words = (buckets.get() as usize + 1).div_ceil(32);
    let mut bitmap = Vec::new();
    bitmap
        .try_reserve_exact(words)
        .map_err(|_| allocation_failed())?;
    bitmap.resize(words, 0_u32);
    let mut starts = Vec::new();
    let mut previous = None;
    for (index, entry) in placed.iter().enumerate() {
        if previous == Some(entry.bucket) {
            continue;
        }
        previous = Some(entry.bucket);
        bitmap[(entry.bucket / 32) as usize] |= 1 << (entry.bucket % 32);
        starts.try_reserve(1).map_err(|_| allocation_failed())?;
        starts.push(scaled(index, OFFSET_SCALE as usize)?);
    }
    let records_size = scaled(placed.len(), RECORD_SIZE)?;
    let buckets_size = if placed.is_empty() && producer == IndexProducer::Microsoft {
        0
    } else {
        scaled(words + starts.len(), 4)?
    };
    let size = [scaled(HEADER_SIZE, 1)?, records_size, buckets_size]
        .into_iter()
        .try_fold(0_u32, u32::checked_add)
        .ok_or_else(|| too_large("symbol hash table size does not fit in 32 bits"))?;
    output
        .try_reserve(size as usize)
        .map_err(|_| allocation_failed())?;
    for value in [SIGNATURE, VERSION, records_size, buckets_size] {
        output.extend_from_slice(&value.to_le_bytes());
    }
    for entry in &placed {
        output.extend_from_slice(&(entry.symbol + 1).to_le_bytes());
        output.extend_from_slice(&entry.references.to_le_bytes());
    }
    if buckets_size != 0 {
        for value in bitmap.iter().chain(&starts) {
            output.extend_from_slice(&value.to_le_bytes());
        }
    }
    Ok(())
}

/// Return the name that the symbol indexes hash for the record at `symbol`.
///
/// The function uses [`Symbol::name`](crate::native::codeview::symbol::Symbol::name)
/// for a kind with a decoder. Some kinds that the producers index have no
/// decoder in this crate. The function reads the name of these kinds from
/// their layout in `cvinfo.h`:
///
/// - `S_LMANDATA` and `S_GMANDATA` (`DATASYM32`).
/// - `S_DATAREF` and `S_ANNOTATIONREF` (`REFSYM2`).
///
/// Both layouts have 10 bytes of fields before the name.
///
/// # Errors
///
/// - The errors of [`SymbolOffset::record`] and of the symbol decoder.
/// - [`ErrorKind::InvalidFormat`] for a record without a name, or a name
///   without a NUL terminator.
pub fn indexed_name(symbol_records: &[u8], symbol: SymbolOffset) -> Result<&[u8]> {
    let record = symbol.record(symbol_records)?;
    let decoded = record.symbol()?;
    if let Some(name) = decoded.name() {
        return Ok(name);
    }
    let no_name = || {
        Error::new(ErrorKind::InvalidFormat, None).with_context("indexed symbol record has no name")
    };
    if !matches!(
        record.kind,
        S_LMANDATA | S_GMANDATA | S_DATAREF | S_ANNOTATIONREF
    ) {
        return Err(no_name());
    }
    let fields = record.data.get(NAMED_FIELDS_SIZE..).ok_or_else(no_name)?;
    let end = fields
        .iter()
        .position(|&byte| byte == 0)
        .ok_or_else(no_name)?;
    Ok(&fields[..end])
}

/// Check each entry, decode its name and select its bucket.
fn place<'a>(
    symbol_records: &'a [u8],
    entries: &[HashEntry],
    buckets: BucketCount,
) -> Result<Vec<Placed<'a>>> {
    let mut placed = Vec::new();
    placed
        .try_reserve_exact(entries.len())
        .map_err(|_| allocation_failed())?;
    for entry in entries {
        if entry.symbol.0 >= MAX_SIGNED {
            return Err(too_large(
                "symbol offset plus 1 is not a positive signed 32-bit value",
            ));
        }
        if entry.reference_count == 0 {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("symbol hash record reference count is 0"));
        }
        if entry.reference_count > MAX_SIGNED {
            return Err(too_large(
                "symbol hash record reference count is not a signed 32-bit value",
            ));
        }
        let name = indexed_name(symbol_records, entry.symbol)?;
        let bucket = match entry.bucket {
            None => buckets.bucket_of(name),
            Some(bucket) if bucket < buckets.get() => bucket,
            Some(_) => {
                return Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("symbol hash bucket is not less than the bucket count"));
            }
        };
        placed.push(Placed {
            bucket,
            name,
            symbol: entry.symbol.0,
            references: entry.reference_count,
        });
    }
    Ok(placed)
}

/// Compare two names in the bucket order of the producers.
///
/// A shorter name comes first. Two ASCII names of the same length compare
/// without case: each letter compares as its lowercase form. Other names
/// compare byte by byte.
pub(in crate::native::symbols) fn compare_names(left: &[u8], right: &[u8]) -> Ordering {
    left.len().cmp(&right.len()).then_with(|| {
        if left.is_ascii() && right.is_ascii() {
            left.iter()
                .map(u8::to_ascii_lowercase)
                .cmp(right.iter().map(u8::to_ascii_lowercase))
        } else {
            left.cmp(right)
        }
    })
}

/// Sort `items` with a stable merge sort.
///
/// The name order of the producers is not a total order: for some names of
/// the same length, two ASCII names and a non-ASCII name can form a cycle.
/// `slice::sort_by` can panic for such an order. This sort always completes.
/// It gives the same result as any other sort when `compare` is a total order.
fn merge_sort<T: Copy>(items: &mut Vec<T>, compare: impl Fn(&T, &T) -> Ordering) -> Result<()> {
    let length = items.len();
    let mut target = Vec::new();
    target
        .try_reserve_exact(length)
        .map_err(|_| allocation_failed())?;
    let mut source = core::mem::take(items);
    let mut width = 1;
    while width < length {
        target.clear();
        for start in (0..length).step_by(2 * width) {
            let middle = (start + width).min(length);
            let end = (start + 2 * width).min(length);
            let (mut left, mut right) = (start, middle);
            while left < middle && right < end {
                if compare(&source[right], &source[left]) == Ordering::Less {
                    target.push(source[right]);
                    right += 1;
                } else {
                    target.push(source[left]);
                    left += 1;
                }
            }
            target.extend_from_slice(&source[left..middle]);
            target.extend_from_slice(&source[right..end]);
        }
        core::mem::swap(&mut source, &mut target);
        width *= 2;
    }
    *items = source;
    Ok(())
}

/// Return `count * scale` as a 32-bit value.
///
/// # Errors
///
/// Returns [`ErrorKind::LimitExceeded`] if the product does not fit in 32 bits.
pub(in crate::native::symbols) fn scaled(count: usize, scale: usize) -> Result<u32> {
    count
        .checked_mul(scale)
        .and_then(|size| u32::try_from(size).ok())
        .ok_or_else(|| too_large("symbol index size does not fit in 32 bits"))
}

/// Return a [`ErrorKind::LimitExceeded`] error with `context`.
pub(in crate::native::symbols) fn too_large(context: &'static str) -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context(context)
}

/// Return the error for a failed allocation.
pub(in crate::native::symbols) fn allocation_failed() -> Error {
    too_large("symbol index allocation failed")
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::cmp::Ordering;

    use super::{compare_names, merge_sort, scaled};
    use crate::ErrorKind;

    #[test]
    fn scaled_sizes_must_fit_in_32_bits() {
        assert_eq!(scaled(0x1555_5555, 12).unwrap(), 0xffff_fffc);
        assert_eq!(
            scaled(0x1555_5556, 12).unwrap_err().kind(),
            ErrorKind::LimitExceeded
        );
        assert_eq!(
            scaled(0x2000_0000, 8).unwrap_err().kind(),
            ErrorKind::LimitExceeded
        );
        assert_eq!(
            scaled(usize::MAX, 1).unwrap_err().kind(),
            ErrorKind::LimitExceeded
        );
    }

    #[test]
    fn names_compare_by_length_then_without_case() {
        assert_eq!(compare_names(b"zz", b"aaa"), Ordering::Less);
        assert_eq!(compare_names(b"ABC", b"abc"), Ordering::Equal);
        // `_` is 0x5f. Letters compare as lowercase (0x61 and up), so `_` comes first.
        assert_eq!(compare_names(b"_", b"A"), Ordering::Less);
        assert_eq!(compare_names(b"B\x80", b"a\x80"), Ordering::Less);
    }

    #[test]
    fn merge_sort_completes_for_a_cycle() {
        // "aa" < "Ba" without case, "Ba" < "C\x80" and "C\x80" < "aa" by bytes.
        let mut names: Vec<&[u8]> = vec![b"C\x80", b"aa", b"Ba", b"aa"];
        merge_sort(&mut names, |left: &&[u8], right: &&[u8]| {
            compare_names(left, right)
        })
        .unwrap();
        let expected: [&[u8]; 4] = [b"C\x80", b"aa", b"aa", b"Ba"];
        assert_eq!(names, expected);
        let mut sorted = vec![5, 1, 4, 1, 3, 9, 2, 6];
        merge_sort(&mut sorted, Ord::cmp).unwrap();
        assert_eq!(sorted, [1, 1, 2, 3, 4, 5, 6, 9]);
    }
}
