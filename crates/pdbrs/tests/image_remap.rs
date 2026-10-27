//! Tests for `ImageRemap`: range validation, OMAP table construction and composition.

use pdbrs::ErrorKind;
use pdbrs::native::address::{AddressTables, PdbRva, Rva, Translation, Unmapped};
use pdbrs::native::debug_stream::{OmapRecord, OmapStream};
use pdbrs::native::msf::StreamId;
use pdbrs::native::section::{SectionHeader, SectionHeaderStream};
use pdbrs::native::{ImageRemap, RemapRange};

/// One range of the test model: old RVA, length, new RVA.
type Range = (u32, u32, u32);

/// A deterministic xorshift generator.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Return a value in `low ..= high`.
    fn between(&mut self, low: u32, high: u32) -> u32 {
        let span = u64::from(high - low) + 1;
        low + u32::try_from(self.next() % span).unwrap()
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for index in (1..items.len()).rev() {
            let other = usize::try_from(self.next() % (index as u64 + 1)).unwrap();
            items.swap(index, other);
        }
    }
}

fn section(name: &[u8], virtual_address: u32, virtual_size: u32) -> SectionHeader {
    let mut bytes = [0; 8];
    bytes[..name.len()].copy_from_slice(name);
    SectionHeader::new(bytes, virtual_size, virtual_address)
}

fn headers(sections: &[SectionHeader]) -> SectionHeaderStream {
    SectionHeaderStream::from_records(sections).unwrap()
}

/// The sections of the old image in the tests.
fn old_sections() -> SectionHeaderStream {
    headers(&[
        section(b".text", 0x1000, 0x4000),
        section(b".rdata", 0x5000, 0x1000),
    ])
}

/// The sections of the new image in the tests.
fn new_sections() -> SectionHeaderStream {
    headers(&[
        section(b".text", 0x1000, 0x2000),
        section(b".text2", 0x8000, 0x3000),
        section(b".rdata", 0x10000, 0x1000),
    ])
}

fn remap(ranges: &[Range]) -> ImageRemap {
    ranges.iter().fold(
        ImageRemap::new(new_sections()),
        |remap, &(old, length, new)| remap.with_range(Rva(old), length, Rva(new)).unwrap(),
    )
}

fn record(rva: u32, rva_to: u32) -> OmapRecord {
    let mut record = OmapRecord::default();
    record.rva = rva;
    record.rva_to = rva_to;
    record
}

fn records(table: &OmapStream) -> Vec<(u32, u32)> {
    table
        .iter()
        .map(|record| (record.rva, record.rva_to))
        .collect()
}

/// Return what `ranges` give for the old RVA `old`.
fn forward(ranges: &[Range], old: u32) -> Option<u32> {
    ranges
        .iter()
        .find(|&&(start, length, _)| old >= start && old - start < length)
        .map(|&(start, _, new)| new + (old - start))
}

/// Return the old RVA that `ranges` give for the new RVA `new`.
fn backward(ranges: &[Range], new: u32) -> Option<u32> {
    ranges
        .iter()
        .find(|&&(_, length, start)| new >= start && new - start < length)
        .map(|&(old, _, start)| old + (new - start))
}

/// Return sample addresses for spans `(start, length)`: the starts, inner addresses, last
/// bytes and ends, an address in each gap, addresses before the first and after the last
/// span, and random addresses.
fn samples(spans: impl Iterator<Item = (u32, u32)>, random: &mut Random) -> Vec<u32> {
    let mut spans: Vec<(u32, u32)> = spans.collect();
    spans.sort_unstable();
    let mut out = vec![0, 1, u32::MAX];
    for &(start, length) in &spans {
        out.extend([
            start,
            start + length / 2,
            start + length - 1,
            start + length,
        ]);
        out.push(start.saturating_sub(1));
    }
    if let (Some(first), Some(last)) = (spans.first(), spans.last()) {
        out.extend([first.0 - 1, last.0 + last.1 + 1, last.0 + last.1 + 0x1000]);
        for _ in 0..200 {
            out.push(random.between(first.0.saturating_sub(0x100), last.0 + last.1 + 0x100));
        }
    }
    out
}

fn mapped<T>(translation: Translation<T>) -> Option<T> {
    translation.mapped()
}

/// Make a random range set: `count` old pieces in the old `.text` and `.rdata`, placed in
/// random order into the new sections. Some pieces are not placed (removed code).
fn random_ranges(random: &mut Random, count: usize) -> Vec<Range> {
    let mut pieces = Vec::new();
    let mut cursor = 0x1000 + random.between(0, 0x20);
    while pieces.len() < count && cursor < 0x5f00 {
        let length = random.between(1, 0x60);
        if cursor + length > 0x6000 || (cursor < 0x5000 && cursor + length > 0x5000) {
            cursor = cursor.max(0x5000) + random.between(0, 4);
            continue;
        }
        pieces.push((cursor, length));
        // Adjacent pieces in some cases, gaps in others.
        cursor += length + random.between(0, 3) * random.between(0, 0x10);
    }
    random.shuffle(&mut pieces);
    let targets = [(0x1000, 0x3000), (0x8000, 0xb000), (0x10000, 0x11000)];
    let mut places = targets.map(|(start, _)| start + random.between(0, 0x10));
    let mut ranges = Vec::new();
    for (old, length) in pieces {
        if random.between(0, 9) == 0 {
            continue;
        }
        let slot = usize::try_from(random.between(0, 2)).unwrap();
        let new = places[slot];
        if new + length > targets[slot].1 {
            continue;
        }
        ranges.push((old, length, new));
        places[slot] = new + length + random.between(0, 2) * random.between(0, 0x20);
    }
    ranges
}

#[test]
fn ranges_with_errors_name_the_range() {
    let base = remap(&[(0x1000, 0x100, 0x2000)]);
    let cases = [
        ((0x1200, 0, 0x1000), ErrorKind::InvalidFormat),
        ((0, 0x10, 0x1000), ErrorKind::InvalidFormat),
        ((0x1200, 0x10, 0), ErrorKind::InvalidFormat),
        ((0xffff_fff0, 0x20, 0x1000), ErrorKind::OffsetOverflow),
        // Overlaps with old 0x1000..0x1100: from the left, from the right, inside, around and
        // at the same start.
        ((0x0f80, 0x81, 0x1000), ErrorKind::InvalidFormat),
        ((0x10ff, 0x10, 0x1000), ErrorKind::InvalidFormat),
        ((0x1010, 0x10, 0x1000), ErrorKind::InvalidFormat),
        ((0x0f00, 0x400, 0x1000), ErrorKind::InvalidFormat),
        ((0x1000, 0x10, 0x1000), ErrorKind::InvalidFormat),
        // Overlaps with new 0x2000..0x2100.
        ((0x3000, 0x20, 0x1ff0), ErrorKind::InvalidFormat),
        ((0x3000, 0x20, 0x20f0), ErrorKind::InvalidFormat),
        ((0x3000, 0x20, 0x2000), ErrorKind::InvalidFormat),
        // Outside the new sections: before the first, after the end of a section, across
        // two sections and in no section.
        ((0x3000, 0x20, 0x0f00), ErrorKind::InvalidReference),
        ((0x3000, 0x20, 0x2ff0), ErrorKind::InvalidReference),
        ((0x3000, 0x10, 0x5000), ErrorKind::InvalidReference),
        ((0x3000, 0x20, 0x10ff0), ErrorKind::InvalidReference),
    ];
    for ((old, length, new), kind) in cases {
        let error = base
            .clone()
            .with_range(Rva(old), length, Rva(new))
            .unwrap_err();
        assert_eq!(error.kind(), kind, "range {old:#x} {length:#x} {new:#x}");
        assert_eq!(
            error.remap_range(),
            Some(RemapRange::new(Rva(old), length, Rva(new)))
        );
        let text = error.to_string();
        let named = format!("old RVA {old:#x}, length {length:#x}, new RVA {new:#x}");
        assert!(text.contains(&named), "{text}");
    }
}

#[test]
fn ranges_at_the_limits_are_accepted() {
    // Adjacent old and new ranges, and new ranges that end at the end of a section.
    let remap = remap(&[
        (0x1000, 0x100, 0x2f00),
        (0x1100, 0x100, 0x2e00),
        (0x1200, 0x1, 0x10fff),
        (0xffff_fff0, 0xf, 0x8000),
    ]);
    let ranges: Vec<RemapRange> = remap.ranges().collect();
    assert_eq!(
        ranges,
        [
            RemapRange::new(Rva(0x1000), 0x100, Rva(0x2f00)),
            RemapRange::new(Rva(0x1100), 0x100, Rva(0x2e00)),
            RemapRange::new(Rva(0x1200), 0x1, Rva(0x10fff)),
            RemapRange::new(Rva(0xffff_fff0), 0xf, Rva(0x8000)),
        ]
    );
    assert_eq!(remap.len(), 4);
    assert!(!remap.is_empty());
    assert_eq!(remap.sections().len(), 3);
}

#[test]
fn tables_have_a_record_for_each_range_and_gap() {
    let remap = remap(&[
        (0x1000, 0x20, 0x8000),
        (0x1020, 0x10, 0x1000),
        (0x1040, 0x10, 0x1010),
        (0x5000, 0x100, 0x10000),
    ]);
    let tables = remap
        .address_tables(&AddressTables::new(old_sections()))
        .unwrap();
    assert_eq!(tables.sections, new_sections());
    assert_eq!(tables.original, Some(old_sections()));
    assert_eq!(
        records(tables.from_source.as_ref().unwrap()),
        [
            (0x1000, 0x8000),
            (0x1020, 0x1000),
            (0x1030, 0),
            (0x1040, 0x1010),
            (0x1050, 0),
            (0x5000, 0x10000),
            (0x5100, 0),
        ]
    );
    assert_eq!(
        records(tables.to_source.as_ref().unwrap()),
        [
            (0x1000, 0x1020),
            (0x1010, 0x1040),
            (0x1020, 0),
            (0x8000, 0x1000),
            (0x8020, 0),
            (0x10000, 0x5000),
            (0x10100, 0),
        ]
    );
    let map = tables.map();
    assert_eq!(
        map.pdb_rva_to_rva(PdbRva(0x0fff)).unwrap(),
        Translation::Unmapped(Unmapped::BeforeFirstRecord)
    );
    assert_eq!(
        map.pdb_rva_to_rva(PdbRva(0x1035)).unwrap(),
        Translation::Unmapped(Unmapped::NoTarget(record(0x1030, 0)))
    );
    assert_eq!(
        map.pdb_rva_to_rva(PdbRva(0x5100)).unwrap(),
        Translation::Unmapped(Unmapped::NoTarget(record(0x5100, 0)))
    );
    assert_eq!(
        map.pdb_rva_to_rva(PdbRva(0x5101)).unwrap(),
        Translation::Unmapped(Unmapped::AfterLastRecord(record(0x5100, 0)))
    );
}

#[test]
fn a_map_without_ranges_maps_no_address() {
    let tables = ImageRemap::new(new_sections())
        .address_tables(&AddressTables::new(old_sections()))
        .unwrap();
    assert!(tables.from_source.as_ref().unwrap().is_empty());
    assert!(tables.to_source.as_ref().unwrap().is_empty());
    let map = tables.map();
    assert_eq!(
        map.pdb_rva_to_rva(PdbRva(0x1000)).unwrap(),
        Translation::Unmapped(Unmapped::BeforeFirstRecord)
    );
    assert_eq!(
        map.rva_to_pdb_rva(Rva(0x1000)).unwrap(),
        Translation::Unmapped(Unmapped::BeforeFirstRecord)
    );
}

#[test]
fn random_tables_translate_as_the_ranges_say() {
    for seed in 1..=64_u64 {
        let mut random = Random(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let count = usize::try_from(random.between(1, 80)).unwrap();
        let ranges = random_ranges(&mut random, count);
        let tables = remap(&ranges)
            .address_tables(&AddressTables::new(old_sections()))
            .unwrap();
        let map = tables.map();
        let from_source = tables.from_source.as_ref().unwrap();
        let to_source = tables.to_source.as_ref().unwrap();
        for old in samples(ranges.iter().map(|r| (r.0, r.1)), &mut random) {
            let want = forward(&ranges, old);
            let got = mapped(map.pdb_rva_to_rva(PdbRva(old)).unwrap()).map(|rva| rva.0);
            assert_eq!(got, want, "seed {seed}: PDB RVA {old:#x}");
            assert_eq!(mapped(from_source.lookup(old).unwrap()), want);
        }
        for new in samples(ranges.iter().map(|r| (r.2, r.1)), &mut random) {
            let want = backward(&ranges, new);
            let got = mapped(map.rva_to_pdb_rva(Rva(new)).unwrap()).map(|rva| rva.0);
            assert_eq!(got, want, "seed {seed}: RVA {new:#x}");
            assert_eq!(mapped(to_source.lookup(new).unwrap()), want);
        }
    }
}

#[test]
fn composition_maps_original_addresses_to_the_new_image() {
    for seed in 1..=32_u64 {
        let mut random = Random(seed.wrapping_mul(0xd1b5_4a32_d192_ed03) | 1);
        let first = random_ranges(&mut random, 60);
        let source = remap(&first)
            .address_tables(&AddressTables::new(old_sections()))
            .unwrap();
        // Remap the image of the first remap again: the image addresses of `first` are the
        // old addresses of `second`.
        let mut second = Vec::new();
        let mut targets = [0x1000 + random.between(0, 0x20), 0x8000, 0x10000];
        let mut image: Vec<(u32, u32)> = first.iter().map(|r| (r.2, r.1)).collect();
        random.shuffle(&mut image);
        for (start, length) in image {
            // Split some image ranges, keep others whole and drop a few.
            let cut = if length > 1 && random.between(0, 2) == 0 {
                random.between(1, length - 1)
            } else {
                length
            };
            for (old, part) in [(start, cut), (start + cut, length - cut)] {
                let slot = usize::try_from(random.between(0, 2)).unwrap();
                let end = [0x3000, 0xb000, 0x11000][slot];
                if part == 0 || random.between(0, 7) == 0 || targets[slot] + part > end {
                    continue;
                }
                second.push((old, part, targets[slot]));
                targets[slot] += part + random.between(0, 1) * 0x10;
            }
        }
        let tables = remap(&second).address_tables(&source).unwrap();
        assert_eq!(tables.original, Some(old_sections()), "seed {seed}");
        assert_eq!(tables.sections, new_sections());
        let map = tables.map();
        for old in samples(first.iter().map(|r| (r.0, r.1)), &mut random) {
            let want = forward(&first, old).and_then(|image| forward(&second, image));
            let got = mapped(map.pdb_rva_to_rva(PdbRva(old)).unwrap()).map(|rva| rva.0);
            assert_eq!(got, want, "seed {seed}: PDB RVA {old:#x}");
        }
        for new in samples(second.iter().map(|r| (r.2, r.1)), &mut random) {
            let want = backward(&second, new).and_then(|image| backward(&first, image));
            let got = mapped(map.rva_to_pdb_rva(Rva(new)).unwrap()).map(|rva| rva.0);
            assert_eq!(got, want, "seed {seed}: RVA {new:#x}");
        }
    }
}

#[test]
fn composition_follows_the_lookup_of_any_source_table() {
    // Source tables of another producer: equal `rva` values, targets of 0, a point mapping in
    // the last record and two source addresses with the same image address.
    let from_records = [
        record(0x1000, 0x1000),
        record(0x1000, 0x2000),
        record(0x1010, 0),
        record(0x1020, 0x2010),
        record(0x1040, 0x2010),
        record(0x1060, 0x2fff),
    ];
    let to_records = [
        record(0x1800, 0x5000),
        record(0x2000, 0x1000),
        record(0x2010, 0x1020),
        record(0x2030, 0),
        record(0x2fff, 0x1060),
    ];
    let mut source = AddressTables::new(old_sections());
    source.original = Some(old_sections());
    source.from_source = Some(OmapStream::from_records(&from_records).unwrap());
    source.to_source = Some(OmapStream::from_records(&to_records).unwrap());
    let source_from = source.from_source.clone().unwrap();
    let source_to = source.to_source.clone().unwrap();
    let ranges = [
        (0x1800, 0x8, 0x1000),
        (0x2008, 0x10, 0x8000),
        (0x2020, 0x8, 0x1100),
        (0x2ff0, 0x10, 0x10000),
    ];
    let tables = remap(&ranges).address_tables(&source).unwrap();
    let map = tables.map();
    for old in 0x0ff0..0x1080 {
        let want =
            mapped(source_from.lookup(old).unwrap()).and_then(|image| forward(&ranges, image));
        let got = mapped(map.pdb_rva_to_rva(PdbRva(old)).unwrap()).map(|rva| rva.0);
        assert_eq!(got, want, "PDB RVA {old:#x}");
    }
    let mut random = Random(7);
    for new in samples(ranges.iter().map(|r| (r.2, r.1)), &mut random) {
        let want =
            backward(&ranges, new).and_then(|image| mapped(source_to.lookup(image).unwrap()));
        let got = mapped(map.rva_to_pdb_rva(Rva(new)).unwrap()).map(|rva| rva.0);
        assert_eq!(got, want, "RVA {new:#x}");
    }
}

#[test]
fn composition_needs_the_complete_source_tables() {
    let table = OmapStream::from_records(&[record(0x1000, 0x1000)]).unwrap();
    let remap = remap(&[(0x1000, 0x10, 0x1000)]);
    let mut source = AddressTables::new(old_sections());
    source.original = Some(old_sections());
    source.from_source = Some(table.clone());
    let error = remap.address_tables(&source).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::MissingStream);
    source.from_source = None;
    source.to_source = Some(table.clone());
    let error = remap.address_tables(&source).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::MissingStream);
    source.from_source = Some(table);
    source.original = None;
    let error = remap.address_tables(&source).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::MissingStream);
}

#[test]
fn the_same_ranges_give_the_same_tables_in_any_order() {
    for seed in 1..=16_u64 {
        let mut random = Random(seed.wrapping_mul(0x2545_f491_4f6c_dd1d) | 1);
        let mut ranges = random_ranges(&mut random, 50);
        let first = remap(&ranges);
        random.shuffle(&mut ranges);
        let second = remap(&ranges);
        assert_eq!(first, second, "seed {seed}");
        let source = AddressTables::new(old_sections());
        let bytes = |remap: &ImageRemap| {
            let tables = remap.address_tables(&source).unwrap();
            (
                tables.from_source.unwrap().to_bytes().unwrap(),
                tables.to_source.unwrap().to_bytes().unwrap(),
                tables.original.unwrap().to_bytes().unwrap(),
                tables.sections.to_bytes().unwrap(),
            )
        };
        assert_eq!(bytes(&first), bytes(&second), "seed {seed}");
        assert_eq!(bytes(&first), bytes(&first.clone()));
    }
}

#[test]
fn tables_read_again_from_stream_bytes() {
    let mut random = Random(99);
    let ranges = random_ranges(&mut random, 40);
    let tables = remap(&ranges)
        .address_tables(&AddressTables::new(old_sections()))
        .unwrap();
    for table in [&tables.from_source, &tables.to_source] {
        let table = table.as_ref().unwrap();
        let read = OmapStream::parse(table.to_bytes().unwrap(), StreamId(9)).unwrap();
        assert_eq!(&read, table);
    }
}
