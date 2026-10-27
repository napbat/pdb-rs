//! The name index of the model of the symbol record stream (#48).
//!
//! A name lookup does not scan the records. It compares the names of two
//! candidate sets:
//!
//! - the source hash records in the bucket of the name, from the global and
//!   the public symbol index of the source;
//! - the records with a changed name: each record that `push` added or that
//!   `get_mut` lent. The set holds the hash of the name at the last update.
//!   `get_mut` cannot see the new name, so the next `&mut` call hashes the
//!   current name of the last lent record. Until then, a lookup compares
//!   that record directly.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use super::super::record::SymbolRecord;
use super::IndexEntry;
use crate::Result;
use crate::native::codeview::index::SymbolHandle;
use crate::native::hash::hash_string_v1;
use crate::native::output::no_memory;
use crate::native::symbols::BucketCount;

/// One of the two symbol indexes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum View {
    /// The global symbol index.
    Globals,
    /// The public symbol index.
    Publics,
}

impl View {
    /// Return the membership bit of the index.
    const fn bit(self) -> u8 {
        match self {
            Self::Globals => 1,
            Self::Publics => 2,
        }
    }
}

/// The source hash records of one index.
#[derive(Clone, Debug, Default)]
struct SourceBuckets {
    /// The bucket count of the source index, or `None` without a source
    /// index.
    count: Option<BucketCount>,
    /// The bucket and the record of each source hash record, in bucket
    /// order.
    records: Vec<(u32, SymbolHandle)>,
}

impl SourceBuckets {
    /// Return the records of the bucket of `name`.
    fn candidates(&self, name: &[u8]) -> &[(u32, SymbolHandle)] {
        let Some(count) = self.count else {
            return &[];
        };
        let bucket = count.bucket_of(name);
        let start = self.records.partition_point(|&(other, _)| other < bucket);
        let end = self.records.partition_point(|&(other, _)| other <= bucket);
        &self.records[start..end]
    }
}

/// The name index of the model of the symbol record stream.
#[derive(Clone, Debug, Default)]
pub(super) struct NameIndex {
    /// The source hash records of the global index.
    globals: SourceBuckets,
    /// The source hash records of the public index.
    publics: SourceBuckets,
    /// The hash of the name and the record of each record with a changed
    /// name, as at the last update.
    changed: BTreeSet<(u32, SymbolHandle)>,
    /// The record that `get_mut` lent last. Its name can differ from
    /// `changed`.
    pending: Option<SymbolHandle>,
    /// The membership bits of each record, by handle (see [`View::bit`]).
    members: Vec<u8>,
}

impl NameIndex {
    /// Make the index of `count` source records and of the entries of the
    /// source indexes. Each source entry has its bucket in the source index
    /// with the bucket count of the pair.
    pub(super) fn from_source(
        count: usize,
        globals: Option<(BucketCount, &[IndexEntry])>,
        publics: Option<(BucketCount, &[IndexEntry])>,
    ) -> Result<Self> {
        let mut index = Self::default();
        index
            .members
            .try_reserve_exact(count)
            .map_err(|_| no_memory())?;
        index.members.resize(count, 0);
        for (view, source) in [(View::Globals, globals), (View::Publics, publics)] {
            let Some((bucket_count, entries)) = source else {
                continue;
            };
            let mut records = Vec::new();
            records
                .try_reserve_exact(entries.len())
                .map_err(|_| no_memory())?;
            for entry in entries {
                if let Some(bucket) = entry.bucket {
                    records.push((bucket, entry.symbol));
                }
                if let Some(bits) = index.members.get_mut(entry.symbol.0 as usize) {
                    *bits |= view.bit();
                }
            }
            records.sort_unstable();
            let buckets = SourceBuckets {
                count: Some(bucket_count),
                records,
            };
            match view {
                View::Globals => index.globals = buckets,
                View::Publics => index.publics = buckets,
            }
        }
        Ok(index)
    }

    /// Hash the current name of the record that `get_mut` lent last.
    fn settle(&mut self, records: &[Option<SymbolRecord>]) {
        if let Some(symbol) = self.pending.take()
            && let Some(name) = name_of(records, symbol)
        {
            self.changed.insert((hash_string_v1(name), symbol));
        }
    }

    /// Record a new record `symbol` of `view` with the name `name`.
    pub(super) fn add(
        &mut self,
        records: &[Option<SymbolRecord>],
        view: View,
        symbol: SymbolHandle,
        name: Option<&[u8]>,
    ) -> Result<()> {
        self.settle(records);
        let position = symbol.0 as usize;
        if self.members.len() <= position {
            self.members
                .try_reserve(position + 1 - self.members.len())
                .map_err(|_| no_memory())?;
            self.members.resize(position + 1, 0);
        }
        self.members[position] |= view.bit();
        if let Some(name) = name {
            self.changed.insert((hash_string_v1(name), symbol));
        }
        Ok(())
    }

    /// Record that `get_mut` lends the record `symbol`: its name can change.
    pub(super) fn lend(&mut self, records: &[Option<SymbolRecord>], symbol: SymbolHandle) {
        self.settle(records);
        self.pending = Some(symbol);
    }

    /// Record the removal of the record `symbol` from both indexes.
    pub(super) fn remove(&mut self, records: &[Option<SymbolRecord>], symbol: SymbolHandle) {
        self.settle(records);
        if let Some(bits) = self.members.get_mut(symbol.0 as usize) {
            *bits = 0;
        }
    }

    /// Return the candidate records of `view` for the name `name`. A
    /// candidate can have another name; the caller compares the names.
    pub(super) fn candidates<'i>(
        &'i self,
        view: View,
        name: &[u8],
    ) -> impl Iterator<Item = SymbolHandle> + 'i {
        let source = match view {
            View::Globals => &self.globals,
            View::Publics => &self.publics,
        };
        let hash = hash_string_v1(name);
        let changed = self
            .changed
            .range((hash, SymbolHandle(0))..=(hash, SymbolHandle(u32::MAX)))
            .map(|&(_, symbol)| symbol);
        source
            .candidates(name)
            .iter()
            .map(|&(_, symbol)| symbol)
            .chain(changed.chain(self.pending).filter(move |symbol| {
                self.members
                    .get(symbol.0 as usize)
                    .is_some_and(|bits| bits & view.bit() != 0)
            }))
    }
}

/// Return the name of the record `symbol`, if the record is present.
fn name_of(records: &[Option<SymbolRecord>], symbol: SymbolHandle) -> Option<&[u8]> {
    records
        .get(symbol.0 as usize)
        .and_then(Option::as_ref)
        .and_then(SymbolRecord::name)
}
