//! Resolution of UDT forward references to their definitions.
//!
//! The algorithm is `TpiStream::findFullDeclForForwardRef` in LLVM. It uses
//! the hash buckets of the TPI hash stream:
//!
//! 1. Hash the name of the forward reference, or its unique name if it is
//!    scoped, and select the bucket of that hash.
//! 2. Visit the UDT records of that bucket in index order. Skip a record of
//!    another leaf kind or with another name hash.
//! 3. If both records have a unique name, the first record with the same
//!    unique name is the result.
//! 4. Otherwise a record with the same name is a candidate. If the forward
//!    reference has no unique name, the first candidate is the result. If it
//!    has a unique name, the last candidate is the result.
//!
//! The result can be a record that is itself a forward reference, as in LLVM.

use alloc::vec::Vec;

use super::hash::{is_udt_kind, tag_hash, type_record_hash};
use super::{TypeHashes, TypeTable};
use crate::native::codeview::LeafKind;
use crate::native::codeview::index::TypeIndex;
use crate::native::codeview::type_record::{Tag, TypeLeaf};
use crate::{Error, ErrorKind, Result};

/// A bucket index of the UDT records of one TPI stream.
#[derive(Clone, Debug)]
pub struct ForwardRefs<'t, 'a> {
    /// The table that holds the records.
    table: &'t TypeTable<'a>,
    /// The number of hash buckets.
    bucket_count: u32,
    /// The bucket and the index of each UDT record, sorted by bucket and index.
    entries: Vec<(u32, u32)>,
}

/// Return the UDT fields of `index` and its complete record bytes.
fn tag<'a>(table: &TypeTable<'a>, index: u32) -> Result<(u16, Tag<'a>, &'a [u8])> {
    let record = table.record_at_index(index)?.record;
    let full = table.stream().full_record(&record)?;
    let tag = TypeLeaf::decode(record)?.into_tag().ok_or_else(|| {
        Error::new(ErrorKind::InvalidReference, None)
            .with_context("type index does not refer to a user-defined type")
    })?;
    Ok((record.kind, tag, full))
}

impl<'t, 'a> ForwardRefs<'t, 'a> {
    /// Build the bucket index of the UDT records in `table`.
    ///
    /// With `hashes`, the buckets are the stored hash values, as in LLVM.
    /// Without `hashes`, or if the hash stream stores no values, this
    /// function computes each bucket with [`super::type_record_hash`]. The
    /// index costs 8 bytes for each UDT record.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if `hashes` belongs to another stream.
    /// - [`ErrorKind::LimitExceeded`] if the index cannot be allocated.
    /// - The framing errors of the records, and the errors of
    ///   [`super::type_record_hash`] when this function computes a bucket.
    pub fn new(table: &'t TypeTable<'a>, hashes: Option<&TypeHashes<'_>>) -> Result<Self> {
        let stream = table.stream();
        let header = stream.header();
        let bucket_count = header.hash_bucket_count;
        let stored = hashes.filter(|hashes| hashes.hash_values().len() != 0);
        if stored.is_some_and(|hashes| hashes.bucket_count() != bucket_count) {
            return Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("type hash stream belongs to another type stream"));
        }
        let mut count = 0_usize;
        for record in stream.records() {
            count += usize::from(is_udt_kind(LeafKind(record?.record.kind)));
        }
        let mut entries = Vec::new();
        entries.try_reserve_exact(count).map_err(|_| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("forward reference index cannot be allocated")
        })?;
        for record in stream.records() {
            let record = record?;
            if !is_udt_kind(LeafKind(record.record.kind)) {
                continue;
            }
            let bucket = match stored.and_then(|hashes| hashes.hash_value(record.index)) {
                Some(value) => value,
                None => type_record_hash(stream.full_record(&record.record)?)? % bucket_count,
            };
            entries.push((bucket, record.index));
        }
        entries.sort_unstable();
        Ok(Self {
            table,
            bucket_count,
            entries,
        })
    }

    /// Find the definition of the UDT forward reference at `index`.
    ///
    /// Returns `Ok(None)` if no record matches. LLVM then reports the forward
    /// reference itself.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if `index` has no record, if the
    ///   record is not a UDT, or if the UDT is not a forward reference.
    /// - The decoding errors of the visited records.
    pub fn resolve(&self, index: TypeIndex) -> Result<Option<TypeIndex>> {
        if index.is_primitive() {
            return Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("primitive type index has no record"));
        }
        let (kind, forward, full) = tag(self.table, index.0)?;
        if !forward.options.is_forward_reference() {
            return Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("type index does not refer to a forward reference"));
        }
        let hash = tag_hash(&forward, full);
        let bucket = hash % self.bucket_count;
        let start = self.entries.partition_point(|&(value, _)| value < bucket);
        let mut best = None;
        for &(value, candidate) in &self.entries[start..] {
            if value != bucket {
                break;
            }
            if self.table.record_at_index(candidate)?.record.kind != kind {
                continue;
            }
            let (_, definition, full) = tag(self.table, candidate)?;
            if tag_hash(&definition, full) != hash {
                continue;
            }
            if let (Some(left), Some(right)) = (&forward.unique_name, &definition.unique_name) {
                if left == right {
                    return Ok(Some(TypeIndex(candidate)));
                }
                continue;
            }
            if forward.name == definition.name {
                if forward.unique_name.is_none() {
                    return Ok(Some(TypeIndex(candidate)));
                }
                best = Some(TypeIndex(candidate));
            }
        }
        Ok(best)
    }
}
