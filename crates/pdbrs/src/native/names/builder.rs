//! A string table that grows with the insertion rules of Microsoft `NMT`.
//!
//! Microsoft `PDB/include/nmt.h` builds a table in this way:
//!
//! 1. A new table has one bucket and a string buffer with one NUL byte. Offset 0 is the empty
//!    name; a bucket value of 0 marks an empty bucket.
//! 2. `NMT::addNiForSz` walks from the home bucket of the name to the first empty bucket. If the
//!    walk finds the name, the table does not change. If not, the name and its NUL go to the end
//!    of the string buffer, and the empty bucket gets the offset of the name.
//! 3. `NMT::grow` then adds 1 to the name count. If `bucket_count * 3 / 4 < name_count`, the
//!    table gets `bucket_count * 3 / 2 + 1` buckets. `NMT::rehash` puts the names again in the
//!    order of the old buckets.
//!
//! LLVM `PDBStringTableBuilder` gives the same bucket count for the same number of names. It
//! puts colliding names in the order of its internal string map, so the buckets of a table
//! from LLVM can differ in the order of colliding names.

use alloc::vec::Vec;

use super::{HashVersion, SIGNATURE, StringTable, home_bucket, name_at};
use crate::native::output::{Output, no_memory, to_u32};
use crate::{Error, ErrorKind, Result};

/// An owned string table that can grow and encode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StringTableBuilder {
    /// The hash function version.
    version: HashVersion,
    /// The string buffer.
    strings: Vec<u8>,
    /// The bucket array: name offsets, or 0 for an empty bucket.
    buckets: Vec<u32>,
    /// The number of names in the buckets.
    name_count: u32,
}

impl StringTableBuilder {
    /// Create an empty table: one bucket and a string buffer with one NUL byte.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the allocation fails.
    pub fn new(version: HashVersion) -> Result<Self> {
        let mut strings = Vec::new();
        strings.try_reserve(1).map_err(|_| no_memory())?;
        strings.push(0);
        let mut buckets = Vec::new();
        buckets.try_reserve(1).map_err(|_| no_memory())?;
        buckets.push(0);
        Ok(Self {
            version,
            strings,
            buckets,
            name_count: 0,
        })
    }

    /// Copy a decoded table, so that new names keep the existing offsets.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the table has no buckets.
    /// - [`ErrorKind::LimitExceeded`] if the allocation fails.
    pub fn from_table(table: &StringTable<'_>) -> Result<Self> {
        if table.bucket_count() == 0 {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("string table has no buckets"));
        }
        let mut strings = Vec::new();
        strings
            .try_reserve_exact(table.strings().len())
            .map_err(|_| no_memory())?;
        strings.extend_from_slice(table.strings());
        let mut buckets = Vec::new();
        buckets
            .try_reserve_exact(table.bucket_count())
            .map_err(|_| no_memory())?;
        buckets.extend(table.bucket_offsets());
        Ok(Self {
            version: table.hash_version(),
            strings,
            buckets,
            name_count: table.name_count(),
        })
    }

    /// Return the hash function version.
    #[must_use]
    pub const fn hash_version(&self) -> HashVersion {
        self.version
    }

    /// Return the string buffer.
    #[must_use]
    pub fn strings(&self) -> &[u8] {
        &self.strings
    }

    /// Return the number of buckets.
    #[must_use]
    pub fn bucket_count(&self) -> usize {
        self.buckets.len()
    }

    /// Return the number of names in the buckets.
    #[must_use]
    pub const fn name_count(&self) -> u32 {
        self.name_count
    }

    /// Return the name at `offset`, without its NUL.
    #[must_use]
    pub fn get(&self, offset: u32) -> Option<&[u8]> {
        name_at(&self.strings, offset)
    }

    /// Return the offset of `name`, and add the name if the table does not have it.
    ///
    /// The empty name has offset 0 and does not change the table.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `name` contains a NUL byte.
    /// - [`ErrorKind::InvalidReference`] if a bucket does not refer to a name.
    /// - [`ErrorKind::LimitExceeded`] if the string buffer, the bucket count or the name count
    ///   does not fit in `u32`, or if an allocation fails.
    pub fn insert(&mut self, name: &[u8]) -> Result<u32> {
        if name.is_empty() {
            return Ok(0);
        }
        let bucket = match self.find(name)? {
            Probe::Found(offset) => return Ok(offset),
            Probe::Empty(bucket) => bucket,
        };
        let offset = to_u32(self.strings.len(), "string table buffer is too large")?;
        let end = self.strings.len() + name.len() + 1;
        to_u32(end, "string table buffer is too large")?;
        Output::new(&mut self.strings).c_string(name)?;
        self.buckets[bucket] = offset;
        self.name_count = self.name_count.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::LimitExceeded, None).with_context("string table name count")
        })?;
        let count = u64::try_from(self.buckets.len()).unwrap_or(u64::MAX);
        if count * 3 / 4 < u64::from(self.name_count) {
            let grown = usize::try_from(count * 3 / 2 + 1).map_err(|_| no_memory())?;
            to_u32(grown, "string table bucket count")?;
            self.rehash(grown)?;
        }
        Ok(offset)
    }

    /// Append the serialized table to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if a size does not fit in `u32` or the output
    /// cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut output = Output::new(out);
        output.reserve(16 + self.strings.len() + self.buckets.len() * 4)?;
        output.u32(SIGNATURE)?;
        output.u32(self.version.value())?;
        output.u32(to_u32(self.strings.len(), "string table buffer size")?)?;
        output.bytes(&self.strings)?;
        output.u32(to_u32(self.buckets.len(), "string table bucket count")?)?;
        for bucket in &self.buckets {
            output.u32(*bucket)?;
        }
        output.u32(self.name_count)
    }

    /// Walk from the home bucket of `name` to the name or to an empty bucket.
    fn find(&self, name: &[u8]) -> Result<Probe> {
        let count = self.buckets.len();
        let start = home_bucket(self.version, name, count);
        for step in 0..count {
            let bucket = (start + step) % count;
            let offset = self.buckets[bucket];
            if offset == 0 {
                return Ok(Probe::Empty(bucket));
            }
            let stored = name_at(&self.strings, offset).ok_or_else(|| {
                Error::new(ErrorKind::InvalidReference, None)
                    .with_context("string table offset does not refer to a NUL-terminated name")
            })?;
            if stored == name {
                return Ok(Probe::Found(offset));
            }
        }
        Err(Error::new(ErrorKind::LimitExceeded, None)
            .with_context("string table has no empty bucket"))
    }

    /// Put the names again into `count` buckets, in the order of the old buckets.
    fn rehash(&mut self, count: usize) -> Result<()> {
        let mut buckets = Vec::new();
        buckets.try_reserve_exact(count).map_err(|_| no_memory())?;
        buckets.resize(count, 0);
        for &offset in self.buckets.iter().filter(|&&offset| offset != 0) {
            let name = name_at(&self.strings, offset).ok_or_else(|| {
                Error::new(ErrorKind::InvalidReference, None)
                    .with_context("string table offset does not refer to a NUL-terminated name")
            })?;
            let mut bucket = home_bucket(self.version, name, count);
            while buckets[bucket] != 0 {
                bucket = (bucket + 1) % count;
            }
            buckets[bucket] = offset;
        }
        self.buckets = buckets;
        Ok(())
    }
}

/// The result of a walk over the buckets.
enum Probe {
    /// The name is in the table at this offset.
    Found(u32),
    /// The name is absent; this empty bucket ends the walk.
    Empty(usize),
}
