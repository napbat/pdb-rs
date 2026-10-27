//! Named streams with noncontiguous pages: the Info stream, `/LinkInfo` and injected sources.

use pdb_rs::native::auxiliary::{LINK_INFO_STREAM_NAME, LINK_INFO_VERSION_TWO, LinkInfo};
use pdb_rs::native::injected_source::{
    HEADER_BLOCK_STREAM_NAME, InjectedFile, InjectedSource, SourceCompression, SourceHeaderBlock,
    SourceHeaderBlockBuilder, content_stream_name,
};
use pdb_rs::native::msf::StreamId;
use pdb_rs::native::names::{HashVersion, StringTableBuilder};
use pdb_rs::native::{Pdb, Producer, ReadOptions};
use pdb_rs::{Bytes, ErrorKind};

use super::{Order, PAGE, container, crosses_page, info, open, to_u32, to_u64};

/// The `/names` stream.
const NAMES: StreamId = StreamId(2);
/// The `/LinkInfo` stream.
const LINK_INFO: StreamId = StreamId(3);
/// The `/src/headerblock` stream.
const HEADER_BLOCK: StreamId = StreamId(4);
/// The content stream of the target injected source.
const CONTENT: StreamId = StreamId(5);
/// The empty stream with a long name.
const LONG: StreamId = StreamId(6);
/// The number of injected sources.
const FILE_COUNT: usize = 12;
/// The size of a key and an entry in the injected source table.
const ENTRY_SIZE: usize = 44;
/// The injected source with a content stream.
const TARGET_FILE: usize = 5;
/// The output file at the end of the link command.
const OUTPUT: &[u8] = b"app.exe";

/// Return a stream name that is longer than one page.
fn long_name() -> Vec<u8> {
    let mut name = b"/long/".to_vec();
    name.resize(PAGE + 20, b'x');
    name
}

/// Return a link command that is longer than one page.
fn command() -> Vec<u8> {
    let mut command = b"link.exe".to_vec();
    while command.len() < PAGE + 100 {
        command.extend_from_slice(b" -opt:ref");
    }
    command.extend_from_slice(b" -out:");
    command.extend_from_slice(OUTPUT);
    command
}

fn virtual_name(index: usize) -> Vec<u8> {
    format!("c:\\src\\file_{index:02}.cpp").into_bytes()
}

/// Return the contents of injected source `index`. The target has more than one page.
fn content(index: usize) -> Vec<u8> {
    let length = if index == TARGET_FILE { PAGE + 200 } else { 16 };
    let seed = u8::try_from(index).unwrap();
    (0..length)
        .map(|byte| u8::try_from(byte % 251).unwrap() ^ seed)
        .collect()
}

/// The named streams of the fixture PDB.
struct Named {
    info: Vec<u8>,
    names: Vec<u8>,
    link_info: Vec<u8>,
    header_block: Vec<u8>,
    content: Vec<u8>,
    /// The injected source of the target file.
    target: InjectedSource,
}

impl Named {
    /// Build the streams. If `long` is `true`, the Info stream also names a stream with a long name.
    fn new(long: bool) -> Self {
        let command = command();
        let output = to_u32(command.len() - OUTPUT.len());
        let record = LinkInfo::new(
            LINK_INFO_VERSION_TWO,
            b"C:\\build",
            command.as_slice(),
            output,
            b"a.lib\0",
        )
        .unwrap();
        let mut link_info = Vec::new();
        record.encode(&mut link_info).unwrap();

        let mut names = StringTableBuilder::new(HashVersion::V1).unwrap();
        let mut builder = SourceHeaderBlockBuilder::new(Producer::Microsoft, 7, 1).unwrap();
        let mut sources = Vec::new();
        for index in 0..FILE_COUNT {
            let (virtual_file_name, stream) = (virtual_name(index), content(index));
            let mut file = InjectedFile::default();
            file.file_name = Bytes::new(&virtual_file_name);
            file.object_name = Bytes::new(b"");
            file.virtual_file_name = Bytes::new(&virtual_file_name);
            file.compression = SourceCompression::NONE;
            file.is_virtual = false;
            file.stream = Bytes::new(&stream);
            sources.push(builder.add_file(&mut names, &file).unwrap());
        }
        let mut header_block = Vec::new();
        builder.encode(&mut header_block).unwrap();
        let mut names_bytes = Vec::new();
        names.encode(&mut names_bytes).unwrap();

        let content_name = content_stream_name(&virtual_name(TARGET_FILE)).unwrap();
        let long_name = long_name();
        let mut map: Vec<(&[u8], StreamId)> = vec![
            (b"/names", NAMES),
            (LINK_INFO_STREAM_NAME, LINK_INFO),
            (HEADER_BLOCK_STREAM_NAME, HEADER_BLOCK),
            (&content_name, CONTENT),
        ];
        if long {
            map.push((&long_name, LONG));
        }
        Self {
            info: info(&map),
            names: names_bytes,
            link_info,
            header_block,
            content: content(TARGET_FILE),
            target: sources[TARGET_FILE],
        }
    }

    fn container(self, order: Order) -> Vec<u8> {
        let streams = [
            Some(Vec::new()),
            Some(self.info),
            Some(self.names),
            Some(self.link_info),
            Some(self.header_block),
            Some(self.content),
            Some(Vec::new()),
        ];
        container(&streams, order)
    }
}

#[test]
fn info_named_stream_map_across_pages_resolves() {
    let info_size = Named::new(true).info.len();
    assert!(info_size > PAGE, "the named stream map crosses a page");
    let content_name = content_stream_name(&virtual_name(TARGET_FILE)).unwrap();
    for order in [Order::Contiguous, Order::Scattered] {
        let bytes = Named::new(true).container(order);
        let pdb = open(&bytes);
        let info = pdb.info();
        assert_eq!(info.named_stream(b"/names"), Some(NAMES));
        assert_eq!(info.named_stream(LINK_INFO_STREAM_NAME), Some(LINK_INFO));
        assert_eq!(
            info.named_stream(HEADER_BLOCK_STREAM_NAME),
            Some(HEADER_BLOCK)
        );
        assert_eq!(info.named_stream(&content_name), Some(CONTENT));
        assert_eq!(info.named_stream(&long_name()), Some(LONG));
        assert_eq!(info.named_streams().count(), 5);
    }
}

#[test]
fn link_info_across_pages_decodes_like_one_buffer() {
    let reference = Named::new(true).link_info;
    let expected = LinkInfo::parse(&reference, LINK_INFO).unwrap().unwrap();
    let bytes = Named::new(true).container(Order::Scattered);
    let stream = open(&bytes)
        .named_stream_bytes(LINK_INFO_STREAM_NAME)
        .unwrap()
        .unwrap();
    let record = LinkInfo::parse(&stream, LINK_INFO).unwrap().unwrap();
    assert_eq!(record, expected);
    assert_eq!(record.command, command());
    assert_eq!(record.output_file(), OUTPUT);
}

#[test]
fn truncated_link_info_across_pages_is_eof() {
    let mut named = Named::new(true);
    named.link_info.truncate(PAGE + 10);
    let bytes = named.container(Order::Scattered);
    let stream = open(&bytes)
        .named_stream_bytes(LINK_INFO_STREAM_NAME)
        .unwrap()
        .unwrap();
    let error = LinkInfo::parse(&stream, LINK_INFO).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(error.stream_location().map(|(id, _)| id), Some(LINK_INFO));
}

#[test]
fn injected_source_across_pages_decodes_and_verifies() {
    let reference = Named::new(true);
    let expected = SourceHeaderBlock::parse(&reference.header_block, HEADER_BLOCK)
        .unwrap()
        .unwrap();
    // The table entries are at the end of the block. One entry crosses offset 512.
    let entries = reference.header_block.len() - FILE_COUNT * ENTRY_SIZE;
    assert!(
        (0..FILE_COUNT).any(|index| crosses_page(entries + index * ENTRY_SIZE, ENTRY_SIZE)),
        "an injected source entry crosses a page"
    );
    assert!(crosses_page(0, reference.content.len()));

    let bytes = Named::new(true).container(Order::Scattered);
    let pdb = open(&bytes);
    let block_bytes = pdb
        .named_stream_bytes(HEADER_BLOCK_STREAM_NAME)
        .unwrap()
        .unwrap();
    let block = SourceHeaderBlock::parse(&block_bytes, HEADER_BLOCK)
        .unwrap()
        .unwrap();
    assert_eq!(block.entries(), expected.entries());
    assert_eq!(block.header(), expected.header());
    let target = reference.target;
    let entry = block
        .entries()
        .iter()
        .find(|entry| entry.key == target.virtual_file_name_offset.0)
        .unwrap();
    assert_eq!(entry.value, target);

    let content_name = content_stream_name(&virtual_name(TARGET_FILE)).unwrap();
    let stream = pdb.named_stream_bytes(&content_name).unwrap().unwrap();
    assert_eq!(
        entry.value.verify_content(&stream, CONTENT).unwrap(),
        content(TARGET_FILE)
    );
}

#[test]
fn truncated_injected_source_block_across_pages_is_eof() {
    // The header size must equal the stream size, so the test changes both.
    let mut named = Named::new(true);
    let entries = named.header_block.len() - FILE_COUNT * ENTRY_SIZE;
    named.header_block.truncate(PAGE + 10);
    named.header_block[4..8].copy_from_slice(&to_u32(PAGE + 10).to_le_bytes());
    let bytes = named.container(Order::Scattered);
    let stream = open(&bytes)
        .named_stream_bytes(HEADER_BLOCK_STREAM_NAME)
        .unwrap()
        .unwrap();
    let error = SourceHeaderBlock::parse(&stream, HEADER_BLOCK).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(
        error.stream_location(),
        Some((HEADER_BLOCK, to_u64(entries)))
    );
}

#[test]
fn named_stream_loads_check_the_stream_size_against_the_limit() {
    let bytes = Named::new(false).container(Order::Scattered);
    let full = open(&bytes);
    let info_size = full.stream_bytes(StreamId(1)).unwrap().len();
    let content_name = content_stream_name(&virtual_name(TARGET_FILE)).unwrap();
    for (name, id) in [
        (LINK_INFO_STREAM_NAME, LINK_INFO),
        (HEADER_BLOCK_STREAM_NAME, HEADER_BLOCK),
        (content_name.as_slice(), CONTENT),
    ] {
        let size = full.stream_bytes(id).unwrap().len();
        assert!(size > info_size, "the Info stream fits in each limit");
        let options =
            |max_stream_bytes| ReadOptions::default().with_max_stream_bytes(max_stream_bytes);
        let exact = Pdb::with_options(bytes.as_slice(), options(size)).unwrap();
        assert_eq!(exact.named_stream_bytes(name).unwrap().unwrap().len(), size);
        let small = Pdb::with_options(bytes.as_slice(), options(size - 1)).unwrap();
        let error = small.named_stream_bytes(name).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::LimitExceeded, "{id:?}");
        assert_eq!(error.stream_location(), Some((id, 0)));
        assert_eq!(error.offset(), None);
    }
}
