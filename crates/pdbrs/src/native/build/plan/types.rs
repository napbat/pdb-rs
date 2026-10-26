//! The TPI and IPI streams of a write plan.

use alloc::vec::Vec;

use super::super::entry::{Entry, encode_id, encode_type};
use super::super::handle::{Resolver, record_index, unresolved};
use super::super::{PdbBuilder, RecordTable};
use super::BuildOptions;
use crate::Result;
use crate::native::codeview::id_record::IdTable;
use crate::native::codeview::index::{FIRST_RECORD_INDEX, IdIndex, TypeIndex};
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::native::types::{
    HashAdjuster, ReferenceLimits, TypeStream, TypeStreamOptions, TypeTable, encode_type_stream,
    validate_type_references,
};

/// A planned TPI or IPI stream.
pub(in crate::native::build) struct PlannedTypes {
    /// The stream bytes.
    pub(in crate::native::build) stream: Vec<u8>,
    /// The hash stream bytes, if the options select a hash stream.
    pub(in crate::native::build) hash: Option<Vec<u8>>,
    /// The parsed stream, for the checks of later streams.
    pub(in crate::native::build) parsed: TypeStream,
}

impl PlannedTypes {
    /// Return the index after the last record.
    pub(in crate::native::build) const fn index_end(&self) -> u32 {
        self.parsed.header().index_end
    }
}

/// Encode the records of `table` with `encode`.
fn records<'a, T>(
    table: &RecordTable<'a, T>,
    resolver: &Resolver<'_>,
    encode: impl Fn(&Entry<'a, T>, &Resolver<'_>, &mut Vec<u8>) -> Result<()>,
) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for entry in &table.records {
        let entry = entry
            .as_ref()
            .ok_or_else(|| unresolved("reserved handle has no record"))?;
        encode(entry, resolver, &mut bytes)?;
    }
    Ok(bytes)
}

/// Return the hash adjusters of `table` with their record indices.
fn adjusters<T>(table: &RecordTable<'_, T>) -> Result<Vec<HashAdjuster>> {
    let mut adjusters = Vec::new();
    adjusters
        .try_reserve_exact(table.adjusters.len())
        .map_err(|_| no_memory())?;
    for &(position, name_offset) in &table.adjusters {
        adjusters.push(HashAdjuster {
            name_offset,
            index: record_index(position, "hash adjuster index does not fit in 32 bits")?,
        });
    }
    Ok(adjusters)
}

/// Write a type stream and its hash stream.
fn write(
    records: &[u8],
    adjusters: &[HashAdjuster],
    options: &BuildOptions,
    hash_stream: Option<StreamId>,
) -> Result<PlannedTypes> {
    let stream_options = TypeStreamOptions::default()
        .with_index_begin(FIRST_RECORD_INDEX)
        .with_hash_bucket_count(options.hash_buckets())
        .with_hash_stream(hash_stream)
        .with_hash_aux_stream(None)
        .with_producer(options.type_hash_producer())
        .with_adjusters(adjusters);
    let written = encode_type_stream(records, &stream_options)?;
    let parsed = TypeStream::parse(written.stream.clone())?;
    Ok(PlannedTypes {
        stream: written.stream,
        hash: written.hash_stream,
        parsed,
    })
}

impl PdbBuilder<'_> {
    /// Encode and check the TPI stream.
    pub(super) fn type_stream(
        &self,
        resolver: &Resolver<'_>,
        options: &BuildOptions,
        hash_stream: Option<StreamId>,
    ) -> Result<PlannedTypes> {
        let bytes = records(&self.types, resolver, encode_type)?;
        let planned = write(&bytes, &adjusters(&self.types)?, options, hash_stream)?;
        validate_type_references(
            &TypeTable::new(&planned.parsed)?,
            ReferenceLimits::default(),
        )?;
        Ok(planned)
    }

    /// Encode and check the IPI stream.
    pub(super) fn id_stream(
        &self,
        resolver: &Resolver<'_>,
        options: &BuildOptions,
        hash_stream: Option<StreamId>,
        tpi: &PlannedTypes,
    ) -> Result<PlannedTypes> {
        let bytes = records(&self.ids, resolver, encode_id)?;
        let planned = write(&bytes, &adjusters(&self.ids)?, options, hash_stream)?;
        IdTable::new(&planned.parsed)?.check(TypeIndex(tpi.index_end()))?;
        Ok(planned)
    }
}

/// Return the ID index end of a planned IPI stream.
pub(in crate::native::build) const fn id_end(ipi: &PlannedTypes) -> IdIndex {
    IdIndex(ipi.index_end())
}
