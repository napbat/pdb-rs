//! Read a Portable PDB into new heaps and raw rows.
//!
//! The rebuild keeps the row IDs and the row order. It walks the `#Blob`, `#US` and `#GUID`
//! heaps in heap order and adds each entry to a new heap, so a heap without duplicate entries
//! keeps its offsets. It adds each string that a row refers to into a new `#Strings` heap and
//! gives the rows the new offsets.
//!
//! Document name blobs and import blobs contain `#Blob` offsets. If a `#Blob` offset changes,
//! the rebuild decodes these blobs with [`DocumentName`] and [`Imports`], gives them the new
//! offsets and adds the encoded blobs.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::heaps::{BlobHeap, GuidHeap, StringHeapBuilder, UserStringHeap, walk_entries};
use super::parts::{DebugTables, ExternalRowCounts, PdbParts};
use crate::PortableLocation;
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::debug::{DocumentName, Imports, RawImport};
use crate::portable::metadata::{
    BlobOffset, GuidIndex, MetadataStream, StringOffset, TableId, TableRow,
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
            .get(&offset.0)
            .map(|&new| BlobOffset(new))
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

/// Return the location of a cell.
fn cell(table: TableId, row: usize, column: &'static str) -> PortableLocation {
    PortableLocation::new()
        .with_stream(MetadataStream::Tables)
        .with_table(table)
        .with_row(u32::try_from(row + 1).unwrap_or(u32::MAX))
        .with_column(column)
}

/// Make an [`ErrorKind::InvalidReference`] error at a cell.
fn cell_error(table: TableId, row: usize, column: &'static str) -> Error {
    Error::new(ErrorKind::InvalidReference, None).with_portable(cell(table, row, column))
}

/// Give each `#Blob` offset of an import a new value.
fn map_import(
    import: RawImport,
    mut map: impl FnMut(BlobOffset) -> Result<BlobOffset>,
) -> Result<RawImport> {
    Ok(match import {
        RawImport::Namespace { namespace } => RawImport::Namespace {
            namespace: map(namespace)?,
        },
        RawImport::AssemblyNamespace {
            assembly,
            namespace,
        } => RawImport::AssemblyNamespace {
            assembly,
            namespace: map(namespace)?,
        },
        RawImport::Type { target_type } => RawImport::Type { target_type },
        RawImport::XmlNamespace { alias, namespace } => RawImport::XmlNamespace {
            alias: map(alias)?,
            namespace: map(namespace)?,
        },
        RawImport::AssemblyReferenceAlias { alias } => {
            RawImport::AssemblyReferenceAlias { alias: map(alias)? }
        }
        RawImport::AliasAssemblyReference { alias, assembly } => {
            RawImport::AliasAssemblyReference {
                alias: map(alias)?,
                assembly,
            }
        }
        RawImport::AliasNamespace { alias, namespace } => RawImport::AliasNamespace {
            alias: map(alias)?,
            namespace: map(namespace)?,
        },
        RawImport::AliasAssemblyNamespace {
            alias,
            assembly,
            namespace,
        } => RawImport::AliasAssemblyNamespace {
            alias: map(alias)?,
            assembly,
            namespace: map(namespace)?,
        },
        RawImport::AliasType { alias, target_type } => RawImport::AliasType {
            alias: map(alias)?,
            target_type,
        },
    })
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
    walk_entries(bytes, stream, |offset, entry| {
        map.insert(offset, add(entry)?);
        Ok(())
    })?;
    Ok(map)
}

impl<S: ReadAt> Pdb<S> {
    /// Read all rows and heap entries into new parts for a write plan.
    ///
    /// The parts keep the version string, the entry point, the external row counts, the row
    /// IDs and the row order. The `#Blob`, `#US` and `#GUID` heaps get the source entries in
    /// heap order without duplicates. The `#Strings` heap gets the strings that the rows refer
    /// to. A plan of the parts with
    /// [`IdentityPolicy::ContentDerived`](crate::portable::IdentityPolicy::ContentDerived) gives the
    /// same bytes as a deterministic `System.Reflection.Metadata` build of the same content,
    /// if all strings are ASCII.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if a `#Blob` or `#US` heap is not a sequence of
    ///   length-prefixed entries, or a `#US` entry has an even length.
    /// - [`ErrorKind::InvalidReference`] if a row or a document name or imports blob refers
    ///   to a blob offset that is not the start of an entry.
    /// - The errors of [`DocumentName::parse`], [`Imports`] and the encoders, if a `#Blob`
    ///   offset changes and a document name or an imports blob does not decode.
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
            blobs.add(entry).map(|offset| offset.0)
        })?);
        drop(source);
        let containers = if blob_map.is_stable(blobs.size()) {
            None
        } else {
            Some(self.rewrite_containers(&tables, &blob_map, &mut blobs)?)
        };
        rewrite_blobs(&mut tables, &blob_map)?;
        if let Some((names, imports)) = containers {
            for (row, name) in tables.documents.iter_mut().zip(names) {
                row.name = name;
            }
            for (row, blob) in tables.import_scopes.iter_mut().zip(imports) {
                row.imports = blob;
            }
        }

        let mut user_strings = UserStringHeap::new();
        let source = self
            .stream_bytes(MetadataStream::UserStrings)?
            .unwrap_or_default();
        walk_prefixed(&source, MetadataStream::UserStrings, |entry| {
            user_strings.add(entry).map(|offset| offset.0)
        })?;
        drop(source);

        let mut guids = GuidHeap::new();
        let mut guid_map = Vec::from([0]);
        let source = self.stream_bytes(MetadataStream::Guid)?.unwrap_or_default();
        for guid in source.as_chunks::<16>().0 {
            guid_map.push(guids.add(*guid)?.0);
        }
        drop(source);
        // The reader checks each GUID index of a row against the heap.
        let guid = |index: GuidIndex| {
            let new = usize::try_from(index.0)
                .ok()
                .and_then(|index| guid_map.get(index));
            GuidIndex(new.copied().unwrap_or(0))
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
            if let alloc::collections::btree_map::Entry::Vacant(entry) = values.entry(offset.0) {
                let value = self.string(offset)?;
                builder.add(&value)?;
                entry.insert(value);
            }
        }
        let heap = builder.finish()?;
        // The builder has each value, so each lookup succeeds.
        let new = |offset: StringOffset| {
            values
                .get(&offset.0)
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

    /// Decode the document name and imports blobs, give them the new `#Blob` offsets and add
    /// the encoded blobs. Return the new offsets of the `Name` and `Imports` columns.
    fn rewrite_containers(
        &self,
        tables: &DebugTables,
        map: &BlobMap,
        blobs: &mut BlobHeap,
    ) -> Result<(Vec<BlobOffset>, Vec<BlobOffset>)> {
        let mut names = Vec::with_capacity(tables.documents.len());
        let mut encoded = Vec::new();
        for (row, document) in tables.documents.iter().enumerate() {
            let (table, column) = (TableId::DOCUMENT, "Name");
            let blob = self.blob(document.name)?;
            let name = DocumentName::parse(&blob)
                .map_err(|error| error.with_portable(cell(table, row, column)))?;
            let parts = name
                .parts()
                .map(|part| map.get(part, table, row, column))
                .collect::<Result<Vec<_>>>()?;
            encoded.clear();
            DocumentName::encode_parts(name.separator(), &parts, &mut encoded)
                .map_err(|error| error.with_portable(cell(table, row, column)))?;
            names.push(blobs.add(&encoded)?);
        }
        let mut imports = Vec::with_capacity(tables.import_scopes.len());
        for (row, scope) in tables.import_scopes.iter().enumerate() {
            let (table, column) = (TableId::IMPORT_SCOPE, "Imports");
            let blob = self.blob(scope.imports)?;
            let mapped = Imports::parse(&blob)
                .map(|import| map_import(import?, |offset| map.get(offset, table, row, column)))
                .collect::<Result<Vec<_>>>()
                .map_err(|error| error.with_portable(cell(table, row, column)))?;
            encoded.clear();
            Imports::encode_parts(&mapped, &mut encoded)
                .map_err(|error| error.with_portable(cell(table, row, column)))?;
            imports.push(blobs.add(&encoded)?);
        }
        Ok((names, imports))
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
