//! Index widths and the encoding of the metadata root, the `#Pdb` stream and the `#~` stream.

use alloc::vec::Vec;

use super::check::Counts;
use super::parts::{DebugTables, ExternalRowCounts, PdbParts};
use crate::PortableLocation;
use crate::portable::metadata::{
    CodedIndex, METADATA_SIGNATURE, MetadataStream, MetadataToken, MethodDefId, TableId,
};
use crate::{Error, ErrorKind, Result};

/// A heap index uses 4 bytes if the heap has more bytes than this value.
const SMALL_HEAP: u32 = 0xFFFF;

/// A table index uses 4 bytes if the table has this many rows or more.
const LARGE_TABLE: u32 = 1 << 16;

/// The `HeapSizes` flag for 4-byte `#Strings` indexes.
const LARGE_STRINGS: u8 = 0x01;
/// The `HeapSizes` flag for 4-byte `#GUID` indexes.
const LARGE_GUIDS: u8 = 0x02;
/// The `HeapSizes` flag for 4-byte `#Blob` indexes.
const LARGE_BLOBS: u8 = 0x04;

/// The debug tables that the `Sorted` mask marks if they have rows.
const SORTED_DEBUG_TABLES: u64 = TableId::LOCAL_SCOPE.bit()
    | TableId::STATE_MACHINE_METHOD.bit()
    | TableId::CUSTOM_DEBUG_INFORMATION.bit();

/// The size of the fixed `#~` header fields.
const TABLES_HEADER_LEN: usize = 24;

/// The stream names in output order.
pub(super) const STREAM_NAMES: [&[u8]; 6] =
    [b"#Pdb", b"#~", b"#Strings", b"#US", b"#GUID", b"#Blob"];

/// The column widths of the rows.
pub(super) struct Widths {
    string: u8,
    guid: u8,
    blob: u8,
    counts: [u32; 64],
}

impl Widths {
    /// Compute the widths from the unpadded heap sizes and the row counts.
    pub(super) fn new(parts: &PdbParts, counts: &Counts) -> Self {
        let heap = |size: u32| if size > SMALL_HEAP { 4 } else { 2 };
        Self {
            string: heap(parts.strings.size()),
            guid: heap(parts.guids.size()),
            blob: heap(parts.blobs.size()),
            counts: *counts.as_array(),
        }
    }

    /// Return the `HeapSizes` flags.
    fn heap_sizes(&self) -> u8 {
        let flag = |width: u8, flag: u8| if width == 4 { flag } else { 0 };
        flag(self.string, LARGE_STRINGS)
            | flag(self.guid, LARGE_GUIDS)
            | flag(self.blob, LARGE_BLOBS)
    }

    /// Return the width of a simple index into `table`.
    fn index(&self, table: TableId) -> u8 {
        if self.counts[table.index()] < LARGE_TABLE {
            2
        } else {
            4
        }
    }

    /// Return the width of a `HasCustomDebugInformation` coded index.
    fn coded(&self) -> u8 {
        CodedIndex::HasCustomDebugInformation.width(&self.counts)
    }

    /// Return the row size of a debug table.
    fn row_size(&self, table: TableId) -> usize {
        let (string, guid, blob) = (self.string, self.guid, self.blob);
        let size = match table {
            TableId::DOCUMENT => blob + guid + blob + guid,
            TableId::METHOD_DEBUG_INFORMATION => self.index(TableId::DOCUMENT) + blob,
            TableId::LOCAL_SCOPE => {
                self.index(TableId::METHOD_DEF)
                    + self.index(TableId::IMPORT_SCOPE)
                    + self.index(TableId::LOCAL_VARIABLE)
                    + self.index(TableId::LOCAL_CONSTANT)
                    + 8
            }
            TableId::LOCAL_VARIABLE => 4 + string,
            TableId::LOCAL_CONSTANT => string + blob,
            TableId::IMPORT_SCOPE => self.index(TableId::IMPORT_SCOPE) + blob,
            TableId::STATE_MACHINE_METHOD => 2 * self.index(TableId::METHOD_DEF),
            _ => self.coded() + guid + blob,
        };
        usize::from(size)
    }
}

/// Return the number of padding bytes after `length` bytes for a 4-byte boundary.
pub(super) const fn padding(length: usize) -> usize {
    (4 - length % 4) % 4
}

/// Make a [`ErrorKind::LimitExceeded`] error for an output that does not fit.
pub(super) fn too_large(stream: MetadataStream) -> Error {
    Error::new(ErrorKind::LimitExceeded, None)
        .with_context("metadata is larger than 4 GiB or does not fit in memory")
        .with_portable(stream.location())
}

/// The bytes of one row.
struct Row<'a>(&'a mut Vec<u8>);

impl Row<'_> {
    /// Append a value of `width` bytes (2 or 4).
    fn value(&mut self, width: u8, value: u32) -> &mut Self {
        if width == 2 {
            // The checks and the widths make sure that the value fits.
            self.0.extend_from_slice(&value.to_le_bytes()[..2]);
        } else {
            self.0.extend_from_slice(&value.to_le_bytes());
        }
        self
    }
}

/// Encode the `#~` stream with its trailing zero byte and padding.
pub(super) fn tables_stream(tables: &DebugTables, widths: &Widths) -> Result<Vec<u8>> {
    let mut valid = 0;
    let mut size = TABLES_HEADER_LEN;
    for table in TableId::DEBUG_TABLES {
        let rows = tables.len(table);
        if rows > 0 {
            valid |= table.bit();
            size = rows
                .checked_mul(widths.row_size(table))
                .and_then(|bytes| bytes.checked_add(size + 4))
                .ok_or_else(|| too_large(MetadataStream::Tables))?;
        }
    }
    size += 1;
    size += padding(size);
    if u32::try_from(size).is_err() {
        return Err(too_large(MetadataStream::Tables));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| too_large(MetadataStream::Tables))?;
    bytes.extend_from_slice(&[0, 0, 0, 0, 2, 0, widths.heap_sizes(), 1]);
    bytes.extend_from_slice(&valid.to_le_bytes());
    bytes.extend_from_slice(&(valid & SORTED_DEBUG_TABLES).to_le_bytes());
    for table in TableId::DEBUG_TABLES {
        if let Ok(count @ 1..) = u32::try_from(tables.len(table)) {
            bytes.extend_from_slice(&count.to_le_bytes());
        }
    }
    encode_rows(tables, widths, &mut bytes)?;
    bytes.push(0);
    bytes.resize(size, 0);
    Ok(bytes)
}

/// Append the rows of all debug tables in table order.
fn encode_rows(tables: &DebugTables, widths: &Widths, bytes: &mut Vec<u8>) -> Result<()> {
    let (string, guid, blob) = (widths.string, widths.guid, widths.blob);
    let index = |table| widths.index(table);
    for row in &tables.documents {
        Row(bytes)
            .value(blob, row.name.0)
            .value(guid, row.hash_algorithm.0)
            .value(blob, row.hash.0)
            .value(guid, row.language.0);
    }
    for row in &tables.method_debug_information {
        Row(bytes)
            .value(index(TableId::DOCUMENT), row.document.0)
            .value(blob, row.sequence_points.0);
    }
    for row in &tables.local_scopes {
        Row(bytes)
            .value(index(TableId::METHOD_DEF), row.method.0)
            .value(index(TableId::IMPORT_SCOPE), row.import_scope.0)
            .value(index(TableId::LOCAL_VARIABLE), row.variable_list.0)
            .value(index(TableId::LOCAL_CONSTANT), row.constant_list.0)
            .value(4, row.start_offset.0)
            .value(4, row.length);
    }
    for row in &tables.local_variables {
        bytes.extend_from_slice(&row.attributes.to_le_bytes());
        bytes.extend_from_slice(&row.index.to_le_bytes());
        Row(bytes).value(string, row.name.0);
    }
    for row in &tables.local_constants {
        Row(bytes)
            .value(string, row.name.0)
            .value(blob, row.signature.0);
    }
    for row in &tables.import_scopes {
        Row(bytes)
            .value(index(TableId::IMPORT_SCOPE), row.parent.0)
            .value(blob, row.imports.0);
    }
    for row in &tables.state_machine_methods {
        Row(bytes)
            .value(index(TableId::METHOD_DEF), row.move_next_method.0)
            .value(index(TableId::METHOD_DEF), row.kickoff_method.0);
    }
    for row in &tables.custom_debug_information {
        // The checks make sure that the parent has a coded value.
        let parent = CodedIndex::HasCustomDebugInformation
            .encode(row.parent)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidReference, None).with_portable(
                    PortableLocation::new()
                        .with_stream(MetadataStream::Tables)
                        .with_table(TableId::CUSTOM_DEBUG_INFORMATION),
                )
            })?;
        Row(bytes)
            .value(widths.coded(), parent)
            .value(guid, row.kind.0)
            .value(blob, row.value.0);
    }
    Ok(())
}

/// Encode the `#Pdb` stream with a zero PDB ID.
pub(super) fn pdb_stream(
    entry_point: Option<MethodDefId>,
    external: &ExternalRowCounts,
) -> Vec<u8> {
    let tables: Vec<(TableId, u32)> = (0_u8..64)
        .filter_map(TableId::from_raw)
        .map(|table| (table, external.get(table)))
        .filter(|&(_, count)| count > 0)
        .collect();
    let mut bytes = Vec::with_capacity(32 + 4 * tables.len());
    bytes.extend_from_slice(&[0; 20]);
    let token = entry_point
        .and_then(MethodDefId::token)
        .map_or(0, MetadataToken::raw);
    bytes.extend_from_slice(&token.to_le_bytes());
    let mask = tables
        .iter()
        .fold(0_u64, |mask, (table, _)| mask | table.bit());
    bytes.extend_from_slice(&mask.to_le_bytes());
    for (_, count) in tables {
        bytes.extend_from_slice(&count.to_le_bytes());
    }
    bytes
}

/// Encode the metadata root and the stream headers for streams of the given unpadded sizes,
/// in the order of [`STREAM_NAMES`].
pub(super) fn root(version: &[u8], sizes: [usize; 6]) -> Result<Vec<u8>> {
    let version_len = version.len() + 1 + padding(version.len() + 1);
    let headers: usize = STREAM_NAMES
        .iter()
        .map(|name| 8 + name.len() + 1 + padding(name.len() + 1))
        .sum();
    let header_len = 16 + version_len + 4 + headers;
    let mut bytes = Vec::with_capacity(header_len);
    bytes.extend_from_slice(&METADATA_SIGNATURE);
    bytes.extend_from_slice(&[1, 0, 1, 0, 0, 0, 0, 0]);
    // The checks limit the version string to 254 bytes.
    let length = u32::try_from(version_len).map_err(|_| too_large(MetadataStream::Root))?;
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(version);
    bytes.resize(16 + version_len, 0);
    bytes.extend_from_slice(&[0, 0, 6, 0]);
    let mut offset = header_len;
    for (name, size) in STREAM_NAMES.iter().zip(sizes) {
        let padded = size + padding(size);
        let fields = u32::try_from(offset).ok().zip(u32::try_from(padded).ok());
        let Some((offset_field, size_field)) = fields else {
            return Err(too_large(MetadataStream::Root));
        };
        bytes.extend_from_slice(&offset_field.to_le_bytes());
        bytes.extend_from_slice(&size_field.to_le_bytes());
        bytes.extend_from_slice(name);
        bytes.resize(bytes.len() + 1 + padding(name.len() + 1), 0);
        offset = offset
            .checked_add(padded)
            .ok_or_else(|| too_large(MetadataStream::Root))?;
    }
    if u32::try_from(offset).is_err() {
        return Err(too_large(MetadataStream::Root));
    }
    Ok(bytes)
}
