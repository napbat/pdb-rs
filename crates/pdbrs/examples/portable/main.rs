//! Read every table row and heap value of a Portable PDB and print counts.
//!
//! The input is a standalone Portable PDB or an embedded PDB payload (`MPDB`, for example the
//! `.mpdb` file that the fixture generator writes). The example decodes a payload with
//! `pdbrs::portable::embedded::decode_embedded` and then reads the decoded PDB.
//!
//! The example opens the PDB with `pdbrs::portable::Pdb`. It prints the metadata version, the
//! stream headers, the heap sizes, the `#Pdb` fields and the row count of each debug table.
//! Then it reads each row of the eight debug tables and each string, blob and GUID that a row
//! refers to. It stops with an error at the first value that it cannot read.
//!
//! Then it decodes each custom debug information value, encodes it again and compares the
//! bytes. A value of an unknown kind is counted, not an error.
//!
//! Then it decodes the debug table blobs: document names, sequence points, local constant
//! signatures and imports. It encodes each blob again and compares the bytes. It prints each
//! blob that encodes to other bytes, with the reason. Last, it validates the debug tables with
//! `Pdb::validate_debug` and prints the report. It stops with an error at the first value that
//! does not decode and at the first rule violation.
//!
//! The example also decodes the embedded source of each document and checks it against the
//! document hash (SHA-256 only).
//!
//! The option `--input file|slice|seek` selects the source: `Pdb::open` (the default),
//! `Pdb::parse` on the file bytes, or `Pdb::read` with `SeekReader` around the file. For an
//! `MPDB` payload, the example writes the decoded PDB to a temporary file and reads that file
//! in the selected mode. All three sources give the same output.

mod debug;
#[path = "../support/input.rs"]
mod input;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::Read as _;
use std::path::{Path, PathBuf};

use pdbrs::io::{ReadAt, SeekReader};
use pdbrs::portable::Pdb;
use pdbrs::portable::custom::CustomDebugValue;
use pdbrs::portable::embedded::{EMBEDDED_PDB_SIGNATURE, decode_embedded};
use pdbrs::portable::metadata::{
    BlobOffset, CustomDebugInformationRow, DocumentRow, GuidIndex, ImportScopeRow,
    LocalConstantRow, LocalScopeRow, LocalVariableRow, MetadataStream, MethodDebugInformationRow,
    StateMachineMethodRow, StringOffset, TableId, TableRow,
};

use input::Input;

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

/// The largest decoded embedded PDB that the example reads.
const MAX_EMBEDDED_PDB: usize = 256 << 20;

/// The counts of read values, in name order.
#[derive(Default)]
struct Counts(BTreeMap<&'static str, usize>);

impl Counts {
    fn add(&mut self, name: &'static str, count: usize) {
        *self.0.entry(name).or_default() += count;
    }
}

/// Read heap values and count them.
struct Reader<'a, S: ReadAt> {
    pdb: &'a Pdb<S>,
    counts: Counts,
}

impl<S: ReadAt> Reader<'_, S> {
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
            let mut encoded = Vec::new();
            value.encode(&mut encoded)?;
            if encoded == row.value() {
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

/// A temporary file that holds a decoded embedded PDB. Dropping the value deletes the file.
struct Decoded(PathBuf);

impl Drop for Decoded {
    fn drop(&mut self) {
        // The file is in the temporary directory, so a failed delete is not an error.
        let _ = fs::remove_file(&self.0);
    }
}

fn main() -> Outcome {
    let (input, path) = input::arguments(
        std::env::args_os().skip(1),
        "usage: cargo run -p pdbrs --example portable -- [--input file|slice|seek] \
         <file.pdb | file.mpdb>",
    )?;
    let mut signature = Vec::new();
    File::open(&path)?
        .take(EMBEDDED_PDB_SIGNATURE.len() as u64)
        .read_to_end(&mut signature)?;
    let decoded = if signature == EMBEDDED_PDB_SIGNATURE {
        let payload = fs::read(&path)?;
        let bytes = decode_embedded(&payload, MAX_EMBEDDED_PDB)?;
        println!(
            "embedded PDB payload: {} bytes, decoded to {} bytes",
            payload.len(),
            bytes.len()
        );
        let file =
            std::env::temp_dir().join(format!("pdbrs-portable-example-{}.pdb", std::process::id()));
        fs::write(&file, bytes)?;
        Some(Decoded(file))
    } else {
        None
    };
    let path = decoded.as_ref().map_or(Path::new(&path), |file| &file.0);
    match input {
        Input::File => run(&Pdb::open(path)?),
        Input::Slice => {
            let bytes = fs::read(path)?;
            run(&Pdb::parse(&bytes)?)
        }
        Input::Seek => run(&Pdb::read(SeekReader::new(File::open(path)?)?)?),
    }
}

/// Print the headers of `pdb`, then read and check every row and heap value.
fn run(pdb: &Pdb<impl ReadAt>) -> Outcome {
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
        header.entry_point_token().raw()
    );
    for (table, rows) in header.external_tables() {
        println!("external table {table}: {rows} rows");
    }
    let tables = pdb.tables();
    for table in TableId::DEBUG_TABLES {
        println!("table {table}: {} rows", tables.row_count(table));
    }

    let mut reader = Reader {
        pdb,
        counts: Counts::default(),
    };
    reader.all_tables()?;
    reader.custom_debug_information()?;
    debug::debug_tables(pdb, &mut reader.counts)?;
    for (name, count) in &reader.counts.0 {
        println!("{name}: {count}");
    }
    println!("read every row and heap value");
    Ok(())
}
