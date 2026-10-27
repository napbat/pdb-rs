//! Split a native PDB into the inputs of the native targets.

use std::collections::BTreeMap;

use pdbrs::ContainerKind;
use pdbrs::native::Pdb;
use pdbrs::native::auxiliary::NamedStreamKind;
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::index::ModuleSymbolOffset;
use pdbrs::native::codeview::symbol::{ModuleSymbols, SymbolData};
use pdbrs::native::dbi::{Dbi, DebugStream};
use pdbrs::native::injected_source::{SourceCompression, SourceHeaderBlock, content_stream_name};
use pdbrs::native::msf::StreamId;
use pdbrs::native::names::StringTable;
use pdbrs::native::types::TypeStream;

use crate::{Corpus, Outcome};

/// The most records of one kind that one PDB gives to the record targets.
const MAX_PER_KIND: usize = 3;

/// The edit bytes of the `native_editor` seeds (see `pdbrs_fuzz::native::editor`).
///
/// The sequences are: no edit; a strip with a new named stream; a new named stream and the
/// removal of a named stream; edits of symbols, types and lines. Each sequence also gives a
/// seed that starts with `decode_all`.
const EDITOR_EDITS: [&[u8]; 4] = [
    &[],
    &[14, 0, 0, 4, 0, 0, 0, b'd', b'a', b't', b'a'],
    &[0, 0, 4, 0, 0, 0, b'd', b'a', b't', b'a', 2, 0, 0],
    // Rename a public symbol, push a global `S_UDT`, push an `LF_ARGLIST`, change the type of
    // a module symbol and change a line number.
    &[
        3, 0, 0, 4, b'r', b'e', b'n', b'1', 8, 0, 0x74, 0, 0, 0, 3, b'u', b'd', b't', 1, 9, 2,
        0xe8, 1, 0, 0, 10, 0, 0, 1, 0, 1, 0x74, 0, 0, 0, 12, 0, 0, 0, 0, 5, 0, 0, 0,
    ],
];

/// The edit byte of `decode_all` in `pdbrs_fuzz::native::editor`.
const DECODE_ALL: u8 = 13;

/// The record kinds seen in the current PDB, with their counts.
#[derive(Default)]
struct Kinds(BTreeMap<(u8, u32), usize>);

impl Kinds {
    /// Return `true` for the first records of a kind in a family.
    fn take(&mut self, family: u8, kind: u32) -> bool {
        let count = self.0.entry((family, kind)).or_default();
        *count += 1;
        *count <= MAX_PER_KIND
    }
}

/// Add the seeds of the native PDB in `bytes`, in an MSF 7.0 or an MSFZ container.
pub fn seeds(bytes: &[u8], corpus: &mut Corpus) -> Outcome {
    if bytes.starts_with(ContainerKind::NativeMsfz.signature()) {
        corpus.add("msfz", bytes)?;
        crate::synthetic::msfz_chunk_seeds(bytes, corpus)?;
    } else {
        corpus.add("msf", bytes)?;
        for selector in [0x03_u8, 0x08, 0x10] {
            corpus.add_parts("msf_repack", &[selector], &[], bytes)?;
        }
    }
    // The selectors cover each MSFZ option (see `pdbrs_fuzz::convert::options`).
    for selector in [0x00_u8, 0x1d, 0xe6] {
        corpus.add_parts("msfz_convert", &[selector], &[], bytes)?;
    }
    corpus.add("native_pdb", bytes)?;
    for edits in EDITOR_EDITS {
        corpus.add_parts("native_editor", &[], &[bytes], edits)?;
        corpus.add_parts(
            "native_editor",
            &[],
            &[bytes],
            &[&[DECODE_ALL], edits].concat(),
        )?;
    }
    let pdb = Pdb::parse(bytes)?;
    let mut kinds = Kinds::default();
    let info = pdb.stream_bytes(StreamId(1))?;
    corpus.add("native_info", &info)?;
    if let Some(table) = info_hash_table(&info) {
        corpus.add("native_hash_table", table)?;
    }
    let names_bytes = pdb.named_stream_bytes(b"/names")?.unwrap_or_default();
    corpus.add("native_names", &names_bytes)?;
    if let Some(msf) = pdb.container().msf()
        && msf.stream(StreamId(0))?.is_some()
    {
        let old = pdb.stream_bytes(StreamId(0))?;
        let selector = u8::try_from(msf.block_size().trailing_zeros().saturating_sub(9))?;
        corpus.add_parts("native_named", &[7, selector], &[], &old)?;
    }
    let tpi = pdb.types()?;
    if let Some(tpi) = &tpi {
        let hash = hash_bytes(&pdb, tpi)?;
        corpus.add_parts(
            "native_types",
            &[],
            &[&hash],
            &stream_of(&pdb, StreamId(2))?,
        )?;
        leaves(tpi.record_bytes(), 0, &mut kinds, corpus)?;
    }
    if let Some(ipi) = pdb.ids()? {
        let hash = hash_bytes(&pdb, &ipi)?;
        let end = tpi.as_ref().map_or(0x1000, |tpi| tpi.header().index_end);
        let ipi_bytes = stream_of(&pdb, StreamId(4))?;
        corpus.add_parts(
            "native_ids",
            &[],
            &[&hash, &names_bytes],
            &[&end.to_le_bytes()[..], &ipi_bytes].concat(),
        )?;
        leaves(ipi.record_bytes(), 1, &mut kinds, corpus)?;
    }
    named_streams(&pdb, &names_bytes, corpus)?;
    if let Some(dbi) = pdb.debug_info()? {
        debug_info(&pdb, dbi, &mut kinds, corpus)?;
    }
    Ok(())
}

/// Return the bytes of a stream.
fn stream_of(pdb: &Pdb<&[u8]>, id: StreamId) -> Outcome<Vec<u8>> {
    Ok(pdb.stream_bytes(id)?)
}

/// Return the hash stream of a type stream, or no bytes.
fn hash_bytes(pdb: &Pdb<&[u8]>, types: &TypeStream) -> Outcome<Vec<u8>> {
    Ok(match types.header().hash_stream {
        Some(id) => pdb.stream_bytes(id)?,
        None => Vec::new(),
    })
}

/// Return the named stream map hash table of an Info stream.
fn info_hash_table(info: &[u8]) -> Option<&[u8]> {
    let size = u32::from_le_bytes(info.get(28..32)?.try_into().ok()?);
    info.get(32 + usize::try_from(size).ok()?..)
}

/// Add the first records of each kind as record payload seeds.
///
/// `family` is 0 for type records, 1 for ID records and 2 for symbol records.
fn leaves(records: &[u8], family: u8, kinds: &mut Kinds, corpus: &mut Corpus) -> Outcome {
    for record in Records::new(records) {
        let record = record?;
        if kinds.take(family, u32::from(record.kind)) {
            corpus.add_parts("native_leaf", &record.kind.to_le_bytes(), &[], record.data)?;
        }
    }
    Ok(())
}

/// Add the named streams, the type maps and the injected sources.
fn named_streams(pdb: &Pdb<&[u8]>, names: &[u8], corpus: &mut Corpus) -> Outcome {
    let table = StringTable::parse(names).ok();
    for named in pdb.info().named_streams() {
        let bytes = pdb.stream_bytes(named.stream)?;
        let selector: u8 = match NamedStreamKind::classify(&named.name) {
            NamedStreamKind::LinkInfo => 0,
            NamedStreamKind::TmCache => {
                if let Ok(cache) = pdbrs::native::auxiliary::TmCache::parse(&bytes, named.stream) {
                    for id in cache.streams().flatten().take(8) {
                        corpus.add_parts("native_named", &[2], &[], &pdb.stream_bytes(id)?)?;
                    }
                }
                1
            }
            NamedStreamKind::UdtSourceLineUndone => 3,
            NamedStreamKind::SourceLink => 4,
            NamedStreamKind::SourceHeaderBlock => {
                injected_sources(pdb, &bytes, names, table.as_ref(), corpus)?;
                continue;
            }
            _ => 8,
        };
        if selector == 8 {
            corpus.add_parts("native_named", &[8], &[], &named.name)?;
        } else {
            corpus.add_parts("native_named", &[selector], &[], &bytes)?;
        }
    }
    Ok(())
}

/// Add the injected source header block with the content of each source.
fn injected_sources(
    pdb: &Pdb<&[u8]>,
    block: &[u8],
    names: &[u8],
    table: Option<&StringTable<'_>>,
    corpus: &mut Corpus,
) -> Outcome {
    let (Some(parsed), Some(table)) = (SourceHeaderBlock::parse(block, StreamId(0))?, table) else {
        return Ok(());
    };
    for entry in parsed.entries() {
        let source = entry.value;
        let name = content_stream_name(table.get(source.virtual_file_name_offset)?)?;
        let Some(content) = pdb.named_stream_bytes(&name)? else {
            continue;
        };
        corpus.add_parts("native_named", &[5], &[block, names], &content)?;
        if source.compression == SourceCompression::DOT_NET {
            corpus.add_parts("native_named", &[6], &[], &content)?;
        }
    }
    Ok(())
}

/// Add the DBI stream, the debug streams, the module streams and the symbol streams.
fn debug_info(pdb: &Pdb<&[u8]>, dbi: &Dbi, kinds: &mut Kinds, corpus: &mut Corpus) -> Outcome {
    let headers = match dbi.debug_stream(DebugStream::SectionHeader) {
        Some(id) => pdb.stream_bytes(id)?,
        None => Vec::new(),
    };
    corpus.add_parts(
        "native_dbi",
        &[],
        &[&headers],
        &stream_of(pdb, StreamId(3))?,
    )?;
    for kind in [
        DebugStream::Fpo,
        DebugStream::NewFpo,
        DebugStream::SectionHeader,
    ] {
        if let Some(id) = dbi.debug_stream(kind) {
            corpus.add("native_frames", &pdb.stream_bytes(id)?)?;
        }
    }
    for module in dbi.modules() {
        let (index, module) = module?;
        let Some(id) = module.stream else {
            continue;
        };
        let bytes = pdb.stream_bytes(id)?;
        let sizes = [module.symbol_bytes, module.c11_bytes, module.c13_bytes];
        let prefix: Vec<u8> = sizes.iter().flat_map(|size| size.to_le_bytes()).collect();
        corpus.add_parts("native_module", &prefix, &[], &bytes)?;
        let Some(stream) = pdb.module(index)? else {
            continue;
        };
        corpus.add("native_symbols", stream.symbol_data())?;
        for subsection in stream.c13_subsections() {
            let subsection = subsection?;
            if kinds.take(3, subsection.kind.0) {
                corpus.add_parts(
                    "native_lines",
                    &subsection.kind.0.to_le_bytes(),
                    &[],
                    &subsection.data,
                )?;
            }
        }
        let base = ModuleSymbolOffset(u32::try_from(stream.symbol_offset())?);
        for scoped in ModuleSymbols::new(stream.symbol_data(), id, base, 64) {
            let Ok(scoped) = scoped else {
                break;
            };
            let record = scoped.symbol.record();
            if kinds.take(2, u32::from(record.kind)) {
                corpus.add_parts("native_leaf", &record.kind.to_le_bytes(), &[], record.data)?;
            }
            if let SymbolData::Oem(oem) = scoped.symbol.data()
                && let Ok(Some(attribute)) = oem.attribute()
                && kinds.take(4, u32::from(attribute.name.first().copied().unwrap_or(0)))
            {
                attribute_seeds(&oem.data, &attribute, corpus)?;
            }
        }
    }
    shared_symbols(pdb, dbi, corpus)
}

/// Add the seeds of a native symbol attribute: the value and the whole OEM data.
fn attribute_seeds(
    data: &[u8],
    attribute: &pdbrs::native::codeview::symbol::SymbolAttribute<'_>,
    corpus: &mut Corpus,
) -> Outcome {
    use pdbrs::native::codeview::symbol::{ASYNC_METHOD_INFO_ATTRIBUTE, MD2_ATTRIBUTE};
    if attribute.name == MD2_ATTRIBUTE {
        corpus.add_parts("custom_debug_info", &[0x80], &[], &attribute.value)?;
    } else if attribute.name == ASYNC_METHOD_INFO_ATTRIBUTE {
        corpus.add_parts("custom_debug_info", &[0x81], &[], &attribute.value)?;
    }
    corpus.add_parts("custom_debug_info", &[0x82], &[], data)
}

/// Add the symbol record stream, the GSI and the PSI.
fn shared_symbols(pdb: &Pdb<&[u8]>, dbi: &Dbi, corpus: &mut Corpus) -> Outcome {
    let Some(id) = dbi.header().symbol_stream else {
        return Ok(());
    };
    let records = pdb.stream_bytes(id)?;
    corpus.add("native_symbols", &records)?;
    let load = |id: Option<StreamId>| -> Outcome<Vec<u8>> {
        Ok(match id {
            Some(id) => pdb.stream_bytes(id)?,
            None => Vec::new(),
        })
    };
    let globals = load(dbi.header().global_stream)?;
    let publics = load(dbi.header().public_stream)?;
    let minimal = u8::from(pdb.symbol_bucket_count().count() != 4096);
    corpus.add_parts(
        "native_indexes",
        &[minimal],
        &[&globals, &publics],
        &records,
    )
}
