//! The canonical Portable PDB dump, printed with pdbrs.
//!
//! The .NET verifier in `fixtures/dotnet/verifier/` prints the same format with
//! `System.Reflection.Metadata`. `fixtures/README.md` describes the format. Each line has a
//! keyword and `name=value` fields. Counts, row IDs and GUID indexes are decimal. Offsets,
//! sizes, flags, masks and tokens are hexadecimal.

mod custom;

use std::fmt::Write as _;

use pdbrs::io::ReadAt;
use pdbrs::portable::Pdb;
use pdbrs::portable::metadata::{
    CodedIndex, CustomDebugInformationRow, DocumentRow, ImportScopeRow, LocalConstantRow,
    LocalScopeRow, LocalVariableRow, MetadataStream, MethodDebugInformationRow,
    StateMachineMethodRow, TableId, TableRow,
};

/// The format version on the first line.
const VERSION: u32 = 1;

/// The heaps in dump order.
const HEAPS: [MetadataStream; 4] = [
    MetadataStream::Strings,
    MetadataStream::UserStrings,
    MetadataStream::Blob,
    MetadataStream::Guid,
];

/// The tables of simple indexes in debug table columns, in dump order.
const INDEXED_TABLES: [TableId; 5] = [
    TableId::METHOD_DEF,
    TableId::DOCUMENT,
    TableId::LOCAL_VARIABLE,
    TableId::LOCAL_CONSTANT,
    TableId::IMPORT_SCOPE,
];

/// Print the dump of a Portable PDB.
pub fn dump<S: ReadAt>(pdb: &Pdb<S>) -> pdbrs::Result<String> {
    let mut out = Dump::default();
    out.line(format_args!("portable-dump version={VERSION}"));
    headers(pdb, &mut out);
    rows(pdb, &mut out)?;
    custom::custom(pdb, &mut out)?;
    Ok(out.text)
}

/// The text of a dump.
#[derive(Default)]
struct Dump {
    text: String,
}

impl Dump {
    fn line(&mut self, line: std::fmt::Arguments<'_>) {
        // Writing to a `String` cannot fail.
        let _ = self.text.write_fmt(line);
        self.text.push('\n');
    }
}

/// Print the metadata root, the heaps, the `#Pdb` stream, the `#~` header and the layout.
fn headers<S: ReadAt>(pdb: &Pdb<S>, out: &mut Dump) {
    let metadata = pdb.metadata();
    out.line(format_args!(
        "metadata version={} streams={}",
        quote(metadata.version_string()),
        metadata.streams().len()
    ));
    for stream in metadata.streams() {
        out.line(format_args!(
            "stream name={} offset={:#x} size={:#x}",
            String::from_utf8_lossy(stream.name()),
            stream.offset(),
            stream.size()
        ));
    }
    for heap in HEAPS {
        out.line(format_args!(
            "heap name={} size={:#x}",
            heap.name(),
            pdb.heap_size(heap).unwrap_or(0)
        ));
    }

    let header = pdb.pdb_stream();
    let id = header.id().iter().fold(String::new(), |mut id, byte| {
        let _ = write!(id, "{byte:02x}");
        id
    });
    out.line(format_args!(
        "pdb id={id} entry-point={:#010x}",
        header.entry_point_token().get()
    ));
    for (table, rows) in header.external_tables() {
        out.line(format_args!(
            "pdb-rows table={:#04x} rows={rows}",
            table.number()
        ));
    }

    let tables = pdb.tables();
    let (major, minor) = tables.version();
    out.line(format_args!(
        "tables version={major}.{minor} heap-sizes={:#04x} valid={:#018x} sorted={:#018x}",
        tables.heap_sizes(),
        tables.valid_mask(),
        tables.sorted_mask()
    ));
    for (heap, width) in [
        (MetadataStream::Strings, tables.string_index_width()),
        (MetadataStream::Guid, tables.guid_index_width()),
        (MetadataStream::Blob, tables.blob_index_width()),
    ] {
        out.line(format_args!("index heap={} width={width}", heap.name()));
    }
    for table in INDEXED_TABLES {
        out.line(format_args!(
            "index table={} width={}",
            table.name().unwrap_or_default(),
            tables.table_index_width(table)
        ));
    }
    out.line(format_args!(
        "index coded=HasCustomDebugInformation width={}",
        tables.coded_index_width(CodedIndex::HasCustomDebugInformation)
    ));
    for table in TableId::DEBUG_TABLES {
        let rows = tables.row_count(table);
        let location = match tables.table_offset(table) {
            Some(offset) if rows > 0 => format!(" offset={offset:#x}"),
            _ => String::new(),
        };
        out.line(format_args!(
            "table name={} id={:#04x} rows={rows} row-size={:#x}{location}",
            table.name().unwrap_or_default(),
            table.number(),
            tables.row_size(table).unwrap_or(0)
        ));
    }
}

/// Print the raw rows of the eight debug tables.
fn rows<S: ReadAt>(pdb: &Pdb<S>, out: &mut Dump) -> pdbrs::Result<()> {
    table(pdb, out, |row: DocumentRow| {
        format!(
            "name={:#x} hash-algorithm={} hash={:#x} language={}",
            row.name.get(),
            row.hash_algorithm.get(),
            row.hash.get(),
            row.language.get()
        )
    })?;
    table(pdb, out, |row: MethodDebugInformationRow| {
        format!(
            "document={} sequence-points={:#x}",
            row.document.get(),
            row.sequence_points.get()
        )
    })?;
    table(pdb, out, |row: LocalScopeRow| {
        format!(
            "method={} import-scope={} variable-list={} constant-list={} start-offset={:#x} length={:#x}",
            row.method.get(),
            row.import_scope.get(),
            row.variable_list.get(),
            row.constant_list.get(),
            row.start_offset.get(),
            row.length
        )
    })?;
    table(pdb, out, |row: LocalVariableRow| {
        format!(
            "attributes={:#06x} index={} name={:#x}",
            row.attributes,
            row.index,
            row.name.get()
        )
    })?;
    table(pdb, out, |row: LocalConstantRow| {
        format!(
            "name={:#x} signature={:#x}",
            row.name.get(),
            row.signature.get()
        )
    })?;
    table(pdb, out, |row: ImportScopeRow| {
        format!(
            "parent={} imports={:#x}",
            row.parent.get(),
            row.imports.get()
        )
    })?;
    table(pdb, out, |row: StateMachineMethodRow| {
        format!(
            "move-next={} kickoff={}",
            row.move_next_method.get(),
            row.kickoff_method.get()
        )
    })?;
    table(pdb, out, |row: CustomDebugInformationRow| {
        format!(
            "parent={:#010x} kind={} value={:#x}",
            row.parent.get(),
            row.kind.get(),
            row.value.get()
        )
    })
}

/// Print each row of one table with the fields that `fields` formats.
fn table<S: ReadAt, T: TableRow>(
    pdb: &Pdb<S>,
    out: &mut Dump,
    fields: impl Fn(T) -> String,
) -> pdbrs::Result<()> {
    let name = T::TABLE.name().unwrap_or_default();
    for (index, row) in pdb.rows::<T>().enumerate() {
        let (_, row) = row?;
        out.line(format_args!(
            "row table={name} id={} {}",
            index + 1,
            fields(row)
        ));
    }
    Ok(())
}

/// Quote bytes. Printable ASCII stays; other bytes, `"` and `\` become `\xNN`.
fn quote(bytes: &[u8]) -> String {
    let mut text = String::from("\"");
    for &byte in bytes {
        if (0x20..0x7f).contains(&byte) && byte != b'"' && byte != b'\\' {
            text.push(char::from(byte));
        } else {
            let _ = write!(text, "\\x{byte:02x}");
        }
    }
    text.push('"');
    text
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn quote_escapes_bytes_outside_printable_ascii() {
        assert_eq!(quote(b"PDB v1.0"), "\"PDB v1.0\"");
        assert_eq!(quote(b"a\"b\\c\x01\xff"), "\"a\\x22b\\x5cc\\x01\\xff\"");
    }
}
