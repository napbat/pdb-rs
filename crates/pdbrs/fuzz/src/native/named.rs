//! The named streams: `/LinkInfo`, `/TMCache` and its type maps, `/UDTSRCLINEUNDONE`,
//! `sourcelink`, the injected sources and the stream 0 directory.

use pdbrs::native::auxiliary::{
    LinkInfo, NamedStreamKind, SourceLink, TmCache, TypeMapKind, TypeMapStream, UdtSourceLineUndone,
};
use pdbrs::native::injected_source::{
    DotNetDocument, InjectedSource, SourceCompression, SourceHeaderBlock, content_stream_name,
};
use pdbrs::native::msf::{OldDirectory, StreamId};
use pdbrs::native::names::StringTable;

use crate::input::Input;
use crate::{MAX_STREAM, ok};

/// The most type map streams that one `/TMCache` walk loads.
const MAX_TYPE_MAPS: usize = 256;

/// The stream number that the targets give to named stream data.
const STREAM: StreamId = StreamId(10);

/// Decode a named stream. The first input byte selects the stream kind.
///
/// The injected source kind reads a header block part, a `/names` part and then the content.
///
/// # Panics
///
/// Panics if an iterator gives another number of items than the stream reports.
pub fn stream(data: &[u8]) {
    let mut input = Input::new(data);
    let selector = input.u8();
    match selector % 9 {
        0 => {
            if let Some(Some(info)) = ok(LinkInfo::parse(input.rest(), STREAM)) {
                let _ = info.output_file();
            }
        }
        1 => {
            if let Some(cache) = ok(TmCache::parse(input.rest(), STREAM)) {
                let _ = cache.is_empty();
                assert_eq!(cache.streams().count(), cache.entries().count());
            }
        }
        2 => type_map(input.rest()),
        3 => {
            if let Some(list) = ok(UdtSourceLineUndone::parse(input.rest(), STREAM)) {
                assert_eq!(list.type_indices().count(), list.len());
            }
        }
        4 => {
            if let Some(link) = ok(SourceLink::parse(input.rest(), STREAM)) {
                let _ = link.json().len();
            }
        }
        5 => {
            let block = input.part();
            let names = ok(StringTable::parse(input.part()));
            let content = input.rest();
            header_block(block, names.as_ref(), |_| Some(content.to_vec()));
        }
        6 => document(input.rest()),
        7 => {
            let block_size = 1 << (9 + (input.u8() % 4));
            if let Some(Some(old)) = ok(OldDirectory::parse(input.rest(), block_size)) {
                assert_eq!(old.streams().count(), old.streams().len());
                for stream in old.streams() {
                    let _ = (stream.size(), stream.pages().count());
                }
            }
        }
        _ => {
            let _ = content_stream_name(input.rest());
            let _ = NamedStreamKind::classify(input.rest());
        }
    }
}

/// Decode a named stream of the kind that its name gives.
///
/// `load` reads a stream by number, for the type maps. `content` reads the content stream of
/// an injected source by the stream name.
pub(crate) fn named(
    name: &[u8],
    bytes: &[u8],
    id: StreamId,
    names: Option<&StringTable<'_>>,
    load: impl Fn(StreamId) -> Option<Vec<u8>>,
    content: impl Fn(&[u8]) -> Option<Vec<u8>>,
) {
    match NamedStreamKind::classify(name) {
        NamedStreamKind::LinkInfo => {
            let _ = LinkInfo::parse(bytes, id);
        }
        NamedStreamKind::TmCache => {
            let Some(cache) = ok(TmCache::parse(bytes, id)) else {
                return;
            };
            for stream in cache.streams().flatten().take(MAX_TYPE_MAPS) {
                if let Some(map) = load(stream) {
                    type_map(&map);
                }
            }
        }
        NamedStreamKind::UdtSourceLineUndone => {
            let _ = UdtSourceLineUndone::parse(bytes, id);
        }
        NamedStreamKind::SourceLink => {
            let _ = SourceLink::parse(bytes, id);
        }
        NamedStreamKind::NumberedSourceLink { .. } => {
            let _ = SourceLink::parse_numbered(bytes, id);
        }
        NamedStreamKind::SourceHeaderBlock => header_block(bytes, names, content),
        _ => {}
    }
}

/// Decode a type map stream and visit its maps.
fn type_map(bytes: &[u8]) {
    let Some(map) = ok(TypeMapStream::parse(bytes, STREAM)) else {
        return;
    };
    if let TypeMapKind::Object { precompiled, .. } = map.kind() {
        let _ = precompiled;
    }
    let _ = map.first_index();
    assert_eq!(map.types().count(), map.types().len());
    assert_eq!(map.ids().count(), map.ids().len());
    assert_eq!(map.record_offsets().count(), map.record_offsets().len());
    assert_eq!(map.function_types().count(), map.function_types().len());
}

/// Decode an injected source header block and verify each source.
///
/// `content` reads the content stream of a source by the stream name. Without a `/names`
/// table, it gets an empty name.
fn header_block(
    bytes: &[u8],
    names: Option<&StringTable<'_>>,
    content: impl Fn(&[u8]) -> Option<Vec<u8>>,
) {
    let Some(Some(block)) = ok(SourceHeaderBlock::parse(bytes, STREAM)) else {
        return;
    };
    let _ = block.header();
    assert_eq!(block.entries().len(), block.table().len());
    for entry in block.entries() {
        let source = entry.value;
        let stream_name = match names {
            Some(names) => {
                let name = ok(names.get(source.virtual_file_name_offset));
                let Some(name) = name.and_then(|name| ok(content_stream_name(name))) else {
                    continue;
                };
                name
            }
            None => Vec::new(),
        };
        if let Some(bytes) = content(&stream_name) {
            injected_source(&source, &bytes);
        }
    }
}

/// Verify the content of one injected source.
fn injected_source(source: &InjectedSource, content: &[u8]) {
    let _ = SourceCompression::from_raw(0);
    if source.compression == SourceCompression::DotNet {
        if let Some(document) = ok(source.verify_document(content, STREAM)) {
            embedded(&document);
        }
    } else {
        let _ = source.verify_content(content, STREAM);
    }
}

/// Decode a .NET document record and its embedded source.
fn document(bytes: &[u8]) {
    if let Some(document) = ok(DotNetDocument::parse(bytes)) {
        embedded(&document);
    }
}

/// Decode the embedded source of a .NET document record and verify its checksum.
fn embedded(document: &DotNetDocument<'_>) {
    let _ = document.verify_checksum(document.checksum);
    let Some(source) = &document.embedded else {
        return;
    };
    let _ = source.decoded_size();
    if let Some(decoded) = ok(source.decode(MAX_STREAM)) {
        let _ = document.verify_checksum(&decoded);
    }
}
