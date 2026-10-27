//! Native PDB readers: the whole PDB and each stream role.

pub mod dbi;
pub mod encode;
pub mod info;
pub mod named;
pub mod symbols;
pub mod types;

use pdbrs::io::ReadAt;
use pdbrs::native::codeview::index::{NameOffset, TypeIndex};
use pdbrs::native::container::ContainerLimits;
use pdbrs::native::dbi::Dbi;
use pdbrs::native::msf::StreamId;
use pdbrs::native::names::StringTable;
use pdbrs::native::{Pdb, ReadOptions};

use crate::{MAX_STREAM, ok};

/// Read a native PDB and decode every stream that the streams refer to.
pub fn pdb(data: &[u8]) {
    let _ = pdbrs::Pdb::parse(data);
    let options = ReadOptions::default()
        .with_max_stream_bytes(MAX_STREAM)
        .with_limits(ContainerLimits::default().with_msfz(crate::msfz::limits()));
    let Some(pdb) = ok(Pdb::with_options(data, options)) else {
        return;
    };
    let names = ok(pdb.names()).flatten();
    if let Some(names) = &names {
        info::walk_names(names);
    }
    for named in pdb.info().named_streams() {
        let Some(bytes) = ok(pdb.stream_bytes(named.stream)) else {
            continue;
        };
        named::named(
            named.name,
            &bytes,
            named.stream,
            names.as_ref(),
            |id| ok(pdb.stream_bytes(id)),
            |name| ok(pdb.named_stream_bytes(name)).flatten(),
        );
    }
    let tpi = ok(pdb.types()).flatten();
    let ipi = ok(pdb.ids()).flatten();
    let _ = ok(pdb.type_offsets());
    let _ = ok(pdb.id_offsets());
    let tpi_end = tpi
        .as_ref()
        .map_or(TypeIndex(0x1000), |tpi| TypeIndex(tpi.header().index_end));
    if let Some(tpi) = &tpi {
        let hash = hash_stream(&pdb, tpi.header().hash_stream);
        types::walk_types(tpi, hash.as_deref());
    }
    if let Some(ipi) = &ipi {
        let hash = hash_stream(&pdb, ipi.header().hash_stream);
        types::walk_ids(ipi, hash.as_deref(), tpi_end, names.as_ref());
    }
    let Some(dbi) = ok(pdb.debug_info()).flatten() else {
        return;
    };
    debug_info(&pdb, dbi, names.as_ref());
}

/// Load a hash stream, if the header names one.
fn hash_stream<S: ReadAt>(pdb: &Pdb<S>, id: Option<StreamId>) -> Option<Vec<u8>> {
    ok(pdb.stream_bytes(id?))
}

/// Decode the DBI substreams, the debug streams, the modules and the symbol indexes.
fn debug_info<S: ReadAt>(pdb: &Pdb<S>, dbi: &Dbi, names: Option<&StringTable<'_>>) {
    let headers = dbi
        .debug_stream(pdbrs::native::dbi::DebugStream::SectionHeader)
        .and_then(|id| ok(pdb.stream_bytes(id)));
    dbi::walk_dbi(dbi, headers.as_deref());
    if let Some(fpo) = ok(pdb.fpo()).flatten() {
        for record in &fpo {
            let _ = record.start;
        }
    }
    if let Some(frames) = ok(pdb.frame_data()).flatten() {
        for frame in &frames {
            if let Some(names) = names {
                let _ = names.get(NameOffset(frame.program));
            }
        }
    }
    for module in dbi.modules() {
        let Some(module) = ok(module) else {
            break;
        };
        if let Some(Some(stream)) = ok(pdb.module(module.index)) {
            dbi::walk_module(&stream);
        }
    }
    let Some(id) = dbi.header().symbol_stream else {
        return;
    };
    let Some(Some(records)) = ok(pdb.symbol_records()) else {
        return;
    };
    let offsets = symbols::walk_records(records, id);
    if let Some(Some(globals)) = ok(pdb.globals()) {
        symbols::walk_globals(globals.index(), records, offsets.as_ref());
    }
    if let Some(Some(publics)) = ok(pdb.publics()) {
        symbols::walk_publics(publics.index(), records, offsets.as_ref());
    }
}
