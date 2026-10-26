//! Select the addresses and the type names for the DIA queries and the symbolizer.
//!
//! The selection reads the `DIA2Dump -all` output of the original PDB. It is deterministic: the
//! same output gives the same sample in each run.
//!
//! - The address ranges come from the `Function` and `Data` lines with
//!   `[RVA][section:offset], len = L` (a line without a length gives a range of 1 byte), and from
//!   the `PublicSymbol: [RVA]` lines (1 byte). RVA 0 and RVA `0xFFFFFFFF` do not count.
//! - The sample has the start and the middle of each range. While it has fewer than
//!   [`MIN_ADDRESSES`] addresses and the ranges have more distinct addresses, it adds one
//!   address in each range from a pseudo-random generator with a fixed seed.
//! - A sample with more than [`MAX_ADDRESSES`] addresses keeps a pseudo-random selection.
//! - The sample also has the separated code addresses of [`separated_addresses`]. The
//!   `DIA2Dump -all` output gives separated code only as `Block` lines, and the inline ranges
//!   in it not at all. The limit of [`MAX_ADDRESSES`] does not remove these addresses.
//! - The type names come from the `UserDefinedType: <name>` lines. More than [`MAX_TYPES`]
//!   names give a pseudo-random selection.

use std::collections::BTreeSet;
use std::path::Path;

use pdbrs::native::Pdb;
use pdbrs::native::locate::SymbolLocator;

/// The address count that the sample tries to reach.
pub(crate) const MIN_ADDRESSES: usize = 200;

/// The maximum address count of a sample, without the separated code addresses.
pub(crate) const MAX_ADDRESSES: usize = 600;

/// The maximum type name count of a sample.
pub(crate) const MAX_TYPES: usize = 250;

/// The seed of the address selection.
const ADDRESS_SEED: u64 = 33;

/// The seed of the type name selection.
const TYPE_SEED: u64 = 34;

/// The addresses and type names of a sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sample {
    /// The RVAs, in increasing order.
    pub rvas: Vec<u32>,
    /// The number of distinct addresses in all ranges.
    pub available: u64,
    /// The number of separated code addresses in `rvas`.
    pub separated: usize,
    /// The type names, in increasing order.
    pub types: Vec<String>,
}

impl Sample {
    /// Select a sample from the `DIA2Dump -all` output of a PDB and its separated code
    /// addresses.
    pub(crate) fn select(dia_all: &str, separated: &[u32]) -> Self {
        let ranges = ranges(dia_all);
        let (rvas, available) = addresses(&ranges);
        let separated: BTreeSet<u32> = separated.iter().copied().collect();
        let mut all: BTreeSet<u32> = rvas.into_iter().collect();
        all.extend(&separated);
        Self {
            rvas: all.into_iter().collect(),
            available,
            separated: separated.len(),
            types: type_names(dia_all),
        }
    }

    /// Describe the address count. A sample with fewer than [`MIN_ADDRESSES`] addresses
    /// tells why. A sample with separated code addresses gives their number.
    pub(crate) fn describe(&self) -> String {
        let count = self.rvas.len();
        let text = if count >= MIN_ADDRESSES {
            format!("{count} addresses")
        } else if self.available == 0 {
            format!("{count} addresses (DIA lists no code or data addresses)")
        } else {
            format!(
                "{count} addresses (all {} distinct addresses used)",
                self.available
            )
        };
        if self.separated == 0 {
            text
        } else {
            format!("{text}, {} in separated code", self.separated)
        }
    }
}

/// Return the separated code addresses of the PDB at `path`, sorted and without duplicates.
///
/// The addresses are the first, the middle and the last byte of each separated code chunk of a
/// procedure (`S_SEPCODE`) and of each inline range in such a chunk. The pdbrs locator gives
/// the chunks ([`SymbolLocator::code_chunks`]) and the inline ranges.
pub(crate) fn separated_addresses(path: &Path) -> Result<Vec<u32>, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let error = |error: pdbrs::Error| format!("separated code of {}: {error}", path.display());
    let pdb = Pdb::parse(bytes.as_slice()).map_err(error)?;
    let locator = SymbolLocator::new(&pdb).map_err(error)?;
    let mut rvas = BTreeSet::new();
    let mut add = |start: u32, length: u32| {
        if let Some(last) = length.checked_sub(1) {
            rvas.extend([
                start,
                start.saturating_add(length / 2),
                start.saturating_add(last),
            ]);
        }
    };
    for procedure in locator.procedures() {
        let procedure = procedure.map_err(error)?;
        let chunks = locator.code_chunks(&procedure).map_err(error)?;
        if chunks.len() < 2 {
            continue;
        }
        for chunk in &chunks[1..] {
            if let Some(start) = chunk.start {
                add(start.0, chunk.length);
            }
        }
        for site in locator.inline_sites(&procedure).map_err(error)? {
            for range in site.ranges.iter().filter(|range| range.chunk > 0) {
                if let Some((start, length)) = range.rva_range(&chunks) {
                    add(start.0, length);
                }
            }
        }
    }
    Ok(rvas.into_iter().collect())
}

/// A pseudo-random generator: `SplitMix64`.
struct Random(u64);

impl Random {
    /// Return the next value.
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^ (value >> 31)
    }

    /// Return a value below `bound`. `bound` must not be 0.
    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    /// Keep `count` items, selected pseudo-randomly, in their original order.
    fn select<T>(&mut self, items: Vec<T>, count: usize) -> Vec<T> {
        if items.len() <= count {
            return items;
        }
        let mut indices: Vec<usize> = (0..items.len()).collect();
        for index in 0..count {
            let remaining = u64::try_from(indices.len() - index).unwrap_or(u64::MAX);
            let offset = usize::try_from(self.below(remaining)).unwrap_or(0);
            indices.swap(index, index + offset);
        }
        let keep: BTreeSet<usize> = indices[..count].iter().copied().collect();
        items
            .into_iter()
            .enumerate()
            .filter_map(|(index, item)| keep.contains(&index).then_some(item))
            .collect()
    }
}

/// Read the address ranges `(rva, length)` of a `DIA2Dump -all` output, sorted and without
/// duplicates.
pub(crate) fn ranges(dia_all: &str) -> Vec<(u32, u32)> {
    let mut ranges = BTreeSet::new();
    for line in dia_all.lines() {
        let range = if line.starts_with("PublicSymbol:") {
            address(line).map(|(rva, _)| (rva, 1))
        } else if is_symbol_line(line, "Function") || is_symbol_line(line, "Data") {
            address(line).map(|(rva, rest)| (rva, length(rest).unwrap_or(1).max(1)))
        } else {
            None
        };
        if let Some((rva, length)) = range.filter(|&(rva, _)| !matches!(rva, 0 | u32::MAX)) {
            ranges.insert((rva, length));
        }
    }
    ranges.into_iter().collect()
}

/// Return `true` if the line starts with `keyword`, spaces and `:`.
fn is_symbol_line(line: &str, keyword: &str) -> bool {
    line.strip_prefix(keyword)
        .is_some_and(|rest| rest.trim_start_matches(' ').starts_with(':'))
}

/// Find the first `[RVA][SSSS:OOOOOOOO]` of a line. Return the RVA and the text after it.
fn address(line: &str) -> Option<(u32, &str)> {
    let mut rest = line;
    while let Some(start) = rest.find('[') {
        let candidate = &rest[start..];
        if let Some(found) = parse_address(candidate) {
            return Some(found);
        }
        rest = &candidate[1..];
    }
    None
}

/// Parse `[RVA][SSSS:OOOOOOOO]` at the start of `text`.
fn parse_address(text: &str) -> Option<(u32, &str)> {
    let rest = text.strip_prefix('[')?;
    let (rva, rest) = hex_field(rest, 8)?;
    let rest = rest.strip_prefix("][")?;
    let (_, rest) = hex_field(rest, 4)?;
    let rest = rest.strip_prefix(':')?;
    let (_, rest) = hex_field(rest, 8)?;
    Some((rva, rest.strip_prefix(']')?))
}

/// Parse `digits` uppercase hexadecimal digits at the start of `text`.
fn hex_field(text: &str, digits: usize) -> Option<(u32, &str)> {
    let field = text.get(..digits)?;
    if !field
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
    {
        return None;
    }
    Some((u32::from_str_radix(field, 16).ok()?, &text[digits..]))
}

/// Parse `, len = LLLLLLLL` at the start of `text`.
fn length(text: &str) -> Option<u32> {
    let (length, _) = hex_field(text.strip_prefix(", len = ")?, 8)?;
    Some(length)
}

/// Select the sample addresses of the ranges. Return them with the number of distinct
/// addresses in all ranges.
fn addresses(ranges: &[(u32, u32)]) -> (Vec<u32>, u64) {
    let end = |rva: u32, length: u32| (u64::from(rva) + u64::from(length)).min(1 << 32);
    let mut available = 0;
    let mut covered = 0_u64;
    for &(rva, length) in ranges {
        let start = u64::from(rva).max(covered);
        let stop = end(rva, length);
        available += stop.saturating_sub(start);
        covered = covered.max(stop);
    }

    let mut rvas = BTreeSet::new();
    for &(rva, length) in ranges {
        rvas.insert(rva);
        rvas.insert(rva + (length / 2).min(u32::MAX - rva));
    }
    let mut random = Random(ADDRESS_SEED);
    while rvas.len() < MIN_ADDRESSES && u64::try_from(rvas.len()).unwrap_or(u64::MAX) < available {
        for &(rva, length) in ranges {
            let span = end(rva, length) - u64::from(rva);
            let offset = u32::try_from(random.below(span)).unwrap_or(0);
            rvas.insert(rva + offset);
        }
    }
    let rvas = random.select(rvas.into_iter().collect(), MAX_ADDRESSES);
    (rvas, available)
}

/// Read the type names of the `UserDefinedType: <name>` lines, sorted and without duplicates.
/// Keep a pseudo-random selection of [`MAX_TYPES`] names.
pub(crate) fn type_names(dia_all: &str) -> Vec<String> {
    let names: BTreeSet<&str> = dia_all
        .lines()
        .filter_map(|line| line.strip_prefix("UserDefinedType: "))
        .filter(|name| {
            name.chars()
                .next()
                .is_some_and(|first| !first.is_whitespace())
        })
        .collect();
    let names = names.into_iter().map(str::to_owned).collect();
    Random(TYPE_SEED).select(names, MAX_TYPES)
}

#[cfg(test)]
mod tests {
    use super::{MAX_ADDRESSES, MAX_TYPES, MIN_ADDRESSES, Sample, ranges, type_names};
    use std::fmt::Write as _;

    const DUMP: &str = "\
PublicSymbol: [00001370][0001:00000370] ?f@@YAXXZ(void __cdecl f(void))
PublicSymbol: [FFFFFFFF][0006:00000100] __guard_flags(__guard_flags)
Function       : static, [00001010][0001:00000010], len = 0000015C, main
Data           :   rsp Relative, [00000038], Local, Type: struct c_record, record
Data           : static, [0000B000][0002:00000000], Global, Type: int, g_value
  Function     : static, [00002000][0001:00001000], len = 00000010, indented
UserDefinedType: c_point
UserDefinedType:     _PMD
UserDefinedType: c_point
UserDefinedType: std::pair<int,int>
";

    #[test]
    fn reads_the_ranges_and_the_type_names() {
        assert_eq!(
            ranges(DUMP),
            vec![(0x1010, 0x15C), (0x1370, 1), (0xB000, 1)]
        );
        assert_eq!(type_names(DUMP), vec!["c_point", "std::pair<int,int>"]);
    }

    #[test]
    fn the_sample_is_deterministic_and_inside_the_ranges() {
        let first = Sample::select(DUMP, &[]);
        assert_eq!(first, Sample::select(DUMP, &[]));
        assert_eq!(first.rvas.len(), MIN_ADDRESSES);
        assert_eq!(first.available, 0x15C + 2);
        assert!(first.rvas.is_sorted());
        for rva in [0x1010, 0x1010 + 0xAE, 0x1370, 0xB000] {
            assert!(first.rvas.contains(&rva), "{rva:#x} is missing");
        }
        assert!(
            first
                .rvas
                .iter()
                .all(|&rva| (0x1010..0x1010 + 0x15C).contains(&rva)
                    || rva == 0x1370
                    || rva == 0xB000)
        );
    }

    #[test]
    fn a_small_dump_uses_all_addresses() {
        let dump = "Function : static, [00001000][0001:00000000], len = 00000004, f\n";
        let sample = Sample::select(dump, &[]);
        assert_eq!(sample.rvas, vec![0x1000, 0x1001, 0x1002, 0x1003]);
        assert_eq!(
            sample.describe(),
            "4 addresses (all 4 distinct addresses used)"
        );
        assert_eq!(
            Sample::select("", &[]).describe(),
            "0 addresses (DIA lists no code or data addresses)"
        );
    }

    #[test]
    fn large_dumps_are_capped() {
        let mut dump = String::new();
        for index in 0..1000_u32 {
            let rva = 0x1000 + index * 0x10;
            writeln!(
                dump,
                "Function : static, [{rva:08X}][0001:{rva:08X}], len = 00000008, f{index}"
            )
            .unwrap();
            writeln!(dump, "UserDefinedType: t{index}").unwrap();
        }
        let sample = Sample::select(&dump, &[]);
        assert_eq!(sample.rvas.len(), MAX_ADDRESSES);
        assert_eq!(sample.types.len(), MAX_TYPES);
        assert_eq!(sample, Sample::select(&dump, &[]));
        assert!(sample.types.is_sorted());
        // The limit does not remove separated code addresses.
        let separated = [0x9000, 0x9001, 0x1000];
        let with = Sample::select(&dump, &separated);
        assert!(separated.iter().all(|rva| with.rvas.contains(rva)));
        assert!(with.rvas.is_sorted());
        assert_eq!(with.separated, 3);
        assert!(with.describe().ends_with(", 3 in separated code"));
    }
}
