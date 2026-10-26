//! The named streams: `/LinkInfo`, `/TMCache` and its type maps, `/UDTSRCLINEUNDONE` and the
//! injected sources.

use std::collections::HashSet;

use pdbrs::io::FileReader;
use pdbrs::native::Pdb;
use pdbrs::native::auxiliary::{
    LinkInfo, NamedStreamKind, TmCache, TypeMapKind, TypeMapStream, UdtSourceLineUndone,
};
use pdbrs::native::injected_source::{SourceHeaderBlock, content_stream_name};
use pdbrs::native::msf::StreamId;
use pdbrs::native::names::StringTable;
use pdbrs::native::types::{TypeStream, TypeTable};

use crate::{Outcome, Report};

/// Decode every named stream and verify the injected sources.
pub(crate) fn named_streams(
    pdb: &Pdb<FileReader>,
    names: &StringTable<'_>,
    tpi: &TypeStream,
    ipi: &TypeStream,
    report: &mut Report,
) -> Outcome {
    let mut handled = HashSet::new();
    let info = pdb.info();
    for named in info.named_streams() {
        report.claim(named.stream);
        let bytes = pdb.stream_bytes(named.stream)?;
        match NamedStreamKind::classify(named.name) {
            NamedStreamKind::Names | NamedStreamKind::SourceFile { .. } => {}
            NamedStreamKind::LinkInfo => {
                report.supported(LinkInfo::parse(&bytes, named.stream), "/LinkInfo")?;
            }
            NamedStreamKind::TmCache => {
                let parsed = TmCache::parse(&bytes, named.stream);
                if let Some(cache) = report.supported(parsed, "/TMCache")? {
                    let ends = [tpi.header().index_end, ipi.header().index_end];
                    type_maps(pdb, &cache, ends, report)?;
                }
            }
            NamedStreamKind::UdtSourceLineUndone => {
                let list = UdtSourceLineUndone::parse(&bytes, named.stream)?;
                let table = TypeTable::new(tpi)?;
                for index in list.type_indices() {
                    table
                        .leaf(index)?
                        .tag()
                        .ok_or("/UDTSRCLINEUNDONE names a type that is not a UDT")?;
                }
                report.add("UDT source lines undone", list.len());
            }
            NamedStreamKind::SourceHeaderBlock => {
                let parsed = SourceHeaderBlock::parse(&bytes, named.stream);
                let Some(Some(block)) = report.supported(parsed, "/src/headerblock")? else {
                    continue;
                };
                for entry in block.entries() {
                    let source = entry.value;
                    names.get(source.file_name_offset)?;
                    names.get(source.object_name_offset)?;
                    let virtual_name = names.get(source.virtual_file_name_offset)?;
                    let stream_name = content_stream_name(virtual_name)?;
                    let id = info
                        .named_stream(&stream_name)
                        .ok_or("injected source has no content stream")?;
                    let content = pdb.stream_bytes(id)?;
                    report.supported(source.verify_content(&content, id), "injected source")?;
                    handled.insert(id);
                }
                report.add("injected sources", block.entries().len());
            }
            _ => report.undecoded(format!(
                "named stream {}",
                String::from_utf8_lossy(named.name)
            )),
        }
    }
    for named in info.named_streams() {
        let content = matches!(
            NamedStreamKind::classify(named.name),
            NamedStreamKind::SourceFile { .. }
        );
        if content && !handled.contains(&named.stream) {
            report.undecoded("injected source stream without a header entry".to_owned());
        }
    }
    Ok(())
}

/// Decode the type map streams that `/TMCache` names, with their precompiled-header maps.
///
/// `ends` holds the TPI and IPI index ends.
fn type_maps(
    pdb: &Pdb<FileReader>,
    cache: &TmCache<'_>,
    ends: [u32; 2],
    report: &mut Report,
) -> Outcome {
    let mut pending: Vec<StreamId> = cache.streams().flatten().collect();
    let mut seen = HashSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        report.claim(id);
        let bytes = pdb.stream_bytes(id)?;
        let map = TypeMapStream::parse(&bytes, id)?;
        if let TypeMapKind::Object {
            precompiled: Some(precompiled),
            ..
        } = map.kind()
        {
            pending.push(precompiled);
        }
        check_type_map(&map, ends)?;
        report.add("type map streams", 1);
        report.type_maps.insert(bytes);
    }
    report.add("/TMCache entries", cache.entries().len());
    Ok(())
}

/// Check that the map values and the function ID pairs of a type map are in range.
fn check_type_map(map: &TypeMapStream<'_>, [tpi_end, ipi_end]: [u32; 2]) -> Outcome {
    let within = |value: u32, end: u32| value == 0 || (0x1000..end).contains(&value);
    let count = |len: usize| u32::try_from(len).map_err(|_| "type map is too long");
    let (type_end, function_ids) = if map.kind() == TypeMapKind::TypeServer {
        if !map.ids().all(|value| within(value, ipi_end)) {
            return Err("type map names an ID past the IPI stream".into());
        }
        (tpi_end, 0x1000..0x1000 + count(map.ids().len())?)
    } else {
        let first = map.first_index();
        let end = first
            .checked_add(count(map.types().len())?)
            .ok_or("type map index range overflows")?;
        (tpi_end.max(ipi_end), first..end)
    };
    if !map.types().all(|value| within(value, type_end)) {
        return Err("type map names an index past the TPI and IPI streams".into());
    }
    for (function, ty) in map.function_types() {
        if !function_ids.contains(&function) || !(0x1000..tpi_end).contains(&ty) {
            return Err("type map function ID pair is out of range".into());
        }
    }
    Ok(())
}
