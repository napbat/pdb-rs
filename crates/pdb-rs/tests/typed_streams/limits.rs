//! The stream size limit of typed stream loads.

use pdb_rs::native::codeview::index::ModuleIndex;
use pdb_rs::native::dbi::DebugStream;
use pdb_rs::native::msf::StreamId;
use pdb_rs::native::{Pdb, ReadOptions};
use pdb_rs::{ErrorKind, Result};

use super::{DBI, GLOBALS, MODULE, NAMES, OMAP, Order, SECTIONS, SYMBOLS, Streams, TPI, open};

/// Load the typed stream `id` with the accessor that decodes it.
fn load(pdb: &Pdb<&[u8]>, id: StreamId) -> Result<()> {
    match id {
        TPI => pdb.types().map(drop),
        DBI => pdb.debug_info().map(drop),
        NAMES => pdb.names().map(drop),
        MODULE => pdb.module(ModuleIndex(4)).map(drop),
        SYMBOLS => pdb.symbol_records().map(drop),
        GLOBALS => pdb.globals().map(drop),
        SECTIONS => pdb.section_headers(DebugStream::SectionHeader).map(drop),
        OMAP => pdb.omap(DebugStream::OmapToSource).map(drop),
        _ => unreachable!("no typed accessor for {id:?}"),
    }
}

/// Return the streams that the accessor of `id` loads, in load order.
///
/// The session loads the DBI stream before a stream that the DBI stream names, and
/// [`Pdb::globals`] loads the symbol record stream after the global symbol index.
fn load_order(id: StreamId) -> Vec<StreamId> {
    match id {
        MODULE | SYMBOLS | SECTIONS | OMAP => vec![DBI, id],
        GLOBALS => vec![DBI, GLOBALS, SYMBOLS],
        _ => vec![id],
    }
}

#[test]
fn typed_loads_check_the_stream_size_against_the_limit() {
    let bytes = Streams::new().container(Order::Scattered);
    let full = open(&bytes);
    let size = |id| full.stream_bytes(id).unwrap().len();
    let info_size = size(StreamId(1));
    for id in [TPI, DBI, NAMES, MODULE, SYMBOLS, GLOBALS, SECTIONS, OMAP] {
        assert!(size(id) > info_size, "the Info stream fits in each limit");
        let order = load_order(id);

        let options =
            |max_stream_bytes| ReadOptions::default().with_max_stream_bytes(max_stream_bytes);
        let largest = order.iter().map(|&id| size(id)).max().unwrap();
        let exact = Pdb::with_options(bytes.as_slice(), options(largest)).unwrap();
        load(&exact, id).unwrap();

        let limit = size(id) - 1;
        let small = Pdb::with_options(bytes.as_slice(), options(limit)).unwrap();
        let error = load(&small, id).unwrap_err();
        let first = order.into_iter().find(|&id| size(id) > limit).unwrap();
        assert_eq!(error.kind(), ErrorKind::LimitExceeded, "{id:?}");
        assert_eq!(error.stream_location(), Some((first, 0)), "{id:?}");
        assert_eq!(error.offset(), None);
    }
}
