//! Typed stream loads through the MSF stream reader.
//!
//! The fixture is a native PDB with 512-byte pages. In each typed stream, one
//! record or field crosses a page boundary. The scattered layout puts the
//! pages of each stream at locations that are not adjacent in the file.

mod cross_page;
mod limits;
mod named;

use pdbrs::native::Pdb;
use pdbrs::native::Producer;
use pdbrs::native::debug_stream::{OmapRecord, OmapStream};
use pdbrs::native::hash::hash_string_v1;
use pdbrs::native::info::InfoBuilder;
use pdbrs::native::lines::{DEBUG_S_FILECHKSMS, DEBUG_S_LINES, SubsectionKind};
use pdbrs::native::msf::StreamId;
use pdbrs::native::names::SIGNATURE;
use pdbrs::native::section::{SectionHeader, SectionHeaderStream};
use pdbrs::native::symbols::BucketCount;
use pdbrs::native::types::{HEADER_SIZE, TPI_V80};

/// The page size of the fixture containers.
const PAGE: usize = 512;
/// The first page that holds stream data. Pages 0 to 2 are reserved, and page 3 is the page map.
const FIRST_DIRECTORY_PAGE: usize = 4;
/// The TPI stream.
const TPI: StreamId = StreamId(2);
/// The DBI stream.
const DBI: StreamId = StreamId(3);
/// The `/names` stream.
const NAMES: StreamId = StreamId(5);
/// The module stream of the target module.
const MODULE: StreamId = StreamId(6);
/// The symbol record stream.
const SYMBOLS: StreamId = StreamId(7);
/// The global symbol index stream.
const GLOBALS: StreamId = StreamId(8);
/// The section header stream.
const SECTIONS: StreamId = StreamId(9);
/// The OMAP stream from the image to the source image.
const OMAP: StreamId = StreamId(10);
/// The kind of a user-defined type symbol (`S_UDT`).
const S_UDT: u16 = 0x1108;
/// The kind of an argument list type record (`LF_ARGLIST`).
const LF_ARGLIST: u16 = 0x1201;
/// The version of the compressed-bucket hash header.
const HASH_VERSION: u32 = 0xf12f_091a;
/// The name of the symbol that crosses a page boundary.
const TARGET_SYMBOL: &[u8] = b"symbol_that_crosses_the_page_boundary_xx";
/// The name of the module record that crosses a page boundary.
const TARGET_MODULE: &[u8] = b"module_that_crosses_a_page.obj";

/// The page order of the stream data in a fixture container.
#[derive(Clone, Copy, Debug)]
enum Order {
    /// The pages of each stream are adjacent and in ascending order.
    Contiguous,
    /// No two consecutive pages of a stream are adjacent in the file.
    Scattered,
}

fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap()
}

fn to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap()
}

fn put(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn u32s(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// Return `true` if the range `start..start + length` crosses a page boundary.
fn crosses_page(start: usize, length: usize) -> bool {
    start / PAGE != (start + length - 1) / PAGE
}

/// Write an MSF 7.00 container with 512-byte pages and a one-page page map.
fn container(streams: &[Option<Vec<u8>>], order: Order) -> Vec<u8> {
    let counts: Vec<usize> = streams
        .iter()
        .map(|stream| stream.as_ref().map_or(0, |data| data.len().div_ceil(PAGE)))
        .collect();
    let total: usize = counts.iter().sum();
    let directory_size = 4 * (1 + streams.len() + total);
    let directory_pages = directory_size.div_ceil(PAGE);
    assert!(directory_pages * 4 <= PAGE, "the page map has one page");
    let first_data = FIRST_DIRECTORY_PAGE + directory_pages;
    let page_count = first_data + total;
    assert!(
        page_count < PAGE + 1,
        "no free page map page of a later interval"
    );
    let mut pool: Vec<u32> = (first_data..page_count).map(to_u32).collect();
    if let Order::Scattered = order {
        let even = pool.iter().step_by(2);
        let odd = pool.iter().skip(1).step_by(2);
        pool = even.chain(odd).copied().collect();
    }
    let mut pool = pool.into_iter();
    let pages: Vec<Vec<u32>> = counts
        .iter()
        .map(|&count| pool.by_ref().take(count).collect())
        .collect();
    if let Order::Scattered = order {
        for list in &pages {
            assert!(list.windows(2).all(|pair| pair[1] != pair[0] + 1));
        }
    }

    let mut bytes = vec![0; page_count * PAGE];
    bytes[..32].copy_from_slice(b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0");
    for (offset, value) in [
        (32, to_u32(PAGE)),
        (36, 1),
        (40, to_u32(page_count)),
        (44, to_u32(directory_size)),
        (52, 3),
    ] {
        put(&mut bytes, offset, value);
    }
    for index in 0..directory_pages {
        put(
            &mut bytes,
            3 * PAGE + 4 * index,
            to_u32(FIRST_DIRECTORY_PAGE + index),
        );
    }
    let sizes = streams
        .iter()
        .map(|stream| stream.as_ref().map_or(u32::MAX, |data| to_u32(data.len())));
    let words: Vec<u32> = core::iter::once(to_u32(streams.len()))
        .chain(sizes)
        .chain(pages.iter().flatten().copied())
        .collect();
    let directory = FIRST_DIRECTORY_PAGE * PAGE;
    bytes[directory..directory + directory_size].copy_from_slice(&u32s(&words));
    for (list, stream) in pages.iter().zip(streams) {
        let data = stream.as_deref().unwrap_or_default();
        for (&page, chunk) in list.iter().zip(data.chunks(PAGE)) {
            let start = usize::try_from(page).unwrap() * PAGE;
            bytes[start..start + chunk.len()].copy_from_slice(chunk);
        }
    }
    bytes
}

/// Build a PDB Info stream with age 1 and the given named streams.
fn info(named: &[(&[u8], StreamId)]) -> Vec<u8> {
    let mut builder = InfoBuilder::new(Producer::Microsoft, 0x1234_5678, [0x42; 16], 1).unwrap();
    for &(name, stream) in named {
        builder.set_named_stream(name, stream).unwrap();
    }
    let mut bytes = Vec::new();
    builder.encode(&mut bytes).unwrap();
    bytes
}

/// Build an `LF_ARGLIST` type record with `count` arguments `0x74`, `0x75`, ...
fn arglist(count: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&u16::try_from(6 + 4 * count).unwrap().to_le_bytes());
    bytes.extend_from_slice(&LF_ARGLIST.to_le_bytes());
    bytes.extend_from_slice(&count.to_le_bytes());
    for index in 0..count {
        bytes.extend_from_slice(&(0x74 + index).to_le_bytes());
    }
    bytes
}

/// The record offset (relative to the record bytes) of the type record that crosses a page.
const TARGET_TYPE_OFFSET: usize = 444;
/// The argument count of the type record that crosses a page.
const TARGET_TYPE_ARGUMENTS: u32 = 8;

/// Build a TPI stream of three `LF_ARGLIST` records. The second record crosses offset 512.
///
/// `record_bytes` is the record size that the header declares. The function
/// writes only that number of record bytes.
fn type_stream(record_bytes: Option<usize>) -> Vec<u8> {
    let records = [arglist(109), arglist(TARGET_TYPE_ARGUMENTS), arglist(2)].concat();
    assert!(crosses_page(HEADER_SIZE + TARGET_TYPE_OFFSET, 40));
    let length = record_bytes.unwrap_or(records.len());
    let count = if length == records.len() { 3 } else { 2 };
    let mut bytes = u32s(&[
        TPI_V80,
        to_u32(HEADER_SIZE),
        0x1000,
        0x1000 + count,
        to_u32(length),
    ]);
    bytes.extend_from_slice(&[0xff; 4]);
    bytes.extend_from_slice(&u32s(&[4, 0x3_ffff, 0, u32::MAX, 0, u32::MAX, 0, u32::MAX]));
    assert_eq!(bytes.len(), HEADER_SIZE);
    bytes.extend_from_slice(&records[..length]);
    bytes
}

/// Build an `S_UDT` symbol record with zero padding to a multiple of 4.
fn udt(name: &[u8]) -> Vec<u8> {
    let mut body = S_UDT.to_le_bytes().to_vec();
    body.extend_from_slice(&0x1000_u32.to_le_bytes());
    body.extend_from_slice(name);
    body.push(0);
    while !(body.len() + 2).is_multiple_of(4) {
        body.push(0);
    }
    let mut record = u16::try_from(body.len()).unwrap().to_le_bytes().to_vec();
    record.extend_from_slice(&body);
    record
}

/// Return the name of filler symbol `index`. Its `S_UDT` record has 32 bytes.
fn filler_name(index: usize) -> Vec<u8> {
    format!("filler_symbol_number_{index:02}").into_bytes()
}

/// Build symbol records: 15 fillers, the target symbol and `after` more fillers.
///
/// The target record starts at offset 480. Return the records and the name of each record.
fn symbol_records(after: usize) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut names: Vec<Vec<u8>> = (0..15).map(filler_name).collect();
    names.push(TARGET_SYMBOL.to_vec());
    names.extend((15..15 + after).map(filler_name));
    let bytes = names.iter().flat_map(|name| udt(name)).collect();
    (bytes, names)
}

/// The offset of the target symbol record in the symbol records.
const TARGET_SYMBOL_OFFSET: usize = 480;
/// The size of the target symbol record.
const TARGET_SYMBOL_SIZE: usize = 52;

/// Build a GSI hash table that indexes the records at `offsets` by `names`.
fn global_index(names: &[Vec<u8>], offsets: &[u32]) -> Vec<u8> {
    let count = BucketCount::Standard;
    let mut entries: Vec<(u32, u32)> = names
        .iter()
        .zip(offsets)
        .map(|(name, &offset)| (count.bucket_of(name), offset))
        .collect();
    entries.sort_unstable();
    let words = (usize::try_from(count.count()).unwrap() + 1).div_ceil(32);
    let mut bitmap = vec![0_u8; words * 4];
    let mut starts = Vec::new();
    for (index, &(bucket, _)) in (0_u32..).zip(&entries) {
        if index == 0 || entries[usize::try_from(index - 1).unwrap()].0 != bucket {
            let bucket = usize::try_from(bucket).unwrap();
            bitmap[bucket / 8] |= 1 << (bucket % 8);
            starts.extend_from_slice(&(index * 12).to_le_bytes());
        }
    }
    let mut bytes = u32s(&[
        u32::MAX,
        HASH_VERSION,
        to_u32(entries.len() * 8),
        to_u32(bitmap.len() + starts.len()),
    ]);
    for &(_, offset) in &entries {
        bytes.extend_from_slice(&u32s(&[offset + 1, 1]));
    }
    bytes.extend_from_slice(&bitmap);
    bytes.extend_from_slice(&starts);
    bytes
}

/// Serialize one C13 subsection with its header and zero padding.
fn subsection(kind: SubsectionKind, data: &[u8]) -> Vec<u8> {
    let mut bytes = u32s(&[kind.0, to_u32(data.len())]);
    bytes.extend_from_slice(data);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    bytes
}

/// Serialize one file checksum entry with zero padding.
fn checksum(name_offset: u32, kind: u8, bytes: &[u8]) -> Vec<u8> {
    let mut entry = name_offset.to_le_bytes().to_vec();
    entry.push(u8::try_from(bytes.len()).unwrap());
    entry.push(kind);
    entry.extend_from_slice(bytes);
    while !entry.len().is_multiple_of(4) {
        entry.push(0);
    }
    entry
}

/// The offset of the SHA-256 entry that crosses a page, in the checksum subsection data.
const TARGET_CHECKSUM_OFFSET: u32 = 24 + 11 * 40;

/// Return the SHA-256 bytes of checksum entry `index`.
fn sha256(index: u8) -> [u8; 32] {
    core::array::from_fn(|byte| {
        index
            .wrapping_mul(32)
            .wrapping_add(u8::try_from(byte).unwrap())
    })
}

/// Build the C13 line region: a checksum subsection and a lines subsection.
///
/// One MD5 entry and 12 SHA-256 entries make the checksum subsection data.
fn c13() -> Vec<u8> {
    let mut checksums = checksum(1, 1, &[0x11; 16]);
    for index in 0..12 {
        checksums.extend_from_slice(&checksum(1 + u32::from(index), 3, &sha256(index)));
    }
    let mut lines = u32s(&[0, 1, 8]);
    lines.extend_from_slice(&u32s(&[TARGET_CHECKSUM_OFFSET, 1, 20, 0, 1]));
    [
        subsection(DEBUG_S_FILECHKSMS, &checksums),
        subsection(DEBUG_S_LINES, &lines),
    ]
    .concat()
}

/// The offset of the symbol region end and the C13 region start in the module stream.
const MODULE_SYMBOL_BYTES: usize = 4 + TARGET_SYMBOL_OFFSET + TARGET_SYMBOL_SIZE;

/// Build the module stream and return it with its region sizes.
///
/// The target symbol record crosses offset 512. The target checksum entry crosses offset 1024.
fn module_stream() -> (Vec<u8>, [u32; 3]) {
    let (symbols, _) = symbol_records(0);
    let mut bytes = 4_u32.to_le_bytes().to_vec();
    bytes.extend_from_slice(&symbols);
    assert_eq!(bytes.len(), MODULE_SYMBOL_BYTES);
    assert!(crosses_page(4 + TARGET_SYMBOL_OFFSET, TARGET_SYMBOL_SIZE));
    let c13 = c13();
    let entry = MODULE_SYMBOL_BYTES + 8 + usize::try_from(TARGET_CHECKSUM_OFFSET).unwrap();
    assert!(crosses_page(entry, 40));
    bytes.extend_from_slice(&c13);
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    (bytes, [to_u32(MODULE_SYMBOL_BYTES), 0, to_u32(c13.len())])
}

/// Build one padded module info record.
fn module(stream: u16, sizes: [u32; 3], name: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[34..36].copy_from_slice(&stream.to_le_bytes());
    for (offset, size) in [36, 40, 44].into_iter().zip(sizes) {
        put(&mut bytes, offset, size);
    }
    for _ in 0..2 {
        bytes.extend_from_slice(name);
        bytes.push(0);
    }
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0xcc);
    }
    bytes
}

/// The offset of the target module record in the DBI stream.
const TARGET_MODULE_OFFSET: usize = 64 + 4 * 92;

/// Build a DBI stream with six module records and an optional debug header.
///
/// The fifth module record crosses offset 512. The debug header names
/// [`SECTIONS`] and [`OMAP`].
///
/// `module_info` is the module info size that the header declares. The function
/// writes only that number of module info bytes.
fn dbi_stream(module_sizes: [u32; 3], module_info: Option<usize>) -> Vec<u8> {
    let filler = |index: usize| module(0xffff, [0; 3], format!("filler_{index:02}.obj").as_bytes());
    let mut info: Vec<u8> = (0..4).flat_map(filler).collect();
    info.extend_from_slice(&module(
        u16::try_from(MODULE.0).unwrap(),
        module_sizes,
        TARGET_MODULE,
    ));
    info.extend_from_slice(&filler(4));
    assert!(crosses_page(TARGET_MODULE_OFFSET, 128));
    let length = module_info.unwrap_or(info.len());
    let mut bytes = vec![0; 64];
    put(&mut bytes, 0, u32::MAX);
    put(&mut bytes, 4, 19_990_903);
    put(&mut bytes, 8, 1);
    bytes[12..14].copy_from_slice(&u16::try_from(GLOBALS.0).unwrap().to_le_bytes());
    bytes[16..18].copy_from_slice(&0xffff_u16.to_le_bytes());
    bytes[20..22].copy_from_slice(&u16::try_from(SYMBOLS.0).unwrap().to_le_bytes());
    put(&mut bytes, 24, to_u32(length));
    put(&mut bytes, 48, 22);
    bytes[58..60].copy_from_slice(&0x8664_u16.to_le_bytes());
    bytes.extend_from_slice(&info[..length]);
    let mut debug = [0xffff_u16; 11];
    debug[3] = u16::try_from(OMAP.0).unwrap();
    debug[5] = u16::try_from(SECTIONS.0).unwrap();
    bytes.extend(debug.iter().flat_map(|slot| slot.to_le_bytes()));
    bytes
}

/// The index of the section header that crosses offset 512.
const TARGET_SECTION: usize = 12;

/// Return section header `index`.
fn section(index: usize) -> SectionHeader {
    let mut name = [0; 8];
    name[..7].copy_from_slice(format!(".sect{index:02}").as_bytes());
    SectionHeader::new(name, 0x800, to_u32(0x1000 * (index + 1)))
}

/// Build a section header stream with 15 headers.
fn sections() -> Vec<u8> {
    assert!(crosses_page(TARGET_SECTION * 40, 40));
    let headers: Vec<SectionHeader> = (0..15).map(section).collect();
    SectionHeaderStream::from_records(&headers)
        .unwrap()
        .to_bytes()
        .unwrap()
}

/// The number of OMAP records. The records of index 64 and later are on the second page.
const OMAP_RECORDS: u32 = 70;

/// Return OMAP record `index`.
fn omap_record(index: u32) -> OmapRecord {
    let mut record = OmapRecord::default();
    record.rva = 0x1000 + 0x10 * index;
    record.rva_to = 0x8000 + 0x10 * index;
    record
}

/// Build an OMAP stream that is longer than one page.
fn omap() -> Vec<u8> {
    let records: Vec<OmapRecord> = (0..OMAP_RECORDS).map(omap_record).collect();
    OmapStream::from_records(&records)
        .unwrap()
        .to_bytes()
        .unwrap()
}

/// Return the name of `/names` string `index`. Each name has 30 bytes and a terminator.
fn string_name(index: usize) -> Vec<u8> {
    format!("C:\\src\\directory\\file_{index:03}.cpp").into_bytes()
}

/// The number of names in the `/names` stream.
const STRING_COUNT: usize = 20;
/// The bucket count of the `/names` stream.
const STRING_BUCKETS: usize = 31;

/// Build a `/names` stream with version 1 hashes. Return it and the offset of each name.
fn names_stream() -> (Vec<u8>, Vec<u32>) {
    let mut strings = vec![0];
    let mut buckets = vec![0_u32; STRING_BUCKETS];
    let mut offsets = Vec::new();
    for index in 0..STRING_COUNT {
        let name = string_name(index);
        let offset = to_u32(strings.len());
        strings.extend_from_slice(&name);
        strings.push(0);
        let mut slot = usize::try_from(hash_string_v1(&name)).unwrap() % STRING_BUCKETS;
        while buckets[slot] != 0 {
            slot = (slot + 1) % STRING_BUCKETS;
        }
        buckets[slot] = offset;
        offsets.push(offset);
    }
    let mut bytes = u32s(&[SIGNATURE, 1, to_u32(strings.len())]);
    bytes.extend_from_slice(&strings);
    bytes.extend_from_slice(&u32s(&[to_u32(STRING_BUCKETS)]));
    bytes.extend_from_slice(&u32s(&buckets));
    bytes.extend_from_slice(&u32s(&[to_u32(STRING_COUNT)]));
    (bytes, offsets)
}

/// The streams of the fixture PDB, in stream ID order.
struct Streams {
    info: Vec<u8>,
    types: Vec<u8>,
    dbi: Vec<u8>,
    names: Vec<u8>,
    module: Vec<u8>,
    symbols: Vec<u8>,
    globals: Vec<u8>,
    sections: Vec<u8>,
    omap: Vec<u8>,
}

impl Streams {
    /// Build valid streams.
    fn new() -> Self {
        let (module, sizes) = module_stream();
        let (symbols, names) = symbol_records(2);
        let offsets: Vec<u32> = names
            .iter()
            .scan(0, |next, name| {
                let offset = *next;
                *next += udt(name).len();
                Some(to_u32(offset))
            })
            .collect();
        assert_eq!(offsets[15], to_u32(TARGET_SYMBOL_OFFSET));
        assert!(crosses_page(TARGET_SYMBOL_OFFSET, TARGET_SYMBOL_SIZE));
        let globals = global_index(&names, &offsets);
        assert!(globals.len() > PAGE, "the bucket bitmap crosses a page");
        Self {
            info: info(&[(b"/names", NAMES)]),
            types: type_stream(None),
            dbi: dbi_stream(sizes, None),
            names: names_stream().0,
            module,
            symbols,
            globals,
            sections: sections(),
            omap: omap(),
        }
    }

    /// Write the streams to a container with the given page order.
    fn container(self, order: Order) -> Vec<u8> {
        let streams = [
            Some(Vec::new()),
            Some(self.info),
            Some(self.types),
            Some(self.dbi),
            None,
            Some(self.names),
            Some(self.module),
            Some(self.symbols),
            Some(self.globals),
            Some(self.sections),
            Some(self.omap),
        ];
        container(&streams, order)
    }
}

/// Open a container and return the PDB reader.
fn open(bytes: &[u8]) -> Pdb<&[u8]> {
    Pdb::read(bytes).unwrap()
}
