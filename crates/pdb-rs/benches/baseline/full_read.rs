//! The full read of a native PDB: the decode calls of the `native` example.
//!
//! The read decodes the Info stream, `/names`, the named streams, the TPI and IPI records, the
//! DBI substreams, the debug streams, the address tables, the module symbols, the C13
//! subsections, the symbol record stream and the global and public indexes. It does not do the
//! producer checks and the re-encode comparisons of the example.

use std::collections::HashSet;

use pdb_rs::io::ReadAt;
use pdb_rs::native::Pdb;
use pdb_rs::native::auxiliary::{
    LinkInfo, NamedStreamKind, SourceLink, SourceServer, TmCache, TypeMapKind, TypeMapStream,
    UdtSourceLineUndone,
};
use pdb_rs::native::codeview::Records;
use pdb_rs::native::codeview::id_record::IdRecord;
use pdb_rs::native::codeview::index::ModuleSymbolOffset;
use pdb_rs::native::codeview::symbol::{ModuleSymbols, Symbol};
use pdb_rs::native::codeview::type_record::TypeLeaf;
use pdb_rs::native::dbi::Dbi;
use pdb_rs::native::injected_source::{SourceHeaderBlock, content_stream_name};
use pdb_rs::native::lines::{LineSubsection, ModuleLines};
use pdb_rs::native::msf::StreamId;
use pdb_rs::native::types::{TypeHashes, TypeStream};
use pdb_rs::{ErrorKind, Result as PdbResult};

use crate::Outcome;

/// The deepest scope nesting that the read accepts in a module, as in the `native` example.
const MAX_SCOPE_DEPTH: usize = 256;

/// Return the value, or `None` for [`ErrorKind::UnsupportedVersion`].
///
/// The `native` example counts these items as undecoded and continues. Other errors stop the
/// read.
fn supported<T>(result: PdbResult<T>) -> Outcome<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == ErrorKind::UnsupportedVersion => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Decode `pdb` and return the number of decoded records and streams.
pub(crate) fn full_read(pdb: &Pdb<impl ReadAt>) -> Outcome<u64> {
    let mut count = 0;
    if let Some(names) = pdb.names()? {
        count += u64::try_from(names.entries().count())?;
    }
    count += named_streams(pdb)?;
    let tpi = pdb.types()?.ok_or("missing TPI stream")?;
    let ipi = pdb.ids()?.ok_or("missing IPI stream")?;
    count += type_records(pdb, &tpi, false)?;
    count += type_records(pdb, &ipi, true)?;
    let dbi = pdb.debug_info()?.ok_or("missing DBI stream")?;
    count += debug_info_tables(pdb, dbi)?;
    count += modules(pdb, dbi)?;
    count += shared_symbols(pdb)?;
    Ok(count)
}

/// Decode the named streams by their kind, and the type map streams that `/TMCache` names.
fn named_streams(pdb: &Pdb<impl ReadAt>) -> Outcome<u64> {
    let info = pdb.info();
    let mut count = 0;
    for named in info.named_streams() {
        let bytes = pdb.stream_bytes(named.stream)?;
        count += 1;
        match NamedStreamKind::classify(&named.name) {
            NamedStreamKind::LinkInfo => {
                supported(LinkInfo::parse(&bytes, named.stream))?;
            }
            NamedStreamKind::TmCache => {
                if let Some(cache) = supported(TmCache::parse(&bytes, named.stream))? {
                    count += type_maps(pdb, cache.streams().flatten().collect())?;
                }
            }
            NamedStreamKind::UdtSourceLineUndone => {
                UdtSourceLineUndone::parse(&bytes, named.stream)?;
            }
            NamedStreamKind::SourceLink => {
                SourceLink::parse(&bytes, named.stream)?;
            }
            NamedStreamKind::NumberedSourceLink { .. } => {
                SourceLink::parse_numbered(&bytes, named.stream)?;
            }
            NamedStreamKind::SourceServer => {
                SourceServer::parse(&bytes, named.stream)?;
            }
            NamedStreamKind::SourceHeaderBlock => {
                count += injected_sources(pdb, &bytes, named.stream)?;
            }
            _ => {}
        }
    }
    Ok(count)
}

/// Decode the type map streams in `pending` and their precompiled-header maps.
fn type_maps(pdb: &Pdb<impl ReadAt>, mut pending: Vec<StreamId>) -> Outcome<u64> {
    let mut seen = HashSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.0) {
            continue;
        }
        let bytes = pdb.stream_bytes(id)?;
        let map = TypeMapStream::parse(&bytes, id)?;
        if let TypeMapKind::Object {
            precompiled: Some(precompiled),
            ..
        } = map.kind()
        {
            pending.push(precompiled);
        }
    }
    Ok(u64::try_from(seen.len())?)
}

/// Decode `/src/headerblock` and read the content stream of each injected source.
fn injected_sources(pdb: &Pdb<impl ReadAt>, bytes: &[u8], id: StreamId) -> Outcome<u64> {
    let Some(Some(block)) = supported(SourceHeaderBlock::parse(bytes, id))? else {
        return Ok(0);
    };
    let Some(names) = pdb.names()? else {
        return Err("injected sources without /names".into());
    };
    let mut count = 0;
    for entry in block.entries() {
        let virtual_name = names.get(entry.value.virtual_file_name_offset)?;
        let stream_name = content_stream_name(virtual_name)?;
        if pdb.named_stream_bytes(&stream_name)?.is_some() {
            count += 1;
        }
    }
    Ok(count)
}

/// Decode the hash stream and every record of a TPI stream, or of an IPI stream if `ids`.
fn type_records(pdb: &Pdb<impl ReadAt>, stream: &TypeStream, ids: bool) -> Outcome<u64> {
    if let Some(id) = stream.header().hash_stream {
        let bytes = pdb.stream_bytes(id)?;
        supported(TypeHashes::parse(stream.header(), &bytes))?;
    }
    let mut count = 0;
    for record in stream.records() {
        let record = record?.record;
        if ids {
            std::hint::black_box(IdRecord::decode(record)?);
        } else {
            std::hint::black_box(TypeLeaf::decode(record)?);
        }
        count += 1;
    }
    Ok(count)
}

/// Decode the DBI substreams, the debug streams and the address tables.
fn debug_info_tables(pdb: &Pdb<impl ReadAt>, dbi: &Dbi) -> Outcome<u64> {
    let mut count = 0;
    for module in dbi.modules() {
        module?;
        count += 1;
    }
    if let Some(contributions) = dbi.section_contributions()? {
        count += u64::try_from((&contributions).into_iter().count())?;
    }
    if let Some(map) = dbi.section_map()? {
        count += u64::try_from(map.iter().count())?;
    }
    if let Some(sources) = dbi.source_info()? {
        for module in &sources.modules {
            for name in sources.file_names(module) {
                name?;
                count += 1;
            }
        }
    }
    if let Some(ec) = dbi.ec_names()? {
        count += u64::try_from(ec.entries().count())?;
    }
    count += u64::try_from(dbi.type_server_map()?.len())?;
    if let Some(fpo) = pdb.fpo()? {
        count += u64::try_from(fpo.len())?;
    }
    if let Some(frames) = pdb.frame_data()? {
        count += u64::try_from(frames.len())?;
    }
    if let Some(tables) = pdb.address_tables()? {
        count += u64::try_from(tables.map().sections().len())?;
    }
    Ok(count)
}

/// Decode the symbols and the C13 subsections of every module stream.
fn modules(pdb: &Pdb<impl ReadAt>, dbi: &Dbi) -> Outcome<u64> {
    let mut count = 0;
    for module in dbi.modules() {
        let (index, _) = module?;
        let Some(stream) = pdb.module(index)? else {
            continue;
        };
        let base = ModuleSymbolOffset(u32::try_from(stream.symbol_offset())?);
        let symbols =
            ModuleSymbols::new(stream.symbol_data(), stream.stream(), base, MAX_SCOPE_DEPTH);
        for scoped in symbols {
            let Some(scoped) = supported(scoped)? else {
                break;
            };
            std::hint::black_box(scoped.symbol);
            count += 1;
        }
        let Some(lines) = supported(ModuleLines::parse(&stream))? else {
            continue;
        };
        for subsection in lines.subsections() {
            count += 1;
            match &subsection.data {
                LineSubsection::Lines(data) => {
                    for block in data.blocks() {
                        count += u64::try_from(block.lines.len())?;
                    }
                }
                LineSubsection::FileChecksums(table) => {
                    count += u64::try_from(table.into_iter().count())?;
                }
                LineSubsection::InlineeLines(data) => {
                    count += u64::try_from(data.into_iter().count())?;
                }
                _ => {}
            }
        }
    }
    Ok(count)
}

/// Decode the symbol record stream and the records of the global and public indexes.
fn shared_symbols(pdb: &Pdb<impl ReadAt>) -> Outcome<u64> {
    let Some(bytes) = pdb.symbol_records()? else {
        return Ok(0);
    };
    let mut count = 0;
    for record in Records::new(bytes) {
        std::hint::black_box(Symbol::decode(record?)?);
        count += 1;
    }
    if let Some(globals) = supported(pdb.globals())?.flatten() {
        for symbol in globals.symbols() {
            std::hint::black_box(symbol?);
            count += 1;
        }
    }
    if let Some(publics) = supported(pdb.publics())?.flatten() {
        for symbol in publics.symbols() {
            std::hint::black_box(symbol?);
            count += 1;
        }
    }
    Ok(count)
}
