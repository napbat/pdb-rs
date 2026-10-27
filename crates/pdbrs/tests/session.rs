//! Tests of the reader session of `native::Pdb` (#42 stage 5): each cached structure loads
//! once, the cache limit, and `Pdb<S>: Sync` with `std`.
//!
//! The source PDB is the synthetic program of the `native_synthetic` example.

#[path = "../examples/native_synthetic/program.rs"]
mod program;

use core::sync::atomic::{AtomicUsize, Ordering};

use pdbrs::ErrorKind;
use pdbrs::io::ReadAt;
use pdbrs::native::codeview::index::ModuleIndex;
use pdbrs::native::types::{IndexOffset, TypeHashes};
use pdbrs::native::{EditOptions, Pdb, ReadOptions};

/// Build the synthetic program and return the PDB bytes.
fn synthetic() -> Vec<u8> {
    program::build()
        .and_then(|editor| editor.plan(EditOptions::default())?.to_vec())
        .expect("the synthetic program builds")
}

/// A source that counts the bytes that the reader reads.
struct Counting<'a> {
    /// The source bytes.
    bytes: &'a [u8],
    /// The number of bytes that `read_exact_at` gave.
    read: AtomicUsize,
}

impl<'a> Counting<'a> {
    /// Make a source of `bytes` without reads.
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            read: AtomicUsize::new(0),
        }
    }

    /// Return the number of bytes that the reader read so far.
    fn read(&self) -> usize {
        self.read.load(Ordering::Relaxed)
    }
}

impl ReadAt for Counting<'_> {
    fn len(&self) -> pdbrs::Result<u64> {
        ReadAt::len(self.bytes)
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> pdbrs::Result<()> {
        self.bytes.read_exact_at(offset, buffer)?;
        self.read.fetch_add(buffer.len(), Ordering::Relaxed);
        Ok(())
    }
}

/// Call each cached accessor of `pdb` and make sure that each structure is present.
fn load_cached<S: ReadAt>(pdb: &Pdb<S>) {
    assert!(pdb.debug_info().expect("DBI").is_some());
    assert!(pdb.names().expect("/names").is_some());
    assert!(pdb.symbol_records().expect("symbol records").is_some());
    assert!(pdb.globals().expect("GSI").is_some());
    assert!(pdb.publics().expect("PSI").is_some());
    assert!(pdb.type_offsets().expect("TPI header").is_some());
    assert!(pdb.id_offsets().expect("IPI header").is_some());
}

#[test]
fn the_session_loads_each_cached_structure_once() {
    let bytes = synthetic();
    let source = Counting::new(&bytes);
    let pdb = Pdb::read(&source).expect("PDB");
    let opened = source.read();
    load_cached(&pdb);
    let loaded = source.read();
    assert!(loaded > opened, "the first calls read the streams");
    for _ in 0..3 {
        load_cached(&pdb);
        pdb.address_tables().expect("address tables");
        pdb.fpo().expect("FPO");
    }
    let after_debug = source.read();
    load_cached(&pdb);
    assert_eq!(
        source.read(),
        after_debug,
        "a cached structure is not read again"
    );
    // A module stream is bulk data: each call loads it again.
    pdb.module(ModuleIndex(0)).expect("module").expect("stream");
    let first = source.read() - after_debug;
    assert!(first > 0);
    pdb.module(ModuleIndex(0)).expect("module").expect("stream");
    assert_eq!(source.read() - after_debug, 2 * first);
}

#[test]
fn cached_structures_equal_the_owned_loads() {
    let bytes = synthetic();
    let pdb = Pdb::parse(&bytes).expect("PDB");
    let dbi = pdb.debug_info().expect("DBI").expect("present");
    let header = dbi.header();
    let symbols = header.symbol_stream.expect("symbol record stream");
    assert_eq!(
        pdb.symbol_records().expect("records"),
        Some(pdb.stream_bytes(symbols).expect("bytes").as_slice())
    );
    let globals = pdb.globals().expect("GSI").expect("present");
    assert_eq!(globals.records(), pdb.symbol_records().unwrap().unwrap());
    assert_eq!(
        globals.symbols().count(),
        globals.index().hash_table().len()
    );
    for symbol in globals.symbols() {
        symbol.expect("an indexed global symbol decodes");
    }
    let publics = pdb.publics().expect("PSI").expect("present");
    let names: Vec<_> = publics
        .symbols()
        .map(|symbol| symbol.expect("public").name().map(|name| name.to_vec()))
        .collect();
    assert!(names.contains(&Some(b"main".to_vec())));
    let names = pdb.names().expect("/names").expect("present");
    let owned = pdb.named_stream_bytes(b"/names").unwrap().unwrap();
    let mut encoded = Vec::new();
    names.encode(&mut encoded).expect("encode");
    assert_eq!(encoded, owned);
    for (offsets, types) in [
        (pdb.type_offsets(), pdb.types()),
        (pdb.id_offsets(), pdb.ids()),
    ] {
        let offsets = offsets.expect("header").expect("present");
        let types = types.expect("stream").expect("present");
        assert_eq!(offsets.header(), types.header());
        let expected: Vec<IndexOffset> = match types.header().hash_stream {
            Some(id) => {
                let hash = pdb.stream_bytes(id).expect("hash stream");
                TypeHashes::parse(types.header(), &hash)
                    .expect("hashes")
                    .index_offsets()
                    .collect()
            }
            None => Vec::new(),
        };
        assert_eq!(offsets.index_offsets(), expected.as_slice());
    }
    let module_count = dbi.modules().count();
    let last = ModuleIndex(u32::try_from(module_count - 1).unwrap());
    let (index, module) = dbi.modules().last().unwrap().unwrap();
    assert_eq!(index, last);
    let stream = pdb.module(last).expect("last module");
    assert_eq!(stream.map(|stream| stream.stream()), module.stream);
    let outside = ModuleIndex(u32::try_from(module_count).unwrap());
    let error = pdb.module(outside).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
}

#[test]
fn the_cache_limit_rejects_a_structure_above_the_limit() {
    assert_eq!(ReadOptions::default().max_cached_bytes(), 256 * 1024 * 1024);
    let bytes = synthetic();
    let dbi_size = Pdb::parse(&bytes)
        .expect("PDB")
        .stream_bytes(pdbrs::native::msf::StreamId(3))
        .expect("DBI")
        .len();
    let open = |limit| {
        let options = ReadOptions::default().with_max_cached_bytes(limit);
        assert_eq!(options.max_cached_bytes(), limit);
        Pdb::with_options(bytes.as_slice(), options).expect("PDB")
    };

    let small = open(dbi_size - 1);
    for _ in 0..2 {
        let error = small.debug_info().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    }
    // The accessors that need the DBI stream fail too.
    assert_eq!(
        small.globals().unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );

    let exact = open(dbi_size);
    assert!(exact.debug_info().expect("DBI fits").is_some());
    // A slice source lends the symbol record stream, so its cell holds no
    // stream bytes and fits (#48).
    assert!(exact.symbol_records().expect("lent records").is_some());
    // The stored DBI stream stays available.
    assert!(exact.debug_info().expect("DBI stays").is_some());
    assert_eq!(
        exact.type_offsets().unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );

    // A source that cannot lend bytes gives a copy, which counts.
    let source = Counting::new(&bytes);
    let options = ReadOptions::default().with_max_cached_bytes(dbi_size);
    let copied = Pdb::with_options(&source, options).expect("PDB");
    assert!(copied.debug_info().expect("DBI fits").is_some());
    assert_eq!(
        copied.symbol_records().unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    assert!(copied.debug_info().expect("DBI stays").is_some());
}

#[cfg(feature = "std")]
#[test]
fn a_pdb_is_sync_if_its_source_is_sync() {
    const fn assert_sync<T: Sync>() {}
    assert_sync::<Pdb<&[u8]>>();
    assert_sync::<Pdb<pdbrs::io::FileReader>>();
    assert_sync::<Pdb<Counting<'_>>>();

    let bytes = synthetic();
    let source = Counting::new(&bytes);
    let pdb = Pdb::read(&source).expect("PDB");
    let records: Vec<usize> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    load_cached(&pdb);
                    pdb.symbol_records().unwrap().unwrap().len()
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().expect("worker"))
            .collect()
    });
    assert!(records.windows(2).all(|pair| pair[0] == pair[1]));
    let read = source.read();
    load_cached(&pdb);
    assert_eq!(source.read(), read);
}
