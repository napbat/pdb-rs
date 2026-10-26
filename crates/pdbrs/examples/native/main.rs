//! Read every stream and record of a native PDB and report undecoded data.
//!
//! The example decodes each stream role, checks the references between
//! streams and prints counts. It stops at the first malformed structure.
//! A recognized format that this crate does not decode, for example a record
//! kind without a decoder, is counted as undecoded and reading continues.
//! At the end, the example fails if anything is undecoded. It reports
//! producer defects that do not stop reading, for example an index record
//! outside the bucket of its name, without failing. It also reports decoded
//! producer data that no stream names, with the reason, without failing.
//! Names stay bytes.
//!
//! The option `--input file|slice|seek` selects the source: `Pdb::open` (the default),
//! `Pdb::parse` on the file bytes, or `Pdb::read` with `SeekReader` around the file. All three
//! sources give the same output.

mod dbi;
mod external;
#[path = "../support/input.rs"]
mod input;
mod lines;
mod module_types;
mod named;
mod symbol_encoding;
mod symbol_indexes;
mod symbol_validation;
mod symbols;
mod types;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::error::Error;
use std::fs::{self, File};

use pdbrs::io::{ReadAt, SeekReader};
use pdbrs::native::Pdb;
use pdbrs::native::address::{AddressMap, AddressTables};
use pdbrs::native::auxiliary::TypeMapStream;
use pdbrs::native::codeview::id_record::IdTable;
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::msf::{OldDirectory, StreamId};
use pdbrs::native::names::StringTable;
use pdbrs::native::types::{ForwardRefs, TypeTable, type_size};
use pdbrs::{ErrorKind, Result as PdbResult};

use dbi::{debug_info_tables, frame_data, reencode_dbi, section_header_bytes};
use external::external_dependencies;
use input::Input;
use named::{info_and_names, named_streams};
use symbols::{modules, shared_symbols};
use types::{hash_stream_bytes, identifiers, types};

type Outcome<T = ()> = Result<T, Box<dyn Error>>;

/// A string table without names: signature, version 1, a buffer with one NUL, one empty bucket
/// and a name count of 0.
const EMPTY_NAMES: [u8; 25] = [
    0xfe, 0xef, 0xfe, 0xef, 1, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// Counts and coverage gathered while reading.
#[derive(Default)]
struct Report {
    /// Every stream that a decoder read.
    claimed: BTreeSet<u32>,
    /// Counters in name order.
    counts: BTreeMap<&'static str, usize>,
    /// Data without a typed decoder, with the number of occurrences.
    undecoded: BTreeMap<String, usize>,
    /// Decoded data that breaks a producer rule but does not stop reading.
    inconsistencies: Vec<String>,
    /// Decoded streams without an owner, with the reason and the number of occurrences.
    unowned: BTreeMap<String, usize>,
    /// The bytes of the type map streams that `/TMCache` names.
    type_maps: HashSet<Vec<u8>>,
    /// Streams and records that an encoder wrote again: the count and the differences by family.
    reencoded: BTreeMap<&'static str, (usize, usize)>,
}

impl Report {
    fn claim(&mut self, id: StreamId) {
        self.claimed.insert(id.0);
    }

    fn add(&mut self, name: &'static str, count: usize) {
        *self.counts.entry(name).or_default() += count;
    }

    fn undecoded(&mut self, what: String) {
        *self.undecoded.entry(what).or_default() += 1;
    }

    fn inconsistency(&mut self, what: String) {
        self.inconsistencies.push(what);
    }

    /// Count one item that an encoder wrote again, and whether the bytes differ.
    fn reencoded(&mut self, family: &'static str, original: &[u8], encoded: &[u8]) {
        let entry = self.reencoded.entry(family).or_default();
        entry.0 += 1;
        if original != encoded {
            entry.1 += 1;
        }
    }

    /// Return the value, or count an unsupported format and return `None`.
    ///
    /// Errors other than [`ErrorKind::UnsupportedVersion`] stop the read.
    fn supported<T>(&mut self, result: PdbResult<T>, what: &str) -> Outcome<Option<T>> {
        match result {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == ErrorKind::UnsupportedVersion => {
                let reason = error.context().unwrap_or("unsupported format");
                self.undecoded(format!("{what}: {reason}"));
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }
}

/// The decoded streams that the module and symbol checks need.
struct Context<'a> {
    names: StringTable<'a>,
    tpi_end: TypeIndex,
    ids: IdTable<'a>,
    addresses: Option<AddressMap<'a>>,
    module_count: usize,
    /// The size of a TPI type, for the subfield def-range checks.
    type_size: &'a dyn Fn(TypeIndex) -> Option<u64>,
}

fn main() -> Outcome {
    let (input, path) = input::arguments(
        std::env::args_os().skip(1),
        "usage: cargo run --example native -- [--input file|slice|seek] <path>",
    )?;
    match input {
        Input::File => run(&Pdb::open(path)?),
        Input::Slice => {
            let bytes = fs::read(path)?;
            run(&Pdb::parse(&bytes)?)
        }
        Input::Seek => run(&Pdb::read(SeekReader::new(File::open(path)?)?)?),
    }
}

/// Read and check every stream of `pdb`, and print the report.
fn run(pdb: &Pdb<impl ReadAt>) -> Outcome {
    let mut report = Report::default();
    container(pdb, &mut report)?;

    let names_bytes = if let Some(bytes) = pdb.named_stream_bytes(b"/names")? {
        bytes
    } else {
        // `link /PDBSTRIPPED` writes no `/names` stream. Use a table without names, so that
        // any reference to a name fails.
        report.add("PDBs without /names (stripped)", 1);
        EMPTY_NAMES.to_vec()
    };
    let names = StringTable::parse(&names_bytes)?;
    info_and_names(pdb, &names, &mut report)?;
    if let Some(placement) = report.supported(names.verify_buckets(), "/names hashing")? {
        report.add(
            "/names strings",
            usize::try_from(placement.home)? + usize::try_from(placement.probed)?,
        );
    }
    let tpi = pdb.types()?.ok_or("missing TPI stream")?;
    let ipi = pdb.ids()?.ok_or("missing IPI stream")?;
    report.claim(StreamId(2));
    report.claim(StreamId(4));
    let tpi_hash_bytes = hash_stream_bytes(pdb, &tpi, &mut report)?;
    let ipi_hash_bytes = hash_stream_bytes(pdb, &ipi, &mut report)?;
    named_streams(pdb, &names, &tpi, &ipi, &mut report)?;
    types(&tpi, tpi_hash_bytes.as_deref(), &mut report)?;
    let ids = identifiers(&ipi, ipi_hash_bytes.as_deref(), &tpi, &names, &mut report)?;

    let dbi = pdb.debug_info()?.ok_or("missing DBI stream")?;
    report.claim(StreamId(3));
    reencode_dbi(pdb, &dbi, &mut report)?;
    let info_age = pdb.info().identity().age();
    if dbi.header().age != info_age {
        // `pdbstr -w` increments only the Info age; the image matches the DBI age.
        report.add("PDB Info age above the DBI age", 1);
        println!("PDB Info age: {info_age}, DBI age: {}", dbi.header().age);
    }
    section_header_bytes(pdb, &dbi, &mut report)?;
    // The tables hold the section headers and, if the DBI stream names them, the OMAP tables
    // and the original section headers.
    let tables = pdb.address_tables(&dbi)?;
    let addresses = tables.as_ref().map(AddressTables::map);
    let module_count = debug_info_tables(&dbi, addresses.as_ref(), &mut report)?;
    let type_table = TypeTable::new(&tpi)?;
    let forward = ForwardRefs::new(&type_table, None)?;
    let size_of_type = |index| type_size(&type_table, Some(&forward), index).ok().flatten();
    let context = Context {
        names,
        tpi_end: TypeIndex(tpi.header().index_end),
        ids,
        addresses,
        module_count,
        type_size: &size_of_type,
    };
    frame_data(pdb, &dbi, &context, &mut report)?;
    let procedures = modules(pdb, &dbi, &context, &mut report)?;
    lines::module_streams(pdb, &dbi, &mut report)?;
    lines::cross_scope(pdb, &dbi, &context, &mut report)?;
    shared_symbols(pdb, &dbi, &context, &procedures, &mut report)?;
    external_dependencies(pdb, &dbi, &mut report)?;
    unclaimed_streams(pdb, &mut report)?;
    print(&report);
    let undecoded: usize = report.undecoded.values().sum();
    let different: usize = report
        .reencoded
        .values()
        .map(|(_, different)| different)
        .sum();
    if undecoded != 0 {
        Err(format!("{undecoded} items have no typed decoder").into())
    } else if different != 0 {
        Err(format!("{different} items encode to other bytes").into())
    } else {
        Ok(())
    }
}

/// Check the container: free-page map, stream 0 and the Info stream.
fn container(pdb: &Pdb<impl ReadAt>, report: &mut Report) -> Outcome {
    let msf = pdb.msf();
    let map = msf.check_free_page_map()?;
    report.add("pages", usize::try_from(msf.page_count())?);
    report.add(
        "unreferenced allocated pages",
        usize::try_from(map.unreferenced_allocated_count())?,
    );
    if msf.stream(StreamId(0))?.is_some() {
        let bytes = pdb.stream_bytes(StreamId(0))?;
        if let Some(old) = OldDirectory::parse(&bytes, msf.block_size())? {
            report.add(
                "previous directory streams",
                usize::try_from(old.stream_count())?,
            );
        }
    }
    report.claim(StreamId(0));
    report.claim(StreamId(1));
    report.add("named streams", pdb.info().named_streams().count());
    Ok(())
}

/// Report present streams that no decoder read.
///
/// An unclaimed stream with the same bytes as a type map stream that `/TMCache` names is an
/// unowned type map: two inputs with the same type section checksum share one `/TMCache` entry,
/// and the entry keeps only the last stream (#13).
fn unclaimed_streams(pdb: &Pdb<impl ReadAt>, report: &mut Report) -> Outcome {
    for index in 0..pdb.msf().stream_count() {
        let id = StreamId(index);
        if pdb.msf().stream(id)?.is_none() || report.claimed.contains(&index) {
            continue;
        }
        let bytes = pdb.stream_bytes(id)?;
        if report.type_maps.contains(&bytes) && TypeMapStream::parse(&bytes, id).is_ok() {
            let reason = "type map stream that /TMCache does not name (same bytes as a named one)";
            *report.unowned.entry(reason.to_owned()).or_default() += 1;
        } else {
            report.undecoded("stream without a known role".to_owned());
        }
    }
    report.add("streams read", report.claimed.len());
    Ok(())
}

fn print(report: &Report) {
    for (name, count) in &report.counts {
        println!("{name}: {count}");
    }
    let undecoded: usize = report.undecoded.values().sum();
    println!("undecoded items: {undecoded}");
    for (item, count) in &report.undecoded {
        println!("  {count} x {item}");
    }
    println!("inconsistencies: {}", report.inconsistencies.len());
    for item in &report.inconsistencies {
        println!("  {item}");
    }
    let unowned: usize = report.unowned.values().sum();
    println!("decoded streams without an owner: {unowned}");
    for (item, count) in &report.unowned {
        println!("  {count} x {item}");
    }
    for (family, (count, different)) in &report.reencoded {
        println!("{family} re-encoded: {count}, different: {different}");
    }
}
