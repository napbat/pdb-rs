//! A synthetic native PDB with each stream role.
//!
//! Streams: 0 (empty), 1 Info, 2 TPI, 3 DBI, 4 IPI, 5 `/names`, 6 TPI hash, 7 GSI, 8 PSI,
//! 9 symbol records, 10 module stream, 11 section headers and 12 `sourcelink`.

use pdbrs::native::Pdb;
use pdbrs::native::hash::hash_string_v1;
use pdbrs::native::msf::{MsfBuilder, MsfWriteOptions};
use pdbrs::native::symbols::BucketCount;
use pdbrs::native::types::{TypeStream, type_record_hash};

use crate::Outcome;

/// A little-endian byte writer.
#[derive(Default)]
struct Bytes(Vec<u8>);

impl Bytes {
    fn u8(&mut self, value: u8) -> &mut Self {
        self.0.push(value);
        self
    }

    fn u16(&mut self, value: u16) -> &mut Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn u32(&mut self, value: u32) -> &mut Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn bytes(&mut self, value: &[u8]) -> &mut Self {
        self.0.extend_from_slice(value);
        self
    }

    /// Write a NUL-terminated name.
    fn name(&mut self, value: &[u8]) -> &mut Self {
        self.bytes(value).u8(0)
    }

    /// Pad with `LF_PAD` bytes to a multiple of 4.
    fn pad(&mut self) -> &mut Self {
        while !self.0.len().is_multiple_of(4) {
            let left = 4 - self.0.len() % 4;
            self.0.push(0xf0 + u8::try_from(left).unwrap_or(0));
        }
        self
    }

    /// Pad with zero bytes to a multiple of 4.
    fn zero_pad(&mut self) -> &mut Self {
        while !self.0.len().is_multiple_of(4) {
            self.0.push(0);
        }
        self
    }

    fn len(&self) -> u32 {
        u32::try_from(self.0.len()).unwrap_or(u32::MAX)
    }
}

/// Frame a type or ID record: the length, the kind, the payload and `LF_PAD` padding.
fn leaf(kind: u16, payload: &[u8]) -> Vec<u8> {
    let mut body = Bytes::default();
    body.u16(0).u16(kind).bytes(payload).pad();
    let length = u16::try_from(body.0.len() - 2).unwrap_or(0);
    body.0[..2].copy_from_slice(&length.to_le_bytes());
    body.0
}

/// Frame a symbol record with zero padding.
fn symbol(kind: u16, payload: &[u8]) -> Vec<u8> {
    let mut body = Bytes::default();
    body.u16(0).u16(kind).bytes(payload).zero_pad();
    let length = u16::try_from(body.0.len() - 2).unwrap_or(0);
    body.0[..2].copy_from_slice(&length.to_le_bytes());
    body.0
}

/// Serialize a TPI or IPI header for `count` records, with an optional hash stream.
fn type_stream(records: &[Vec<u8>], hash: Option<(u16, u32)>) -> Vec<u8> {
    let bytes = records.concat();
    let count = u32::try_from(records.len()).unwrap_or(0);
    let mut out = Bytes::default();
    out.u32(20_040_203).u32(56).u32(0x1000).u32(0x1000 + count);
    out.u32(u32::try_from(bytes.len()).unwrap_or(0));
    let (stream, values) = hash.unwrap_or((0xffff, 0));
    out.u16(stream).u16(0xffff).u32(4).u32(0x3_ffff);
    if hash.is_some() {
        out.u32(0)
            .u32(values)
            .u32(values)
            .u32(8)
            .u32(values + 8)
            .u32(0);
    } else {
        out.u32(0)
            .u32(u32::MAX)
            .u32(0)
            .u32(u32::MAX)
            .u32(0)
            .u32(u32::MAX);
    }
    out.bytes(&bytes);
    out.0
}

/// The TPI records, from index 0x1000.
fn type_records() -> Vec<Vec<u8>> {
    let mut members = Bytes::default();
    for (value, name) in [(0_u16, &b"Red"[..]), (1, b"Green")] {
        members.u16(0x1502).u16(3).u16(value).name(name).pad();
    }
    let mut fields = Bytes::default();
    for (offset, name) in [(0_u16, &b"x"[..]), (4, b"y")] {
        fields
            .u16(0x150d)
            .u16(3)
            .u32(0x74)
            .u16(offset)
            .name(name)
            .pad();
    }
    let mut point = Bytes::default();
    point
        .u16(2)
        .u16(0)
        .u32(0x1004)
        .u32(0)
        .u32(0)
        .u16(8)
        .name(b"Point");
    let mut forward = Bytes::default();
    forward
        .u16(0)
        .u16(0x80)
        .u32(0)
        .u32(0)
        .u32(0)
        .u16(0)
        .name(b"Point");
    vec![
        leaf(0x1201, &Bytes::default().u32(1).u32(0x74).0),
        leaf(
            0x1008,
            &Bytes::default().u32(0x74).u8(0).u8(0).u16(1).u32(0x1000).0,
        ),
        leaf(0x1203, &members.0),
        leaf(
            0x1507,
            &Bytes::default()
                .u16(2)
                .u16(0)
                .u32(0x74)
                .u32(0x1002)
                .name(b"Color")
                .0,
        ),
        leaf(0x1203, &fields.0),
        leaf(0x1505, &point.0),
        leaf(0x1002, &Bytes::default().u32(0x1005).u32(0x0001_000c).0),
        leaf(0x1001, &Bytes::default().u32(0x1005).u16(1).0),
        leaf(0x1505, &forward.0),
    ]
}

/// The IPI records, from index 0x1000.
fn id_records() -> Vec<Vec<u8>> {
    vec![
        leaf(0x1601, &Bytes::default().u32(0).u32(0x1001).name(b"main").0),
        leaf(0x1605, &Bytes::default().u32(0).name(b"C:\\src").0),
        leaf(0x1605, &Bytes::default().u32(0).name(b"cl.exe").0),
        leaf(0x1605, &Bytes::default().u32(0).name(b"main.c").0),
        leaf(
            0x1603,
            &Bytes::default()
                .u16(5)
                .u32(0x1001)
                .u32(0x1002)
                .u32(0x1003)
                .u32(0)
                .u32(0)
                .0,
        ),
        leaf(0x1606, &Bytes::default().u32(0x1005).u32(0x1003).u32(3).0),
    ]
}

/// Build the TPI hash stream: one hash value per record and one index offset pair.
fn type_hashes(tpi: &[u8]) -> Outcome<Vec<u8>> {
    let stream = TypeStream::parse(tpi.to_vec())?;
    let mut out = Bytes::default();
    for record in stream.records() {
        let record = record?;
        out.u32(type_record_hash(stream.full_record(&record.record)?)? % 0x3_ffff);
    }
    out.u32(0x1000).u32(0);
    Ok(out.0)
}

/// Build a `/names` string table and return it with the offset of each name.
fn names(list: &[&[u8]]) -> (Vec<u8>, Vec<u32>) {
    let mut strings = vec![0];
    let mut offsets = Vec::new();
    let count = list.len() * 2 + 1;
    let mut buckets = vec![0_u32; count];
    for name in list {
        let offset = u32::try_from(strings.len()).unwrap_or(0);
        strings.extend_from_slice(name);
        strings.push(0);
        let mut slot = usize::try_from(hash_string_v1(name)).unwrap_or(0) % count;
        while buckets[slot] != 0 {
            slot = (slot + 1) % count;
        }
        buckets[slot] = offset;
        offsets.push(offset);
    }
    let mut out = Bytes::default();
    out.u32(0xeffe_effe)
        .u32(1)
        .u32(u32::try_from(strings.len()).unwrap_or(0));
    out.bytes(&strings).u32(u32::try_from(count).unwrap_or(0));
    for bucket in buckets {
        out.u32(bucket);
    }
    out.u32(u32::try_from(list.len()).unwrap_or(0));
    (out.0, offsets)
}

/// Build the Info stream with a named stream map of `(name, stream)` pairs.
fn info(named: &[(&[u8], u32)]) -> Vec<u8> {
    let mut strings = Vec::new();
    let capacity = u32::try_from(named.len() * 2).unwrap_or(2);
    let mut buckets: Vec<Option<(u32, u32)>> = vec![None; usize::try_from(capacity).unwrap_or(0)];
    for (name, stream) in named {
        let key = u32::try_from(strings.len()).unwrap_or(0);
        strings.extend_from_slice(name);
        strings.push(0);
        let mut slot = (hash_string_v1(name) & 0xffff) % capacity;
        while buckets[usize::try_from(slot).unwrap_or(0)].is_some() {
            slot = (slot + 1) % capacity;
        }
        buckets[usize::try_from(slot).unwrap_or(0)] = Some((key, *stream));
    }
    let present = buckets
        .iter()
        .enumerate()
        .filter(|(_, bucket)| bucket.is_some())
        .fold(0_u32, |bits, (index, _)| bits | (1 << index));
    let mut out = Bytes::default();
    out.u32(20_000_404)
        .u32(0x6000_0000)
        .u32(1)
        .bytes(&[0x42; 16]);
    out.u32(u32::try_from(strings.len()).unwrap_or(0))
        .bytes(&strings);
    out.u32(u32::try_from(named.len()).unwrap_or(0))
        .u32(capacity);
    out.u32(1).u32(present).u32(0);
    for (key, stream) in buckets.into_iter().flatten() {
        out.u32(key).u32(stream);
    }
    out.u32(u32::try_from(named.len() + 1).unwrap_or(0))
        .u32(20_140_508);
    out.0
}

/// The module symbols, after the signature, and the offset of the procedure record.
fn module_symbols() -> (Vec<u8>, u32) {
    let objname = symbol(
        0x1101,
        &Bytes::default().u32(0).name(b"C:\\src\\main.obj").0,
    );
    let mut compile = Bytes::default();
    compile
        .u32(0)
        .u16(0xd0)
        .u16(19)
        .u16(0)
        .u16(0)
        .u16(0)
        .u16(19)
        .u16(0)
        .u16(0)
        .u16(0);
    compile.name(b"Microsoft (R) Optimizing Compiler");
    let compile = symbol(0x113c, &compile.0);
    let build = symbol(0x114c, &Bytes::default().u32(0x1004).0);
    let frame = symbol(
        0x1012,
        &Bytes::default()
            .u32(0x28)
            .u32(0)
            .u32(0)
            .u32(0)
            .u32(0)
            .u16(0)
            .u32(0)
            .0,
    );
    let local = symbol(
        0x1111,
        &Bytes::default()
            .u32(0x30)
            .u32(0x74)
            .u16(335)
            .name(b"argc")
            .0,
    );
    let procedure_offset = 4 + objname.len() + compile.len() + build.len();
    let mut procedure = Bytes::default();
    procedure
        .u32(0)
        .u32(0)
        .u32(0)
        .u32(0x20)
        .u32(4)
        .u32(0x1c)
        .u32(0x1000)
        .u32(0)
        .u16(1)
        .u8(0);
    procedure.name(b"main");
    let procedure_len = symbol(0x1147, &procedure.0).len();
    let end = procedure_offset + procedure_len + frame.len() + local.len();
    procedure.0[4..8].copy_from_slice(&u32::try_from(end).unwrap_or(0).to_le_bytes());
    let procedure = symbol(0x1147, &procedure.0);
    let close = symbol(0x114f, &[]);
    let records = [objname, compile, build, procedure, frame, local, close].concat();
    (records, u32::try_from(procedure_offset).unwrap_or(0))
}

/// The C13 line region: file checksums and the lines of `main`.
fn c13_lines(file_name: u32) -> Vec<u8> {
    let mut checksums = Bytes::default();
    checksums
        .u32(file_name)
        .u8(16)
        .u8(1)
        .bytes(&[0x11; 16])
        .zero_pad();
    let mut lines = Bytes::default();
    lines.u32(0).u16(1).u16(0).u32(0x20);
    lines.u32(0).u32(2).u32(12 + 16);
    lines.u32(0).u32(0x8000_0003).u32(0x10).u32(0x8000_0004);
    let mut out = Bytes::default();
    for (kind, data) in [(0xf4_u32, checksums.0), (0xf2, lines.0)] {
        out.u32(kind)
            .u32(u32::try_from(data.len()).unwrap_or(0))
            .bytes(&data)
            .zero_pad();
    }
    out.0
}

/// Build a symbol hash table (GSI layout) over records at `offsets` with `names`.
fn hash_table(entries: &[(&[u8], u32)]) -> Vec<u8> {
    let count = BucketCount::Standard;
    let mut sorted: Vec<(u32, u32)> = entries
        .iter()
        .map(|(name, offset)| (count.bucket_of(name), *offset))
        .collect();
    sorted.sort_unstable();
    let words = usize::try_from(count.count() + 1).unwrap_or(0).div_ceil(32);
    let mut bitmap = vec![0_u8; words * 4];
    let mut starts = Bytes::default();
    let mut previous = None;
    for (index, &(bucket, _)) in (0_u32..).zip(&sorted) {
        if previous != Some(bucket) {
            let bucket_index = usize::try_from(bucket).unwrap_or(0);
            bitmap[bucket_index / 8] |= 1 << (bucket_index % 8);
            starts.u32(index * 12);
            previous = Some(bucket);
        }
    }
    let mut out = Bytes::default();
    out.u32(u32::MAX)
        .u32(0xf12f_091a)
        .u32(u32::try_from(sorted.len() * 8).unwrap_or(0));
    out.u32(u32::try_from(bitmap.len()).unwrap_or(0) + starts.len());
    for (_, offset) in &sorted {
        out.u32(offset + 1).u32(1);
    }
    out.bytes(&bitmap).bytes(&starts.0);
    out.0
}

/// The shared symbol records, the GSI and the PSI.
fn shared_symbols(procedure: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let public = symbol(
        0x110e,
        &Bytes::default().u32(2).u32(0).u16(1).name(b"main").0,
    );
    let reference = symbol(
        0x1125,
        &Bytes::default()
            .u32(0)
            .u32(procedure)
            .u16(1)
            .name(b"main")
            .0,
    );
    let data = symbol(
        0x110d,
        &Bytes::default()
            .u32(0x74)
            .u32(0x10)
            .u16(2)
            .name(b"counter")
            .0,
    );
    let udt = symbol(0x1108, &Bytes::default().u32(0x1005).name(b"Point").0);
    let constant = symbol(0x1107, &Bytes::default().u32(0x74).u16(5).name(b"FIVE").0);
    let mut offsets = Vec::new();
    let mut records = Vec::new();
    for record in [&public, &reference, &data, &udt, &constant] {
        offsets.push(u32::try_from(records.len()).unwrap_or(0));
        records.extend_from_slice(record);
    }
    let globals = hash_table(&[
        (b"main", offsets[1]),
        (b"counter", offsets[2]),
        (b"Point", offsets[3]),
        (b"FIVE", offsets[4]),
    ]);
    let table = hash_table(&[(b"main", offsets[0])]);
    let mut publics = Bytes::default();
    publics
        .u32(u32::try_from(table.len()).unwrap_or(0))
        .u32(4)
        .u32(0)
        .u32(0);
    publics
        .u16(0)
        .u16(0)
        .u32(0)
        .u32(0)
        .bytes(&table)
        .u32(offsets[0]);
    (records, globals, publics.0)
}

/// The DBI stream for one module with a stream and one module without a stream.
fn debug_info(sizes: [u32; 3]) -> Vec<u8> {
    let mut modules = Bytes::default();
    for (stream, sizes, name) in [
        (10_u16, sizes, &b"C:\\src\\main.obj"[..]),
        (0xffff, [0; 3], b"* Linker *"),
    ] {
        let start = modules.0.len();
        modules
            .u32(0)
            .u16(1)
            .u16(0)
            .u32(0)
            .u32(0x20)
            .u32(0x6000_0020)
            .u16(0)
            .u16(0);
        modules.u32(0).u32(0).u16(0).u16(stream);
        modules
            .u32(sizes[0])
            .u32(sizes[1])
            .u32(sizes[2])
            .u16(u16::from(stream != 0xffff));
        modules.u16(0).u32(0).u32(0).u32(0);
        debug_assert_eq!(modules.0.len() - start, 64);
        modules.name(name).name(name).zero_pad();
    }
    let mut contributions = Bytes::default();
    contributions.u32(0xeffe_0000 + 19_970_605);
    contributions
        .u16(1)
        .u16(0)
        .u32(0)
        .u32(0x20)
        .u32(0x6000_0020)
        .u16(0)
        .u16(0)
        .u32(0)
        .u32(0);
    let mut map = Bytes::default();
    map.u16(2).u16(2);
    map.u16(0x010d)
        .u16(0)
        .u16(0)
        .u16(1)
        .u16(0xffff)
        .u16(0xffff)
        .u32(0)
        .u32(0x1000);
    map.u16(0x0208)
        .u16(0)
        .u16(0)
        .u16(2)
        .u16(0xffff)
        .u16(0xffff)
        .u32(0)
        .u32(u32::MAX);
    let mut sources = Bytes::default();
    sources
        .u16(2)
        .u16(1)
        .u16(0)
        .u16(1)
        .u16(1)
        .u16(0)
        .u32(0)
        .name(b"main.c")
        .zero_pad();
    let mut debug = Bytes::default();
    for slot in 0..11 {
        debug.u16(if slot == 5 { 11 } else { 0xffff });
    }
    let parts = [
        modules.0,
        contributions.0,
        map.0,
        sources.0,
        Vec::new(),
        Vec::new(),
        debug.0,
    ];
    let mut out = Bytes::default();
    out.u32(u32::MAX)
        .u32(19_990_903)
        .u32(1)
        .u16(7)
        .u16(0x8e00)
        .u16(8)
        .u16(0)
        .u16(9)
        .u16(0);
    for part in &parts[..5] {
        out.u32(u32::try_from(part.len()).unwrap_or(0));
    }
    out.u32(0)
        .u32(u32::try_from(parts[6].len()).unwrap_or(0))
        .u32(0);
    out.u16(0).u16(0x8664).u32(0);
    for (index, part) in parts.iter().enumerate() {
        if index != 5 {
            out.bytes(part);
        }
    }
    out.0
}

/// Two section headers: `.text` and `.data`.
fn section_headers() -> Vec<u8> {
    let mut out = Bytes::default();
    for (name, address, characteristics) in [
        (b".text\0\0\0", 0x1000, 0x6000_0020),
        (b".data\0\0\0", 0x2000, 0xc000_0040),
    ] {
        out.bytes(name)
            .u32(0x1000)
            .u32(address)
            .u32(0x200)
            .u32(address / 8);
        out.u32(0).u32(0).u16(0).u16(0).u32(characteristics);
    }
    out.0
}

/// Build the synthetic native PDB and check that it reads back.
pub fn native_pdb() -> Outcome<Vec<u8>> {
    let tpi = type_stream(&type_records(), Some((6, 4 * 9)));
    let hashes = type_hashes(&tpi)?;
    let ipi = type_stream(&id_records(), None);
    let (names, offsets) = names(&[b"main.c", b"C:\\src\\main.c"]);
    let (symbols, procedure) = module_symbols();
    let c13 = c13_lines(offsets[1]);
    let mut module = Bytes::default();
    module.u32(4).bytes(&symbols).bytes(&c13).u32(0);
    let sizes = [
        4 + u32::try_from(symbols.len())?,
        0,
        u32::try_from(c13.len())?,
    ];
    let (records, globals, publics) = shared_symbols(procedure);
    let source_link = br#"{"documents":{"C:\\src\\*":"https://example.com/src/*"}}"#;
    let streams: [&[u8]; 13] = [
        b"",
        &info(&[(b"/names", 5), (b"sourcelink", 12)]),
        &tpi,
        &debug_info(sizes),
        &ipi,
        &names,
        &hashes,
        &globals,
        &publics,
        &records,
        &module.0,
        &section_headers(),
        source_link,
    ];
    let mut builder = MsfBuilder::new();
    for stream in streams {
        builder.push(stream)?;
    }
    let bytes = builder.plan(MsfWriteOptions::default())?.to_vec()?;
    let pdb = Pdb::parse(&bytes)?;
    pdb.types()?.ok_or("synthetic PDB has no TPI")?;
    pdb.ids()?.ok_or("synthetic PDB has no IPI")?;
    let dbi = pdb.debug_info()?.ok_or("synthetic PDB has no DBI")?;
    for module in dbi.modules() {
        pdb.module(module?.0)?;
    }
    for (what, result) in [
        ("GSI", pdb.globals().map(|_| ())),
        ("PSI", pdb.publics().map(|_| ())),
    ] {
        if let Err(error) = result {
            println!("synthetic native PDB: {what} does not read: {error}");
        }
    }
    Ok(bytes)
}
