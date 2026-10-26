//! The named streams: `/LinkInfo`, `/TMCache` and its type maps, `/UDTSRCLINEUNDONE`,
//! `sourcelink` and the injected sources.

use std::collections::HashSet;

use pdbrs::io::ReadAt;
use pdbrs::native::auxiliary::{
    LinkInfo, NamedStreamKind, SourceLink, SourceServer, TmCache, TypeMapContext, TypeMapKind,
    TypeMapStream, UdtSourceLineUndone,
};
use pdbrs::native::codeview::id_record::IdTable;
use pdbrs::native::codeview::index::{IdIndex, TypeIndex};
use pdbrs::native::hash_table::HashTableProducer;
use pdbrs::native::info::Feature;
use pdbrs::native::info::InfoBuilder;
use pdbrs::native::injected_source::{
    ChecksumCheck, InjectedSource, InjectedSourceKind, SourceCompression, SourceHeaderBlock,
    content_stream_name,
};
use pdbrs::native::msf::StreamId;
use pdbrs::native::names::{StringTable, StringTableBuilder};
use pdbrs::native::types::{TypeStream, TypeTable};
use pdbrs::native::{Identity, Pdb};
use pdbrs::portable::embedded::EmbeddedFormat;

use crate::{Outcome, Report};

/// The largest embedded source that the example decodes.
const MAX_EMBEDDED_SOURCE: usize = 64 << 20;

/// Encode the Info stream and the `/names` table again, and build both again from their names.
///
/// The decoded forms must give the same bytes. The rebuilds insert the names in string buffer
/// order with the rules of Microsoft `NMTNI` and `NMT`; the report counts the rebuilds that give
/// other bytes, for example the `/names` bucket layout of LLVM, without failing.
pub(crate) fn info_and_names(
    pdb: &Pdb<impl ReadAt>,
    names: &StringTable<'_>,
    report: &mut Report,
) -> Outcome {
    let info = pdb.info();
    let mut encoded = Vec::new();
    info.encode(&mut encoded)?;
    report.reencoded("PDB Info streams", info.as_bytes(), &encoded);

    let Identity::Modern { guid, age } = info.identity() else {
        return Err("PDB Info stream has no GUID identity".into());
    };
    let mut model = InfoBuilder::new(HashTableProducer::Microsoft, info.signature(), guid, age)?;
    let mut streams: Vec<_> = info.named_streams().collect();
    streams.sort_by_key(|named| named.name_offset);
    for named in &streams {
        model.set_named_stream(named.name, named.stream)?;
    }
    model.set_next_name_index(info.next_name_index());
    let features: Vec<Feature> = info.typed_features().collect();
    model.set_features(&features)?;
    let mut rebuilt = Vec::new();
    model.encode(&mut rebuilt)?;
    let key = if rebuilt == info.as_bytes() {
        "PDB Info streams rebuilt from names with the NMTNI rules: same bytes"
    } else {
        "PDB Info streams rebuilt from names with the NMTNI rules: other bytes"
    };
    report.add(key, 1);

    let Some(original) = pdb.named_stream_bytes(b"/names")? else {
        return Ok(());
    };
    let mut encoded = Vec::new();
    names.encode(&mut encoded)?;
    report.reencoded("/names string tables", &original, &encoded);
    let mut offsets: Vec<u32> = names.entries().map(|entry| entry.offset).collect();
    offsets.sort_unstable();
    let mut builder = StringTableBuilder::new(names.hash_version())?;
    for offset in offsets {
        builder.insert(names.get(offset)?)?;
    }
    let mut rebuilt = Vec::new();
    builder.encode(&mut rebuilt)?;
    let key = if rebuilt == original {
        "/names tables rebuilt from names with the NMT rules: same bytes"
    } else {
        "/names tables rebuilt from names with the NMT rules: other bytes"
    };
    report.add(key, 1);
    Ok(())
}

/// Decode every named stream and verify the injected sources.
pub(crate) fn named_streams(
    pdb: &Pdb<impl ReadAt>,
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
                let parsed = LinkInfo::parse(&bytes, named.stream);
                if let Some(link) = report.supported(parsed, "/LinkInfo")? {
                    let mut encoded = Vec::new();
                    if let Some(link) = link {
                        link.encode(&mut encoded)?;
                    }
                    report.reencoded("/LinkInfo streams", &bytes, &encoded);
                }
            }
            NamedStreamKind::TmCache => {
                let parsed = TmCache::parse(&bytes, named.stream);
                if let Some(cache) = report.supported(parsed, "/TMCache")? {
                    let mut encoded = Vec::new();
                    cache.encode(&mut encoded)?;
                    report.reencoded("/TMCache streams", &bytes, &encoded);
                    let ids = IdTable::new(ipi)?;
                    let context = TypeMapContext::new(
                        TypeIndex(tpi.header().index_end),
                        IdIndex(ipi.header().index_end),
                    )
                    .with_ids(&ids);
                    type_maps(pdb, &cache, &context, report)?;
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
                let mut encoded = Vec::new();
                list.encode(&mut encoded)?;
                report.reencoded("/UDTSRCLINEUNDONE streams", &bytes, &encoded);
            }
            NamedStreamKind::SourceLink => {
                let link = SourceLink::parse(&bytes, named.stream)?;
                report.add("Source Link streams", 1);
                let mut encoded = Vec::new();
                link.encode(&mut encoded)?;
                report.reencoded("sourcelink streams", &bytes, &encoded);
            }
            NamedStreamKind::NumberedSourceLink { .. } => {
                let link = SourceLink::parse_numbered(&bytes, named.stream)?;
                report.add("numbered Source Link streams", 1);
                let mut encoded = Vec::new();
                link.encode(&mut encoded)?;
                report.reencoded("sourcelink$<n> streams", &bytes, &encoded);
            }
            NamedStreamKind::SourceServer => {
                let server = SourceServer::parse(&bytes, named.stream)?;
                report.add("source server files", server.source_files().count());
                let mut encoded = Vec::new();
                server.encode(&mut encoded)?;
                report.reencoded("srcsrv streams", &bytes, &encoded);
            }
            NamedStreamKind::SourceHeaderBlock => {
                let parsed = SourceHeaderBlock::parse(&bytes, named.stream);
                let Some(block) = report.supported(parsed, "/src/headerblock")? else {
                    continue;
                };
                let mut encoded = Vec::new();
                if let Some(block) = &block {
                    block.encode(&mut encoded)?;
                }
                report.reencoded("/src/headerblock streams", &bytes, &encoded);
                let Some(block) = block else {
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
                    if source.kind(virtual_name) == InjectedSourceKind::Natvis {
                        source.verify_natvis(&content, id)?;
                        report.add("injected Natvis files", 1);
                    }
                    injected_source(&source, &content, id, report)?;
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

/// Verify one injected source stream: file contents or a .NET document record.
fn injected_source(
    source: &InjectedSource,
    content: &[u8],
    id: StreamId,
    report: &mut Report,
) -> Outcome {
    if source.compression != SourceCompression::DotNet {
        report.supported(source.verify_content(content, id), "injected source")?;
        return Ok(());
    }
    let document = source.verify_document(content, id)?;
    report.add(".NET document records", 1);
    let mut encoded = Vec::new();
    document.encode(&mut encoded)?;
    report.reencoded(".NET document records", content, &encoded);
    let Some(embedded) = document.embedded else {
        report.add(".NET document records without embedded source", 1);
        return Ok(());
    };
    let decoded = embedded
        .decode(MAX_EMBEDDED_SOURCE)
        .map_err(|error| error.with_stream(id, 0))?;
    report.add(
        match embedded.format() {
            EmbeddedFormat::Raw => ".NET embedded sources (raw)",
            EmbeddedFormat::Deflate => ".NET embedded sources (DEFLATE)",
        },
        1,
    );
    let check = document
        .verify_checksum(&decoded)
        .map_err(|error| error.with_stream(id, 0))?;
    report.add(
        match check {
            ChecksumCheck::Verified => ".NET document checksums verified (SHA-256)",
            ChecksumCheck::NotVerified => ".NET document checksums not verified",
        },
        1,
    );
    Ok(())
}

/// Decode the type map streams that `/TMCache` names, with their precompiled-header maps, and
/// report the findings of [`TypeMapStream::validate`] as inconsistencies.
fn type_maps(
    pdb: &Pdb<impl ReadAt>,
    cache: &TmCache<'_>,
    context: &TypeMapContext<'_>,
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
        let mut encoded = Vec::new();
        map.encode(&mut encoded)?;
        report.reencoded("type map streams", &bytes, &encoded);
        if let TypeMapKind::Object {
            precompiled: Some(precompiled),
            ..
        } = map.kind()
        {
            pending.push(precompiled);
        }
        let violations = map.validate(context)?;
        for violation in &violations {
            report.inconsistency(format!(
                "type map stream {}: {} at position {}, value {:#x}",
                id.0,
                violation.rule.description(),
                violation.position,
                violation.value
            ));
        }
        report.add(
            "type map function ID pairs validated",
            map.function_types().len(),
        );
        report.add("type map streams", 1);
        report.type_maps.insert(bytes);
    }
    report.add("/TMCache entries", cache.entries().len());
    Ok(())
}
