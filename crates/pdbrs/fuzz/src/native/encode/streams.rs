//! Round trips of the stream encoders: C13 subsections, module streams, the Info stream,
//! `/names`, the DBI stream, the optional debug streams and the named streams.

use pdbrs::native::Producer;
use pdbrs::native::auxiliary::{
    LinkInfo, SourceLink, SourceServer, TmCache, TmCacheBuilder, TypeMapStream, UdtSourceLineUndone,
};
use pdbrs::native::dbi::{
    Dbi, DbiBuilder, ModuleStream, ModuleStreamParts, OwnedModule, Substream,
};
use pdbrs::native::debug_stream::{
    FixupStream, FunctionEntryStream, OmapStream, RvaVaBlob, TokenRidMap,
};
use pdbrs::native::fpo::{FpoStream, FrameDataStream};
use pdbrs::native::info::{Info, InfoBuilder};
use pdbrs::native::injected_source::{DotNetDocument, SourceHeaderBlock, SourceHeaderBlockBuilder};
use pdbrs::native::lines::{LineSubsection, ModuleLines, SubsectionKind};
use pdbrs::native::msf::StreamId;
use pdbrs::native::names::{StringTable, StringTableBuilder};
use pdbrs::native::section::SectionHeaderStream;

use crate::input::Input;
use crate::native::dbi::module_dbi;
use crate::ok;

/// The stream number that the targets give to stream data.
const STREAM: StreamId = StreamId(10);

/// The substreams of a DBI stream in disk order.
const SUBSTREAMS: [Substream; 7] = [
    Substream::ModuleInfo,
    Substream::SectionContributions,
    Substream::SectionMap,
    Substream::SourceInfo,
    Substream::TypeServerMap,
    Substream::Ec,
    Substream::OptionalDebugHeader,
];

/// Round-trip one C13 subsection. The first four bytes are the kind.
///
/// # Panics
///
/// Panics if the encoder rejects a decoded subsection or gives other data.
pub fn lines(data: &[u8]) {
    let mut input = Input::new(data);
    let kind = input.u32();
    let data = input.rest();
    let Some(decoded) = ok(LineSubsection::decode(SubsectionKind(kind), data)) else {
        return;
    };
    let mut out = Vec::new();
    decoded
        .encode(&mut out)
        .expect("a decoded subsection encodes");
    assert_eq!(out, data, "a decoded subsection gives its data again");
    let mut framed = Vec::new();
    decoded
        .encode_framed(&mut framed)
        .expect("a decoded subsection encodes with its header");
    let length = u32::from_le_bytes(framed[4..8].try_into().expect("a header has a length"));
    let end = 8 + usize::try_from(length).expect("a length fits");
    assert_eq!(framed[..4], kind.to_le_bytes());
    assert_eq!(framed[8..end], *data);
    assert!(framed.len().is_multiple_of(4));
}

/// Round-trip a module stream and its C13 line region.
///
/// The input has the layout of the `native_module` target.
///
/// # Panics
///
/// Panics if an encoder rejects a decoded stream, or if the output does not parse with the
/// same regions and line data.
pub fn module(data: &[u8]) {
    let mut input = Input::new(data);
    let sizes = [input.u32(), input.u32(), input.u32()];
    let Some(dbi) = module_dbi(sizes) else {
        return;
    };
    let Some(Ok(module)) = dbi.modules().next() else {
        return;
    };
    let Some(stream) = ok(ModuleStream::parse(input.rest().to_vec(), &module)) else {
        return;
    };
    let mut encoded = Vec::new();
    let Some(region_sizes) = ok(stream.encode(&mut encoded)) else {
        return;
    };
    let sizes = [
        region_sizes.symbol_bytes,
        region_sizes.c11_bytes,
        region_sizes.c13_bytes,
    ];
    let again_dbi = module_dbi(sizes).expect("module region sizes fit a module record");
    let again_module = again_dbi
        .modules()
        .next()
        .expect("a module record decodes")
        .expect("a module record decodes");
    let again =
        ModuleStream::parse(encoded, &again_module).expect("an encoded module stream parses");
    assert_eq!(again.symbol_data(), stream.symbol_data());
    assert_eq!(again.c11_data(), stream.c11_data());
    assert_eq!(again.c13_data(), stream.c13_data());
    assert_eq!(again.global_refs().ok(), stream.global_refs().ok());

    let Some(lines) = ok(ModuleLines::parse(&stream)) else {
        return;
    };
    let mut c13 = Vec::new();
    lines.encode(&mut c13).expect("decoded line data encodes");
    let Some(refs) = ok(stream.global_refs()) else {
        return;
    };
    let refs: Vec<_> = refs.iter().collect();
    let mut parts = ModuleStreamParts::default();
    parts.symbols = Some(stream.symbol_data());
    parts.c11 = stream.c11_data();
    parts.c13 = &c13;
    parts.global_refs = &refs;
    let mut rebuilt = Vec::new();
    let region_sizes = parts
        .encode(&mut rebuilt)
        .expect("module stream parts encode");
    let sizes = [
        region_sizes.symbol_bytes,
        region_sizes.c11_bytes,
        region_sizes.c13_bytes,
    ];
    let rebuilt_dbi = module_dbi(sizes).expect("module region sizes fit a module record");
    let rebuilt_module = rebuilt_dbi
        .modules()
        .next()
        .expect("a module record decodes")
        .expect("a module record decodes");
    let rebuilt =
        ModuleStream::parse(rebuilt, &rebuilt_module).expect("a rebuilt module stream parses");
    let again = ModuleLines::parse(&rebuilt).expect("encoded line data decodes");
    assert_eq!(again.subsections().len(), lines.subsections().len());
    for (left, right) in again.subsections().iter().zip(lines.subsections()) {
        assert_eq!((left.kind, &left.data), (right.kind, &right.data));
    }
}

/// Round-trip an Info stream and its builder model.
///
/// # Panics
///
/// Panics if the encoder gives other bytes, or if the builder gives other named streams.
pub fn info(data: &[u8]) {
    let Some(info) = ok(Info::parse(data.to_vec())) else {
        return;
    };
    let mut out = Vec::new();
    info.encode(&mut out)
        .expect("a decoded Info stream encodes");
    assert_eq!(out, data, "a decoded Info stream gives its bytes again");
    let builder = InfoBuilder::from_info(&info).expect("a decoded Info stream has a model");
    let mut rebuilt = Vec::new();
    builder.encode(&mut rebuilt).expect("an Info model encodes");
    let again = Info::parse(rebuilt).expect("an encoded Info model parses");
    let named = |info: &Info| -> Vec<(Vec<u8>, StreamId)> {
        let mut named: Vec<_> = info
            .named_streams()
            .map(|named| (named.name.to_vec(), named.stream))
            .collect();
        named.sort_unstable();
        named
    };
    assert_eq!(named(&again), named(&info));
    assert_eq!(again.identity(), info.identity());
    assert!(again.features().eq(info.features()));
}

/// Round-trip a `/names` string table and its builder.
///
/// # Panics
///
/// Panics if the encoder gives other bytes, or if the rebuilt table loses a name.
pub fn names(data: &[u8]) {
    let Some(table) = ok(StringTable::parse(data)) else {
        return;
    };
    let mut out = Vec::new();
    table
        .encode(&mut out)
        .expect("a decoded string table encodes");
    assert_eq!(out, data, "a decoded string table gives its bytes again");
    let Some(builder) = ok(StringTableBuilder::from_table(&table)) else {
        return;
    };
    let mut rebuilt = Vec::new();
    builder
        .encode(&mut rebuilt)
        .expect("a string table model encodes");
    let again = StringTable::parse(&rebuilt).expect("an encoded string table parses");
    for entry in table.entries() {
        assert_eq!(again.get(entry.offset).ok(), Some(entry.name));
    }
}

/// Round-trip a DBI stream, its substreams, its module records and its builder model.
///
/// The input has the layout of the `native_dbi` target.
///
/// # Panics
///
/// Panics if an encoder rejects a decoded value or gives other bytes.
pub fn dbi(data: &[u8]) {
    let mut input = Input::new(data);
    let _headers = input.part();
    let bytes = input.rest();
    let Some(dbi) = ok(Dbi::parse(bytes.to_vec())) else {
        return;
    };
    let mut out = Vec::new();
    if dbi.encode(&mut out).is_ok() {
        let again = Dbi::parse(out).expect("an encoded DBI stream parses");
        for kind in SUBSTREAMS {
            assert_eq!(
                again.substream(kind),
                dbi.substream(kind),
                "substream {kind:?}"
            );
        }
    }
    for kind in SUBSTREAMS {
        let mut out = Vec::new();
        if dbi.encode_substream(kind, &mut out).is_ok() {
            assert_eq!(
                out,
                dbi.substream(kind),
                "substream {kind:?} re-encodes to other bytes"
            );
        }
    }
    for module in dbi.modules() {
        let Some(module) = ok(module) else {
            break;
        };
        let owned = OwnedModule::from_module(&module).expect("a decoded module record has a model");
        let mut out = Vec::new();
        owned
            .encode(&mut out)
            .expect("a decoded module record encodes");
        let mut direct = Vec::new();
        module
            .encode(&mut direct)
            .expect("a decoded module record encodes");
        assert_eq!(out, direct);
        let start = usize::try_from(module.offset).unwrap_or(usize::MAX)
            - dbi.substream_offset(Substream::ModuleInfo);
        let stored = dbi
            .substream(Substream::ModuleInfo)
            .get(start..start + out.len());
        assert_eq!(
            stored,
            Some(&out[..]),
            "a decoded module record gives its bytes again"
        );
    }
    if let Some(builder) = ok(DbiBuilder::from_dbi(&dbi)) {
        let rebuilt = builder.to_bytes().expect("a DBI model encodes");
        assert_eq!(
            rebuilt, bytes,
            "a DBI model gives the bytes of the stream again"
        );
    }
}

/// Round-trip one optional debug stream. The first input byte selects the stream kind.
///
/// # Panics
///
/// Panics if an encoder gives other bytes than the decoded stream.
pub fn debug_stream(data: &[u8]) {
    let mut input = Input::new(data);
    let selector = input.u8();
    let bytes = input.rest();
    let encoded = match selector % 8 {
        0 => ok(FpoStream::parse(bytes.to_vec(), STREAM)).map(|s| s.to_bytes()),
        1 => ok(FrameDataStream::parse(bytes.to_vec(), STREAM)).map(|s| s.to_bytes()),
        2 => ok(FixupStream::parse(bytes.to_vec(), STREAM)).map(|s| s.to_bytes()),
        3 => ok(FunctionEntryStream::parse(bytes.to_vec(), STREAM)).map(|s| s.to_bytes()),
        4 => ok(TokenRidMap::parse(bytes.to_vec(), STREAM)).map(|s| s.to_bytes()),
        5 => ok(SectionHeaderStream::parse(bytes.to_vec(), STREAM)).map(|s| s.to_bytes()),
        6 => ok(OmapStream::parse(bytes.to_vec(), STREAM)).map(|s| s.to_bytes()),
        _ => ok(RvaVaBlob::parse(bytes.to_vec(), STREAM)).map(|s| s.to_bytes()),
    };
    if let Some(encoded) = encoded {
        let encoded = encoded.expect("a decoded debug stream encodes");
        assert_eq!(
            encoded, bytes,
            "a decoded debug stream gives its bytes again"
        );
    }
}

/// Check that an encoder gives the input bytes again.
fn same(encode: impl FnOnce(&mut Vec<u8>) -> pdbrs::Result<()>, bytes: &[u8], what: &str) {
    let mut out = Vec::new();
    encode(&mut out).unwrap_or_else(|error| panic!("a decoded {what} does not encode: {error}"));
    assert_eq!(out, bytes, "a decoded {what} gives other bytes");
}

/// Round-trip one named stream. The first input byte selects the stream kind.
///
/// # Panics
///
/// Panics if an encoder rejects a decoded stream or gives other bytes.
pub fn named(data: &[u8]) {
    let mut input = Input::new(data);
    let selector = input.u8();
    let bytes = input.rest();
    match selector % 8 {
        0 => {
            if let Some(Some(link)) = ok(LinkInfo::parse(bytes, STREAM)) {
                same(|out| link.encode(out), bytes, "/LinkInfo stream");
            }
        }
        1 => {
            if let Some(cache) = ok(TmCache::parse(bytes, STREAM)) {
                same(|out| cache.encode(out), bytes, "/TMCache stream");
                let builder =
                    TmCacheBuilder::from_cache(&cache).expect("a decoded cache has a model");
                let mut out = Vec::new();
                builder.encode(&mut out).expect("a cache model encodes");
                let again = TmCache::parse(&out, STREAM).expect("an encoded cache parses");
                assert!(again.entries().eq(cache.entries()));
                assert!(again.streams().eq(cache.streams()));
            }
        }
        2 => {
            if let Some(map) = ok(TypeMapStream::parse(bytes, STREAM)) {
                same(|out| map.encode(out), bytes, "type map stream");
            }
        }
        3 => {
            if let Some(list) = ok(UdtSourceLineUndone::parse(bytes, STREAM)) {
                same(|out| list.encode(out), bytes, "/UDTSRCLINEUNDONE stream");
            }
        }
        4 => {
            if let Some(link) = ok(SourceLink::parse(bytes, STREAM)) {
                same(|out| link.encode(out), bytes, "sourcelink stream");
            }
        }
        5 => {
            if let Some(server) = ok(SourceServer::parse(bytes, STREAM)) {
                same(|out| server.encode(out), bytes, "srcsrv stream");
            }
        }
        6 => header_block(bytes),
        _ => {
            if let Some(document) = ok(DotNetDocument::parse(bytes)) {
                same(|out| document.encode(out), bytes, ".NET document record");
            }
        }
    }
}

/// Round-trip an injected source header block and its builder model.
fn header_block(bytes: &[u8]) {
    let Some(Some(block)) = ok(SourceHeaderBlock::parse(bytes, STREAM)) else {
        return;
    };
    same(|out| block.encode(out), bytes, "/src/headerblock stream");
    for producer in [Producer::Microsoft, Producer::Llvm] {
        let Some(builder) = ok(SourceHeaderBlockBuilder::from_block(&block, producer)) else {
            continue;
        };
        let mut out = Vec::new();
        builder
            .encode(&mut out)
            .expect("a header block model encodes");
        let again = SourceHeaderBlock::parse(&out, STREAM)
            .expect("an encoded header block parses")
            .expect("an encoded header block has a header");
        let values = |block: &SourceHeaderBlock<'_>| {
            let mut values: Vec<_> = block
                .entries()
                .iter()
                .map(|entry| (entry.key, entry.value))
                .collect();
            values.sort_unstable_by_key(|(key, _)| *key);
            values
        };
        assert_eq!(values(&again), values(&block));
    }
}
