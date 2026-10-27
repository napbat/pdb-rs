//! The change and removal cases of `verify-edit`: one field change or one removal in each case.
//!
//! Each case edits the source with `IdentityPolicy::ContentDerived`, writes
//! `<fixture>.<case>.pdb` and its verifier dump `<fixture>.<case>.srm.txt`, and checks:
//!
//! 1. The decoded sections of the verifier dump are the expected change of the source
//!    sections, line for line. For a removed custom debug information value, the custom debug
//!    information blocks are compared sorted and without row IDs, as in step 3.
//! 2. The streams whose bytes are different from the source are exactly the streams that the
//!    plan report does not give as [`StreamChange::Copied`]. The stream headers come from the
//!    verifier dumps, so pdbrs code does not find the streams of this comparison.
//! 3. The same edit on a second editor gives the same bytes.
//!
//! A case that does not apply gives a `note:` line and no output.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use pdbrs::Bytes;
use pdbrs::io::ReadAt;
use pdbrs::portable::custom::CustomDebugKind;
use pdbrs::portable::debug::SequencePoints;
use pdbrs::portable::metadata::DocumentId;
use pdbrs::portable::{EditOptions, IdentityPolicy, Pdb, PdbEditor, StreamChange};

use super::expect::{self, field, keyword};
use super::{RENAME_SUFFIX, pdbrs, same, source, write_and_dump};
use crate::jobs::Jobs;
use crate::verify::Comparison;
use crate::verify::rebuild::yes;

/// The table number of `LocalConstant` in a metadata token.
const LOCAL_CONSTANT_TABLE: u32 = 0x34;

/// The edit cases in log order.
const CASES: [Case; 5] = [
    Case::SequencePointLine,
    Case::DocumentName,
    Case::LocalVariableName,
    Case::RemoveLocalConstant,
    Case::RemoveCustomDebugInformation,
];

/// One edit case.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Case {
    /// In method order, the first visible sequence point: start line + 1 and end line + 1.
    SequencePointLine,
    /// Document 1: the name gets the suffix `.edited`.
    DocumentName,
    /// In row order, the first local variable: the name gets the suffix `.edited`.
    LocalVariableName,
    /// In row order, the first local constant: removed.
    RemoveLocalConstant,
    /// In row order, the first custom debug information value whose kind is not embedded
    /// source: removed.
    RemoveCustomDebugInformation,
}

impl Case {
    /// Return the name of the case in the log and in the output file names.
    const fn name(self) -> &'static str {
        match self {
            Self::SequencePointLine => "sequence-point-line",
            Self::DocumentName => "document-name",
            Self::LocalVariableName => "local-variable-name",
            Self::RemoveLocalConstant => "remove-local-constant",
            Self::RemoveCustomDebugInformation => "remove-custom-debug-information",
        }
    }

    /// Return the reason why the case does not apply to a PDB.
    const fn note(self) -> &'static str {
        match self {
            Self::SequencePointLine => "the PDB has no visible sequence point",
            Self::DocumentName => "the PDB has no document 1",
            Self::LocalVariableName => "the PDB has no local variable",
            Self::RemoveLocalConstant => "the PDB has no local constant",
            Self::RemoveCustomDebugInformation => {
                "the PDB has no custom debug information value other than embedded source"
            }
        }
    }
}

/// The counts of the section `change and removal cases:`.
#[derive(Default)]
struct Counts {
    /// The cases that applied.
    applied: usize,
    /// The cases that did not apply.
    notes: usize,
    /// The cases with a failed check, and the fixtures with an error.
    problems: usize,
}

/// Run the cases on each comparison and print the section `change and removal cases:`.
///
/// Return `false` if a check fails or gives an error.
pub(super) fn run(jobs: &Jobs, assembly: &Path, comparisons: &[Comparison], out: &Path) -> bool {
    println!("change and removal cases:");
    let results = jobs.map(comparisons, |comparison| {
        let mut log = String::new();
        let result = fixture(assembly, comparison, out, &mut log);
        (log, result)
    });
    let mut total = Counts::default();
    for (comparison, (log, result)) in comparisons.iter().zip(results) {
        print!("{log}");
        match result {
            Ok(counts) => {
                total.applied += counts.applied;
                total.notes += counts.notes;
                total.problems += counts.problems;
            }
            Err(error) => {
                println!("{}: error: {error}", comparison.fixture);
                total.problems += 1;
            }
        }
    }
    println!(
        "change and removal section: portable PDBs: {}, cases: {}, applied: {}, notes: {}, problems: {}",
        comparisons.len(),
        comparisons.len() * CASES.len(),
        total.applied,
        total.notes,
        total.problems
    );
    total.problems == 0
}

/// Run each case on one Portable PDB. Write the results to `log`.
fn fixture(
    assembly: &Path,
    comparison: &Comparison,
    out: &Path,
    log: &mut String,
) -> Result<Counts, String> {
    let fixture = &comparison.fixture;
    let (original, theirs) = source(assembly, comparison)?;
    let pdb = Pdb::parse(&original).map_err(pdbrs)?;
    let lines = expect::decoded(&theirs);
    let _ = writeln!(log, "{fixture}:");
    let mut counts = Counts::default();
    for case in CASES {
        let name = case.name();
        let Some(Edited { bytes, reported }) = edited(&pdb, case)? else {
            let _ = writeln!(log, "  note: {name}: {}", case.note());
            counts.notes += 1;
            continue;
        };
        counts.applied += 1;
        let dump = write_and_dump(assembly, out, &format!("{fixture}.{name}"), &bytes)?;
        let expected = expected(case, &lines)?;
        let actual = expect::decoded(&dump);
        let mut passed = if case == Case::RemoveCustomDebugInformation {
            let records = same(
                log,
                &format!("{name}: decoded dump (without custom debug information)"),
                &expect::without_custom(&expected),
                &expect::without_custom(&actual),
            );
            let custom = same(
                log,
                &format!("{name}: custom debug information (sorted, without row IDs)"),
                &expect::custom_blocks(&expected),
                &expect::custom_blocks(&actual),
            );
            records && custom
        } else {
            same(log, &format!("{name}: decoded dump"), &expected, &actual)
        };

        let changed = changed_streams(&original, &theirs, &bytes, &dump)?;
        let agree = changed == reported;
        let _ = writeln!(
            log,
            "  {name}: streams with other bytes: {}; the report gives the same streams: {}",
            list(&changed),
            yes(agree)
        );
        if !agree {
            let _ = writeln!(log, "    the report gives: {}", list(&reported));
        }
        let again = edited(&pdb, case)?.is_some_and(|repeated| repeated.bytes == bytes);
        let _ = writeln!(
            log,
            "  {name}: the same edit again gives the same bytes: {}",
            yes(again)
        );
        passed &= agree && again;
        if !passed {
            counts.problems += 1;
        }
    }
    Ok(counts)
}

/// Return the stream names, or `none`.
pub(super) fn list(streams: &BTreeSet<String>) -> String {
    if streams.is_empty() {
        return "none".to_owned();
    }
    streams
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The output of an applied case.
struct Edited {
    /// The output bytes.
    bytes: Vec<u8>,
    /// The names of the streams that the plan report does not give as copied.
    reported: BTreeSet<String>,
}

/// Apply `case` to a new editor of `pdb` and plan. Return `None` if the case does not apply.
fn edited<S: ReadAt>(pdb: &Pdb<S>, case: Case) -> Result<Option<Edited>, String> {
    let mut editor = pdb.edit(IdentityPolicy::ContentDerived);
    if !apply(&mut editor, case).map_err(pdbrs)? {
        return Ok(None);
    }
    let plan = editor.plan(EditOptions::default()).map_err(pdbrs)?;
    let reported = plan
        .report()
        .streams()
        .iter()
        .filter(|(_, change)| *change != StreamChange::Copied)
        .map(|(stream, _)| stream.name().to_owned())
        .collect();
    let bytes = plan.to_vec().map_err(pdbrs)?;
    Ok(Some(Edited { bytes, reported }))
}

/// Add the suffix `.edited` to a name.
fn rename(name: &mut Bytes<'static>) {
    let mut bytes = name.to_vec();
    bytes.extend_from_slice(RENAME_SUFFIX);
    *name = bytes.into();
}

/// Apply `case` to the model. Return `false` if the case does not apply.
fn apply<S: ReadAt>(editor: &mut PdbEditor<'_, S>, case: Case) -> pdbrs::Result<bool> {
    match case {
        Case::SequencePointLine => {
            for method in editor.methods_mut()?.values_mut() {
                let Some(points) = method.decode()? else {
                    continue;
                };
                let signature = points.local_signature();
                let form = points.local_signature_form();
                let mut points = points.collect::<pdbrs::Result<Vec<_>>>()?;
                let Some(span) = points.iter_mut().find_map(|point| point.span.as_mut()) else {
                    continue;
                };
                span.start_line += 1;
                span.end_line += 1;
                *method = SequencePoints::encode_parts(signature, form, &points)?;
                return Ok(true);
            }
            Ok(false)
        }
        Case::DocumentName => Ok(editor
            .documents_mut()?
            .get_mut(DocumentId(1))
            .map(|document| rename(&mut document.name))
            .is_some()),
        Case::LocalVariableName => Ok(editor
            .local_scopes_mut()?
            .iter_mut()
            .find_map(|(_, scope)| scope.variables.first_mut())
            .map(|variable| rename(&mut variable.name))
            .is_some()),
        Case::RemoveLocalConstant => Ok(editor
            .local_scopes_mut()?
            .iter_mut()
            .find(|(_, scope)| !scope.constants.is_empty())
            .map(|(_, scope)| scope.constants.remove(0))
            .is_some()),
        Case::RemoveCustomDebugInformation => {
            let custom = editor.custom_debug_information_mut()?;
            let handle = custom
                .iter()
                .find(|(_, information)| information.kind != CustomDebugKind::EMBEDDED_SOURCE)
                .map(|(handle, _)| handle);
            Ok(handle.and_then(|handle| custom.remove(handle)).is_some())
        }
    }
}

/// Return the index of the first line that passes `test`.
fn position(lines: &[String], test: impl Fn(&str) -> bool) -> Result<usize, String> {
    lines
        .iter()
        .position(|line| test(line))
        .ok_or_else(|| "the source dump has no line for the change".to_owned())
}

/// Return the decimal number in the field `name` of a line.
fn number(line: &str, name: &str) -> Result<u32, String> {
    field(line, name)
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| format!("no number `{name}` in `{line}`"))
}

/// Return the hexadecimal number (`0x...`) in the field `name` of a line.
fn hex_number(line: &str, name: &str) -> Result<u32, String> {
    field(line, name)
        .and_then(|value| value.strip_prefix("0x"))
        .and_then(|value| u32::from_str_radix(value, 16).ok())
        .ok_or_else(|| format!("no hexadecimal number `{name}` in `{line}`"))
}

/// Return a line with the field `name` set to `value`.
fn with_field(line: &str, name: &str, value: &str) -> String {
    line.split(' ')
        .map(|part| {
            if part
                .strip_prefix(name)
                .and_then(|rest| rest.strip_prefix('='))
                .is_some()
            {
                format!("{name}={value}")
            } else {
                part.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Return the expected decoded lines of the output of `case`.
fn expected(case: Case, lines: &[String]) -> Result<Vec<String>, String> {
    let mut result = lines.to_vec();
    let suffix = String::from_utf8_lossy(RENAME_SUFFIX);
    match case {
        Case::SequencePointLine => {
            let index = position(&result, |line| {
                line.starts_with("sequence-point ") && line.contains(" start-line=")
            })?;
            result[index] = expect::shift_lines(&result[index], 1)?;
        }
        Case::DocumentName => {
            let index = position(&result, |line| line.starts_with("document id=1 "))?;
            result[index] = expect::renamed(&result[index], &suffix)?;
        }
        Case::LocalVariableName => {
            let index = position(&result, |line| keyword(line) == "scope-variable")?;
            result[index] = expect::renamed(&result[index], &suffix)?;
        }
        Case::RemoveLocalConstant => remove_constant(&mut result)?,
        Case::RemoveCustomDebugInformation => remove_custom(&mut result)?,
    }
    Ok(result)
}

/// Remove the first local constant from the lines. Give the later constants, the constant
/// count of its scope and the custom debug information parents of later constants their new
/// values.
fn remove_constant(lines: &mut Vec<String>) -> Result<(), String> {
    let index = position(lines, |line| keyword(line) == "scope-constant")?;
    let removed = lines.remove(index);
    let row = number(&removed, "id")?;
    let scope = field(&removed, "scope")
        .ok_or_else(|| format!("no scope in `{removed}`"))?
        .to_owned();
    for line in lines.iter_mut() {
        match keyword(line) {
            "scope-constant" => {
                let id = number(line, "id")?;
                if id > row {
                    *line = with_field(line, "id", &(id - 1).to_string());
                }
            }
            "scope" if field(line, "id") == Some(scope.as_str()) => {
                let count = number(line, "constants")?;
                *line = with_field(line, "constants", &(count - 1).to_string());
            }
            "cdi" => {
                let parent = hex_number(line, "parent")?;
                if parent >> 24 == LOCAL_CONSTANT_TABLE && parent & 0x00FF_FFFF >= row {
                    if parent & 0x00FF_FFFF == row {
                        return Err(format!("`{line}` names the removed constant"));
                    }
                    *line = with_field(line, "parent", &format!("{:#010x}", parent - 1));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Remove the first custom debug information value that is not embedded source, with its
/// detail lines. Give the later rows and the `cdi` field of the `source` lines their new
/// values.
fn remove_custom(lines: &mut Vec<String>) -> Result<(), String> {
    let index = position(lines, |line| {
        keyword(line) == "cdi" && field(line, "codec") != Some("embedded-source")
    })?;
    let row = number(&lines[index], "id")?;
    let text = row.to_string();
    lines.retain(|line| !(keyword(line).starts_with("cdi") && field(line, "id") == Some(&text)));
    for line in lines.iter_mut() {
        let name = match keyword(line) {
            "source" => "cdi",
            name if name.starts_with("cdi") => "id",
            _ => continue,
        };
        let value = number(line, name)?;
        if value == row {
            return Err(format!(
                "`{line}` names the removed custom debug information"
            ));
        }
        if value > row {
            *line = with_field(line, name, &(value - 1).to_string());
        }
    }
    Ok(())
}

/// Return the names of the streams whose bytes are different in the source and the output,
/// or that only one of them has. The stream headers come from the verifier dumps.
pub(super) fn changed_streams(
    source: &[u8],
    source_dump: &str,
    output: &[u8],
    output_dump: &str,
) -> Result<BTreeSet<String>, String> {
    let before = streams(source, source_dump)?;
    let after = streams(output, output_dump)?;
    Ok(before
        .keys()
        .chain(after.keys())
        .filter(|name| before.get(*name) != after.get(*name))
        .map(|name| (*name).to_owned())
        .collect())
}

/// Return the bytes of each stream that a verifier dump gives, by stream name.
fn streams<'a>(bytes: &'a [u8], dump: &'a str) -> Result<BTreeMap<&'a str, &'a [u8]>, String> {
    dump.lines()
        .filter(|line| keyword(line) == "stream")
        .map(|line| {
            let name = field(line, "name").ok_or_else(|| format!("no name in `{line}`"))?;
            let offset = usize::try_from(hex_number(line, "offset")?).map_err(|e| e.to_string())?;
            let size = usize::try_from(hex_number(line, "size")?).map_err(|e| e.to_string())?;
            let stream = offset
                .checked_add(size)
                .and_then(|end| bytes.get(offset..end))
                .ok_or_else(|| format!("the stream {name} is outside the PDB"))?;
            Ok((name, stream))
        })
        .collect()
}
