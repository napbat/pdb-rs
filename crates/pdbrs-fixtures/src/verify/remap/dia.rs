//! The `DIA2Dump` check of `verify-remap`.
//!
//! DIA translates each address of a PDB with OMAP to the new image. It prints the new RVA and
//! keeps the section and offset of the record. An address without a mapping gets the RVA 0.
//! The check runs the same `DIA2Dump` commands on the source and on the output, and compares
//! the outputs line by line. The expected output line is the source line with each RVA
//! replaced by the RVA that the ranges give ([`RangeSet::map`]), or 0 without a range:
//!
//! - Symbols: `DIA2Dump -p`, `-g` and `-s` give the public symbols, the global symbols and the
//!   symbols of each compiland. The check counts the procedures (`Function` and `Thunk`
//!   lines), the public symbols, the data symbols with an address and the other lines with an
//!   address (labels, blocks, COFF groups, call sites). A command that fails on the source
//!   gives a note; the check does not run it on the outputs.
//! - Lines: `DIA2Dump -l` on the source gives the start RVA of each line. For each start RVA
//!   `r` with a range, the check compares the answer of `-l <map(r)> 1` on the output with the
//!   answer of `-l <r> 1` on the source.
//!
//! An address appears as `[RRRRRRRR][SSSS:OOOOOOOO]`, or as `[0xSSSS:0xOOOOOOOO]  0xRRRRRRRR`
//! in the call site, heap allocation site and COFF group lines. The RVAs 0 and `0xFFFFFFFF`
//! mean "no address" and do not change.
//!
//! One class of differences has an explanation ([`EXPLAINED`]): in a PDB of
//! `link /DEBUG:LAZY`, DIA prints a procedure without a range with an empty name. The same
//! procedures have their names in the other range sets.

use std::fmt::{self, Write as _};
use std::path::Path;

use super::super::native::consumers::dia2dump;
use super::ranges::RangeSet;

/// The number of line queries in one `DIA2Dump` run. The command line of Windows has at most
/// 32767 characters.
const LINE_BATCH: usize = 1500;

/// The number of differences that the check prints for one output.
const SHOWN_DIFFERENCES: usize = 10;

/// The `DIA2Dump` outputs of a source PDB.
#[derive(Debug, Default)]
pub(super) struct Baseline {
    /// The symbol dump commands that `DIA2Dump` runs on the source, with their output lines.
    symbols: Vec<(&'static str, Vec<String>)>,
    /// The start RVA of each source line, with the answer of `-l <rva> 1`, sorted by RVA.
    lines: Vec<(u32, Vec<String>)>,
    /// `true` for a PDB without type merging (feature `NOTM`, `link /DEBUG:LAZY`).
    lazy: bool,
    /// The notes about the source: the symbol dump commands that fail on it.
    pub notes: Vec<String>,
}

/// The number of checked items of one or more outputs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Counts {
    /// The `Function` and `Thunk` lines.
    pub procedures: usize,
    /// The `PublicSymbol` lines.
    pub publics: usize,
    /// The `Data` lines with an address.
    pub data: usize,
    /// The other lines with an address.
    pub other: usize,
    /// The source lines with a new RVA.
    pub lines: usize,
    /// The addresses without a range: symbol addresses that DIA must print as RVA 0, and the
    /// source lines that the check does not query.
    pub unmapped: usize,
    /// The explained differences (see [`EXPLAINED`]).
    pub explained: usize,
}

/// The explained class of differences: in a PDB of `link /DEBUG:LAZY`, DIA prints a procedure
/// without a range with an empty name.
pub(super) const EXPLAINED: &str =
    "DIA prints a procedure without a range in a /DEBUG:LAZY (NOTM) PDB with an empty name";

impl Counts {
    /// Add the counts of `other`.
    pub(super) fn add(&mut self, other: &Self) {
        self.procedures += other.procedures;
        self.publics += other.publics;
        self.data += other.data;
        self.other += other.other;
        self.lines += other.lines;
        self.unmapped += other.unmapped;
        self.explained += other.explained;
    }
}

impl fmt::Display for Counts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "DIA procedures {}, publics {}, data {}, other {}, lines {}, unmapped {}, explained {}",
            self.procedures,
            self.publics,
            self.data,
            self.other,
            self.lines,
            self.unmapped,
            self.explained
        )
    }
}

/// The kind of a symbol line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A procedure or a thunk.
    Procedure,
    /// A public symbol.
    Public,
    /// A data symbol.
    Data,
    /// Another symbol with an address.
    Other,
}

/// Return the kind of a symbol line from its first word.
fn kind(line: &str) -> Kind {
    let word = line
        .trim_start()
        .split([' ', ':'])
        .next()
        .unwrap_or_default();
    match word {
        "Function" | "Thunk" => Kind::Procedure,
        "PublicSymbol" => Kind::Public,
        "Data" => Kind::Data,
        _ => Kind::Other,
    }
}

/// Parse `digits` hexadecimal digits at the start of `text`.
fn hex(text: &str, digits: usize) -> Option<u32> {
    let field = text.get(..digits)?;
    if field.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        u32::from_str_radix(field, 16).ok()
    } else {
        None
    }
}

/// Find the next address of `text`. Return the byte range of its RVA digits and the RVA.
fn next_address(text: &str) -> Option<(usize, u32)> {
    let mut from = 0;
    while let Some(found) = text[from..].find('[') {
        let start = from + found;
        let rest = &text[start..];
        // `[RRRRRRRR][SSSS:OOOOOOOO]`
        if rest.get(9..11) == Some("][")
            && rest.get(15..16) == Some(":")
            && rest.get(24..25) == Some("]")
            && hex(&rest[11..], 4).is_some()
            && hex(&rest[16..], 8).is_some()
            && let Some(rva) = hex(&rest[1..], 8)
        {
            return Some((start + 1, rva));
        }
        // `[0xSSSS:0xOOOOOOOO]  0xRRRRRRRR`
        if rest.get(1..3) == Some("0x")
            && rest.get(7..10) == Some(":0x")
            && rest.get(18..23) == Some("]  0x")
            && hex(&rest[3..], 4).is_some()
            && hex(&rest[10..], 8).is_some()
            && let Some(rva) = hex(&rest[23..], 8)
        {
            return Some((start + 23, rva));
        }
        from = start + 1;
    }
    None
}

/// Return the line that DIA prints for the output: `line` with each RVA translated by `set`,
/// and the number of addresses and of addresses without a range.
fn translate(line: &str, set: &RangeSet) -> (String, usize, usize) {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    let (mut addresses, mut unmapped) = (0, 0);
    while let Some((at, rva)) = next_address(rest) {
        out.push_str(&rest[..at]);
        let new = if rva == 0 || rva == u32::MAX {
            rva
        } else {
            addresses += 1;
            set.map(rva).unwrap_or_else(|| {
                unmapped += 1;
                0
            })
        };
        let _ = write!(out, "{new:08X}");
        rest = &rest[at + 8..];
    }
    out.push_str(rest);
    (out, addresses, unmapped)
}

/// Run `DIA2Dump` and return the lines of its output.
fn run(dia2dump_path: &Path, arguments: &[String], pdb: &Path) -> Result<Vec<String>, String> {
    let run = dia2dump(dia2dump_path, arguments, pdb)?;
    if !run.success() {
        return Err(format!(
            "DIA2Dump {} {} failed (exit {:?}): {}",
            arguments.first().map_or("", String::as_str),
            pdb.display(),
            run.code,
            crate::step::last_lines(&run.text, 3)
        ));
    }
    Ok(run.text.lines().map(str::to_owned).collect())
}

/// Return the answers of `-l <rva> 1` for each RVA of `rvas`, in order.
fn query_lines(dia2dump_path: &Path, pdb: &Path, rvas: &[u32]) -> Result<Vec<Vec<String>>, String> {
    let mut answers = Vec::with_capacity(rvas.len());
    for batch in rvas.chunks(LINE_BATCH) {
        let arguments: Vec<String> = batch
            .iter()
            .flat_map(|rva| ["-l".to_owned(), format!("{rva:X}"), "1".to_owned()])
            .collect();
        let mut answer = Vec::new();
        let before = answers.len();
        for line in run(dia2dump_path, &arguments, pdb)? {
            if line.is_empty() {
                answers.push(core::mem::take(&mut answer));
            } else {
                answer.push(line);
            }
        }
        if answers.len() - before != batch.len() || !answer.is_empty() {
            return Err(format!(
                "DIA2Dump gives {} answers for {} line queries on {}",
                answers.len() - before,
                batch.len(),
                pdb.display()
            ));
        }
    }
    Ok(answers)
}

/// The symbol dump commands: the public symbols, the global symbols and the symbols of each
/// compiland.
const SYMBOL_COMMANDS: [&str; 3] = ["-p", "-g", "-s"];

/// Run the `DIA2Dump` commands on the source PDB. `lazy` is `true` for a PDB without type
/// merging.
///
/// A symbol dump command that fails on the source gives a note, and the check does not run it
/// on the outputs.
pub(super) fn baseline(dia2dump_path: &Path, pdb: &Path, lazy: bool) -> Result<Baseline, String> {
    let mut symbols = Vec::new();
    let mut notes = Vec::new();
    for command in SYMBOL_COMMANDS {
        match run(dia2dump_path, &[command.to_owned()], pdb) {
            Ok(lines) => symbols.push((command, lines)),
            Err(error) => notes.push(format!(
                "the check does not compare `DIA2Dump {command}`, which fails on the source: {error}"
            )),
        }
    }
    let mut starts: Vec<u32> = run(dia2dump_path, &["-l".to_owned()], pdb)?
        .iter()
        .filter(|line| line.starts_with("\tline "))
        .filter_map(|line| next_address(line).map(|(_, rva)| rva))
        .filter(|&rva| rva != 0 && rva != u32::MAX)
        .collect();
    starts.sort_unstable();
    starts.dedup();
    let answers = query_lines(dia2dump_path, pdb, &starts)?;
    Ok(Baseline {
        symbols,
        lines: starts.into_iter().zip(answers).collect(),
        lazy,
        notes,
    })
}

/// Return `true` if `actual` is `expected` without the name at the end, for a procedure
/// without a range.
fn nameless(expected: &str, actual: &str) -> bool {
    kind(expected) == Kind::Procedure
        && expected.contains("[00000000]")
        && expected
            .rsplit_once(", ")
            .is_some_and(|(head, name)| !name.is_empty() && actual == format!("{head}, "))
}

/// The differences of one output.
#[derive(Debug, Default)]
struct Differences {
    /// The first differences, as text.
    shown: Vec<String>,
    /// The number of differences.
    count: usize,
}

impl Differences {
    /// Record a difference.
    fn push(&mut self, text: impl FnOnce() -> String) {
        if self.shown.len() < SHOWN_DIFFERENCES {
            self.shown.push(text());
        }
        self.count += 1;
    }

    /// Return the problems: the shown differences and a line for the others.
    fn problems(mut self) -> Vec<String> {
        if self.count > self.shown.len() {
            let more = self.count - self.shown.len();
            self.shown.push(format!("{more} more DIA2Dump differences"));
        }
        self.shown
    }
}

/// Compare the symbol dump of the output with the source dump.
fn check_symbols(
    baseline: &Baseline,
    set: &RangeSet,
    command: &str,
    source: &[String],
    output: &[String],
    counts: &mut Counts,
    differences: &mut Differences,
) {
    if output.len() != source.len() {
        differences.push(|| {
            format!(
                "DIA2Dump {command} prints {} lines for the output and {} for the source",
                output.len(),
                source.len()
            )
        });
        return;
    }
    for (source, actual) in source.iter().zip(output) {
        let (expected, addresses, unmapped) = translate(source, set);
        if addresses > 0 {
            match kind(source) {
                Kind::Procedure => counts.procedures += 1,
                Kind::Public => counts.publics += 1,
                Kind::Data => counts.data += 1,
                Kind::Other => counts.other += 1,
            }
            counts.unmapped += unmapped;
        }
        if expected == *actual {
            continue;
        }
        if baseline.lazy && nameless(&expected, actual) {
            counts.explained += 1;
        } else {
            differences.push(|| format!("DIA2Dump prints `{actual}`, expected `{expected}`"));
        }
    }
}

/// Compare the line answers of the output with the source answers.
fn check_lines(
    dia2dump_path: &Path,
    baseline: &Baseline,
    set: &RangeSet,
    path: &Path,
    counts: &mut Counts,
    differences: &mut Differences,
) -> Result<(), String> {
    let mut queries = Vec::new();
    let mut expected = Vec::new();
    for (rva, answer) in &baseline.lines {
        match set.map(*rva) {
            Some(new) => {
                queries.push(new);
                expected.push(
                    answer
                        .iter()
                        .map(|line| translate(line, set).0)
                        .collect::<Vec<_>>(),
                );
            }
            None => counts.unmapped += 1,
        }
    }
    let answers = query_lines(dia2dump_path, path, &queries)?;
    counts.lines += queries.len();
    for ((new, expected), actual) in queries.iter().zip(&expected).zip(&answers) {
        if expected != actual {
            differences.push(|| {
                format!(
                    "DIA2Dump -l {new:X} 1 prints {:?}, expected {:?}",
                    actual.first(),
                    expected.first()
                )
            });
        }
    }
    Ok(())
}

/// Run the `DIA2Dump` commands on the output at `path` of the range set `set`, and compare
/// them with the source. Return the counts and the problems.
pub(super) fn check(
    dia2dump_path: &Path,
    baseline: &Baseline,
    set: &RangeSet,
    path: &Path,
) -> Result<(Counts, Vec<String>), String> {
    let mut counts = Counts::default();
    let mut differences = Differences::default();
    for (command, source) in &baseline.symbols {
        let output = run(dia2dump_path, &[(*command).to_owned()], path)?;
        check_symbols(
            baseline,
            set,
            command,
            source,
            &output,
            &mut counts,
            &mut differences,
        );
    }
    check_lines(
        dia2dump_path,
        baseline,
        set,
        path,
        &mut counts,
        &mut differences,
    )?;
    Ok((counts, differences.problems()))
}

#[cfg(test)]
mod tests {
    use super::super::ranges::{Range, SetKind};
    use super::*;

    fn set() -> RangeSet {
        RangeSet {
            kind: SetKind::Reorder,
            sections: Vec::new(),
            ranges: vec![
                Range {
                    old: 0x1000,
                    length: 0x100,
                    new: 0x5000,
                },
                Range {
                    old: 0x2000,
                    length: 0x10,
                    new: 0x2100,
                },
            ],
        }
    }

    #[test]
    fn translates_both_address_forms() {
        let (line, addresses, unmapped) = translate(
            "Function       : static, [00001010][0001:00000010], len = 00000008, f",
            &set(),
        );
        assert_eq!(
            line,
            "Function       : static, [00005010][0001:00000010], len = 00000008, f"
        );
        assert_eq!((addresses, unmapped), (1, 0));
        let (line, addresses, _) = translate(
            "HeapAllocationSite:   [0x0002:0x0000000f]  0x0000200F  int",
            &set(),
        );
        assert_eq!(
            line,
            "HeapAllocationSite:   [0x0002:0x0000000f]  0x0000210F  int"
        );
        assert_eq!(addresses, 1);
    }

    #[test]
    fn an_address_without_a_range_becomes_zero() {
        let (line, addresses, unmapped) =
            translate("PublicSymbol: [00003000][0002:00001000] p", &set());
        assert_eq!(line, "PublicSymbol: [00000000][0002:00001000] p");
        assert_eq!((addresses, unmapped), (1, 1));
    }

    #[test]
    fn other_brackets_and_no_address_values_stay() {
        for line in [
            "Data           :   rsp Relative, [00000030], Local, Type: int, x",
            "PublicSymbol: [00000000][0000:00000000] absolute",
            "PublicSymbol: [FFFFFFFF][0000:00000000] none",
        ] {
            let (translated, addresses, _) = translate(line, &set());
            assert_eq!(translated, line);
            assert_eq!(addresses, 0);
        }
    }

    #[test]
    fn kinds_come_from_the_first_word() {
        assert_eq!(
            kind("Function: [00001000][0001:00000000] f"),
            Kind::Procedure
        );
        assert_eq!(
            kind("Thunk          : [000037B0][0001:000027B0], target f"),
            Kind::Procedure
        );
        assert_eq!(
            kind("PublicSymbol: [00001000][0001:00000000] f"),
            Kind::Public
        );
        assert_eq!(
            kind("Data           :   static, [00006430][0003:00000430], x"),
            Kind::Data
        );
        assert_eq!(
            kind("Label          :   static, [0000174C][0001:0000074C], l"),
            Kind::Other
        );
    }
}
