//! The explained differences of the clean `DIA2Dump` comparison.
//!
//! Each class has a positive condition. The check counts a difference as explained only if
//! the condition holds, and it prints the count of each class:
//!
//! - [`SEPARATED`]: a `Block` line of the clean output without an OMAP line. DIA shows an
//!   `S_SEPCODE` scope as a block, and the clean output adds one for each part of a split
//!   procedure after the first one.
//! - [`SEPARATED_PART`]: `-sym` at an address in a later part of a split procedure. DIA finds
//!   the procedure in the OMAP output, and the `S_SEPCODE` block of the part in the clean
//!   output. The clean block must hold the address, and its parent must be the procedure of
//!   the OMAP answer with the length of the first part.
//! - [`SPLIT`]: the lines are equal except for the length, the clean length is smaller and a
//!   range boundary cuts the extent of the OMAP line. The clean record describes a part.
//! - [`DEBUG_END`]: a `FuncDebugEnd` line at the end of its procedure. The OMAP output
//!   translates the address after the procedure through OMAP, so it gets the RVA of another
//!   range (or 0). The clean output keeps the offset in the procedure, so it gets the new end
//!   of the procedure.
//! - [`DEBUG_PART`]: a `FuncDebugStart` or `FuncDebugEnd` line of a split procedure whose
//!   old address is outside the first part. The clean output limits the debug start and end
//!   to the length of the first part, so the clean line gives the end of the first part.
//!   The OMAP line gives the new RVA of the old address in a later part.
//! - [`LAZY`]: in a PDB without type merging (`NOTM`), DIA stops the `-s` list of a module
//!   at the first symbol that it cannot read (a procedure whose type is in the object file).
//!   If the clean output removed that symbol, DIA lists more symbols of the module. The
//!   extra clean lines must end the list of the same module.
//! - [`FOLDED`]: `-sym` at the address of procedures that share their code (identical COMDAT
//!   folding). DIA selects one of them, and the selection changes with the record order.
//!   The two answers must name two procedures that the OMAP `-s` dump lists at that RVA.
//! - [`LAST_PUBLIC`]: DIA gives the length 0 to the public symbol with the highest RVA. The
//!   source PDBs of the linkers show the same behavior (for example
//!   `__arm64x_redirection_metadata` in `msvc-arm64ec-debug`). A section shift that moves a
//!   code section to the end of the image makes a code public symbol the last one.
//! - [`COFF_GROUP`]: a PGO phase line (`Phase Name: <phase>`) of one output only. DIA takes
//!   the phase from the COFF group `<section>$lp<phase>` that holds the procedure. A COFF
//!   group is a point: the clean group keeps its start and length, so after a
//!   reorder it holds other procedures. The source group must hold the old procedure start
//!   exactly if the OMAP output prints the line, and the clean group must hold the new
//!   procedure start exactly if the clean output prints it.

use std::collections::{BTreeMap, BTreeSet};

use super::super::super::ranges::RangeSet;
use super::super::Group;
use super::{first_rva, is_function, is_separated, length, procedure, property};

/// The class of a separated code block of the clean output without an OMAP line.
pub(super) const SEPARATED: &str = "a separated code block of the clean output has no OMAP line";

/// The class of a `-sym` answer in a later part of a split procedure.
pub(super) const SEPARATED_PART: &str = "-sym in a later part of a split procedure: the clean output finds the S_SEPCODE block of the part";

/// The class of a split record whose clean line gives the length of a part.
pub(super) const SPLIT: &str = "the clean output gives the length of a part of a split record";

/// The class of a `FuncDebugEnd` line at the end of its procedure.
pub(super) const DEBUG_END: &str =
    "a FuncDebugEnd at the end of its procedure: OMAP translates the address after the procedure";

/// The class of a debug start or end outside the first part of a split procedure.
pub(super) const DEBUG_PART: &str = "a FuncDebugStart or FuncDebugEnd outside the first part of a split procedure: the clean record gives the end of the first part";

/// The class of the extra symbols of a module list in a PDB without type merging.
pub(super) const LAZY: &str = "DIA lists more symbols of a /DEBUG:LAZY (NOTM) module after a removed symbol that it cannot read";

/// The class of a `-sym` answer at the address of folded procedures.
pub(super) const FOLDED: &str =
    "DIA selects another one of the folded procedures at the same address";

/// The class of the length of the public symbol with the highest RVA.
pub(super) const LAST_PUBLIC: &str =
    "DIA gives the length 0 to the public symbol with the highest RVA";

/// The class of a PGO phase line of a procedure that left or entered its COFF group.
pub(super) const COFF_GROUP: &str =
    "a COFF group is a point: the PGO phase line of a procedure that left or entered the group";

/// The facts that the explanations use, in addition to the two lines.
#[derive(Debug)]
pub(super) struct Evidence<'a> {
    /// The range set.
    pub set: &'a RangeSet,
    /// `true` for a PDB without type merging (feature `NOTM`, `link /DEBUG:LAZY`).
    pub lazy: bool,
    /// The normalized `Function` lines of the OMAP `-s` dump at each new RVA.
    pub functions: BTreeMap<u32, BTreeSet<String>>,
    /// The highest RVA of a public symbol in the clean `-p` dump.
    pub last_public: Option<u32>,
    /// The COFF groups of the source, with image RVAs of the source.
    pub source_groups: &'a [Group],
    /// The COFF groups of the clean output, with new RVAs.
    pub clean_groups: &'a [Group],
}

impl<'a> Evidence<'a> {
    /// Return the evidence of `set` without the facts of the dumps.
    pub(super) const fn new(set: &'a RangeSet, lazy: bool) -> Self {
        Self {
            set,
            lazy,
            functions: BTreeMap::new(),
            last_public: None,
            source_groups: &[],
            clean_groups: &[],
        }
    }

    /// Add the normalized `Function` lines of the OMAP `-s` dump.
    pub(super) fn add_functions<'l>(&mut self, lines: impl IntoIterator<Item = &'l String>) {
        for line in lines {
            if let Some((rva, _)) = procedure(line) {
                self.functions.entry(rva).or_default().insert(line.clone());
            }
        }
    }
}

/// The procedure of a `-s` scope or of a `-sym` answer: the new RVA, the clean length and the
/// OMAP length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Scope {
    /// The new RVA.
    pub start: u32,
    /// The length of the clean record.
    pub length: u64,
    /// The length of the OMAP record.
    pub omap_length: u64,
}

/// Return the scope of the normalized `Function` lines `omap` and `clean`.
pub(super) fn scope(omap: &str, clean: &str) -> Option<Scope> {
    let (start, length) = procedure(clean)?;
    let (omap_start, omap_length) = procedure(omap)?;
    (omap_start == start).then_some(Scope {
        start,
        length,
        omap_length,
    })
}

/// Return `true` if the normalized lines `omap` and `clean` differ only in a smaller clean
/// length, and a range boundary cuts the extent of the OMAP line. `start` is the new RVA of
/// the extent if the line has none.
pub(super) fn split_extent(omap: &str, clean: &str, start: Option<u32>, set: &RangeSet) -> bool {
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

/// Return the name of a `FuncDebugStart` or `FuncDebugEnd` line.
fn debug_name(line: &str) -> Option<&str> {
    let name = line.split([' ', ':']).next()?;
    (name == "FuncDebugStart" || name == "FuncDebugEnd").then_some(name)
}

/// Return `true` for a `FuncDebugEnd` line.
pub(super) fn is_debug_end(line: &str) -> bool {
    debug_name(line) == Some("FuncDebugEnd")
}

/// Return [`DEBUG_END`] or [`DEBUG_PART`] if the normalized lines `omap` and `clean` are
/// debug start or end lines of `scope` that differ only in the RVA, and the clean line gives
/// the end of the clean record.
///
/// - [`DEBUG_END`]: a `FuncDebugEnd` line. The OMAP line has the new RVA of the old address
///   after the procedure, which another range (or no range, RVA 0) gives.
/// - [`DEBUG_PART`]: the procedure splits, and the OMAP line has the new RVA of an old
///   address in the procedure after the first part.
pub(super) fn debug_bound(
    omap: &str,
    clean: &str,
    scope: Option<Scope>,
    set: &RangeSet,
) -> Option<&'static str> {
    let (scope, x, y) = (scope?, first_rva(omap)?, first_rva(clean)?);
    let name = debug_name(omap)?;
    if omap.replacen(&format!("[{x:08X}]"), "", 1) != clean.replacen(&format!("[{y:08X}]"), "", 1)
        || u64::from(y) != u64::from(scope.start) + scope.length
    {
        return None;
    }
    let old_start = u64::from(set.old(scope.start)?);
    let after = old_start + scope.omap_length;
    if name == "FuncDebugEnd"
        && u32::try_from(after).is_ok_and(|after| set.map(after).unwrap_or(0) == x)
    {
        return Some(DEBUG_END);
    }
    let offset = u64::from(set.old(x)?).checked_sub(old_start)?;
    (scope.length <= offset && offset < scope.omap_length).then_some(DEBUG_PART)
}

/// Return the number of the lines of `clean` from `j` that [`LAZY`] explains, or `None`.
///
/// The OMAP output must end the list of a module at `omap[i]`: an empty line before the next
/// module or at the end of the output. The clean output must end the same list later, before
/// the same next line.
pub(super) fn lazy_extra(omap: &[String], i: usize, clean: &[String], j: usize) -> Option<usize> {
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

/// Return `true` if the normalized `-sym` answers `omap` and `clean` select two folded
/// procedures (see [`FOLDED`]).
pub(super) fn folded(omap: &[String], clean: &[String], evidence: &Evidence<'_>) -> bool {
    let first = |answer: &[String]| answer.iter().find(|line| is_function(line)).cloned();
    let (Some(x), Some(y)) = (first(omap), first(clean)) else {
        return false;
    };
    let (Some(rva), Some(other)) = (first_rva(&x), first_rva(&y)) else {
        return false;
    };
    x != y
        && rva == other
        && evidence
            .functions
            .get(&rva)
            .is_some_and(|lines| lines.contains(&x) && lines.contains(&y))
}

/// Return the first `SymTag` value of an answer: the tag of the symbol at the address.
fn tag(answer: &[String]) -> Option<&str> {
    answer
        .iter()
        .find(|line| property(line) == Some("SymTag"))
        .and_then(|line| line.split_once(":\t"))
        .map(|(_, value)| value.trim())
}

/// Return `true` if the normalized `-sym` answers `omap` and `clean` at the new RVA `rva`
/// are the procedure and the `S_SEPCODE` block of a later part (see [`SEPARATED_PART`]).
pub(super) fn separated_part(rva: u32, omap: &[String], clean: &[String], set: &RangeSet) -> bool {
    let function = |answer: &[String]| answer.iter().find(|line| is_function(line)).cloned();
    let block = clean
        .iter()
        .find(|line| is_separated(line))
        .and_then(|line| Some((first_rva(line)?, length(line)?.2)));
    let (Some(x), Some(y), Some((block_start, block_length))) =
        (function(omap), function(clean), block)
    else {
        return false;
    };
    let (Some((start, length)), Some((_, omap_length))) = (procedure(&y), procedure(&x)) else {
        return false;
    };
    let inside = |start: u32, length: u64| u64::from(rva.wrapping_sub(start)) < length;
    // The old address must be in the OMAP procedure.
    let in_procedure = set
        .old(rva)
        .zip(set.old(start))
        .is_some_and(|(old, old_start)| {
            old.checked_sub(old_start)
                .is_some_and(|offset| u64::from(offset) < omap_length)
        });
    tag(omap) == Some("0x5")
        && tag(clean) == Some("0x6")
        && split_extent(&x, &y, None, set)
        && inside(block_start, block_length)
        && !inside(start, length)
        && in_procedure
}

/// Return `true` if the normalized lines `omap` and `clean` of the `-sym` answer `answer`
/// are `Length` lines that [`LAST_PUBLIC`] explains: the answer is a public symbol at the
/// highest public RVA, and only the clean length is 0.
pub(super) fn last_public(
    omap: &str,
    clean: &str,
    answer: (&[String], Option<u32>),
    evidence: &Evidence<'_>,
) -> bool {
    let (lines, rva) = answer;
    property(omap) == Some("Length")
        && length(omap).is_some_and(|(_, _, value)| value != 0)
        && length(clean).is_some_and(|(_, _, value)| value == 0)
        && tag(lines) == Some("0xA")
        && rva.is_some()
        && rva == evidence.last_public
}

/// Return the PGO phase of a phase line: `Phase Name: <phase>` in `-s`, and also the phase
/// property in `-sym`.
pub(super) fn phase(line: &str) -> Option<&str> {
    let line = line.trim_start();
    line.strip_prefix("Phase Name: ").or_else(|| {
        (property(line) == Some("phase this function is a member of for PGO multiphased builds"))
            .then(|| line.split_once(":\t").map(|(_, value)| value.trim()))
            .flatten()
    })
}

/// Return `true` if a group of `groups` with the name `<section>$lp<phase>` holds `rva`.
fn in_group(groups: &[Group], suffix: &[u8], rva: u32) -> bool {
    groups
        .iter()
        .any(|group| group.name.ends_with(suffix) && rva.wrapping_sub(group.start) < group.length)
}

/// Return `true` if [`COFF_GROUP`] explains a phase line `line` of the procedure at the new
/// RVA `start` that only one output prints. `omap` is `true` if the OMAP output prints it.
pub(super) fn group_phase(
    line: &str,
    start: Option<u32>,
    omap: bool,
    evidence: &Evidence<'_>,
) -> bool {
    let (Some(phase), Some(start)) = (phase(line), start) else {
        return false;
    };
    let Some(old) = evidence.set.old(start) else {
        return false;
    };
    let suffix = format!("$lp{phase}");
    let source = in_group(evidence.source_groups, suffix.as_bytes(), old);
    let clean = in_group(evidence.clean_groups, suffix.as_bytes(), start);
    // The output that prints the line must have the group that holds the procedure.
    source == omap && clean != omap
}

#[cfg(test)]
mod tests {
    use super::super::super::super::ranges::{Range, SetKind};
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
    fn a_debug_end_after_the_procedure_is_explained() {
        // The old address after the procedure at old 0x1000 (new 0x5000) is 0x1010, which
        // has the new RVA 0x4000.
        let scope = Some(Scope {
            start: 0x5000,
            length: 0x10,
            omap_length: 0x10,
        });
        let omap = "FuncDebugEnd   :   static, [00004000]";
        let clean = "FuncDebugEnd   :   static, [00005010]";
        assert_eq!(debug_bound(omap, clean, scope, &set()), Some(DEBUG_END));
        // Another RVA in the clean output is a difference.
        let other = "FuncDebugEnd   :   static, [0000500F]";
        assert_eq!(debug_bound(omap, other, scope, &set()), None);
        let start = "FuncDebugStart :   static, [00005010]";
        assert_eq!(debug_bound(omap, start, scope, &set()), None);
        assert_eq!(debug_bound(omap, clean, None, &set()), None);
        // The procedure at old 0x1010 (new 0x4000) ends at an old address without a range.
        let scope = Some(Scope {
            start: 0x4000,
            length: 0x10,
            omap_length: 0x10,
        });
        let omap = "FuncDebugEnd   :   static, [00000000]";
        let clean = "FuncDebugEnd   :   static, [00004010]";
        assert_eq!(debug_bound(omap, clean, scope, &set()), Some(DEBUG_END));
    }

    #[test]
    fn a_debug_bound_in_a_later_part_is_explained() {
        // The procedure at old 0x1000 has 0x20 bytes: the first part (0x10 bytes) moves to
        // 0x5000, the second part to 0x4000.
        let scope = Some(Scope {
            start: 0x5000,
            length: 0x10,
            omap_length: 0x20,
        });
        let clean = "FuncDebugStart :   static, [00005010]";
        // The old debug start 0x1014 is in the second part.
        let omap = "FuncDebugStart :   static, [00004004]";
        assert_eq!(debug_bound(omap, clean, scope, &set()), Some(DEBUG_PART));
        // An old debug start in the first part is a difference.
        let omap = "FuncDebugStart :   static, [00005004]";
        assert_eq!(debug_bound(omap, clean, scope, &set()), None);
        // The clean line must give the end of the first part.
        let omap = "FuncDebugStart :   static, [00004004]";
        let clean = "FuncDebugStart :   static, [0000500C]";
        assert_eq!(debug_bound(omap, clean, scope, &set()), None);
        // A procedure that does not split has no later part.
        let scope = Some(Scope {
            start: 0x5000,
            length: 0x10,
            omap_length: 0x10,
        });
        let clean = "FuncDebugStart :   static, [00005010]";
        assert_eq!(debug_bound(omap, clean, scope, &set()), None);
    }

    #[test]
    fn folded_procedures_must_share_the_omap_address() {
        let f = "Function       : static, [00005000], len = 00000010, f";
        let g = "Function       : static, [00005000], len = 00000010, g";
        let h = "Function       : static, [00005000], len = 00000010, h";
        let set = set();
        let mut evidence = Evidence::new(&set, false);
        let omap_dump = lines(&[f, g]);
        evidence.add_functions(&omap_dump);
        let answer = |line: &str| lines(&["Displacement = 0x1", "  Name:\t x", line]);
        assert!(folded(&answer(f), &answer(g), &evidence));
        // A procedure that the OMAP dump does not list at the address is a difference.
        assert!(!folded(&answer(f), &answer(h), &evidence));
        assert!(!folded(&answer(f), &answer(f), &evidence));
        let moved = "Function       : static, [00004000], len = 00000010, g";
        assert!(!folded(&answer(f), &answer(moved), &evidence));
    }

    #[test]
    fn a_later_part_answers_with_its_separated_block() {
        // The procedure at old 0x1000 has 0x20 bytes: the first part (0x10 bytes) moves to
        // 0x5000, the second part to 0x4000.
        let omap = lines(&[
            "Displacement = 0x14",
            "  SymTag:\t 0x5",
            "Function       : static, [00005000], len = 00000020, f",
        ]);
        let clean = lines(&[
            "Displacement = 0x4",
            "  SymTag:\t 0x6",
            "Block          : static, [00004000], len = 00000010, (none)",
            "Parent",
            "Function       : static, [00005000], len = 00000010, f",
        ]);
        assert!(separated_part(0x4004, &omap, &clean, &set()));
        // The block must hold the address, and the address must be outside the first part.
        assert!(!separated_part(0x5004, &omap, &clean, &set()));
        assert!(!separated_part(0x4014, &omap, &clean, &set()));
        // The parent must be the procedure of the OMAP answer.
        let other = clean
            .iter()
            .map(|line| line.replace(", f", ", g"))
            .collect::<Vec<_>>();
        assert!(!separated_part(0x4004, &omap, &other, &set()));
    }

    #[test]
    fn only_the_last_public_symbol_has_the_length_0() {
        let set = set();
        let mut evidence = Evidence::new(&set, false);
        evidence.last_public = Some(0x5000);
        let answer = lines(&["  SymTag:\t 0xA", "  Length:\t 0x0"]);
        let omap = "                          Length:\t 0x6";
        let clean = "                          Length:\t 0x0";
        assert!(last_public(omap, clean, (&answer, Some(0x5000)), &evidence));
        assert!(!last_public(
            omap,
            clean,
            (&answer, Some(0x4000)),
            &evidence
        ));
        assert!(!last_public(
            omap,
            "                          Length:\t 0x2",
            (&answer, Some(0x5000)),
            &evidence
        ));
        let function = lines(&["  SymTag:\t 0x5", "  Length:\t 0x0"]);
        assert!(!last_public(
            omap,
            clean,
            (&function, Some(0x5000)),
            &evidence
        ));
    }

    #[test]
    fn a_phase_line_needs_a_change_of_the_coff_group() {
        // The source group holds old 0x1000..0x1010. The clean group starts at the new RVA
        // of its old start (0x5000) and keeps its length.
        let source = [Group {
            name: b".text$lp00demo".to_vec(),
            start: 0x1000,
            length: 0x10,
        }];
        let clean = [Group {
            name: b".text$lp00demo".to_vec(),
            start: 0x5000,
            length: 0x10,
        }];
        let set = set();
        let mut evidence = Evidence::new(&set, false);
        evidence.source_groups = &source;
        evidence.clean_groups = &clean;
        let line = "                 Phase Name: 00demo";
        let property = "phase this function is a member of for PGO multiphased builds:\t 00demo";
        assert_eq!(phase(property), Some("00demo"));
        // The procedure at new 0x4000 (old 0x1010) is outside the source group.
        assert!(!group_phase(line, Some(0x4000), true, &evidence));
        // The procedure at new 0x5000 (old 0x1000) stays in the group.
        assert!(!group_phase(line, Some(0x5000), true, &evidence));
        assert!(!group_phase(line, Some(0x5000), false, &evidence));
        assert!(!group_phase(line, None, true, &evidence));
        // The source group holds old 0x1010, but the clean group does not hold new 0x4000.
        let source = [Group {
            name: b".text$lp00demo".to_vec(),
            start: 0x1000,
            length: 0x20,
        }];
        evidence.source_groups = &source;
        assert!(group_phase(line, Some(0x4000), true, &evidence));
        assert!(group_phase(property, Some(0x4000), true, &evidence));
        assert!(!group_phase(line, Some(0x4000), false, &evidence));
        assert!(!group_phase(
            "Phase Name: other",
            Some(0x4000),
            true,
            &evidence
        ));
    }
}
