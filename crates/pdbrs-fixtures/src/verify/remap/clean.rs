//! The clean output check of `verify-remap` (decision 12 of #43).
//!
//! For each range set, the check remaps the source PDB with `PdbEditor::remap_addresses` and
//! `RemapMode::Clean`. It writes the output of `RemovedCode::Drop` to
//! `target/verify-remap/<fixture>/<PDB file stem>/<set>.clean.pdb` and does these checks:
//!
//! 1. The policies: the plan with `RemovedCode::Fail` fails exactly if the plan with
//!    `RemovedCode::Drop` removes records. The error is `UnsupportedEdit`, and its address has
//!    no range. Without removed records, both policies give the same bytes. If the plan fails
//!    for a record without a split form (decision 4), a range boundary must cut that record
//!    (see [`explain`]). The check then counts the set as "no split form" and does no other
//!    check.
//! 2. The unwind streams: for a source with `.pdata` or `.xdata`, the call without the
//!    streams of the new image fails with `UnsupportedEdit`. The check gives the streams of
//!    [`super::unwind`].
//! 3. The plan report: the output has no OMAP stream and no original section headers, slot 5
//!    has the section headers of the set, and the plan removes no stream. For the identity set
//!    of a source without OMAP and without removed records, the changed streams are the Info
//!    stream and the DBI stream.
//! 4. The pdbrs reader reads the address tables of the output: the section headers of the
//!    set and no OMAP. The ranges in reverse order give the same bytes.
//! 5. `pdbtool check` finds no error, and `llvm-pdbutil dump --all` exits as on the source.
//! 6. `DIA2Dump` gives the same answers for the clean output and the OMAP output (see
//!    [`dia`]).
//! 7. `SymbolLocator::locate` gives the same answers for both outputs (see [`locate`]).

mod dia;
mod explain;
mod locate;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use pdbrs::ErrorKind;
use pdbrs::native::address::{AddressTables, SectionOffset};
use pdbrs::native::dbi::DebugStream;
use pdbrs::native::debug_stream::{OmapStream, RvaVaBlob};
use pdbrs::native::locate::SymbolLocator;
use pdbrs::native::section::{Machine, SectionHeader, SectionHeaderStream};
use pdbrs::native::{
    EditOptions, IdentityPolicy, ImageRemap, Pdb, RemapMode, RemapReport, RemovedCode,
};

use super::super::native::consumers::{pdbtool_check, pdbutil_dump};
use super::dia::OmapDumps;
use super::ranges::{Procedure, RangeSet, SetKind};
use super::{Source, Tools, image_remap, unwind};

/// The first line of the clean part of the command output.
pub(super) const HEADER: &str =
    "clean output (RemapMode::Clean with RemovedCode::Drop and RemovedCode::Fail):";

/// The number of procedures with `DIA2Dump -sym` and `-fpo` samples in one output.
const DIA_PROCEDURES: usize = 100;

/// The number of differences that the check prints for one output.
const SHOWN_DIFFERENCES: usize = 10;

/// The facts of the source PDB that the clean check uses.
#[derive(Debug)]
pub(super) struct Facts {
    /// The machine of the DBI header.
    machine: Machine,
    /// The `.pdata` stream (slot 8).
    pdata: Option<RvaVaBlob>,
    /// The `.xdata` stream (slot 7).
    xdata: Option<RvaVaBlob>,
    /// The section headers that the addresses of the records use.
    pdb_sections: Vec<SectionHeader>,
    /// The OMAP table from the PDB addresses to the image, for a source with OMAP.
    from_source: Option<OmapStream>,
    /// The code extent (image RVA and length) of each procedure, sorted.
    procedures: Vec<(u32, u32)>,
    /// The code extent of each FPO and new FPO record, sorted.
    frames: Vec<(u32, u32)>,
    /// The code extent of each exception data record, sorted.
    exceptions: Vec<(u32, u32)>,
}

impl Facts {
    /// Read the facts of the source `pdb`.
    pub(super) fn read(
        pdb: &Pdb<&[u8]>,
        locator: &SymbolLocator,
        tables: &AddressTables,
    ) -> Result<Self, String> {
        let error = |error: pdbrs::Error| format!("pdbrs: {error}");
        let machine = pdb
            .debug_info()
            .map_err(error)?
            .map_or_else(Machine::default, |dbi| dbi.header().machine);
        let mut facts = Self {
            machine,
            pdata: pdb.debug_blob(DebugStream::Pdata).map_err(error)?,
            xdata: pdb.debug_blob(DebugStream::Xdata).map_err(error)?,
            pdb_sections: tables
                .original
                .as_ref()
                .unwrap_or(&tables.sections)
                .iter()
                .collect(),
            from_source: tables.from_source.clone(),
            procedures: Vec::new(),
            frames: Vec::new(),
            exceptions: Vec::new(),
        };
        for procedure in locator.procedures() {
            let procedure = procedure.map_err(|error| format!("procedures: {error}"))?;
            if let Some(start) = procedure.start {
                facts.procedures.push((start.0, procedure.length));
            }
        }
        let mut frames = Vec::new();
        if let Some(stream) = pdb.fpo().map_err(error)? {
            frames.extend(stream.iter().map(|record| (record.start, record.size)));
        }
        if let Some(stream) = pdb.frame_data().map_err(error)? {
            frames.extend(stream.iter().map(|record| (record.start, record.size)));
        }
        facts.frames = facts.extents(frames);
        if let Some(stream) = pdb.exception_data().map_err(error)? {
            let entries = stream
                .iter()
                .map(|entry| (entry.start, entry.end.saturating_sub(entry.start)))
                .collect();
            facts.exceptions = facts.extents(entries);
        }
        facts.procedures.sort_unstable();
        Ok(facts)
    }

    /// Translate PDB extents to image extents, and sort them.
    fn extents(&self, list: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
        let mut out: Vec<(u32, u32)> = list
            .into_iter()
            .filter_map(|(start, length)| Some((self.image_rva(start)?, length)))
            .collect();
        out.sort_unstable();
        out
    }

    /// Return the image RVA of the PDB RVA `rva`: the OMAP translation for a source with
    /// OMAP.
    fn image_rva(&self, rva: u32) -> Option<u32> {
        match &self.from_source {
            Some(table) => table.lookup(rva).ok()?.mapped(),
            None => Some(rva),
        }
    }

    /// Return the image RVA of a section-relative address of a record.
    fn section_rva(&self, address: SectionOffset) -> Option<u32> {
        let index = usize::from(address.section).checked_sub(1)?;
        let header = self.pdb_sections.get(index)?;
        self.image_rva(header.virtual_address.checked_add(address.offset)?)
    }

    /// Return `true` if the source has a `.pdata` or an `.xdata` stream.
    const fn has_unwind(&self) -> bool {
        self.pdata.is_some() || self.xdata.is_some()
    }
}

/// Return `true` if an extent of the sorted list `extents` holds `rva`.
fn covers(extents: &[(u32, u32)], rva: u32) -> bool {
    let end = extents.partition_point(|&(start, _)| start <= rva);
    extents[..end]
        .iter()
        .any(|&(start, length)| rva - start < length)
}

/// The results of the consumer tools on the source PDB.
#[derive(Debug)]
pub(super) struct Baseline {
    /// The exit code of `llvm-pdbutil dump --all`.
    pdbutil: Option<i32>,
}

/// Run the consumer tools on the source PDB at `pdb`.
pub(super) fn baseline(tools: &Tools, pdb: &Path) -> Result<Baseline, String> {
    Ok(Baseline {
        pdbutil: pdbutil_dump(&tools.pdbutil, pdb)?.code,
    })
}

/// The OMAP output of the same set, for the comparison.
#[derive(Clone, Copy, Debug)]
pub(super) struct OmapOutput<'a> {
    /// The output bytes.
    pub bytes: &'a [u8],
    /// The path of the output.
    pub path: &'a Path,
    /// The `DIA2Dump` outputs, if the OMAP check ran `DIA2Dump`.
    pub dumps: Option<&'a OmapDumps>,
}

/// One sampled address of a procedure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Sample {
    /// The old RVA.
    pub old: u32,
    /// The new RVA.
    pub new: u32,
}

/// Return the samples of each `step`-th procedure: the second byte, the middle and the last
/// byte. Count the samples without a range in `tally`.
fn samples(
    procedures: &[Procedure],
    set: &RangeSet,
    limit: usize,
    tally: &mut Tally,
) -> Vec<Sample> {
    let step = procedures.len().div_ceil(limit.max(1)).max(1);
    let mut out = Vec::new();
    for procedure in procedures.iter().step_by(step) {
        let last = procedure.start + (procedure.length - 1);
        let mut olds = [
            procedure.start + procedure.length.min(2) - 1,
            procedure.start + procedure.length / 2,
            last,
        ];
        olds.sort_unstable();
        let mut previous = None;
        for old in olds {
            if previous == Some(old) {
                continue;
            }
            previous = Some(old);
            match set.map(old) {
                Some(new) => out.push(Sample { old, new }),
                None => tally.removed += 1,
            }
        }
    }
    out
}

/// The counts and differences of the comparison of one clean output with its OMAP output.
#[derive(Debug, Default)]
pub(super) struct Tally {
    /// The compared items.
    pub compared: usize,
    /// The items of the OMAP output without a range, which the clean output removed.
    pub removed: usize,
    /// The explained differences of each class.
    pub explained: BTreeMap<&'static str, usize>,
    /// The first differences, as text.
    shown: Vec<String>,
    /// The number of differences.
    differences: usize,
}

impl Tally {
    /// Count an explained difference of `class`.
    pub(super) fn explain(&mut self, class: &'static str) {
        *self.explained.entry(class).or_default() += 1;
    }

    /// Record a difference.
    pub(super) fn push(&mut self, text: impl FnOnce() -> String) {
        if self.shown.len() < SHOWN_DIFFERENCES {
            self.shown.push(text());
        }
        self.differences += 1;
    }
}

/// The result of the clean check of one range set.
#[derive(Debug, Default)]
pub(super) struct SetResult {
    /// `true` if the check wrote the clean output.
    written: bool,
    /// The class of the plan failure, for a record without a split form.
    no_split: Option<String>,
    /// The removed records of each kind.
    removed_records: Vec<(String, usize)>,
    /// The added parts of each kind.
    added_parts: Vec<(String, usize)>,
    /// The number of contributions with new CRC values.
    cleared_crcs: usize,
    /// The comparison with the OMAP output.
    tally: Tally,
    /// The problems.
    problems: Vec<String>,
}

/// A planned clean output.
struct Planned {
    /// The output bytes.
    bytes: Vec<u8>,
    /// The output streams that the plan changed.
    changed: BTreeSet<u32>,
    /// The source streams that the plan removed.
    dropped: Vec<u32>,
    /// The report of the clean output.
    report: Option<RemapReport>,
}

/// Remap the source with `remap` in clean mode with `policy`.
fn plan(source: &Source, remap: &ImageRemap, policy: RemovedCode) -> pdbrs::Result<Planned> {
    let pdb = Pdb::parse(source.bytes.as_slice())?;
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor.remap_addresses(remap, RemapMode::Clean(policy))?;
    let plan = editor.plan(EditOptions::default())?;
    Ok(Planned {
        changed: plan.report().changed().map(|id| id.0).collect(),
        dropped: plan.report().dropped().map(|id| id.0).collect(),
        report: plan.report().remap().cloned(),
        bytes: plan.to_vec()?,
    })
}

/// Make the `ImageRemap` of a set with the unwind streams of the new image.
fn clean_remap(source: &Source, set: &RangeSet, reverse: bool) -> Result<ImageRemap, String> {
    let remap = image_remap(set, reverse)?;
    match (&source.facts.pdata, &source.facts.xdata) {
        (None, None) => Ok(remap),
        (Some(pdata), Some(xdata)) => {
            let (pdata, xdata) = unwind::unwind_streams(pdata, xdata, source.facts.machine, set)?;
            Ok(remap.with_unwind_streams(pdata, xdata))
        }
        _ => Err("the source has only one of the .pdata and .xdata streams".to_owned()),
    }
}

/// Check that clean output of a source with unwind streams fails without the streams of the
/// new image.
fn check_without_unwind(source: &Source, set: &RangeSet, result: &mut SetResult) {
    let outcome = image_remap(set, false).map(|remap| {
        Pdb::parse(source.bytes.as_slice()).and_then(|pdb| {
            pdb.edit(IdentityPolicy::IncrementAge)
                .remap_addresses(&remap, RemapMode::Clean(RemovedCode::Drop))
        })
    });
    match outcome {
        Ok(Err(error)) if error.kind() == ErrorKind::UnsupportedEdit => {}
        Ok(Err(error)) => result.problems.push(format!(
            "without unwind streams, remap_addresses fails with {:?}, not UnsupportedEdit: {error}",
            error.kind()
        )),
        Ok(Ok(())) => result.problems.push(
            "remap_addresses accepts a source with .pdata or .xdata without unwind streams"
                .to_owned(),
        ),
        Err(error) => result.problems.push(error),
    }
}

/// Check the outcome of the policy `RemovedCode::Fail` for an output of `RemovedCode::Drop`
/// with `removed` removed records.
fn check_fail(
    source: &Source,
    set: &RangeSet,
    drop: &Planned,
    removed: usize,
    fail: pdbrs::Result<Planned>,
    result: &mut SetResult,
) {
    match fail {
        Ok(fail) if removed == 0 => {
            if fail.bytes != drop.bytes {
                result.problems.push(
                    "RemovedCode::Fail and RemovedCode::Drop give other bytes without removed code"
                        .to_owned(),
                );
            }
        }
        Ok(_) => result.problems.push(format!(
            "RemovedCode::Fail gives an output, but RemovedCode::Drop removes {removed} records"
        )),
        Err(error) if removed == 0 => result.problems.push(format!(
            "RemovedCode::Fail fails, but RemovedCode::Drop removes no record: {error}"
        )),
        Err(error) => {
            if let Err(problem) = explain::removed_code(source, set, &error) {
                result.problems.push(problem);
            }
        }
    }
}

/// Plan the clean output with both policies and check them. Return the output of
/// `RemovedCode::Drop`, or `None` if the plan fails.
fn check_policies(
    source: &Source,
    set: &RangeSet,
    remap: &ImageRemap,
    result: &mut SetResult,
) -> Option<Planned> {
    let fail = plan(source, remap, RemovedCode::Fail);
    let drop = match plan(source, remap, RemovedCode::Drop) {
        Ok(drop) => drop,
        Err(error) => {
            match explain::classify(source, set, &error) {
                Ok(class) => {
                    match &fail {
                        Err(fail) if fail.kind() == ErrorKind::UnsupportedEdit => {}
                        Err(fail) => result.problems.push(format!(
                            "RemovedCode::Fail fails with {:?}, not UnsupportedEdit: {fail}",
                            fail.kind()
                        )),
                        Ok(_) => result.problems.push(format!(
                            "RemovedCode::Fail gives an output, but RemovedCode::Drop fails: {error}"
                        )),
                    }
                    result.no_split = Some(class);
                }
                Err(problem) => result.problems.push(problem),
            }
            return None;
        }
    };
    let Some(report) = &drop.report else {
        result
            .problems
            .push("the plan report of clean output has no remap report".to_owned());
        return Some(drop);
    };
    result.removed_records = report
        .removed_items()
        .map(|(item, count)| (explain::item_name(item), count))
        .collect();
    result.added_parts = report
        .added_items()
        .map(|(item, count)| (explain::item_name(item), count))
        .collect();
    result.cleared_crcs = report.cleared_crcs();
    let removed: usize = report.removed_items().map(|(_, count)| count).sum();
    let procedures = set.removed(&source.procedures);
    if procedures > 0 && removed == 0 {
        result.problems.push(format!(
            "the set removes {procedures} procedures, but RemovedCode::Drop removes no record"
        ));
    }
    check_fail(source, set, &drop, removed, fail, result);
    Some(drop)
}

/// Check the plan report and the address tables of a clean output.
fn check_report(source: &Source, set: &RangeSet, planned: &Planned, result: &mut SetResult) {
    let problems = &mut result.problems;
    let pdb = match Pdb::parse(planned.bytes.as_slice()) {
        Ok(pdb) => pdb,
        Err(error) => {
            problems.push(format!("pdbrs cannot read the clean output: {error}"));
            return;
        }
    };
    if let Ok(Some(dbi)) = pdb.debug_info() {
        for kind in [
            DebugStream::OmapToSource,
            DebugStream::OmapFromSource,
            DebugStream::OriginalSectionHeader,
        ] {
            if dbi.debug_stream(kind).is_some() {
                problems.push(format!(
                    "the clean output DBI stream names the slot {kind:?}"
                ));
            }
        }
    }
    match (
        pdb.address_tables(),
        SectionHeaderStream::from_records(&set.sections),
    ) {
        (Ok(Some(tables)), Ok(sections)) => {
            if tables.sections != sections {
                problems.push("the clean output section headers are not the set's".to_owned());
            }
            if tables.original.is_some()
                || tables.to_source.is_some()
                || tables.from_source.is_some()
            {
                problems.push("the clean output has OMAP or original section headers".to_owned());
            }
        }
        (Ok(None), _) => problems.push("the clean output has no section headers".to_owned()),
        (Err(error), _) | (_, Err(error)) => problems.push(format!("address tables: {error}")),
    }
    let with_omap = source.facts.from_source.is_some();
    if !with_omap && !planned.dropped.is_empty() {
        problems.push(format!(
            "the report lists removed streams {:?}",
            planned.dropped
        ));
    }
    let untouched = planned.report.as_ref().is_some_and(|report| {
        report.removed_items().next().is_none() && report.added_items().next().is_none()
    });
    if set.kind == SetKind::Identity && !with_omap && untouched {
        let expected = BTreeSet::from([1, 3]);
        if planned.changed != expected {
            problems.push(format!(
                "the clean identity changes the streams {:?}, not {expected:?}",
                planned.changed
            ));
        }
    }
}

/// Run the consumer tools on the clean output at `path`.
fn check_tools(tools: &Tools, baseline: &Baseline, path: &Path, result: &mut SetResult) {
    match pdbtool_check(&tools.pdbtool, path) {
        Ok(0) => {}
        Ok(errors) => result
            .problems
            .push(format!("pdbtool check: {errors} PDBs with errors")),
        Err(error) => result.problems.push(error),
    }
    match pdbutil_dump(&tools.pdbutil, path) {
        Ok(run) if run.code == baseline.pdbutil => {}
        Ok(run) => result.problems.push(format!(
            "llvm-pdbutil dump --all exits with {:?}, on the source with {:?}: {}",
            run.code,
            baseline.pdbutil,
            crate::step::last_lines(&run.text, 3)
        )),
        Err(error) => result.problems.push(error),
    }
}

/// Compare the clean output at `path` with the OMAP output.
fn compare(
    tools: &Tools,
    source: &Source,
    set: &RangeSet,
    omap: OmapOutput<'_>,
    clean: &[u8],
    path: &Path,
    result: &mut SetResult,
) {
    let tally = &mut result.tally;
    if let Some(dumps) = omap.dumps {
        // The SymbolLocator samples below count the samples without a range.
        let samples = samples(
            &source.procedures,
            set,
            DIA_PROCEDURES,
            &mut Tally::default(),
        );
        let context = dia::Context {
            dia2dump: &tools.dia2dump,
            set,
            omap: omap.path,
            clean: path,
        };
        if let Err(error) = dia::compare(&context, dumps, &samples, &source.facts.frames, tally) {
            result.problems.push(error);
        }
    } else {
        result
            .problems
            .push("the OMAP output has no DIA2Dump outputs to compare".to_owned());
    }
    let samples = samples(&source.procedures, set, usize::MAX, &mut result.tally);
    if let Err(error) = locate::compare(omap.bytes, clean, &samples, &mut result.tally) {
        result.problems.push(error);
    }
}

/// Remap one PDB with one range set in clean mode and check the output.
pub(super) fn check_set(
    tools: &Tools,
    source: &Source,
    baseline: &Baseline,
    set: &RangeSet,
    omap: Option<OmapOutput<'_>>,
    path: &Path,
) -> SetResult {
    let mut result = SetResult::default();
    let remap = match clean_remap(source, set, false) {
        Ok(remap) => remap,
        Err(error) => {
            result.problems.push(error);
            return result;
        }
    };
    if source.facts.has_unwind() {
        check_without_unwind(source, set, &mut result);
    }
    let Some(planned) = check_policies(source, set, &remap, &mut result) else {
        return result;
    };
    match clean_remap(source, set, true).and_then(|remap| {
        plan(source, &remap, RemovedCode::Drop).map_err(|error| error.to_string())
    }) {
        Ok(again) if again.bytes == planned.bytes => {}
        Ok(_) => result
            .problems
            .push("the ranges in reverse order give other clean output bytes".to_owned()),
        Err(error) => result.problems.push(format!("reverse order: {error}")),
    }
    check_report(source, set, &planned, &mut result);
    let _ = fs::remove_file(path);
    if let Err(error) = fs::write(path, &planned.bytes) {
        result
            .problems
            .push(format!("cannot write {}: {error}", path.display()));
        return result;
    }
    result.written = true;
    check_tools(tools, baseline, path, &mut result);
    match omap {
        Some(omap) => compare(tools, source, set, omap, &planned.bytes, path, &mut result),
        None => result
            .problems
            .push("no OMAP output to compare with".to_owned()),
    }
    result
}

/// Format the counts of each kind as `n (kind m, ...)`.
fn kinds(list: &[(String, usize)]) -> String {
    let total: usize = list.iter().map(|(_, count)| count).sum();
    if list.is_empty() {
        return "0".to_owned();
    }
    let mut text = format!("{total} (");
    for (index, (kind, count)) in list.iter().enumerate() {
        if index > 0 {
            text.push_str(", ");
        }
        let _ = write!(text, "{kind} {count}");
    }
    text.push(')');
    text
}

/// The totals of the clean part.
#[derive(Debug, Default)]
pub(super) struct Totals {
    /// The checked sets.
    outputs: usize,
    /// The written outputs.
    written: usize,
    /// The sets that fail for a record without a split form, by class.
    no_split: BTreeMap<String, usize>,
    /// The compared items.
    compared: usize,
    /// The removed items.
    removed: usize,
    /// The explained differences of each class.
    explained: BTreeMap<&'static str, usize>,
    /// The problems.
    pub problems: usize,
}

impl Totals {
    /// Print the clean results of one PDB and add them to the totals.
    pub(super) fn print_fixture(&mut self, label: &str, sets: &[(SetKind, SetResult)]) {
        println!("{label}:");
        for (kind, set) in sets {
            self.outputs += 1;
            let mut problems = set.problems.clone();
            if set.tally.differences > set.tally.shown.len() {
                problems.extend(set.tally.shown.iter().cloned());
                let more = set.tally.differences - set.tally.shown.len();
                problems.push(format!("{more} more differences"));
            } else {
                problems.extend(set.tally.shown.iter().cloned());
            }
            let status = if problems.is_empty() { "pass" } else { "FAIL" };
            let explained: usize = set.tally.explained.values().sum();
            if let Some(class) = &set.no_split {
                println!("  {}: {status}, {class}", kind.name());
                *self.no_split.entry(class.clone()).or_default() += 1;
            } else if set.written {
                println!(
                    "  {}: {status}, removed records {}, added parts {}, cleared CRCs {}, compared {}, removed {}, explained {explained}",
                    kind.name(),
                    kinds(&set.removed_records),
                    kinds(&set.added_parts),
                    set.cleared_crcs,
                    set.tally.compared,
                    set.tally.removed,
                );
                self.written += 1;
            } else {
                println!("  {}: {status}, no output", kind.name());
            }
            for (class, count) in &set.tally.explained {
                println!("    explained: {class}: {count}");
                *self.explained.entry(class).or_default() += count;
            }
            for problem in &problems {
                println!("    problem: {problem}");
            }
            self.compared += set.tally.compared;
            self.removed += set.tally.removed;
            self.problems += problems.len();
        }
    }

    /// Print the summary of the clean part.
    pub(super) fn print_summary(&self) {
        for (class, count) in &self.no_split {
            println!("explained (clean): {class}: {count} sets");
        }
        for (class, count) in &self.explained {
            println!("explained (clean): {class}: {count}");
        }
        let explained: usize = self.explained.values().sum();
        let no_split: usize = self.no_split.values().sum();
        println!(
            "clean outputs: {}, written {}, no split form {no_split}, compared {}, removed {}, explained {explained}, problems: {}",
            self.outputs, self.written, self.compared, self.removed, self.problems
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::ranges::Range;
    use super::*;

    #[test]
    fn samples_take_the_second_the_middle_and_the_last_byte() {
        let set = RangeSet {
            kind: SetKind::Removal,
            sections: Vec::new(),
            ranges: vec![Range {
                old: 0x1000,
                length: 0x100,
                new: 0x5000,
            }],
        };
        let procedure = |start, length| Procedure {
            start,
            length,
            removable: false,
        };
        let mut tally = Tally::default();
        let list = samples(
            &[
                procedure(0x1000, 0x10),
                procedure(0x10ff, 1),
                procedure(0x2000, 4),
            ],
            &set,
            usize::MAX,
            &mut tally,
        );
        let olds: Vec<u32> = list.iter().map(|sample| sample.old).collect();
        assert_eq!(olds, [0x1001, 0x1008, 0x100f, 0x10ff]);
        assert_eq!(list[0].new, 0x5001);
        // The procedure at 0x2000 has no range: 3 samples are removed.
        assert_eq!(tally.removed, 3);
        // A limit of 1 takes the first procedure only.
        let mut tally = Tally::default();
        assert_eq!(
            samples(
                &[procedure(0x1000, 0x10), procedure(0x1020, 0x10)],
                &set,
                1,
                &mut tally
            )
            .len(),
            3
        );
    }

    #[test]
    fn covers_finds_overlapping_extents() {
        let extents = [(0x1000, 0x10), (0x1008, 0x100), (0x2000, 4)];
        assert!(covers(&extents, 0x1000));
        assert!(covers(&extents, 0x1100));
        assert!(!covers(&extents, 0x1108));
        assert!(!covers(&extents, 0xfff));
        assert!(covers(&extents, 0x2003));
    }
}
