//! The builder of the public symbol index stream.
//!
//! The builder writes the parts in the order that [`PublicSymbolIndex::parse`]
//! reads them. `PSGSI1::writeStream` in `PDB/dbi/gsi.cpp` and
//! `GSIStreamBuilder::commitPublicsHashStream` in LLVM write the same order.
//!
//! The address map is sorted by section, then by offset, then by name
//! (`cmpAddrMapByAddrAndName` in `PDB/dbi/gsi.cpp`, `computeAddrMap` in
//! LLVM). Two `S_PUB32` records with the same address and name keep the order
//! of their symbol offsets. The producers do not define this order.

use alloc::vec::Vec;
use core::cmp::Ordering;

use super::super::BucketCount;
use super::super::table::write::{HashEntry, allocation_failed, append_table, scaled, too_large};
use super::{HEADER_SIZE, PublicSymbolIndex, SECTION_OFFSET_SIZE, SectionStart};
use crate::native::Producer;
use crate::native::codeview::symbol::{Symbol, SymbolData};
use crate::{Bytes, Error, ErrorKind, Result};

/// The incremental thunk table of a public symbol index.
///
/// An incremental link puts a thunk for each function in a table in the
/// image. The [`Default`] value is a table without thunks, which full links
/// write.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ThunkTable<'a> {
    /// The size of one thunk in the image, in bytes.
    pub thunk_size: u32,
    /// The section number of the thunk table in the image.
    pub section: u16,
    /// The offset of the thunk table in its section.
    pub offset: u32,
    /// The flat target address of each thunk, in thunk order.
    pub targets: &'a [u32],
}

/// The address of one `S_PUB32` record.
#[derive(Clone, Debug)]
struct Address<'a> {
    /// The section number.
    section: u16,
    /// The offset in the section.
    offset: u32,
    /// The name without the NUL terminator.
    name: Bytes<'a>,
    /// The symbol offset.
    symbol: u32,
}

impl PublicSymbolIndex {
    /// Append the bytes of a public symbol index stream to `out`.
    ///
    /// `symbol_records` is the symbol record stream. `entries` gives the
    /// `S_PUB32` records to index, in any order. Each entry goes in the name
    /// hash table and in the address map. Select `buckets` as for
    /// [`PublicSymbolIndex::parse`]. [`HashEntry`] gives the order of the
    /// records in a bucket.
    ///
    /// `producer` selects these conventions:
    ///
    /// - [`Producer::Microsoft`]: a hash table without records has no bucket
    ///   bitmap (`GSI1::CompressBuckets` in `PDB/dbi/gsi.cpp`). The address
    ///   map compares sections as signed 16-bit values and offsets as signed
    ///   32-bit values (`cmpAddrMapByAddr`).
    /// - [`Producer::Llvm`]: a hash table without records has a bucket bitmap
    ///   with no bits set. The address map compares sections and offsets as
    ///   unsigned values (`computeAddrMap` in LLVM `GSIStreamBuilder.cpp`).
    ///
    /// The header takes the thunk fields from `thunks`. The section count is
    /// the length of `section_starts`. The padding bytes of the header are 0.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`] if a symbol offset, a reference count,
    ///   a count or a size does not fit its field, or if an allocation fails.
    /// - [`ErrorKind::InvalidFormat`] for an entry that is not an `S_PUB32`
    ///   record, a reference count of 0, or a bucket that is not less than
    ///   the bucket count.
    /// - [`ErrorKind::InvalidReference`] or the errors of the symbol decoder
    ///   for an entry that does not identify a valid record.
    ///
    /// The errors do not have a location. If the function fails, `out` does
    /// not change.
    pub fn encode_parts(
        symbol_records: &[u8],
        entries: &[HashEntry],
        buckets: BucketCount,
        producer: Producer,
        thunks: ThunkTable<'_>,
        section_starts: &[SectionStart],
        out: &mut Vec<u8>,
    ) -> Result<()> {
        let start = out.len();
        let result = append_index(
            symbol_records,
            entries,
            buckets,
            producer,
            thunks,
            section_starts,
            out,
        );
        if result.is_err() {
            out.truncate(start);
        }
        result
    }
}

/// Append a public symbol index stream to `out`.
///
/// See [`PublicSymbolIndex::encode_parts`]. On error, `out` can hold a part
/// of the stream.
fn append_index(
    symbol_records: &[u8],
    entries: &[HashEntry],
    buckets: BucketCount,
    producer: Producer,
    thunks: ThunkTable<'_>,
    section_starts: &[SectionStart],
    out: &mut Vec<u8>,
) -> Result<()> {
    let addresses = address_map(symbol_records, entries, producer)?;
    let address_map_size = scaled(addresses.len(), 4)?;
    let thunk_count = u32::try_from(thunks.targets.len())
        .map_err(|_| too_large("public thunk count does not fit in 32 bits"))?;
    let thunk_bytes = scaled(thunks.targets.len(), 4)?;
    let section_count = u32::try_from(section_starts.len())
        .map_err(|_| too_large("public section count does not fit in 32 bits"))?;
    let section_bytes = scaled(section_starts.len(), SECTION_OFFSET_SIZE)?;
    let start = out.len();
    out.try_reserve(HEADER_SIZE)
        .map_err(|_| allocation_failed())?;
    out.resize(start + HEADER_SIZE, 0);
    append_table(symbol_records, entries, buckets, producer, out)?;
    let written = out.len() - start;
    let hash_size = scaled(written - HEADER_SIZE, 1)?;
    let total = [
        scaled(written, 1)?,
        address_map_size,
        thunk_bytes,
        section_bytes,
    ]
    .into_iter()
    .try_fold(0_u32, u32::checked_add)
    .ok_or_else(|| too_large("public symbol index size does not fit in 32 bits"))?;
    out.try_reserve(total as usize - written)
        .map_err(|_| allocation_failed())?;
    let header = [
        hash_size,
        address_map_size,
        thunk_count,
        thunks.thunk_size,
        u32::from(thunks.section),
        thunks.offset,
        section_count,
    ];
    let fields = out[start..start + HEADER_SIZE].as_chunks_mut::<4>().0;
    for (field, value) in fields.iter_mut().zip(header) {
        field.copy_from_slice(&value.to_le_bytes());
    }
    for address in &addresses {
        out.extend_from_slice(&address.symbol.to_le_bytes());
    }
    for target in thunks.targets {
        out.extend_from_slice(&target.to_le_bytes());
    }
    for section_start in section_starts {
        out.extend_from_slice(&section_start.offset.to_le_bytes());
        out.extend_from_slice(&section_start.section.to_le_bytes());
        out.extend_from_slice(&section_start.padding);
    }
    Ok(())
}

/// Decode the address of each entry and sort the addresses.
fn address_map<'a>(
    symbol_records: &'a [u8],
    entries: &[HashEntry],
    producer: Producer,
) -> Result<Vec<Address<'a>>> {
    let mut addresses = Vec::new();
    addresses
        .try_reserve_exact(entries.len())
        .map_err(|_| allocation_failed())?;
    for entry in entries {
        let symbol = Symbol::decode(entry.symbol.record(symbol_records)?)?;
        let SymbolData::Public(public) = symbol.data() else {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("public symbol index entry is not an S_PUB32 record"));
        };
        addresses.push(Address {
            section: public.address.section,
            offset: public.address.offset,
            name: public.name.clone(),
            symbol: entry.symbol.0,
        });
    }
    addresses.sort_unstable_by(|left, right| {
        compare_addresses(left, right, producer)
            .then_with(|| left.name.cmp(&right.name))
            .then(left.symbol.cmp(&right.symbol))
    });
    Ok(addresses)
}

/// Compare the sections and the offsets of two records as `producer` does.
fn compare_addresses(left: &Address<'_>, right: &Address<'_>, producer: Producer) -> Ordering {
    match producer {
        Producer::Microsoft => left
            .section
            .cast_signed()
            .cmp(&right.section.cast_signed())
            .then(left.offset.cast_signed().cmp(&right.offset.cast_signed())),
        Producer::Llvm => left
            .section
            .cmp(&right.section)
            .then(left.offset.cmp(&right.offset)),
    }
}
