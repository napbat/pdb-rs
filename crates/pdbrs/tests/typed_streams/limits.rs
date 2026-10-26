//! The stream size limit of typed stream loads.

use pdbrs::native::dbi::{Dbi, DebugStream};
use pdbrs::native::msf::StreamId;
use pdbrs::native::{Pdb, ReadOptions};
use pdbrs::{ErrorKind, Result};

use super::{DBI, GLOBALS, MODULE, NAMES, OMAP, Order, SECTIONS, SYMBOLS, Streams, TPI, open};

/// Load the typed stream `id` with the accessor that decodes it.
fn load(pdb: &Pdb<&[u8]>, dbi: &Dbi, id: StreamId) -> Result<()> {
    match id {
        TPI => pdb.types().map(drop),
        DBI => pdb.debug_info().map(drop),
        NAMES => pdb.named_stream_bytes(b"/names").map(drop),
        MODULE => {
            let module = dbi.modules().nth(4).unwrap()?;
            pdb.module_stream(&module).map(drop)
        }
        SYMBOLS => pdb.stream_bytes(SYMBOLS).map(drop),
        GLOBALS => pdb.global_symbols(dbi).map(drop),
        SECTIONS => pdb
            .section_headers(dbi, DebugStream::SectionHeader)
            .map(drop),
        OMAP => pdb.omap(dbi, DebugStream::OmapToSource).map(drop),
        _ => unreachable!("no typed accessor for {id:?}"),
    }
}

#[test]
fn typed_loads_check_the_stream_size_against_the_limit() {
    let bytes = Streams::new().container(Order::Scattered);
    let full = open(&bytes);
    let dbi = full.debug_info().unwrap().unwrap();
    let info_size = full.stream_bytes(StreamId(1)).unwrap().len();
    for id in [TPI, DBI, NAMES, MODULE, SYMBOLS, GLOBALS, SECTIONS, OMAP] {
        let size = full.stream_bytes(id).unwrap().len();
        assert!(size > info_size, "the Info stream fits in each limit");

        let options = |max_stream_bytes| ReadOptions {
            max_stream_bytes,
            ..ReadOptions::default()
        };
        let exact = Pdb::with_options(bytes.as_slice(), options(size)).unwrap();
        load(&exact, &dbi, id).unwrap();

        let small = Pdb::with_options(bytes.as_slice(), options(size - 1)).unwrap();
        let error = load(&small, &dbi, id).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::LimitExceeded, "{id:?}");
        assert_eq!(error.stream_location(), Some((id, 0)), "{id:?}");
        assert_eq!(error.offset(), None);
    }
}
