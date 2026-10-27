//! The consumer checks of one edited PDB.
//!
//! Each check runs an independent consumer on the edited PDB: `llvm-pdbutil`, `DIA2Dump`,
//! `pdbtool` and `pdbstr`. A check returns the problems that it found. An empty list is a
//! pass.

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::Path;

use pdbrs::native::Identity;

use super::super::native::consumers::{Run, dia2dump, pdbtool_check, pdbtool_compare, run};
use super::Tools;
use super::edits::{Edited, Expectation};

/// The results of `llvm-pdbutil dump --all` and `DIA2Dump -all` on a source PDB.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Baseline {
    /// The exit code of `llvm-pdbutil dump --all`.
    pub pdbutil: Option<i32>,
    /// The exit code of `DIA2Dump -all`.
    pub dia2dump: Option<i32>,
    /// The number of streams.
    pub streams: u32,
}

/// Run the full dumps on a source PDB.
pub(crate) fn baseline(tools: &Tools, pdb: &Path) -> Result<Baseline, String> {
    let bytes =
        std::fs::read(pdb).map_err(|error| format!("cannot read {}: {error}", pdb.display()))?;
    let streams = pdbrs::native::Pdb::parse(bytes.as_slice())
        .map_err(|error| format!("cannot read {}: {error}", pdb.display()))?
        .container()
        .stream_count();
    Ok(Baseline {
        pdbutil: pdbutil(tools, ["dump", "--all"], pdb)?.code,
        dia2dump: dia2dump(&tools.dia2dump, ["-all"], pdb)?.code,
        streams,
    })
}

/// Run `llvm-pdbutil` with `arguments` and then the PDB.
fn pdbutil<const N: usize>(tools: &Tools, arguments: [&str; N], pdb: &Path) -> Result<Run, String> {
    let mut all: Vec<&OsStr> = arguments.iter().map(OsStr::new).collect();
    all.push(pdb.as_os_str());
    run(&tools.pdbutil, all, None)
}

/// Run `llvm-pdbutil dump <section>` on a PDB and return the output.
pub(crate) fn pdbutil_text(tools: &Tools, section: &str, pdb: &Path) -> Result<String, String> {
    Ok(pdbutil(tools, ["dump", section], pdb)?.text)
}

/// The result of the checks of one edited PDB.
#[derive(Debug, Default)]
pub(crate) struct Findings {
    /// The problems without an explanation.
    pub problems: Vec<String>,
    /// The explained classes of differences from the source.
    pub explained: Vec<&'static str>,
}

/// Compare the exit code of a full dump of the edited PDB with the source.
fn compare_exit(
    tool: &str,
    edited: &Run,
    source: Option<i32>,
    findings: &mut Findings,
    classes: [&'static str; 2],
) {
    match (edited.code, source) {
        (Some(0), Some(0)) => {}
        (Some(0), _) => findings.explained.push(classes[0]),
        (code, source) if code == source => findings.explained.push(classes[1]),
        (code, source) => findings.problems.push(format!(
            "{tool} exit {code:?}, source {source:?}: {}",
            crate::step::last_lines(&edited.text, 2)
        )),
    }
}

/// Check one edited PDB at `path`. `source` is the path of the source PDB.
pub(crate) fn check(
    tools: &Tools,
    source: &Path,
    baseline: Baseline,
    path: &Path,
    edited: &Edited,
) -> Result<Findings, String> {
    let mut findings = Findings::default();
    let dump = pdbutil(tools, ["dump", "--all"], path)?;
    compare_exit(
        "llvm-pdbutil dump --all",
        &dump,
        baseline.pdbutil,
        &mut findings,
        [
            "llvm-pdbutil dump --all succeeds; it fails on the source",
            "llvm-pdbutil dump --all fails as on the source",
        ],
    );
    let dia = dia2dump(&tools.dia2dump, ["-all"], path)?;
    compare_exit(
        "DIA2Dump -all",
        &dia,
        baseline.dia2dump,
        &mut findings,
        [
            "DIA2Dump -all succeeds; it fails on the source",
            "DIA2Dump -all fails as on the source",
        ],
    );
    let problems = &mut findings.problems;
    let errors = pdbtool_check(&tools.pdbtool, path)?;
    if errors != 0 {
        problems.push(format!("pdbtool check: {errors} PDBs with errors"));
    }
    let differences = pdbtool_compare(&tools.pdbtool, source, path)?;
    let mut expected: Vec<u32> = edited
        .changed
        .iter()
        .copied()
        .filter(|&id| id < baseline.streams)
        .collect();
    expected.sort_unstable();
    let mut found = differences.streams.clone();
    found.sort_unstable();
    if found != expected {
        problems.push(format!(
            "pdbtool compare differs in streams {found:?}, the plan changed {expected:?}"
        ));
    }
    let added = edited
        .changed
        .iter()
        .filter(|&&id| id >= baseline.streams)
        .count();
    let counts = (added != 0).then(|| {
        (
            baseline.streams,
            baseline.streams + u32::try_from(added).unwrap_or(u32::MAX),
        )
    });
    if differences.counts != counts {
        problems.push(format!(
            "pdbtool compare stream counts {:?}, expected {counts:?}",
            differences.counts
        ));
    }
    if let Some((_, guid, age)) = edited.expect.identity
        && edited.identity != (Identity::Modern { guid, age })
    {
        problems.push("PdbWritePlan::identity differs from the new identity".to_owned());
    }
    lookups(tools, source, path, &edited.expect, &mut findings)?;
    Ok(findings)
}

/// Return `true` if `text` has a line with `` `name` `` (the `llvm-pdbutil` name form).
fn has_quoted(text: &str, name: &str) -> bool {
    let quoted = format!("`{name}`");
    text.lines().any(|line| line.contains(&quoted))
}

/// Return `true` if the `DIA2Dump -sym` output has the symbol `name`.
fn dia_has_symbol(text: &str, name: &str) -> bool {
    text.lines().any(|line| {
        line.trim_start()
            .strip_prefix("Name:")
            .is_some_and(|rest| rest.trim() == name)
    })
}

/// Return the names of the `DIA2Dump -p` or `-g` output: the text after the address and
/// before an undecorated name in parentheses.
fn dia_names(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|line| {
            let (_, rest) = line.rsplit_once("] ")?;
            Some(rest.split_once('(').map_or(rest, |(name, _)| name).trim())
        })
        .collect()
}

/// Return the GUID in the text form of `llvm-pdbutil`.
fn guid_text(guid: [u8; 16]) -> String {
    let [a0, a1, a2, a3, b0, b1, c0, c1, rest @ ..] = guid;
    let mut text = format!(
        "{{{:08X}-{:04X}-{:04X}-",
        u32::from_le_bytes([a0, a1, a2, a3]),
        u16::from_le_bytes([b0, b1]),
        u16::from_le_bytes([c0, c1])
    );
    for (index, byte) in rest.iter().enumerate() {
        if index == 2 {
            text.push('-');
        }
        let _ = write!(text, "{byte:02X}");
    }
    text.push('}');
    text
}

/// Run the independent lookups of `expect`. `source` is the source PDB.
fn lookups(
    tools: &Tools,
    source: &Path,
    path: &Path,
    expect: &Expectation,
    findings: &mut Findings,
) -> Result<(), String> {
    let problems = &mut findings.problems;
    if !expect.named_present.is_empty() || !expect.named_absent.is_empty() {
        let named = pdbutil(tools, ["dump", "--named-streams"], path)?.text;
        let present = |name: &str| named.lines().any(|line| line.trim() == name);
        for name in &expect.named_present {
            if !present(name) {
                problems.push(format!("llvm-pdbutil --named-streams has no {name}"));
            }
        }
        for name in &expect.named_absent {
            if present(name) {
                problems.push(format!("llvm-pdbutil --named-streams still has {name}"));
            }
        }
    }
    if let Some(expected) = &expect.source_server {
        let pdb = format!("-p:{}", path.display());
        let read = run(&tools.pdbstr, ["-r", pdb.as_str(), "-s:srcsrv"], None)?;
        let normalize = |text: &str| text.replace('\r', "").trim_end().to_owned();
        if normalize(&read.text) != normalize(expected) {
            problems.push(format!(
                "pdbstr -r -s:srcsrv reads other text: {}",
                crate::step::last_lines(&read.text, 2)
            ));
        }
    }
    symbol_lookups(tools, source, path, expect, findings)?;
    let problems = &mut findings.problems;
    for name in &expect.types {
        if !has_quoted(&pdbutil(tools, ["dump", "--types"], path)?.text, name) {
            problems.push(format!("llvm-pdbutil --types has no {name}"));
        }
        let dia = dia2dump(&tools.dia2dump, ["-type", name.as_str()], path)?.text;
        if !dia.contains(&format!("UserDefinedType: {name}")) {
            // A FASTLINK PDB and a managed PDB give DIA no TPI types.
            let types = dia2dump(&tools.dia2dump, ["-t"], source)?.text;
            if types.contains("UserDefinedType:") {
                problems.push(format!("DIA2Dump -type finds no {name}"));
            } else {
                findings
                    .explained
                    .push("DIA2Dump lists no UDT of the source PDB, so -type finds no added type");
            }
        }
    }
    let problems = &mut findings.problems;
    for string in &expect.ids {
        let ids = pdbutil(tools, ["dump", "--ids"], path)?.text;
        if !ids.contains(&format!("String: {string}")) {
            problems.push(format!("llvm-pdbutil --ids has no {string}"));
        }
    }
    if let Some((signature, guid, age)) = expect.identity {
        let summary = pdbutil(tools, ["dump", "--summary"], path)?.text;
        for line in [
            format!("Signature: {signature}"),
            format!("Age: {age}"),
            format!("GUID: {}", guid_text(guid)),
        ] {
            if !summary.lines().any(|text| text.trim() == line) {
                problems.push(format!("llvm-pdbutil --summary has no `{line}`"));
            }
        }
    }
    Ok(())
}

/// Return the number of symbols with the name `name` in a `DIA2Dump -sym` output.
fn dia_symbol_count(text: &str, name: &str) -> usize {
    text.lines()
        .filter(|line| {
            line.trim_start()
                .strip_prefix("Name:")
                .is_some_and(|rest| rest.trim() == name)
        })
        .count()
}

/// Look up the added, renamed and removed public and global names.
///
/// - `llvm-pdbutil --publics` and `--globals` list each present name and no removed name. If
///   the section fails on the source PDB too (`msvc-x64-fastlink`), the class is explained and
///   the lookup uses `DIA2Dump` only.
/// - `DIA2Dump -p` lists each present public name and no removed public name.
/// - `DIA2Dump -sym` finds each present global name. For a removed global name, it finds one
///   symbol less than on the source PDB: a module can have another symbol with the same name.
fn symbol_lookups(
    tools: &Tools,
    source: &Path,
    path: &Path,
    expect: &Expectation,
    findings: &mut Findings,
) -> Result<(), String> {
    for (section, public, present, absent) in [
        ("--publics", true, &expect.publics, &expect.publics_absent),
        ("--globals", false, &expect.globals, &expect.globals_absent),
    ] {
        if present.is_empty() && absent.is_empty() {
            continue;
        }
        let dump = pdbutil(tools, ["dump", section], path)?;
        let llvm = if dump.success() {
            Some(dump.text)
        } else if pdbutil(tools, ["dump", section], source)?.success() {
            findings
                .problems
                .push(format!("llvm-pdbutil {section} exit {:?}", dump.code));
            None
        } else {
            findings.explained.push(if public {
                "llvm-pdbutil --publics fails on the source too; DIA2Dump finds the names"
            } else {
                "llvm-pdbutil --globals fails on the source too; DIA2Dump finds the names"
            });
            None
        };
        let problems = &mut findings.problems;
        let names = if public {
            dia_names(&dia2dump(&tools.dia2dump, ["-p"], path)?.text)
                .into_iter()
                .map(str::to_owned)
                .collect()
        } else {
            Vec::new()
        };
        for name in present {
            if llvm.as_ref().is_some_and(|text| !has_quoted(text, name)) {
                problems.push(format!("llvm-pdbutil {section} has no {name}"));
            }
            if public && !names.contains(name) {
                problems.push(format!("DIA2Dump -p has no {name}"));
            }
            if !public {
                let symbol = dia2dump(&tools.dia2dump, ["-sym", name.as_str()], path)?.text;
                if !dia_has_symbol(&symbol, name) {
                    problems.push(format!("DIA2Dump -sym finds no {name}"));
                }
            }
        }
        for name in absent {
            if llvm.as_ref().is_some_and(|text| has_quoted(text, name)) {
                problems.push(format!("llvm-pdbutil {section} still has {name}"));
            }
            if public && names.contains(name) {
                problems.push(format!("DIA2Dump -p still has {name}"));
            }
            if !public {
                let before = dia2dump(&tools.dia2dump, ["-sym", name.as_str()], source)?.text;
                let after = dia2dump(&tools.dia2dump, ["-sym", name.as_str()], path)?.text;
                let (before, after) = (
                    dia_symbol_count(&before, name),
                    dia_symbol_count(&after, name),
                );
                if after + 1 != before {
                    problems.push(format!(
                        "DIA2Dump -sym finds {after} symbols {name}, {before} in the source"
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{dia_has_symbol, dia_names, guid_text};

    #[test]
    fn formats_the_guid_as_llvm_does() {
        let guid = [
            0x96, 0xa8, 0xcc, 0xeb, 0x6a, 0xd9, 0x0a, 0x47, 0xa8, 0xd7, 0x23, 0xa8, 0x29, 0x04,
            0x57, 0xba,
        ];
        assert_eq!(guid_text(guid), "{EBCCA896-D96A-470A-A8D7-23A8290457BA}");
    }

    #[test]
    fn reads_the_dia_names() {
        let text = "PublicSymbol: [00006610][0001:00005610] pub_name(pub_name)\n\
                    Data: [FFFFFFFF][0000:00000000] C_MODE\n";
        assert_eq!(dia_names(text), ["pub_name", "C_MODE"]);
        assert!(dia_has_symbol("    Name:\t C_MODE\n", "C_MODE"));
        assert!(!dia_has_symbol("    Name:\t C_MODE_2\n", "C_MODE"));
    }
}
