//! The TPI and IPI records of a compaction.

use alloc::vec::Vec;

use super::map::{IndexMaps, SpaceMap};
use super::records;
use crate::native::IndexSpace;
use crate::native::codeview::id_record::IdRecord;
use crate::native::codeview::type_record::TypeLeaf;
use crate::native::editor::types::{ListRecord, RecordList};
use crate::native::output::no_memory;
use crate::native::types::HashAdjuster;
use crate::{Error, ErrorKind, Result};

/// A TPI or IPI record that a compaction rewrites.
pub(in crate::native::editor::plan) trait CompactRecord:
    ListRecord
{
    /// The index space of the records.
    const SPACE: IndexSpace;

    /// Return the record with output indices, or `None` if no index changes.
    fn remap(&self, maps: &IndexMaps) -> Result<Option<Self>>;

    /// Write the output indices into the framed source bytes of a record.
    fn remap_bytes(bytes: &mut [u8], maps: &IndexMaps) -> Result<bool>;
}

impl CompactRecord for TypeLeaf<'static> {
    const SPACE: IndexSpace = IndexSpace::TypeIndex;

    fn remap(&self, maps: &IndexMaps) -> Result<Option<Self>> {
        records::type_record(self, maps)
    }

    fn remap_bytes(bytes: &mut [u8], maps: &IndexMaps) -> Result<bool> {
        records::type_record_bytes(bytes, maps)
    }
}

impl CompactRecord for IdRecord<'static> {
    const SPACE: IndexSpace = IndexSpace::IdIndex;

    fn remap(&self, maps: &IndexMaps) -> Result<Option<Self>> {
        records::id_record(self, maps)
    }

    fn remap_bytes(bytes: &mut [u8], maps: &IndexMaps) -> Result<bool> {
        records::id_record_bytes(bytes, maps)
    }
}

/// The records of a compacted TPI or IPI stream.
pub(in crate::native::editor::plan) struct Compacted {
    /// The framed records in output order.
    pub(in crate::native::editor::plan) records: Vec<u8>,
    /// The hash adjusters with output indices. An adjuster of a removed
    /// record goes away.
    pub(in crate::native::editor::plan) adjusters: Vec<HashAdjuster>,
}

/// Write the records of `list` that are not removed, with output indices.
///
/// `own` is the map of the space of `list`, or `None` if the space does not
/// compact. A source record keeps its source bytes, except the changed index
/// fields. An error of a record names it: [`Error::record`] gives its space
/// and its model index.
pub(in crate::native::editor::plan) fn compact<T: CompactRecord>(
    list: &RecordList<T>,
    own: Option<&SpaceMap>,
    maps: &IndexMaps,
) -> Result<Compacted> {
    let mut records = Vec::new();
    for (index, entry) in (list.begin..).zip(&list.entries) {
        if entry.removed {
            continue;
        }
        let named = |error: Error| error.with_record(T::SPACE, index);
        if list.unchanged(entry).map_err(named)? {
            let bytes = list.source_bytes(entry).map_err(named)?.ok_or_else(|| {
                Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("source record has no bytes")
            })?;
            let start = records.len();
            records.try_reserve(bytes.len()).map_err(|_| no_memory())?;
            records.extend_from_slice(bytes);
            T::remap_bytes(&mut records[start..], maps).map_err(named)?;
        } else {
            match entry.value.remap(maps).map_err(named)? {
                Some(value) => value.encode_record(&mut records),
                None => entry.value.encode_record(&mut records),
            }
            .map_err(named)?;
        }
    }
    let mut adjusters = Vec::new();
    for adjuster in &list.adjusters {
        let index = match own {
            Some(map) if map.is_removed(adjuster.index) => continue,
            Some(map) => match map.lookup(adjuster.index) {
                super::map::Lookup::Index(index) => index,
                _ => {
                    return Err(Error::new(ErrorKind::InvalidReference, None)
                        .with_context("a hash adjuster names an index after the last record")
                        .with_index_space(T::SPACE));
                }
            },
            None => adjuster.index,
        };
        adjusters.try_reserve(1).map_err(|_| no_memory())?;
        adjusters.push(HashAdjuster {
            name_offset: adjuster.name_offset,
            index,
        });
    }
    Ok(Compacted { records, adjusters })
}
