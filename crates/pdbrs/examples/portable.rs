//! Read every table row and heap value of a standalone Portable PDB and print counts.
//!
//! The example opens the file with `pdbrs::portable::Pdb`. It prints the metadata version, the
//! stream headers, the heap sizes, the `#Pdb` fields and the row count of each debug table.
//! Then it reads each row of the eight debug tables and each string, blob and GUID that a row
//! refers to. It stops with an error at the first value that it cannot read.
//!
//! Then it decodes each custom debug information value, encodes it again and compares the
//! bytes. It stops with an error at a value that does not decode. A value of an unknown kind
//! is counted, not an error. The example does not decode the other blobs (document names,
//! sequence points and imports). It reads their bytes only.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Write as _;

use pdbrs::portable::Pdb;
use pdbrs::portable::custom::CustomDebugValue;
use pdbrs::portable::metadata::{
    BlobOffset, CustomDebugInformationRow, DocumentRow, GuidIndex, ImportScopeRow,
    LocalConstantRow, LocalScopeRow, LocalVariableRow, MetadataStream, MethodDebugInformationRow,
    StateMachineMethodRow, StringOffset, TableId, TableRow,
};

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

/// The counts of read values, in name order.
#[derive(Default)]
struct Counts(BTreeMap<&'static str, usize>);

impl Counts {
    fn add(&mut self, name: &'static str, count: usize) {
        *self.0.entry(name).or_default() += count;
    }
}

/// Read heap values and count them.
struct Reader<'a> {
    pdb: &'a Pdb<pdbrs::io::FileReader>,
    counts: Counts,
}

impl Reader<'_> {
    fn string(&mut self, offset: StringOffset) -> Outcome {
        let bytes = self.pdb.string(offset)?;
        self.counts.add("string values", 1);
        self.counts.add("string bytes", bytes.len());
        Ok(())
    }

    fn blob(&mut self, offset: BlobOffset) -> Outcome {
        let bytes = self.pdb.blob(offset)?;
        self.counts.add("blob values", 1);
        self.counts.add("blob bytes", bytes.len());
        Ok(())
    }

    fn guid(&mut self, index: GuidIndex) -> Outcome {
        if self.pdb.guid(index)?.is_some() {
            self.counts.add("GUID values", 1);
        }
        Ok(())
    }

    /// Read all rows of one table and call `visit` for each row.
    fn table<T: TableRow>(&mut self, mut visit: impl FnMut(&mut Self, T) -> Outcome) -> Outcome {
        let pdb = self.pdb;
        for row in pdb.rows::<T>() {
            let (_, row) = row?;
            self.counts.add("rows", 1);
            visit(self, row)?;
        }
        Ok(())
    }

    fn all_tables(&mut self) -> Outcome {
        self.table(|reader, row: DocumentRow| {
            reader.blob(row.name)?;
            reader.guid(row.hash_algorithm)?;
            reader.blob(row.hash)?;
            reader.guid(row.language)
        })?;
        self.table(|reader, row: MethodDebugInformationRow| reader.blob(row.sequence_points))?;
        self.table(|_, _: LocalScopeRow| Ok(()))?;
        self.table(|reader, row: LocalVariableRow| reader.string(row.name))?;
        self.table(|reader, row: LocalConstantRow| {
            reader.string(row.name)?;
            reader.blob(row.signature)
        })?;
        self.table(|reader, row: ImportScopeRow| reader.blob(row.imports))?;
        self.table(|_, _: StateMachineMethodRow| Ok(()))?;
        self.table(|reader, row: CustomDebugInformationRow| {
            reader.guid(row.kind)?;
            reader.blob(row.value)
        })
    }

    /// Decode each custom debug information value and encode it again.
    fn custom_debug_information(&mut self) -> Outcome {
        let pdb = self.pdb;
        for row in pdb.custom_debug_informations() {
            let row = row?;
            let value = row.decode()?;
            self.counts.add("custom debug values decoded", 1);
            if matches!(value, CustomDebugValue::Unknown { .. }) {
                self.counts.add("custom debug values of unknown kinds", 1);
            }
            if value.encode()? == row.value() {
                self.counts
                    .add("custom debug values encoded to the same bytes", 1);
            } else {
                self.counts
                    .add("custom debug values encoded to other bytes", 1);
            }
        }
        Ok(())
    }
}

fn main() -> Outcome {
    let mut arguments = std::env::args().skip(1);
    let (Some(path), None) = (arguments.next(), arguments.next()) else {
        return Err("usage: cargo run -p pdbrs --example portable -- <file.pdb>".into());
    };
    let pdb = Pdb::open(&path)?;

    let metadata = pdb.metadata();
    println!(
        "metadata version {}",
        String::from_utf8_lossy(metadata.version_string())
    );
    for stream in metadata.streams() {
        println!(
            "stream {:<10} offset {:#8x} size {:#8x}",
            String::from_utf8_lossy(stream.name()),
            stream.offset(),
            stream.size()
        );
    }
    for heap in [
        MetadataStream::Strings,
        MetadataStream::UserStrings,
        MetadataStream::Blob,
        MetadataStream::Guid,
    ] {
        println!(
            "heap {:<8} size {:#x}",
            heap.name(),
            pdb.heap_size(heap).unwrap_or(0)
        );
    }
    let header = pdb.pdb_stream();
    let id = header.id().iter().fold(String::new(), |mut id, byte| {
        let _ = write!(id, "{byte:02x}");
        id
    });
    println!(
        "pdb id {id} entry point {:#010x}",
        header.entry_point_token().get()
    );
    for (table, rows) in header.external_tables() {
        println!("external table {table}: {rows} rows");
    }
    let tables = pdb.tables();
    for table in TableId::DEBUG_TABLES {
        println!("table {table}: {} rows", tables.row_count(table));
    }

    let mut reader = Reader {
        pdb: &pdb,
        counts: Counts::default(),
    };
    reader.all_tables()?;
    reader.custom_debug_information()?;
    for (name, count) in &reader.counts.0 {
        println!("{name}: {count}");
    }
    println!("read every row and heap value");
    Ok(())
}
