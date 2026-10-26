//! Read a Portable PDB into new heaps and raw rows.
//!
//! The rebuild keeps the row IDs and the row order. It walks the `#Blob`, `#US` and `#GUID`
//! heaps in heap order and adds each entry to a new heap, so a heap without duplicate entries
//! keeps its offsets. It adds each string that a row refers to into a new `#Strings` heap and
//! gives the rows the new offsets.
//!
//! Document name blobs and import blobs contain `#Blob` offsets. This module does not decode
//! them, so it cannot change these offsets. If a `#Blob` offset changes and the `Document` or
//! `ImportScope` table has rows, the rebuild fails.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::heaps::{BlobHeap, GuidHeap, StringHeapBuilder, UserStringHeap};
use super::parts::{DebugTables, ExternalRowCounts, PdbParts};
use crate::error::PortableLocation;
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::compressed;
use crate::portable::metadata::{
    BlobOffset, GuidIndex, MetadataStream, StringOffset, TableId, TableRow, UserStringOffset,
};
use crate::{Error, ErrorKind, Result};

/// The new offset of each old `#Blob` entry.
struct BlobMap(BTreeMap<u32, u32>);

impl BlobMap {
    /// Return the new offset of an old blob offset of a cell.
    fn get(
        &self,
        offset: BlobOffset,
        table: TableId,
        row: usize,
        column: &'static str,
    ) -> Result<BlobOffset> {
        self.0
            .get(&offset.get())
            .map(|&new| BlobOffset::new(new))
            .ok_or_else(|| {
                cell_error(table, row, column)
                    .with_context("blob offset is not the start of a #Blob entry")
            })
    }

    /// Return `true` if each entry keeps its offset in a new heap of `size` bytes.
    ///
    /// An empty entry after the end of the new heap (for example a padding byte of the source
    /// heap) becomes the empty blob at offset 0. No entry of the new heap uses its offset.
    fn is_stable(&self, size: u32) -> bool {
        self.0
            .iter()
            .all(|(&old, &new)| old == new || (new == 0 && old >= size))
    }
}

/// Make an [`ErrorKind::InvalidReference`] error at a cell.
fn cell_error(table: TableId, row: usize, column: &'static str) -> Error {
    Error::new(ErrorKind::InvalidReference, None).with_portable(
        PortableLocation::new()
            .with_stream(MetadataStream::Tables)
            .with_table(table)
            .with_row(u32::try_from(row + 1).unwrap_or(u32::MAX))
            .with_column(column),
    )
}

/// Read all rows of a table.
fn rows<S: ReadAt, T: TableRow>(pdb: &Pdb<S>) -> Result<Vec<T>> {
    pdb.rows::<T>().map(|row| row.map(|(_, row)| row)).collect()
}

/// Walk a heap of length-prefixed entries and call `add` for each entry. Return the new
/// offset of each old entry.
fn walk_prefixed(
    bytes: &[u8],
    stream: MetadataStream,
    mut add: impl FnMut(&[u8]) -> Result<u32>,
) -> Result<BTreeMap<u32, u32>> {
    let mut map = BTreeMap::from([(0, 0)]);
    let mut position = 0;
    while position < bytes.len() {
        let rest = &bytes[position..];
        let invalid = || {
            Error::new(ErrorKind::InvalidFormat, None)
                .with_context("heap is not a sequence of length-prefixed entries")
                .with_portable(
                    stream
                        .location()
                        .with_heap_offset(u32::try_from(position).unwrap_or(u32::MAX)),
                )
        };
        let (length, after) = compressed::decode_unsigned(rest).map_err(|_| invalid())?;
        let start = rest.len() - after.len();
        let length = usize::try_from(length).map_err(|_| invalid())?;
        let entry = after.get(..length).ok_or_else(invalid)?;
        let old = u32::try_from(position).map_err(|_| invalid())?;
        map.insert(old, add(entry)?);
        position += start + length;
    }
    Ok(map)
}

impl<S: ReadAt> Pdb<S> {
    /// Read all rows and heap entries into new parts for a write plan.
    ///
    /// The parts keep the version string, the entry point, the external row counts, the row
    /// IDs and the row order. The `#Blob`, `#US` and `#GUID` heaps get the source entries in
    /// heap order without duplicates. The `#Strings` heap gets the strings that the rows refer
    /// to. A plan of the parts with [`IdPolicy::ContentDerived`](super::IdPolicy) gives the
    /// same bytes as a deterministic `System.Reflection.Metadata` build of the same content,
    /// if all strings are ASCII.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if a `#Blob` or `#US` heap is not a sequence of
    ///   length-prefixed entries, or a `#US` entry has an even length.
    /// - [`ErrorKind::InvalidReference`] if a row refers to a blob offset that is not the
    ///   start of an entry, or if a `#Blob` offset changes and the `Document` or
    ///   `ImportScope` table has rows. The document name and import blobs contain `#Blob`
    ///   offsets, and this function cannot change them.
    /// - [`ErrorKind::LimitExceeded`] if a heap is larger than
    ///   [`ReadOptions::max_stream_bytes`](crate::portable::ReadOptions::max_stream_bytes)
    ///   or an allocation fails.
    /// - The errors of [`Pdb::rows`] and [`Pdb::string`], and errors of the source.
    pub fn to_parts(&self) -> Result<PdbParts> {
        let mut tables = DebugTables {
            documents: rows(self)?,
            method_debug_information: rows(self)?,
            local_scopes: rows(self)?,
            local_variables: rows(self)?,
            local_constants: rows(self)?,
            import_scopes: rows(self)?,
            state_machine_methods: rows(self)?,
            custom_debug_information: rows(self)?,
        };

        let mut blobs = BlobHeap::new();
        let source = self.stream_bytes(MetadataStream::Blob)?.unwrap_or_default();
        let blob_map = BlobMap(walk_prefixed(&source, MetadataStream::Blob, |entry| {
            blobs.add(entry).map(BlobOffset::get)
        })?);
        drop(source);
        let has_blob_references = !tables.documents.is_empty() || !tables.import_scopes.is_empty();
        if has_blob_references && !blob_map.is_stable(blobs.size()) {
            return Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context(
                    "the rebuild changes #Blob offsets, and document name or import blobs can contain them",
                )
                .with_portable(MetadataStream::Blob.location()));
        }
        rewrite_blobs(&mut tables, &blob_map)?;

        let mut user_strings = UserStringHeap::new();
        let source = self
            .stream_bytes(MetadataStream::UserStrings)?
            .unwrap_or_default();
        walk_prefixed(&source, MetadataStream::UserStrings, |entry| {
            user_strings.add(entry).map(UserStringOffset::get)
        })?;
        drop(source);

        let mut guids = GuidHeap::new();
        let mut guid_map = Vec::from([0]);
        let source = self.stream_bytes(MetadataStream::Guid)?.unwrap_or_default();
        for guid in source.as_chunks::<16>().0 {
            guid_map.push(guids.add(*guid)?.get());
        }
        drop(source);
        // The reader checks each GUID index of a row against the heap.
        let guid = |index: GuidIndex| {
            let new = usize::try_from(index.get())
                .ok()
                .and_then(|index| guid_map.get(index));
            GuidIndex::new(new.copied().unwrap_or(0))
        };
        for row in &mut tables.documents {
            row.hash_algorithm = guid(row.hash_algorithm);
            row.language = guid(row.language);
        }
        for row in &mut tables.custom_debug_information {
            row.kind = guid(row.kind);
        }

        let strings = self.rewrite_strings(&mut tables)?;
        Ok(PdbParts {
            version: self.metadata().version_string().to_vec(),
            entry_point: self.pdb_stream().entry_point(),
            external: ExternalRowCounts::from_pdb_stream(self.pdb_stream()),
            strings,
            user_strings,
            blobs,
            guids,
            tables,
        })
    }

    /// Add the strings of the rows to a new heap and give the rows the new offsets.
    fn rewrite_strings(&self, tables: &mut DebugTables) -> Result<super::StringHeap> {
        let mut values: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
        let mut builder = StringHeapBuilder::new();
        let offsets = tables
            .local_variables
            .iter()
            .map(|row| row.name)
            .chain(tables.local_constants.iter().map(|row| row.name));
        for offset in offsets {
            if let alloc::collections::btree_map::Entry::Vacant(entry) = values.entry(offset.get())
            {
                let value = self.string(offset)?;
                builder.add(&value)?;
                entry.insert(value);
            }
        }
        let heap = builder.finish()?;
        // The builder has each value, so each lookup succeeds.
        let new = |offset: StringOffset| {
            values
                .get(&offset.get())
                .and_then(|value| heap.offset(value))
                .unwrap_or_default()
        };
        for row in &mut tables.local_variables {
            row.name = new(row.name);
        }
        for row in &mut tables.local_constants {
            row.name = new(row.name);
        }
        Ok(heap)
    }
}

/// Give each blob column the new offset.
fn rewrite_blobs(tables: &mut DebugTables, map: &BlobMap) -> Result<()> {
    for (row, document) in tables.documents.iter_mut().enumerate() {
        document.name = map.get(document.name, TableId::DOCUMENT, row, "Name")?;
        document.hash = map.get(document.hash, TableId::DOCUMENT, row, "Hash")?;
    }
    for (row, info) in tables.method_debug_information.iter_mut().enumerate() {
        let table = TableId::METHOD_DEBUG_INFORMATION;
        info.sequence_points = map.get(info.sequence_points, table, row, "SequencePoints")?;
    }
    for (row, constant) in tables.local_constants.iter_mut().enumerate() {
        constant.signature = map.get(
            constant.signature,
            TableId::LOCAL_CONSTANT,
            row,
            "Signature",
        )?;
    }
    for (row, scope) in tables.import_scopes.iter_mut().enumerate() {
        scope.imports = map.get(scope.imports, TableId::IMPORT_SCOPE, row, "Imports")?;
    }
    for (row, information) in tables.custom_debug_information.iter_mut().enumerate() {
        let table = TableId::CUSTOM_DEBUG_INFORMATION;
        information.value = map.get(information.value, table, row, "Value")?;
    }
    Ok(())
}
