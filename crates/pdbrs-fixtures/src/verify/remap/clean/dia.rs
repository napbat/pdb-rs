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
//! Explained differences:
//!
//! - [`SEPARATED`]: a `Block` line of the clean output without an OMAP line. DIA shows an
//!   `S_SEPCODE` scope as a block, and the clean output adds one for each part of a split
//!   procedure after the first one.
//! - [`SPLIT`]: the lines are equal except for the length, the clean length is smaller and a
//!   range boundary cuts the extent of the OMAP line. The clean record describes a part.
//! - [`DEBUG_END`]: a `FuncDebugEnd` line at the end of its procedure. The OMAP output
//!   translates the address after the procedure through OMAP, so it gets the RVA of another
//!   range (or 0). The clean output keeps the offset in the procedure, so it gets the new end
//!   of the procedure.
//! - [`LAZY`]: in a PDB without type merging (`NOTM`), DIA stops the `-s` list of a module
//!   at the first symbol that it cannot read (a procedure whose type is in the object file).
//!   If the clean output removed that symbol, DIA lists more symbols of the module. The
//!   extra clean lines must end the list of the same module.
//!
//! `DIA2Dump` stops at the first query without an answer (for example `-fpo` at an address
//! without frame data) and exits with -1. The check then counts that query as one without an
//! answer and runs the queries after it again.

use std::fmt::Write as _;
use std::iter;
use std::path::Path;

use super::super::super::native::consumers::dia2dump;
use super::super::dia::{OmapDumps, next_address, run};
use super::super::ranges::RangeSet;
use super::{Sample, Tally, covers};

/// The number of queries in one `DIA2Dump` run.
const QUERY_BATCH: usize = 500;

/// The exit code of `DIA2Dump` after a query without an answer: `wmain` returns -1.
const STOPPED: i32 = -1;

/// The class of a separated code block of the clean output without an OMAP line.
pub(super) const SEPARATED: &str = "a separated code block of the clean output has no OMAP line";

/// The class of a split record whose clean line gives the length of a part.
pub(super) const SPLIT: &str = "the clean output gives the length of a part of a split record";

/// The class of a `FuncDebugEnd` line at the end of its procedure.
pub(super) const DEBUG_END: &str =
    "a FuncDebugEnd at the end of its procedure: OMAP translates the address after the procedure";

/// The class of the extra symbols of a module list in a PDB without type merging.
pub(super) const LAZY: &str = "DIA lists more symbols of a /DEBUG:LAZY (NOTM) module after a removed symbol that it cannot read";

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

/// Return `true` for a `FuncDebugEnd` line.
fn is_debug_end(line: &str) -> bool {
    line.split([' ', ':']).next() == Some("FuncDebugEnd")
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

/// Return `true` if the normalized lines `omap` and `clean` differ only in a smaller clean
/// length, and a range boundary cuts the extent of the OMAP line. `start` is the new RVA of
/// the extent if the line has none.
fn split_extent(omap: &str, clean: &str, start: Option<u32>, set: &RangeSet) -> bool {
    let (Some((a0, a1, omap_length)), Some((b0, b1, clean_length))) = (length(omap), length(clean))
    else {
        return false;
    };
    if clean_length >= omap_length || omap[..a0] != clean[..b0] || omap[a1..] != clean[b1..] {
        return false;
    }
    let Some(old) = first_rva(omap).or(start).and_then(|new| set.old(new)) else {
        return false;
    };
    u32::try_from(omap_length).is_ok_and(|length| set.cuts(old, length))
}

/// Return `true` if the normalized lines `omap` and `clean` are `FuncDebugEnd` lines at the
/// end of `procedure` (new RVA and length) that differ only in the RVA. The clean line has
/// the end of the procedure. The OMAP line has the new RVA of the old address after the
/// procedure, which another range (or no range, RVA 0) gives.
fn debug_end(omap: &str, clean: &str, procedure: Option<(u32, u64)>, set: &RangeSet) -> bool {
    let (Some((start, length)), Some(x), Some(y)) = (procedure, first_rva(omap), first_rva(clean))
    else {
        return false;
    };
    if !is_debug_end(omap)
        || omap.replacen(&format!("[{x:08X}]"), "", 1)
            != clean.replacen(&format!("[{y:08X}]"), "", 1)
        || u64::from(y) != u64::from(start) + length
    {
        return false;
    }
    let after = set
        .old(start)
        .zip(u32::try_from(length).ok())
        .and_then(|(old, length)| old.checked_add(length));
    after.is_some_and(|after| set.map(after).unwrap_or(0) == x)
}

/// Count an equal line as compared if it has an address.
fn count(line: &str, tally: &mut Tally) {
    if first_rva(line).is_some() {
        tally.compared += 1;
    }
}

/// Return the number of the lines of `clean` from `j` that [`LAZY`] explains, or `None`.
///
/// The OMAP output must end the list of a module at `omap[i]`: an empty line before the next
/// module or at the end of the output. The clean output must end the same list later, before
/// the same next line.
fn lazy_extra(omap: &[String], i: usize, clean: &[String], j: usize) -> Option<usize> {
    if omap.get(i).is_none_or(|line| !line.is_empty()) {
        return None;
    }
    let next = omap.get(i + 1);
    if next.is_some_and(|line| !line.starts_with("** Module: ")) {
        return None;
    }
    (j..clean.len())
        .find(|&k| clean[k].is_empty() && clean.get(k + 1) == next)
        .map(|k| k - j)
}

/// Compare the normalized lines of `DIA2Dump -s` in order. `lazy` is `true` for a PDB without
/// type merging (see [`LAZY`]).
fn merge_ordered(
    command: &str,
    omap: &[String],
    clean: &[String],
    (set, lazy): (&RangeSet, bool),
    tally: &mut Tally,
) {
    let (mut i, mut j) = (0, 0);
    // The new RVA and length of the last equal `Function` line.
    let mut scope = None;
    loop {
        if lazy
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
                    scope = procedure(x);
                }
                i += 1;
                j += 1;
            }
            (_, Some(y)) if is_separated(y) => {
                tally.explain(SEPARATED);
                j += 1;
            }
            (Some(x), Some(y)) => {
                if split_extent(x, y, None, set) {
                    tally.compared += 1;
                    tally.explain(SPLIT);
                } else if debug_end(x, y, scope, set) {
                    tally.compared += 1;
                    tally.explain(DEBUG_END);
                } else {
                    tally.push(|| {
                        format!("DIA2Dump {command}: the clean output prints `{y}`, the OMAP output `{x}`")
                    });
                }
                if is_function(x) {
                    scope = None;
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
    set: &RangeSet,
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
            (Some(x), Some(y)) if split_extent(x, y, None, set) => {
                tally.compared += 1;
                tally.explain(SPLIT);
            }
            (x, y) => tally.push(|| {
                format!("DIA2Dump {command}: the clean output prints {y:?}, the OMAP output {x:?}")
            }),
        }
    }
}

/// Compare one symbol dump command of the OMAP output and the clean output. `lazy` is `true`
/// for a PDB without type merging.
fn compare_dump(
    command: &str,
    omap: &[String],
    clean: &[String],
    (set, lazy): (&RangeSet, bool),
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
        // The address after a procedure can have no range; `debug_end` compares that line.
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
        merge_ordered(command, &kept, &clean, (set, lazy), tally);
    } else {
        kept.sort_unstable();
        clean.sort_unstable();
        merge_sorted(command, &kept, &clean, set, tally);
    }
}

/// How `DIA2Dump` separates the answers of one query kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Split {
    /// An empty line ends each answer (`-l`, `-fpo`).
    EmptyLine,
    /// A `Displacement = ` line starts each answer (`-sym`).
    Displacement,
}

/// Split the output of a query run into the complete answers and the lines after them, or
/// return `None` for an output that does not have the form.
fn split_answers<'a>(
    lines: impl Iterator<Item = &'a str>,
    split: Split,
) -> Option<(Vec<Vec<String>>, Vec<String>)> {
    let mut answers = Vec::new();
    let mut answer: Vec<String> = Vec::new();
    match split {
        Split::EmptyLine => {
            for line in lines {
                if line.is_empty() {
                    answers.push(std::mem::take(&mut answer));
                } else {
                    answer.push(line.to_owned());
                }
            }
            Some((answers, answer))
        }
        Split::Displacement => {
            let mut started = false;
            for line in lines {
                if line.starts_with("Displacement = ") {
                    if started {
                        answers.push(std::mem::take(&mut answer));
                    }
                    started = true;
                } else if !started && !line.is_empty() {
                    return None;
                }
                if started {
                    answer.push(line.to_owned());
                }
            }
            if started {
                answers.push(answer);
            }
            Some((answers, Vec::new()))
        }
    }
}

/// Return the answers of the query `option <rva> extra...` for each RVA of `rvas` on `pdb`
/// (`None` for a query without an answer).
///
/// One `DIA2Dump` run takes up to [`QUERY_BATCH`] queries. A run that stops at a query
/// without an answer (exit code [`STOPPED`]) gives the complete answers of the queries
/// before it. The next run starts after that query.
fn query(
    context: &Context<'_>,
    query: (&str, &[&str], Split),
    rvas: &[u32],
    pdb: &Path,
) -> Result<Vec<Option<Vec<String>>>, String> {
    let (option, extra, split) = query;
    let mut answers = Vec::with_capacity(rvas.len());
    while answers.len() < rvas.len() {
        let start = answers.len();
        let batch = &rvas[start..rvas.len().min(start + QUERY_BATCH)];
        let arguments: Vec<String> = batch
            .iter()
            .flat_map(|rva| {
                iter::once(option.to_owned())
                    .chain(iter::once(format!("0{rva:X}")))
                    .chain(extra.iter().map(|&argument| argument.to_owned()))
            })
            .collect();
        let run = dia2dump(context.dia2dump, &arguments, pdb)?;
        match split_answers(run.text.lines(), split) {
            Some((parts, rest))
                if run.success() && rest.is_empty() && parts.len() == batch.len() =>
            {
                answers.extend(parts.into_iter().map(Some));
            }
            // The query after the complete answers has no answer. A stopped `-fpo` query
            // prints an error line.
            Some((parts, _)) if run.code == Some(STOPPED) && parts.len() < batch.len() => {
                answers.extend(parts.into_iter().map(Some));
                answers.push(None);
            }
            _ => {
                return Err(format!(
                    "DIA2Dump {option} with {} queries on {} gives an output without the answer form (exit {:?}): {}",
                    batch.len(),
                    pdb.display(),
                    run.code,
                    crate::step::last_lines(&run.text, 3)
                ));
            }
        }
    }
    Ok(answers)
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

/// Return the class of the explained differences of two normalized answers with the same
/// number of lines, or `None` if a difference has no explanation. [`SPLIT`] goes before
/// [`DEBUG_END`].
fn explain_answer(omap: &[String], clean: &[String], set: &RangeSet) -> Option<&'static str> {
    let start = answer_rva(omap);
    // The new RVA and length of the last equal `Function` line.
    let mut scope = None;
    let mut class = None;
    for (x, y) in omap.iter().zip(clean) {
        if x == y {
            if is_function(x) {
                scope = procedure(x);
            }
        } else if split_extent(x, y, start, set) {
            class = Some(SPLIT);
        } else if debug_end(x, y, scope, set) {
            class = class.or(Some(DEBUG_END));
        } else {
            return None;
        }
    }
    class
}

/// Compare the answers of the query `option` at each RVA of `rvas`.
fn compare_answers(
    option: &str,
    rvas: &[u32],
    omap: &[Option<Vec<String>>],
    clean: &[Option<Vec<String>>],
    set: &RangeSet,
    tally: &mut Tally,
) {
    let normalized = |answer: &Option<Vec<String>>| {
        answer.as_ref().map(|lines| {
            lines
                .iter()
                .filter_map(|line| normalize(line))
                .collect::<Vec<_>>()
        })
    };
    for ((rva, omap), clean) in rvas.iter().zip(omap).zip(clean) {
        tally.compared += 1;
        let (omap, clean) = (normalized(omap), normalized(clean));
        if omap == clean {
            continue;
        }
        if let (Some(x), Some(y)) = (&omap, &clean)
            && x.len() == y.len()
            && let Some(class) = explain_answer(x, y, set)
        {
            tally.explain(class);
            continue;
        }
        tally.push(|| {
            let (x, y) = match (&omap, &clean) {
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
    for (command, omap) in &dumps.symbols {
        let clean = run(context.dia2dump, &[(*command).to_owned()], context.clean)?;
        compare_dump(command, omap, &clean, (set, context.lazy), tally);
    }
    let lines = ("-l", &["1"][..], Split::EmptyLine);
    let rvas: Vec<u32> = dumps.lines.iter().map(|(rva, _)| *rva).collect();
    let omap: Vec<Option<Vec<String>>> = dumps
        .lines
        .iter()
        .map(|(_, answer)| Some(answer.clone()))
        .collect();
    let clean = query(context, lines, &rvas, context.clean)?;
    compare_answers("-l", &rvas, &omap, &clean, set, tally);
    let symbols = ("-sym", &[][..], Split::Displacement);
    let rvas: Vec<u32> = samples.iter().map(|sample| sample.new).collect();
    let omap = query(context, symbols, &rvas, context.omap)?;
    let clean = query(context, symbols, &rvas, context.clean)?;
    compare_answers("-sym", &rvas, &omap, &clean, set, tally);
    let frame_data = ("-fpo", &[][..], Split::EmptyLine);
    let rvas: Vec<u32> = samples
        .iter()
        .filter(|sample| covers(frames, sample.old))
        .map(|sample| sample.new)
        .collect();
    let omap = query(context, frame_data, &rvas, context.omap)?;
    let clean = query(context, frame_data, &rvas, context.clean)?;
    compare_answers("-fpo", &rvas, &omap, &clean, set, tally);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::super::ranges::{Range, SetKind};
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
    }

    #[test]
    fn rva_zero_in_a_section_is_unmapped() {
        assert!(unmapped("PublicSymbol: [00000000][0001:00000040] f"));
        assert!(!unmapped(
            "PublicSymbol: [00000000][0000:00000000] absolute"
        ));
        assert!(!unmapped("PublicSymbol: [00005000][0001:00000000] f"));
    }

    #[test]
    fn a_shorter_clean_length_of_a_split_record_is_explained() {
        let omap = "Function       : static, [00005000], len = 00000020, f";
        let clean = "Function       : static, [00005000], len = 00000010, f";
        assert!(split_extent(omap, clean, None, &set()));
        // A longer clean length or another name is a difference.
        assert!(!split_extent(clean, omap, None, &set()));
        assert!(!split_extent(
            omap,
            &clean.replace(", f", ", g"),
            None,
            &set()
        ));
        // The record at 0x4000 (old 0x1010) is not split.
        let omap = "Block : static, [00004000], len = 00000010, (none)";
        let clean = "Block : static, [00004000], len = 00000008, (none)";
        assert!(!split_extent(omap, clean, None, &set()));
        // A property length takes the start from the answer.
        let omap = "                          Length:\t 0x20";
        let clean = "                          Length:\t 0x10";
        assert!(split_extent(omap, clean, Some(0x5000), &set()));
        assert!(!split_extent(omap, clean, None, &set()));
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
        let mut tally = Tally::default();
        compare_dump("-s", &omap, &clean, (&set(), false), &mut tally);
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
        let mut tally = Tally::default();
        compare_dump("-s", &omap, &clean, (&set(), false), &mut tally);
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
        let mut tally = Tally::default();
        compare_dump("-s", &omap, &clean, (&set(), true), &mut tally);
        assert_eq!(tally.differences, 0, "{:?}", tally.shown);
        assert_eq!(tally.explained.get(LAZY), Some(&2));
        // Only a PDB without type merging has the class.
        let mut tally = Tally::default();
        compare_dump("-s", &omap, &clean, (&set(), false), &mut tally);
        assert!(tally.differences > 0);
        // Extra lines inside the list of a module are a difference.
        clean.swap(3, 5);
        let mut tally = Tally::default();
        compare_dump("-s", &omap, &clean, (&set(), true), &mut tally);
        assert!(tally.differences > 0);
    }

    #[test]
    fn a_debug_end_after_the_procedure_is_explained() {
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
        let mut tally = Tally::default();
        compare_dump("-s", &omap, &clean, (&set(), false), &mut tally);
        assert_eq!(tally.differences, 0, "{:?}", tally.shown);
        assert_eq!(tally.explained.get(DEBUG_END), Some(&1));
        // The old address after the procedure at old 0x1000 (new 0x5000) is 0x1010, which
        // has the new RVA 0x4000.
        let scope = Some((0x5000, 0x10));
        let omap = "FuncDebugEnd   :   static, [00004000]";
        assert!(debug_end(
            omap,
            "FuncDebugEnd   :   static, [00005010]",
            scope,
            &set()
        ));
        // Another RVA in the clean output is a difference.
        assert!(!debug_end(
            omap,
            "FuncDebugEnd   :   static, [0000500F]",
            scope,
            &set()
        ));
        assert!(!debug_end(
            omap,
            "FuncDebugStart :   static, [00005010]",
            scope,
            &set()
        ));
        assert!(!debug_end(
            omap,
            "FuncDebugEnd   :   static, [00005010]",
            None,
            &set()
        ));
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
        let mut tally = Tally::default();
        compare_dump("-p", &omap, &clean, (&set(), false), &mut tally);
        assert_eq!(
            (tally.compared, tally.removed, tally.differences),
            (1, 1, 1)
        );
    }

    #[test]
    fn answers_split_at_their_separators() {
        let text = "\tline 1 at [00001000][0001:00000000]\n\n\tline 2\n\tline 3\n\n";
        let (answers, rest) = split_answers(text.lines(), Split::EmptyLine).unwrap();
        assert_eq!(
            answers,
            [
                lines(&["\tline 1 at [00001000][0001:00000000]"]),
                lines(&["\tline 2", "\tline 3"])
            ]
        );
        assert_eq!(rest.len(), 0);
        // A stopped `-fpo` query prints an error line after the complete answers.
        let text = "\tframe 1\n\nERROR - DumpFPO() frameByRVA invalid RVA: 0x00001000\n";
        let (answers, rest) = split_answers(text.lines(), Split::EmptyLine).unwrap();
        assert_eq!(answers, [lines(&["\tframe 1"])]);
        assert_eq!(rest.len(), 1);
        let text = "Displacement = 0x1\nFunction\n\nParent\nDisplacement = 0x0\nData\n";
        let (answers, rest) = split_answers(text.lines(), Split::Displacement).unwrap();
        assert_eq!(answers.len(), 2);
        assert_eq!(answers[0].len(), 4);
        assert_eq!(rest.len(), 0);
        assert_eq!(
            split_answers("Function\n".lines(), Split::Displacement),
            None
        );
        assert_eq!(
            answer_rva(&lines(&["   RelativeVirtualAddress:\t 0x5000"])),
            Some(0x5000)
        );
    }
}
