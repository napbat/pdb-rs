//! The string table of the `/names` stream and of the DBI EC substream.
//!
//! Both locations use the same layout, in this order:
//!
//! 1. A header with the signature `0xeffe_effe`, the hash version and the
//!    byte size of the string buffer.
//! 2. The string buffer with NUL-terminated names.
//! 3. The bucket count and the bucket array. Each bucket holds the byte
//!    offset of one name in the string buffer, or 0 for an empty bucket.
//! 4. The name count.
//!
//! Other structures refer to a name by its byte offset in the string buffer.
//! [`StringTable::get`] resolves such an offset.
//!
//! A table puts each name at bucket [`HashVersion::hash`] `% bucket_count` and
//! uses linear probing on collisions. Version 1 uses [`hash_string_v1`] and
//! version 2 uses [`hash_string_v2`] (Microsoft `NMT::setHashFunc`).
//! [`StringTable::find`] and [`StringTable::verify_buckets`] use the hash of the
//! table version.
//!
//! [`StringTable::encode`] writes a decoded table again with the same bytes.
//! [`StringTableBuilder`] makes a new table or changes a table with the
//! insertion rules of Microsoft `NMT` (`PDB/include/nmt.h`).
//!
//! The layout follows LLVM `PDBStringTable.cpp` and `PDBStringTableBuilder.cpp`
//! and the `PDBStringTableHeader` type in `RawTypes.h`.

mod builder;

pub use builder::StringTableBuilder;

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::codeview::Fields;
use super::hash::{hash_string_v1, hash_string_v2};
use super::output::{Output, to_u32};
use crate::{Error, ErrorKind, Result};

/// The signature at the start of a string table.
pub const SIGNATURE: u32 = 0xeffe_effe;

/// The hash function version of a string table.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HashVersion {
    /// Version 1 (`verLongHash`): [`hash_string_v1`] with linear probing.
    V1,
    /// Version 2 (`verLongHashV2`): [`hash_string_v2`] with linear probing.
    V2,
}

impl HashVersion {
    /// Return the serialized value of the version.
    #[must_use]
    pub const fn value(self) -> u32 {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
        }
    }

    /// Return the hash of a name before the reduction modulo the bucket count.
    ///
    /// Microsoft `NMT::hashSz` reduces the hash modulo `0xffff_ffff` first. This function
    /// applies that reduction too, so a hash of `0xffff_ffff` becomes 0.
    #[must_use]
    pub fn hash(self, name: &[u8]) -> u32 {
        let hash = match self {
            Self::V1 => hash_string_v1(name),
            Self::V2 => hash_string_v2(name),
        };
        hash % u32::MAX
    }
}

/// A name in a non-empty bucket of a string table.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StringEntry<'a> {
    /// The zero-based index of the bucket.
    pub bucket: u32,
    /// The byte offset of the name in the string buffer.
    pub offset: u32,
    /// The name without its terminating NUL. It is not decoded as text.
    pub name: &'a [u8],
}

/// The bucket positions of the names in a version 1 string table.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct BucketPlacement {
    /// The number of names in the bucket that their hash selects.
    pub home: u32,
    /// The number of names that linear probing moved to a later bucket.
    pub probed: u32,
}

/// A decoded string table that borrows its serialized bytes.
#[derive(Clone, Copy, Debug)]
pub struct StringTable<'a> {
    /// The hash function version.
    hash_version: HashVersion,
    /// The string buffer.
    strings: &'a [u8],
    /// The bucket array.
    buckets: &'a [[u8; 4]],
    /// The name count from the end of the table.
    name_count: u32,
}

impl<'a> StringTable<'a> {
    /// Decode a string table that fills `bytes` exactly.
    ///
    /// The function checks that each non-empty bucket refers to a
    /// NUL-terminated name and that the name count equals the number of
    /// non-empty buckets. It does not check the hash placement of names;
    /// [`StringTable::verify_buckets`] does that.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] if a part of the table is truncated.
    /// - [`ErrorKind::InvalidSignature`] if the signature is not [`SIGNATURE`].
    /// - [`ErrorKind::UnsupportedVersion`] if the hash version is not 1 or 2.
    /// - [`ErrorKind::InvalidReference`] if a bucket refers to an offset
    ///   outside the string buffer or to a name without a terminating NUL.
    /// - [`ErrorKind::InvalidFormat`] if the name count does not equal the
    ///   number of non-empty buckets or if bytes follow the name count.
    /// - [`ErrorKind::LimitExceeded`] if a size does not fit in memory.
    ///
    /// Errors do not have a stream location. The caller adds it.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut fields = Fields::new(bytes, ErrorKind::UnexpectedEof, "string table is truncated");
        if fields.u32()? != SIGNATURE {
            return Err(Error::new(ErrorKind::InvalidSignature, None)
                .with_context("string table signature is not 0xeffeeffe"));
        }
        let hash_version = match fields.u32()? {
            1 => HashVersion::V1,
            2 => HashVersion::V2,
            _ => {
                return Err(Error::new(ErrorKind::UnsupportedVersion, None)
                    .with_context("string table hash version is not 1 or 2"));
            }
        };
        let string_size = to_usize(fields.u32()?)?;
        let strings = fields.bytes(string_size)?;
        let bucket_count = to_usize(fields.u32()?)?;
        // A bucket count that overflows cannot fit in the remaining bytes.
        let bucket_bytes = bucket_count.checked_mul(4).ok_or_else(|| {
            Error::new(ErrorKind::UnexpectedEof, None).with_context("string table is truncated")
        })?;
        let buckets = fields.bytes(bucket_bytes)?.as_chunks::<4>().0;
        let name_count = fields.u32()?;
        if !fields.is_empty() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("string table has bytes after the name count"));
        }
        let table = Self {
            hash_version,
            strings,
            buckets,
            name_count,
        };
        let mut used = 0_u32;
        for offset in table.bucket_offsets().filter(|&offset| offset != 0) {
            table.get(offset)?;
            // The bucket count fits in `u32`, so the count of used buckets fits too.
            used = used.saturating_add(1);
        }
        if used != name_count {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("string table name count differs from the used bucket count"));
        }
        Ok(table)
    }

    /// Append the serialized table to `out`.
    ///
    /// The output keeps the string buffer, the bucket array and the name count, so a decoded
    /// table gives the same bytes again.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if a size does not fit in `u32` or the output
    /// cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut output = Output::new(out);
        output.reserve(16 + self.strings.len() + self.buckets.len() * 4)?;
        output.u32(SIGNATURE)?;
        output.u32(self.hash_version.value())?;
        output.u32(to_u32(self.strings.len(), "string table buffer size")?)?;
        output.bytes(self.strings)?;
        output.u32(to_u32(self.buckets.len(), "string table bucket count")?)?;
        output.bytes(self.buckets.as_flattened())?;
        output.u32(self.name_count)
    }

    /// Return the hash function version.
    #[must_use]
    pub const fn hash_version(&self) -> HashVersion {
        self.hash_version
    }

    /// Return the raw string buffer.
    #[must_use]
    pub const fn strings(&self) -> &'a [u8] {
        self.strings
    }

    /// Return the number of buckets.
    #[must_use]
    pub const fn bucket_count(&self) -> usize {
        self.buckets.len()
    }

    /// Return the name count. It equals the number of non-empty buckets.
    #[must_use]
    pub const fn name_count(&self) -> u32 {
        self.name_count
    }

    /// Return the value of each bucket in order; 0 marks an empty bucket.
    pub fn bucket_offsets(&self) -> impl ExactSizeIterator<Item = u32> + FusedIterator + 'a {
        self.buckets
            .iter()
            .map(|bucket| u32::from_le_bytes(*bucket))
    }

    /// Return the names in the non-empty buckets, in bucket order.
    pub fn entries(&self) -> impl FusedIterator<Item = StringEntry<'a>> + 'a {
        let strings = self.strings;
        self.bucket_offsets()
            .zip(0_u32..)
            .filter(|&(offset, _)| offset != 0)
            // `parse` checked every non-empty bucket, so `filter_map` keeps all of them.
            .filter_map(move |(offset, bucket)| {
                name_at(strings, offset).map(|name| StringEntry {
                    bucket,
                    offset,
                    name,
                })
            })
    }

    /// Return the NUL-terminated name at a byte offset of the string buffer.
    ///
    /// The name excludes the terminator. The offset does not have to be the
    /// start of a name; other structures can refer to the middle of a name.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if the offset is outside the
    /// string buffer or no NUL follows it.
    pub fn get(&self, offset: u32) -> Result<&'a [u8]> {
        name_at(self.strings, offset).ok_or_else(|| {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("string table offset does not refer to a NUL-terminated name")
        })
    }

    /// Find the offset of a name through the hash buckets.
    ///
    /// The search starts at bucket [`HashVersion::hash`] `% bucket_count` and
    /// stops at the first empty bucket, as in Microsoft `NMT::find` and LLVM
    /// `getIDForString`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if a bucket on the probe
    /// sequence does not refer to a NUL-terminated name.
    pub fn find(&self, name: &[u8]) -> Result<Option<u32>> {
        let count = self.buckets.len();
        if count == 0 {
            return Ok(None);
        }
        let start = home_bucket(self.hash_version, name, count);
        for step in 0..count {
            let offset = u32::from_le_bytes(self.buckets[(start + step) % count]);
            if offset == 0 {
                return Ok(None);
            }
            if self.get(offset)? == name {
                return Ok(Some(offset));
            }
        }
        Ok(None)
    }

    /// Check that a lookup can reach the bucket of each name.
    ///
    /// A lookup starts at the home bucket of a name and stops at an empty
    /// bucket. The check therefore makes sure that no empty bucket lies
    /// between the home bucket and the bucket that holds the name. It uses
    /// one pass over the buckets.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if an empty bucket hides a name.
    /// - [`ErrorKind::InvalidReference`] if a bucket does not refer to a name.
    pub fn verify_buckets(&self) -> Result<BucketPlacement> {
        let count = self.buckets.len();
        let mut placement = BucketPlacement::default();
        // Start after an empty bucket. A full table has no empty bucket, and
        // then a lookup can visit all buckets.
        let first_empty = self.bucket_offsets().position(|offset| offset == 0);
        let start = first_empty.map_or(0, |index| index + 1);
        // The number of used buckets right before the current bucket.
        let mut run = if first_empty.is_some() { 0 } else { count };
        for step in 0..count {
            let slot = (start + step) % count;
            let offset = u32::from_le_bytes(self.buckets[slot]);
            if offset == 0 {
                run = 0;
                continue;
            }
            let home = home_bucket(self.hash_version, self.get(offset)?, count);
            let distance = (slot + count - home) % count;
            if distance > run {
                return Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("string table bucket cannot be reached from its hash"));
            }
            if distance == 0 {
                placement.home = placement.home.saturating_add(1);
            } else {
                placement.probed = placement.probed.saturating_add(1);
            }
            run = run.saturating_add(1);
        }
        Ok(placement)
    }
}

/// Return the home bucket of a name in a table with `count` buckets.
fn home_bucket(version: HashVersion, name: &[u8], count: usize) -> usize {
    // `count` comes from a `u32` field, so the remainder fits in `usize`.
    let count = u32::try_from(count).unwrap_or(u32::MAX);
    usize::try_from(version.hash(name) % count).unwrap_or(usize::MAX)
}

/// Return the NUL-terminated name at `offset` of `strings`, if it exists.
fn name_at(strings: &[u8], offset: u32) -> Option<&[u8]> {
    let rest = strings.get(usize::try_from(offset).ok()?..)?;
    let end = rest.iter().position(|&byte| byte == 0)?;
    Some(&rest[..end])
}

/// Convert a serialized size to `usize`.
fn to_usize(value: u32) -> Result<usize> {
    usize::try_from(value).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("string table size does not fit in memory")
    })
}
