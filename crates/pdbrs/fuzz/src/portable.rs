//! Portable PDB readers: the metadata, the tables, the heaps and the debug blobs.

pub mod blobs;
pub mod build;
pub mod custom;
pub mod embedded;
pub mod rebuild;

use pdbrs::io::ReadAt;
use pdbrs::portable::embedded::{EMBEDDED_PDB_SIGNATURE, decode_embedded};
use pdbrs::portable::metadata::{
    CustomDebugInformationRow, DocumentRow, ImportScopeRow, LocalConstantRow, LocalScopeRow,
    LocalVariableRow, MetadataRoot, MetadataStream, MethodDebugInformationRow,
    StateMachineMethodRow, TableId, TableRow,
};
use pdbrs::portable::{Pdb, ReadOptions};

use crate::{MAX_STREAM, ok};

/// The most rows of one table that a walk visits, to bound the time of one run.
const MAX_ROWS: usize = 1 << 14;

/// The read options of the targets: the limits keep one run small.
#[must_use]
pub(crate) fn options() -> ReadOptions {
    ReadOptions::default()
        .with_max_streams(64)
        .with_max_value_bytes(MAX_STREAM)
        .with_max_stream_bytes(MAX_STREAM)
}

/// Read a Portable PDB or an `MPDB` payload and decode every row, heap value and blob.
pub fn pdb(data: &[u8]) {
    if data.starts_with(&EMBEDDED_PDB_SIGNATURE) {
        if let Some(decoded) = ok(decode_embedded(data, MAX_STREAM)) {
            walk_bytes(&decoded);
        }
    } else {
        walk_bytes(data);
    }
}

/// Read the metadata root and detect the format of the input.
pub fn metadata(data: &[u8]) {
    let _ = pdbrs::probe(data);
    let _ = pdbrs::Pdb::parse(data).map(|pdb| pdb.format());
    if let Some(root) = ok(MetadataRoot::read(data, 64)) {
        let _ = (root.version(), root.version_string(), root.flags());
        let _ = root.header_len();
        for stream in root.streams() {
            let _ = (
                stream.name(),
                stream.stream(),
                stream.offset(),
                stream.size(),
            );
        }
        for kind in [
            MetadataStream::Strings,
            MetadataStream::UserStrings,
            MetadataStream::Blob,
            MetadataStream::Guid,
        ] {
            let _ = root.stream(kind);
        }
    }
}

/// Parse `data` as a Portable PDB and walk it.
fn walk_bytes(data: &[u8]) {
    if let Some(pdb) = ok(Pdb::with_options(data, options())) {
        walk(&pdb);
    }
}

/// Decode every row, heap value and blob of a Portable PDB, and validate the tables.
pub(crate) fn walk<S: ReadAt>(pdb: &Pdb<S>) {
    let _ = pdb.identity();
    let header = pdb.pdb_stream();
    let _ = (header.entry_point(), header.referenced_tables());
    let _ = header.external_tables().count();
    let tables = pdb.tables();
    for table in TableId::DEBUG_TABLES {
        let _ = (tables.row_count(table), tables.row_size(table));
        let _ = tables.table_offset(table);
    }
    for stream in [
        MetadataStream::Strings,
        MetadataStream::UserStrings,
        MetadataStream::Blob,
        MetadataStream::Guid,
    ] {
        let _ = pdb.heap_size(stream);
        let _ = pdb.stream_bytes(stream);
    }
    rows::<_, DocumentRow>(pdb, |pdb, row| {
        let _ = (pdb.blob(row.name), pdb.blob(row.hash));
        let _ = (pdb.guid(row.hash_algorithm), pdb.guid(row.language));
    });
    rows::<_, MethodDebugInformationRow>(pdb, |pdb, row| {
        let _ = pdb.blob(row.sequence_points);
    });
    rows::<_, LocalScopeRow>(pdb, |_, _| {});
    rows::<_, LocalVariableRow>(pdb, |pdb, row| {
        let _ = pdb.string(row.name);
    });
    rows::<_, LocalConstantRow>(pdb, |pdb, row| {
        let _ = (pdb.string(row.name), pdb.blob(row.signature));
    });
    rows::<_, ImportScopeRow>(pdb, |pdb, row| {
        let _ = pdb.blob(row.imports);
    });
    rows::<_, StateMachineMethodRow>(pdb, |_, _| {});
    rows::<_, CustomDebugInformationRow>(pdb, |pdb, row| {
        let _ = (pdb.guid(row.kind), pdb.blob(row.value));
    });
    blobs::walk_debug(pdb);
    custom::walk_rows(pdb);
}

/// Read the rows of one table and call `visit` for each row.
fn rows<S: ReadAt, T: TableRow>(pdb: &Pdb<S>, mut visit: impl FnMut(&Pdb<S>, T)) {
    for row in pdb.rows::<T>().take(MAX_ROWS) {
        let Some((_, row)) = ok(row) else {
            break;
        };
        visit(pdb, row);
    }
}
