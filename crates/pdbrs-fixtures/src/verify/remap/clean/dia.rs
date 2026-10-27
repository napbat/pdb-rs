//! The `DIA2Dump` comparison of the clean output with the OMAP output (decision 12 of #43).
//!
//! DIA translates each address of the OMAP output to the new image, and it keeps the source
//! section and offset. The clean output has the new section and offset in its records. The
//! check thus compares only the RVA of each address ([`normalize`]):
//!
//! - Symbols: `DIA2Dump -s` line by line; `-p` and `-g` as sorted lists, because the order of
//!   the public and global symbols follows their hash tables. The OMAP lines with RVA 0 (an
//!   address without a range) have no clean line: the clean output with `RemovedCode::Drop`
//!   removes these records. In `-s`, the child lines of a removed symbol (the lines up to the
//!   next empty line or the next symbol line with the same or a smaller indent) go with it.
//!   The empty line that `DIA2Dump` prints before a procedure also goes with it. The check
//!   counts these lines as removed.
//! - Lines: `-l <rva> 1` for each line start of the OMAP check.
//! - Symbols at an address: `-sym <rva>` at the samples of the procedures.
//! - Frame data: `-fpo <rva>` at the samples that an FPO or new FPO record of the source
//!   holds.
//!
//! The normalization removes the section and offset properties (`AddressSection`,
//! `AddressOffset`) and the DIA symbol IDs (`SymIndex` and the properties that end with `Id`),
//! which DIA gives in the order of its symbol creation. It replaces the PDB name in the
//! `Executable (Global)` line, because the two outputs have other file names.
//!
//! `-sym` also prints addresses as a section property and an offset property without an RVA
//! (the exception handler of a procedure and the parent of an `S_SEPCODE` scope). The check
//! replaces each pair with the new RVA ([`properties`]). For the OMAP output, that is the new
//! RVA of the source address; for the clean output, the RVA in the section headers of the
//! set.
//!
//! [`classes`] gives the explained differences, and [`mod@query`] runs the address queries.

mod classes;
mod properties;
mod query;

use std::fmt::Write as _;
use std::path::Path;

use pdbrs::native::address::SectionOffset;

use self::classes::{
    COFF_GROUP, Evidence, FOLDED, LAST_PUBLIC, LAZY, SEPARATED, SEPARATED_PART, SPLIT, Scope,
    debug_bound, folded, group_phase, is_debug_end, last_public, lazy_extra, separated_part,
    split_extent,
};
use self::properties::normalize_answers;
use self::query::{Split, query};
use super::super::dia::{OmapDumps, next_address, run};
use super::super::ranges::RangeSet;
use super::{Facts, Group, Sample, Tally, covers, set_rva};

/// The tool and the outputs of one comparison.
#[derive(Clone, Copy, Debug)]
pub(super) struct Context<'a> {
    /// `DIA2Dump.exe`.
    pub dia2dump: &'a Path,
    /// The range set.
    pub set: &'a RangeSet,
    /// The OMAP output.
    pub omap: &'a Path,
    /// The clean output.
    pub clean: &'a Path,
    /// `true` for a PDB without type merging (feature `NOTM`, `link /DEBUG:LAZY`).
    pub lazy: bool,
    /// The facts of the source PDB.
    pub facts: &'a Facts,
    /// The COFF groups of the clean output, with new RVAs.
    pub clean_groups: &'a [Group],
}

/// An address in a `DIA2Dump` line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Address {
    /// The byte offset of the first character of the address text.
    start: usize,
    /// The byte offset after the address text.
    end: usize,
    /// The RVA.
    rva: u32,
    /// The section number.
    section: u32,
}

/// Parse the `digits` hexadecimal digits at `from` in `text`, or return 0.
fn hex(text: &str, from: usize, digits: usize) -> u32 {
    text.get(from..from + digits)
        .and_then(|field| u32::from_str_radix(field, 16).ok())
        .unwrap_or(0)
}

/// Return the addresses of `line`: `[RRRRRRRR][SSSS:OOOOOOOO]` and
/// `[0xSSSS:0xOOOOOOOO]  0xRRRRRRRR`.
fn addresses(line: &str) -> Vec<Address> {
    let mut list = Vec::new();
    let mut from = 0;
    while let Some((at, rva)) = next_address(&line[from..]) {
        let at = from + at;
        let address = if line[..at].ends_with('[') {
            Address {
                start: at - 1,
                end: at + 24,
                rva,
                section: hex(line, at + 10, 4),
            }
        } else {
            Address {
                start: at - 23,
                end: at + 8,
                rva,
                section: hex(line, at - 20, 4),
            }
        };
        from = address.end;
        list.push(address);
    }
    list
}

/// Return the name of a property line (`name:<tab> value`).
fn property(line: &str) -> Option<&str> {
    line.split_once(":\t").map(|(name, _)| name.trim())
}

/// The start of the line of `-sym` that names the PDB: DIA uses the file stem of the PDB.
const GLOBAL_SCOPE: &str = "Executable (Global): ";

/// Return the line with each address replaced by `[RRRRRRRR]`, or `None` for a property
/// that the comparison ignores. The PDB name of [`GLOBAL_SCOPE`] becomes `<pdb>`, because the
/// two outputs have other file names.
fn normalize(line: &str) -> Option<String> {
    if let Some(name) = property(line)
        && (name == "SymIndex"
            || name.ends_with("Id")
            || name.eq_ignore_ascii_case("AddressSection")
            || name.eq_ignore_ascii_case("AddressOffset"))
    {
        return None;
    }
    if line.starts_with(GLOBAL_SCOPE) {
        return Some(format!("{GLOBAL_SCOPE}<pdb>"));
    }
    let mut out = String::with_capacity(line.len());
    let mut last = 0;
    for address in addresses(line) {
        out.push_str(&line[last..address.start]);
        let _ = write!(out, "[{:08X}]", address.rva);
        last = address.end;
    }
    out.push_str(&line[last..]);
    Some(out)
}

/// Return `true` if `line` has an address without a range: RVA 0 in a section.
fn unmapped(line: &str) -> bool {
    addresses(line)
        .iter()
        .any(|address| address.rva == 0 && address.section != 0)
}

/// Return the indent of a symbol line of `DIA2Dump -s` (`<tag>: `, the indent and the
/// symbol), or `None` for another line. A line of a symbol has no space at the start.
fn indent(line: &str) -> Option<usize> {
    if line.is_empty() || line.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = line.split_once(':')?.1.strip_prefix(' ')?;
    Some(rest.len() - rest.trim_start_matches(' ').len())
}

/// Return `true` for a `Function` line. `DIA2Dump` prints an empty line before it.
fn is_function(line: &str) -> bool {
    line.split([' ', ':']).next() == Some("Function")
}

/// Return the new RVA and the length of a normalized `Function` line.
fn procedure(line: &str) -> Option<(u32, u64)> {
    if !is_function(line) {
        return None;
    }
    Some((first_rva(line)?, length(line)?.2))
}

/// Return `true` for a block line (DIA shows an `S_SEPCODE` scope as a block).
fn is_separated(line: &str) -> bool {
    line.trim_start()
        .strip_prefix("Block")
        .is_some_and(|rest| rest.trim_start().starts_with(':'))
}

/// Return the first `[RRRRRRRR]` RVA of a normalized line.
fn first_rva(line: &str) -> Option<u32> {
    line.match_indices('[').find_map(|(at, _)| {
        let field = line.get(at + 1..at + 9)?;
        (line.get(at + 9..at + 10) == Some("]") && field.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| u32::from_str_radix(field, 16).ok())
            .flatten()
    })
}

/// Return the byte range and the value of the length of a line: after `len = ` or in the
/// `Length` property.
fn length(line: &str) -> Option<(usize, usize, u64)> {
    let at = match line.find("len = ") {
        Some(at) => at + 6,
        None if property(line) == Some("Length") => line.find(":\t")? + 2,
        None => return None,
    };
    let at = at + line[at..].len() - line[at..].trim_start().len();
    let start = if line[at..].starts_with("0x") {
        at + 2
    } else {
        at
    };
    let digits = line[start..]
        .bytes()
        .take_while(u8::is_ascii_hexdigit)
        .count();
    let value = u64::from_str_radix(line.get(start..start + digits)?, 16).ok()?;
    Some((start, start + digits, value))
}

/// Count an equal line as compared if it has an address.
fn count(line: &str, tally: &mut Tally) {
    if first_rva(line).is_some() {
        tally.compared += 1;
    }
}

/// Compare the normalized lines of `DIA2Dump -s` in order.
fn merge_ordered(
    command: &str,
    omap: &[String],
    clean: &[String],
    evidence: &Evidence<'_>,
    tally: &mut Tally,
) {
    let set = evidence.set;
    let (mut i, mut j) = (0, 0);
    // The procedure of the last equal or split `Function` line.
    let mut scope: Option<Scope> = None;
    let start = |scope: Option<Scope>| scope.map(|scope| scope.start);
    loop {
        if evidence.lazy
            && omap.get(i) != clean.get(j)
            && let Some(extra) = lazy_extra(omap, i, clean, j)
        {
            for _ in 0..extra {
                tally.explain(LAZY);
            }
            j += extra;
            continue;
        }
        match (omap.get(i), clean.get(j)) {
            (None, None) => break,
            (Some(x), Some(y)) if x == y => {
                count(x, tally);
                if is_function(x) {
                    scope = classes::scope(x, y);
                }
                i += 1;
                j += 1;
            }
            (_, Some(y)) if is_separated(y) => {
                tally.explain(SEPARATED);
                j += 1;
            }
            (Some(x), _) if group_phase(x, start(scope), true, evidence) => {
                tally.explain(COFF_GROUP);
                i += 1;
            }
            (_, Some(y)) if group_phase(y, start(scope), false, evidence) => {
                tally.explain(COFF_GROUP);
                j += 1;
            }
            (Some(x), Some(y)) => {
                if split_extent(x, y, None, set) {
                    tally.compared += 1;
                    tally.explain(SPLIT);
                    if is_function(x) {
                        scope = classes::scope(x, y);
                    }
                } else if let Some(class) = debug_bound(x, y, scope, set) {
                    tally.compared += 1;
                    tally.explain(class);
                } else {
                    tally.push(|| {
                        format!("DIA2Dump {command}: the clean output prints `{y}`, the OMAP output `{x}`")
                    });
                    if is_function(x) {
                        scope = None;
                    }
                }
                i += 1;
                j += 1;
            }
            (Some(x), None) => {
                tally.push(|| format!("DIA2Dump {command}: the clean output has no line `{x}`"));
                i += 1;
            }
            (None, Some(y)) => {
                tally.push(|| format!("DIA2Dump {command}: the OMAP output has no line `{y}`"));
                j += 1;
            }
        }
    }
}

/// Compare the sorted normalized lines of `DIA2Dump -p` or `-g`.
fn merge_sorted(
    command: &str,
    omap: &[String],
    clean: &[String],
    evidence: &Evidence<'_>,
    tally: &mut Tally,
) {
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    while i < omap.len() && j < clean.len() {
        match omap[i].cmp(&clean[j]) {
            std::cmp::Ordering::Equal => {
                count(&omap[i], tally);
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => {
                left.push(&omap[i]);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                right.push(&clean[j]);
                j += 1;
            }
        }
    }
    left.extend(&omap[i..]);
    right.extend(&clean[j..]);
    right.retain(|line| {
        let separated = is_separated(line);
        if separated {
            tally.explain(SEPARATED);
        }
        !separated
    });
    for index in 0..left.len().max(right.len()) {
        match (left.get(index), right.get(index)) {
            (Some(x), Some(y)) if split_extent(x, y, None, evidence.set) => {
                tally.compared += 1;
                tally.explain(SPLIT);
            }
            (x, y) => tally.push(|| {
                format!("DIA2Dump {command}: the clean output prints {y:?}, the OMAP output {x:?}")
            }),
        }
    }
}

/// Compare one symbol dump command of the OMAP output and the clean output.
fn compare_dump(
    command: &str,
    omap: &[String],
    clean: &[String],
    evidence: &Evidence<'_>,
    tally: &mut Tally,
) {
    let ordered = command == "-s";
    let mut kept: Vec<String> = Vec::with_capacity(omap.len());
    // The indent of the removed symbol whose child lines the loop removes.
    let mut removed: Option<usize> = None;
    for line in omap {
        if let Some(depth) = removed {
            // An empty line or a symbol line at the same or a smaller indent ends the scope.
            if line.is_empty() || indent(line).is_some_and(|other| other <= depth) {
                removed = None;
            } else {
                tally.removed += 1;
                continue;
            }
        }
        // The address after a procedure can have no range; `debug_bound` compares that line.
        if unmapped(line) && !(ordered && is_debug_end(line)) {
            tally.removed += 1;
            if ordered {
                // `DIA2Dump` prints an empty line before a procedure: it goes with it.
                if is_function(line) && kept.last().is_some_and(String::is_empty) {
                    kept.pop();
                }
                removed = indent(line);
            }
        } else {
            kept.extend(normalize(line));
        }
    }
    let mut clean: Vec<String> = clean.iter().filter_map(|line| normalize(line)).collect();
    if ordered {
        merge_ordered(command, &kept, &clean, evidence, tally);
    } else {
        kept.sort_unstable();
        clean.sort_unstable();
        merge_sorted(command, &kept, &clean, evidence, tally);
    }
}

/// Return the new RVA of the record of an answer: the first address, or the RVA property.
fn answer_rva(lines: &[String]) -> Option<u32> {
    lines.iter().find_map(|line| {
        first_rva(line).or_else(|| {
            let name = property(line)?;
            if name != "RelativeVirtualAddress" && name != "RVA" {
                return None;
            }
            let value = line.split_once(":\t")?.1.trim().strip_prefix("0x")?;
            u32::from_str_radix(value, 16).ok()
        })
    })
}

/// Return the classes of the explained differences of two normalized answers, or `None` if
/// a difference has no explanation.
fn explain_answer(
    omap: &[String],
    clean: &[String],
    evidence: &Evidence<'_>,
) -> Option<Vec<&'static str>> {
    let set = evidence.set;
    let start = answer_rva(omap);
    let clean_rva = answer_rva(clean);
    // The procedure of the answer, for the phase lines of `-sym`.
    let function = clean
        .iter()
        .find_map(|line| procedure(line).map(|(rva, _)| rva));
    let mut scope: Option<Scope> = None;
    let mut found = Vec::new();
    let (mut i, mut j) = (0, 0);
    loop {
        match (omap.get(i), clean.get(j)) {
            (None, None) => break,
            (Some(x), Some(y)) if x == y => {
                if is_function(x) {
                    scope = classes::scope(x, y);
                }
                i += 1;
                j += 1;
            }
            (_, Some(y)) if is_separated(y) => {
                found.push(SEPARATED);
                j += 1;
            }
            (Some(x), _) if group_phase(x, function, true, evidence) => {
                found.push(COFF_GROUP);
                i += 1;
            }
            (_, Some(y)) if group_phase(y, function, false, evidence) => {
                found.push(COFF_GROUP);
                j += 1;
            }
            (Some(x), Some(y)) => {
                if split_extent(x, y, start, set) {
                    found.push(SPLIT);
                    if is_function(x) {
                        scope = classes::scope(x, y);
                    }
                } else if let Some(class) = debug_bound(x, y, scope, set) {
                    found.push(class);
                } else if last_public(x, y, (clean, clean_rva), evidence) {
                    found.push(LAST_PUBLIC);
                } else {
                    return None;
                }
                i += 1;
                j += 1;
            }
            _ => return None,
        }
    }
    Some(found)
}

/// The answers of a query at each RVA (`None` for a query without an answer).
type Answers = [Option<Vec<String>>];

/// Compare the normalized answers of the query `option` at each RVA of `rvas`.
fn compare_answers(
    option: &str,
    rvas: &[u32],
    (omap, clean): (&Answers, &Answers),
    evidence: &Evidence<'_>,
    tally: &mut Tally,
) {
    for ((rva, omap), clean) in rvas.iter().zip(omap).zip(clean) {
        tally.compared += 1;
        if omap == clean {
            continue;
        }
        if let (Some(x), Some(y)) = (omap, clean) {
            if folded(x, y, evidence) {
                tally.explain(FOLDED);
                continue;
            }
            // The procedure lines of both answers (from the `Function` line) must agree.
            let tail = |answer: &[String]| {
                let at = answer.iter().position(|line| is_function(line));
                at.map_or(0, |at| answer.len() - at)
            };
            let (x_tail, y_tail) = (&x[x.len() - tail(x)..], &y[y.len() - tail(y)..]);
            if separated_part(*rva, x, y, evidence.set)
                && let Some(classes) = explain_answer(x_tail, y_tail, evidence)
            {
                tally.explain(SEPARATED_PART);
                for class in classes {
                    tally.explain(class);
                }
                continue;
            }
            if let Some(classes) = explain_answer(x, y, evidence) {
                for class in classes {
                    tally.explain(class);
                }
                continue;
            }
        }
        tally.push(|| {
            let (x, y) = match (omap, clean) {
                (Some(x), Some(y)) => {
                    let index = x.iter().zip(y).take_while(|(x, y)| x == y).count();
                    (x.get(index).cloned(), y.get(index).cloned())
                }
                (x, y) => (
                    x.as_ref().map(|_| "an answer".to_owned()),
                    y.as_ref().map(|_| "an answer".to_owned()),
                ),
            };
            format!(
                "DIA2Dump {option} {rva:X}: the clean output prints {y:?}, the OMAP output {x:?}"
            )
        });
    }
}

/// Return the highest RVA of a public symbol in the lines of `DIA2Dump -p`. The absolute
/// symbols (RVA `0xFFFFFFFF`) do not count.
fn last_public_rva(lines: &[String]) -> Option<u32> {
    lines
        .iter()
        .filter(|line| line.starts_with("PublicSymbol:"))
        .filter_map(|line| addresses(line).first().map(|address| address.rva))
        .filter(|&rva| rva != 0 && rva != u32::MAX)
        .max()
}

/// Compare the `DIA2Dump` answers of the clean output with the OMAP output. `samples` are
/// the sampled procedure addresses, and `frames` the code extents of the frame data of the
/// source.
///
/// # Errors
///
/// Returns an error if `DIA2Dump` fails on a symbol dump of the clean output.
pub(super) fn compare(
    context: &Context<'_>,
    dumps: &OmapDumps,
    samples: &[Sample],
    frames: &[(u32, u32)],
    tally: &mut Tally,
) -> Result<(), String> {
    let set = context.set;
    let mut evidence = Evidence::new(set, context.lazy);
    evidence.source_groups = &context.facts.groups;
    evidence.clean_groups = context.clean_groups;
    for (command, omap) in &dumps.symbols {
        let clean = run(context.dia2dump, &[(*command).to_owned()], context.clean)?;
        match *command {
            "-s" => {
                let lines: Vec<String> = omap.iter().filter_map(|line| normalize(line)).collect();
                evidence.add_functions(&lines);
            }
            "-p" => evidence.last_public = last_public_rva(&clean),
            _ => {}
        }
        compare_dump(command, omap, &clean, &evidence, tally);
    }
    let facts = context.facts;
    let omap_rva = |address: SectionOffset| facts.section_rva(address).and_then(|old| set.map(old));
    let clean_rva = |address: SectionOffset| set_rva(set, address);
    let lines = ("-l", &["1"][..], Split::EmptyLine);
    let rvas: Vec<u32> = dumps.lines.iter().map(|(rva, _)| *rva).collect();
    let omap: Vec<Option<Vec<String>>> = dumps
        .lines
        .iter()
        .map(|(_, answer)| Some(answer.clone()))
        .collect();
    let clean = query(context, lines, &rvas, context.clean)?;
    let answers = (
        &normalize_answers(&omap, &omap_rva)[..],
        &normalize_answers(&clean, &clean_rva)[..],
    );
    compare_answers("-l", &rvas, answers, &evidence, tally);
    let symbols = ("-sym", &[][..], Split::Displacement);
    let rvas: Vec<u32> = samples.iter().map(|sample| sample.new).collect();
    let omap = query(context, symbols, &rvas, context.omap)?;
    let clean = query(context, symbols, &rvas, context.clean)?;
    let answers = (
        &normalize_answers(&omap, &omap_rva)[..],
        &normalize_answers(&clean, &clean_rva)[..],
    );
    compare_answers("-sym", &rvas, answers, &evidence, tally);
    let frame_data = ("-fpo", &[][..], Split::EmptyLine);
    let rvas: Vec<u32> = samples
        .iter()
        .filter(|sample| covers(frames, sample.old))
        .map(|sample| sample.new)
        .collect();
    let omap = query(context, frame_data, &rvas, context.omap)?;
    let clean = query(context, frame_data, &rvas, context.clean)?;
    let answers = (
        &normalize_answers(&omap, &omap_rva)[..],
        &normalize_answers(&clean, &clean_rva)[..],
    );
    compare_answers("-fpo", &rvas, answers, &evidence, tally);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::super::ranges::{Range, SetKind};
    use super::classes::{DEBUG_END, DEBUG_PART};
    use super::*;

    /// 0x1000 to 0x1010 moves to 0x5000; 0x1010 to 0x1020 moves to 0x4000.
    fn set() -> RangeSet {
        RangeSet {
            kind: SetKind::Split,
            sections: Vec::new(),
            ranges: vec![
                Range {
                    old: 0x1000,
                    length: 0x10,
                    new: 0x5000,
                },
                Range {
                    old: 0x1010,
                    length: 0x10,
                    new: 0x4000,
                },
            ],
        }
    }

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|&line| line.to_owned()).collect()
    }

    /// Compare the dump `command` with the evidence of [`set`] and return the tally.
    fn dump(command: &str, omap: &[String], clean: &[String], lazy: bool) -> Tally {
        let set = set();
        let mut tally = Tally::default();
        compare_dump(command, omap, clean, &Evidence::new(&set, lazy), &mut tally);
        tally
    }

    #[test]
    fn normalization_keeps_only_the_rva() {
        assert_eq!(
            normalize("Function       : static, [00005000][0001:00000000], len = 00000020, f")
                .unwrap(),
            "Function       : static, [00005000], len = 00000020, f"
        );
        assert_eq!(
            normalize("HeapAllocationSite:   [0x0002:0x0000000f]  0x0000500F  int").unwrap(),
            "HeapAllocationSite:   [0000500F]  int"
        );
        for property in [
            "                  AddressSection:\t 0x1",
            "                   addressOffset:\t 0x2D90",
            "                        SymIndex:\t 0x1",
            "                 LexicalParentId:\t 0x2",
        ] {
            assert_eq!(normalize(property), None);
        }
        let rva = "          RelativeVirtualAddress:\t 0x5000";
        assert_eq!(normalize(rva).as_deref(), Some(rva));
        assert_eq!(
            normalize("Executable (Global): removal.clean").unwrap(),
            normalize("Executable (Global): removal").unwrap()
        );
        assert_eq!(
            answer_rva(&lines(&["   RelativeVirtualAddress:\t 0x5000"])),
            Some(0x5000)
        );
    }

    #[test]
    fn rva_zero_in_a_section_is_unmapped() {
        assert!(unmapped("PublicSymbol: [00000000][0001:00000040] f"));
        assert!(!unmapped(
            "PublicSymbol: [00000000][0000:00000000] absolute"
        ));
        assert!(!unmapped("PublicSymbol: [00005000][0001:00000000] f"));
        assert_eq!(
            last_public_rva(&lines(&[
                "PublicSymbol: [00005000][0001:00000000] a",
                "PublicSymbol: [FFFFFFFF][0003:00000100] absolute",
                "PublicSymbol: [00004000][0002:00000000] b",
            ])),
            Some(0x5000)
        );
    }

    #[test]
    fn removed_scopes_and_separated_blocks_are_counted() {
        let omap = lines(&[
            "",
            "Function       : static, [00000000][0001:00000100], len = 00000010, removed",
            "                 Function attribute:",
            "Data           :   rsp Relative, [00000008], Param, Type: int, x",
            "",
            "Function       : static, [00004000][0001:00000010], len = 00000010, kept",
            "",
        ]);
        let clean = lines(&[
            "",
            "Function       : static, [00004000][0002:00000000], len = 00000010, kept",
            "Block          :   static, [00006000][0002:00002000], len = 00000004, (none)",
            "",
        ]);
        let tally = dump("-s", &omap, &clean, false);
        assert_eq!(tally.removed, 3);
        assert_eq!(tally.compared, 1);
        assert_eq!(tally.explained.get(SEPARATED), Some(&1));
        assert_eq!(tally.differences, 0, "{:?}", tally.shown);
    }

    #[test]
    fn a_removed_thunk_keeps_the_next_symbols() {
        let omap = lines(&[
            "",
            "Thunk          : [00000000][0001:00000100], target f",
            "CompilandEnv   : obj = \"a.dll\"",
            "CompilandDetails: ",
            "\tLanguage: LINK",
            "",
            "Thunk          : [00000000][0001:00000110], target g",
            "",
            "** Module: b",
            "",
            "Function       : static, [00004000][0001:00000010], len = 00000010, kept",
            "Data           :   rsp Relative, [00000008], Param, Type: int, x",
            "",
            "Function       : static, [00000000][0001:00000200], len = 00000010, removed",
            "                 Function attribute:",
            "Block          :   static, [00000000][0001:00000204], len = 00000004, (none)",
            "Data           :     rsp Relative, [00000008], Local, Type: int, y",
            "Data           : static, [00004100][0003:00000000], Global, Type: int, z",
        ]);
        let clean = lines(&[
            "",
            "CompilandEnv   : obj = \"a.dll\"",
            "CompilandDetails: ",
            "\tLanguage: LINK",
            "",
            "",
            "** Module: b",
            "",
            "Function       : static, [00004000][0002:00000000], len = 00000010, kept",
            "Data           :   rsp Relative, [00000008], Param, Type: int, x",
            "Data           : static, [00004100][0003:00000000], Global, Type: int, z",
        ]);
        let tally = dump("-s", &omap, &clean, false);
        assert_eq!(tally.differences, 0, "{:?}", tally.shown);
        // The kept function and the lines `[00000008]` and `[00004100]` have an address.
        assert_eq!((tally.removed, tally.compared), (6, 3));
    }

    #[test]
    fn extra_symbols_at_the_end_of_a_lazy_module_are_explained() {
        let omap = lines(&[
            "",
            "** Module: a.obj",
            "",
            "UsingNamespace : demo",
            "",
            "** Module: b.obj",
            "",
        ]);
        let mut clean = omap.clone();
        clean.insert(
            4,
            "Data           : static, [00004100][0003:00000000], Global, z".to_owned(),
        );
        clean.insert(5, "Typedef        : size_t".to_owned());
        let tally = dump("-s", &omap, &clean, true);
        assert_eq!(tally.differences, 0, "{:?}", tally.shown);
        assert_eq!(tally.explained.get(LAZY), Some(&2));
        // Only a PDB without type merging has the class.
        assert!(dump("-s", &omap, &clean, false).differences > 0);
        // Extra lines inside the list of a module are a difference.
        clean.swap(3, 5);
        assert!(dump("-s", &omap, &clean, true).differences > 0);
    }

    #[test]
    fn debug_bounds_of_a_procedure_are_explained() {
        // The procedure at old 0x1010 (new 0x4000) has 0x10 bytes. The old address after it,
        // 0x1020, has no range.
        let function = "Function       : static, [00004000][0001:00000010], len = 00000010, f";
        let omap = lines(&[
            "",
            function,
            "FuncDebugEnd   :   static, [00000000][0001:00000020]",
        ]);
        let clean = lines(&[
            "",
            function,
            "FuncDebugEnd   :   static, [00004010][0002:00000010]",
        ]);
        let tally = dump("-s", &omap, &clean, false);
        assert_eq!(tally.differences, 0, "{:?}", tally.shown);
        assert_eq!(tally.explained.get(DEBUG_END), Some(&1));
        // The procedure at old 0x1000 has 0x20 bytes and splits: the clean record has the
        // first part (0x10 bytes at 0x5000). The old debug start 0x1014 is in the second
        // part.
        let omap = lines(&[
            "",
            "Function       : static, [00005000][0001:00000000], len = 00000020, g",
            "FuncDebugStart :   static, [00004004][0001:00000014]",
        ]);
        let clean = lines(&[
            "",
            "Function       : static, [00005000][0002:00000000], len = 00000010, g",
            "FuncDebugStart :   static, [00005010][0002:00000010]",
        ]);
        let tally = dump("-s", &omap, &clean, false);
        assert_eq!(tally.differences, 0, "{:?}", tally.shown);
        assert_eq!(tally.explained.get(SPLIT), Some(&1));
        assert_eq!(tally.explained.get(DEBUG_PART), Some(&1));
    }

    #[test]
    fn a_phase_line_of_a_procedure_that_left_its_group_is_explained() {
        // The source group holds old 0x1000..0x1020; the clean group starts at 0x5000 and
        // keeps its length, so it does not hold the procedure at new 0x4000.
        let source = [Group {
            name: b".text$lp00demo".to_vec(),
            start: 0x1000,
            length: 0x20,
        }];
        let clean_groups = [Group {
            name: b".text$lp00demo".to_vec(),
            start: 0x5000,
            length: 0x20,
        }];
        let set = set();
        let mut evidence = Evidence::new(&set, false);
        evidence.source_groups = &source;
        evidence.clean_groups = &clean_groups;
        let function = "Function       : static, [00004000][0001:00000010], len = 00000010, f";
        let omap = lines(&[
            "",
            function,
            "                 Phase Name: 00demo",
            "FuncDebugStart :   static, [00004000][0001:00000010]",
        ]);
        let clean = lines(&[
            "",
            function,
            "FuncDebugStart :   static, [00004000][0002:00000000]",
        ]);
        let mut tally = Tally::default();
        compare_dump("-s", &omap, &clean, &evidence, &mut tally);
        assert_eq!(tally.differences, 0, "{:?}", tally.shown);
        assert_eq!(tally.explained.get(COFF_GROUP), Some(&1));
        // Without the group change, the missing line is a difference.
        assert!(dump("-s", &omap, &clean, false).differences > 0);
    }

    #[test]
    fn answers_explain_folded_procedures_and_the_last_public_symbol() {
        let set = set();
        let mut evidence = Evidence::new(&set, false);
        let f = "Function       : static, [00005000], len = 00000010, f";
        let g = "Function       : static, [00005000], len = 00000010, g";
        evidence.add_functions(&lines(&[f, g]));
        evidence.last_public = Some(0x5000);
        let rvas = [0x5000, 0x5000, 0x5000];
        let public = |length: &str| {
            Some(lines(&[
                "Displacement = 0x0",
                "                          SymTag:\t 0xA",
                "          RelativeVirtualAddress:\t 0x5000",
                &format!("                          Length:\t {length}"),
            ]))
        };
        let omap = [
            Some(lines(&["Displacement = 0x1", f])),
            public("0x6"),
            public("0x6"),
        ];
        let clean = [
            Some(lines(&["Displacement = 0x1", g])),
            public("0x0"),
            public("0x2"),
        ];
        let mut tally = Tally::default();
        compare_answers("-sym", &rvas, (&omap, &clean), &evidence, &mut tally);
        assert_eq!(tally.explained.get(FOLDED), Some(&1));
        assert_eq!(tally.explained.get(LAST_PUBLIC), Some(&1));
        // Only the length 0 has the explanation.
        assert_eq!(tally.differences, 1, "{:?}", tally.shown);
    }

    #[test]
    fn sorted_dumps_report_each_difference() {
        let omap = lines(&[
            "PublicSymbol: [00004000][0001:00000010] b",
            "PublicSymbol: [00005000][0001:00000000] a",
            "PublicSymbol: [00000000][0001:00000040] removed",
        ]);
        let clean = lines(&[
            "PublicSymbol: [00005000][0002:00001000] a",
            "PublicSymbol: [00004004][0002:00000004] b",
        ]);
        let tally = dump("-p", &omap, &clean, false);
        assert_eq!(
            (tally.compared, tally.removed, tally.differences),
            (1, 1, 1)
        );
    }
}
