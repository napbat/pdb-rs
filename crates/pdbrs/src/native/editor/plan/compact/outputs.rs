//! The walk over the output streams of a compaction.
//!
//! The walk reads the output Info and DBI streams of the assembly. They give
//! the role of each stream in both layouts. The walk rewrites each stream
//! that can hold a type or ID index, and rejects each stream that can hold
//! one without a decoder.

use alloc::vec::Vec;

use super::map::IndexMaps;
use super::{lines, streams, symbols};
use crate::io::ReadAt;
use crate::native::IndexSpace;
use crate::native::auxiliary::{NamedStreamKind, TmCache, TypeMapStream};
use crate::native::dbi::Dbi;
use crate::native::editor::plan::patch::{Base, Patched, patches_between};
use crate::native::editor::plan::{Assembly, Planned, missing};
use crate::native::editor::report::StreamChange;
use crate::native::info::{Feature, Info};
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::native::types::{HEADER_SIZE, parse_header_prefix};
use crate::{Error, ErrorKind, Result};

/// The stream IDs of the fixed streams.
const INFO: StreamId = StreamId(1);
/// The TPI stream.
const TPI: StreamId = StreamId(2);
/// The DBI stream.
const DBI: StreamId = StreamId(3);
/// The IPI stream.
const IPI: StreamId = StreamId(4);

/// Return the space of a refusal: the type space if it compacts, else the
/// ID space.
const fn space(maps: &IndexMaps) -> IndexSpace {
    if maps.types.is_some() {
        IndexSpace::TypeIndex
    } else {
        IndexSpace::IdIndex
    }
}

/// Give `error` the space of a refusal.
fn refused_space(maps: &IndexMaps, error: Error) -> Error {
    error.with_index_space(space(maps))
}

/// Return the refusal error with `context` for the space of `maps`.
fn refused(maps: &IndexMaps, context: &'static str) -> Error {
    refused_space(
        maps,
        Error::new(ErrorKind::UnsupportedEdit, None).with_context(context),
    )
}

/// Read all bytes of a planned stream.
fn read_all<R: ReadAt + ?Sized>(stream: &R) -> Result<Vec<u8>> {
    let length = usize::try_from(stream.len()?).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None).with_context("stream does not fit in memory")
    })?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| no_memory())?;
    bytes.resize(length, 0);
    stream.read_exact_at(0, &mut bytes)?;
    Ok(bytes)
}

/// Return the bytes of the output stream `id`, or `None` for a nil stream or
/// a stream outside the assembly.
fn stream_bytes<S: ReadAt>(assembly: &Assembly<'_, S>, id: StreamId) -> Result<Option<Vec<u8>>> {
    match assembly.streams.get(id.0 as usize) {
        Some(Some(planned)) => read_all(planned).map(Some),
        _ => Ok(None),
    }
}

/// Return the hash streams of the TPI or IPI stream `id` of the output.
fn hash_streams<S: ReadAt>(
    assembly: &Assembly<'_, S>,
    id: StreamId,
) -> Result<[Option<StreamId>; 2]> {
    let Some(Some(planned)) = assembly.streams.get(id.0 as usize) else {
        return Ok([None, None]);
    };
    let length = usize::try_from(planned.len()?).unwrap_or(usize::MAX);
    if length == 0 {
        return Ok([None, None]);
    }
    let mut head = [0; HEADER_SIZE];
    let head = &mut head[..length.min(HEADER_SIZE)];
    planned.read_exact_at(0, head)?;
    let header = parse_header_prefix(head, length).map_err(|error| error.with_stream(id, 0))?;
    Ok([header.hash_stream, header.hash_aux_stream])
}

/// Rewrite the output stream `id` with `rewrite`, which returns the new bytes
/// and `true` if a byte changed.
///
/// A copied stream that changes becomes a patched stream. The report entry
/// of an unchanged stream (`Copied`) becomes `Remapped` with the space of
/// the change.
fn rewrite_stream<S: ReadAt>(
    assembly: &mut Assembly<'_, S>,
    id: StreamId,
    maps: &IndexMaps,
    rewrite: impl FnOnce(Vec<u8>) -> Result<(Vec<u8>, bool)>,
) -> Result<()> {
    let rewrite = |bytes| {
        rewrite(bytes).map_err(|error| {
            if error.kind() == ErrorKind::UnsupportedEdit && error.index_space().is_none() {
                refused_space(maps, error)
            } else {
                error
            }
        })
    };
    let Some(slot) = assembly.streams.get_mut(id.0 as usize) else {
        return Ok(());
    };
    let Some(planned) = slot.take() else {
        return Ok(());
    };
    maps.take_changes();
    let (planned, changed) = match planned {
        Planned::Owned(bytes) => {
            let (bytes, changed) = rewrite(bytes)?;
            (Planned::Owned(bytes), changed)
        }
        Planned::Source(stream) => {
            let old = read_all(&stream)?;
            let (new, changed) = rewrite(old.clone())?;
            if changed {
                (patched(Base::Source(stream), &old, new)?, true)
            } else {
                (Planned::Source(stream), false)
            }
        }
        Planned::Borrowed(bytes) => {
            let (new, changed) = rewrite(bytes.to_vec())?;
            if changed {
                (patched(Base::Session(bytes), bytes, new)?, true)
            } else {
                (Planned::Borrowed(bytes), false)
            }
        }
        Planned::Patched(stream) => {
            let (new, changed) = rewrite(read_all(&stream)?)?;
            if changed {
                (Planned::Owned(new), true)
            } else {
                (Planned::Patched(stream), false)
            }
        }
    };
    *slot = Some(planned);
    let space = maps.changed_space();
    if let (true, Some(space), Some(report)) =
        (changed, space, assembly.reports.get_mut(id.0 as usize))
        && report.change == StreamChange::Copied
    {
        report.change = StreamChange::Remapped(space);
    }
    Ok(())
}

/// Return a patched stream over `base`, or the new bytes if the patches
/// would overlap.
fn patched<'e, S: ReadAt>(base: Base<'e, S>, old: &[u8], new: Vec<u8>) -> Result<Planned<'e, S>> {
    Ok(match patches_between(old, &new)? {
        Some(patches) => Planned::Patched(Patched::new(base, patches)?),
        None => Planned::Owned(new),
    })
}

/// Mark `id` as a stream with a known role.
fn claim(claimed: &mut [bool], id: Option<StreamId>) {
    if let Some(slot) = id.and_then(|id| claimed.get_mut(id.0 as usize)) {
        *slot = true;
    }
}

/// Rewrite a type map stream.
fn rewrite_type_map<S: ReadAt>(
    assembly: &mut Assembly<'_, S>,
    id: StreamId,
    maps: &IndexMaps,
) -> Result<()> {
    rewrite_stream(assembly, id, maps, |mut bytes| {
        let changed = streams::type_map(&mut bytes, id, maps)?;
        Ok((bytes, changed))
    })
}

/// Write the output indices into each output stream that can hold a type or
/// ID index. Reject the output if a stream can hold one without a decoder. Also
/// reject a `/DEBUG:FASTLINK` PDB, a PDB without type merging, a PDB without a DBI
/// stream and a PDB whose modules use type servers.
pub(in crate::native::editor::plan) fn rewrite<S: ReadAt>(
    assembly: &mut Assembly<'_, S>,
    maps: &IndexMaps,
) -> Result<()> {
    let info = Info::parse(stream_bytes(assembly, INFO)?.ok_or_else(|| missing("Info"))?)?;
    if info.has_feature(Feature::MINIMAL_DEBUG_INFO) {
        return Err(refused(
            maps,
            "the index space cannot compact in a /DEBUG:FASTLINK PDB (MINI): the object files keep the records with their indices",
        ));
    }
    if info.has_feature(Feature::NO_TYPE_MERGE) {
        return Err(refused(
            maps,
            "the index space cannot compact in a PDB without type merging (NOTM, /DEBUG:LAZY): the module symbols refer to the module type streams",
        ));
    }
    let dbi = Dbi::parse(stream_bytes(assembly, DBI)?.ok_or_else(|| {
        refused(
            maps,
            "the index space cannot compact in a PDB without a DBI stream: object files can use it as a type server",
        )
    })?)?;
    if !dbi.type_server_map()?.is_empty() {
        return Err(refused(
            maps,
            "the index space cannot compact in a PDB whose modules use type servers",
        ));
    }
    let count = assembly.streams.len();
    let mut claimed = Vec::new();
    claimed.try_reserve_exact(count).map_err(|_| no_memory())?;
    claimed.resize(count, false);
    for id in 0..5 {
        claim(&mut claimed, Some(StreamId(id)));
    }
    for id in [TPI, IPI] {
        for hash in hash_streams(assembly, id)? {
            claim(&mut claimed, hash);
        }
    }
    let header = dbi.header();
    claim(&mut claimed, header.global_stream);
    claim(&mut claimed, header.public_stream);
    claim(&mut claimed, header.symbol_stream);
    for id in dbi.debug_streams() {
        claim(&mut claimed, id);
    }
    if let Some(id) = header.symbol_stream {
        rewrite_stream(assembly, id, maps, |mut bytes| {
            let changed = symbols::symbol_records(&mut bytes, id, 0, maps)?;
            Ok((bytes, changed))
        })?;
    }
    for module in dbi.modules() {
        let (_, module) = module?;
        let Some(id) = module.stream else {
            continue;
        };
        claim(&mut claimed, Some(id));
        rewrite_stream(assembly, id, maps, |bytes| {
            lines::module_stream(bytes, &module, id, maps)
        })?;
    }
    named_streams(assembly, &info, maps, &mut claimed)?;
    for (index, claimed) in claimed.into_iter().enumerate() {
        let id = StreamId(u32::try_from(index).map_err(|_| no_memory())?);
        if claimed {
            continue;
        }
        let Some(bytes) = stream_bytes(assembly, id)? else {
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        if TypeMapStream::parse(&bytes, id).is_err() {
            return Err(refused(
                maps,
                "the index space compacts, and a stream that no other stream names can refer to it",
            )
            .with_stream(id, 0));
        }
        rewrite_type_map(assembly, id, maps)?;
    }
    Ok(())
}

/// Rewrite the named streams that hold indices: `/UDTSRCLINEUNDONE` and the
/// type maps that `/TMCache` names. Reject a named stream of an unknown kind.
fn named_streams<S: ReadAt>(
    assembly: &mut Assembly<'_, S>,
    info: &Info,
    maps: &IndexMaps,
    claimed: &mut [bool],
) -> Result<()> {
    for named in info.named_streams() {
        let id = named.stream;
        if claimed.get(id.0 as usize).copied().unwrap_or(true) {
            continue;
        }
        claim(claimed, Some(id));
        match NamedStreamKind::classify(&named.name) {
            NamedStreamKind::Other => {
                return Err(refused(
                    maps,
                    "the index space compacts, and a named stream of an unknown kind can refer to it",
                )
                .with_stream(id, 0));
            }
            NamedStreamKind::UdtSourceLineUndone => {
                rewrite_stream(assembly, id, maps, |mut bytes| {
                    let changed = streams::udt_source_line_undone(&mut bytes, id, maps)?;
                    Ok((bytes, changed))
                })?;
            }
            NamedStreamKind::TmCache => {
                let Some(bytes) = stream_bytes(assembly, id)? else {
                    continue;
                };
                let mut maps_of_cache = Vec::new();
                for map in TmCache::parse(&bytes, id)?.streams().flatten() {
                    if !maps_of_cache.contains(&map) {
                        maps_of_cache.try_reserve(1).map_err(|_| no_memory())?;
                        maps_of_cache.push(map);
                    }
                }
                for map in maps_of_cache {
                    if claimed.get(map.0 as usize).copied().unwrap_or(true) {
                        continue;
                    }
                    claim(claimed, Some(map));
                    rewrite_type_map(assembly, map, maps)?;
                }
            }
            NamedStreamKind::Names
            | NamedStreamKind::LinkInfo
            | NamedStreamKind::SourceHeaderBlock
            | NamedStreamKind::SourceFile { .. }
            | NamedStreamKind::SourceLink
            | NamedStreamKind::NumberedSourceLink { .. }
            | NamedStreamKind::SourceServer => {}
        }
    }
    Ok(())
}
