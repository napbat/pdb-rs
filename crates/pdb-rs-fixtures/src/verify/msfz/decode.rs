//! The decode part of `verify-msfz`: each PDZ fixture (`pdz-*` and `msfz-*`) to MSF 7.0.

use std::fs;
use std::path::{Path, PathBuf};

use pdb_rs::native::container::Container;
use pdb_rs::native::msf::{Msf, MsfWriteOptions};

use super::super::native::consumers::{
    Run, StreamDifferences, dia2dump, normalize_pdbutil, pdbtool_check, pdbtool_compare,
    pdbutil_dump,
};
use super::super::outputs;
use super::{
    POLICY, Tools, available, describe, empty_stream_0, file_name, native_output, not_run,
    read_streams, same_output, stream_differences, write_file,
};

/// One PDZ fixture with its MSF 7.0 original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PdzFixture {
    /// The name of the PDZ fixture, for example `pdz-default` or `msfz-deflate`.
    pub fixture: String,
    /// The name of the fixture of the original, for example `msvc-x64-debug`.
    pub source: String,
    /// The path of the PDZ file.
    pub pdz: PathBuf,
    /// The path of the MSF 7.0 original.
    pub original: PathBuf,
}

impl PdzFixture {
    /// The name in the output: `<PDZ fixture>/<source fixture>/<file name>`.
    pub(crate) fn label(&self) -> String {
        format!("{}/{}/{}", self.fixture, self.source, file_name(&self.pdz))
    }
}

/// Read the `native-pdz` outputs of the manifest and find the original of each.
///
/// A PDZ fixture writes `<PDZ fixture>/<source fixture>/<file name>`. The original is the
/// `native-pdb` output of the source fixture with the same file name.
pub(crate) fn pdz_fixtures(fixtures: &Path) -> Result<Vec<PdzFixture>, String> {
    let outputs = outputs(fixtures)?;
    let mut found = Vec::new();
    for output in outputs.iter().filter(|output| output.kind == "native-pdz") {
        let source = output
            .path
            .parent()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{}: no source directory", output.path.display()))?;
        let name = file_name(&output.path);
        let original = outputs
            .iter()
            .find(|other| {
                other.fixture == source
                    && other.kind == "native-pdb"
                    && file_name(&other.path) == name
            })
            .ok_or_else(|| {
                format!(
                    "{}: the original is not in the manifest",
                    output.path.display()
                )
            })?;
        found.push(PdzFixture {
            fixture: output.fixture.clone(),
            source,
            pdz: output.path.clone(),
            original: original.path.clone(),
        });
    }
    Ok(found)
}

/// The results of one PDZ fixture.
struct Report {
    /// The size of the MSF 7.0 file, or the conversion error.
    converted: Result<u64, String>,
    deterministic: bool,
    check: Result<u32, String>,
    pdbutil: Result<usize, String>,
    dia: Result<usize, String>,
    streams: Result<(), String>,
    /// `pdbtool compare` of the original and the PDZ file.
    compare: Result<StreamDifferences, String>,
    /// The `native` example output of the PDZ file and of the original.
    native: Result<usize, String>,
}

impl Report {
    /// A report for a conversion that fails.
    fn failed(error: String) -> Self {
        Self {
            converted: Err(error),
            deterministic: false,
            check: not_run(),
            pdbutil: not_run(),
            dia: not_run(),
            streams: not_run(),
            compare: not_run(),
            native: not_run(),
        }
    }

    /// Return `true` if every check passes.
    fn passed(&self) -> bool {
        self.converted.is_ok()
            && self.deterministic
            && self.check == Ok(0)
            && self.pdbutil.is_ok()
            && self.dia.is_ok()
            && self.streams.is_ok()
            && self
                .compare
                .as_ref()
                .is_ok_and(|differences| *differences == StreamDifferences::default())
            && self.native.is_ok()
    }

    /// Describe the results.
    fn summary(&self) -> String {
        [
            describe("converted", &self.converted, |size| {
                format!(
                    "{size} bytes, {}",
                    if self.deterministic {
                        "deterministic"
                    } else {
                        "NOT DETERMINISTIC"
                    }
                )
            }),
            describe("pdbtool check", &self.check, |errors| {
                format!("{errors} errors")
            }),
            describe("llvm-pdbutil", &self.pdbutil, |lines| {
                format!("same as the original ({lines} lines)")
            }),
            describe("DIA -all", &self.dia, |lines| {
                format!("same as the original ({lines} lines)")
            }),
            describe("pdb-rs streams", &self.streams, |()| {
                "equal to the PDZ file and the original".to_owned()
            }),
            describe("pdbtool compare", &self.compare, |differences| {
                if *differences == StreamDifferences::default() {
                    "PDZ file identical to the original".to_owned()
                } else {
                    format!(
                        "DIFFERENT: counts {:?}, streams {:?}",
                        differences.counts, differences.streams
                    )
                }
            }),
            describe("native example", &self.native, |lines| {
                format!("PDZ file same as the original ({lines} lines)")
            }),
        ]
        .join("; ")
    }
}

/// Convert each PDZ fixture to MSF 7.0 and run the checks. Print one line for each fixture.
///
/// Return `false` if a check fails.
pub(super) fn run(list: &[PdzFixture], tools: &Tools<'_>, work: &Path) -> bool {
    println!("== decode: {} PDZ fixtures", list.len());
    let reports = tools.jobs.map(list, |fixture| {
        let directory = work.join(&fixture.fixture).join(&fixture.source);
        check(fixture, tools, &directory)
    });
    let mut passed = 0;
    for (fixture, report) in list.iter().zip(&reports) {
        let pass = report.passed();
        passed += usize::from(pass);
        println!(
            "{}: {}; {}",
            fixture.label(),
            if pass { "pass" } else { "FAIL" },
            report.summary()
        );
    }
    println!("decode: {passed} of {} pass", list.len());
    !list.is_empty() && passed == list.len()
}

/// Convert one PDZ fixture twice and run the checks.
fn check(fixture: &PdzFixture, tools: &Tools<'_>, directory: &Path) -> Report {
    let stem = Path::new(&file_name(&fixture.pdz))
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
    let directory = directory.join(stem);
    if directory.exists()
        && let Err(error) = fs::remove_dir_all(&directory)
    {
        return Report::failed(format!("cannot remove {}: {error}", directory.display()));
    }
    let pdz = match fs::read(&fixture.pdz) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Report::failed(format!("cannot read {}: {error}", fixture.pdz.display()));
        }
    };
    // MSFZ has no page size. The conversion uses the page size of the original, so the
    // `Block Size` lines of `llvm-pdbutil` can be equal.
    let converted = page_size(&fixture.original)
        .and_then(|size| Ok((decode(&pdz, size)?, decode(&pdz, size)?)));
    let (first, second) = match converted {
        Ok(pair) => pair,
        Err(error) => return Report::failed(error),
    };
    let path = directory.join(file_name(&fixture.pdz));
    if let Err(error) = write_file(&path, &first) {
        return Report::failed(error);
    }
    let check = available(&tools.consumers.pdbtool).and_then(|tool| pdbtool_check(tool, &path));
    let pdbutil = available(&tools.consumers.pdbutil).and_then(|tool| {
        let runs = [&fixture.original, &path].map(|pdb| {
            pdbutil_dump(tool, pdb).map(|run| Run {
                code: run.code,
                text: normalize_pdbutil(&run.text),
            })
        });
        let [original, converted] = runs;
        same_output([&original?, &converted?], &directory, "llvm")
    });
    let dia = available(&tools.consumers.dia2dump).and_then(|tool| {
        let [original, converted] =
            [&fixture.original, &path].map(|pdb| dia2dump(tool, ["-all"], pdb));
        same_output([&original?, &converted?], &directory, "dia-all")
    });
    let streams = streams(fixture, &pdz, &first);
    let compare = available(&tools.consumers.pdbtool)
        .and_then(|tool| pdbtool_compare(tool, &fixture.original, &fixture.pdz));
    let native = tools.example.map_err(Clone::clone).and_then(|example| {
        let original = native_output(example, &fixture.original)?;
        let pdz = native_output(example, &fixture.pdz)?;
        same_output([&original, &pdz], &directory, "native")
    });
    Report {
        converted: Ok(u64::try_from(first.len()).unwrap_or(u64::MAX)),
        deterministic: first == second,
        check,
        pdbutil,
        dia,
        streams,
        compare,
        native,
    }
}

/// Compare the streams of the MSF 7.0 output with the PDZ file (all streams) and with the
/// original (from stream 1).
fn streams(fixture: &PdzFixture, pdz: &[u8], converted: &[u8]) -> Result<(), String> {
    let converted = read_streams(converted)?;
    if let Some(difference) = stream_differences(&read_streams(pdz)?, &converted, 0) {
        return Err(format!("PDZ file: {difference}"));
    }
    let original = fs::read(&fixture.original)
        .map_err(|error| format!("cannot read {}: {error}", fixture.original.display()))?;
    if let Some(difference) = stream_differences(&read_streams(&original)?, &converted, 1) {
        return Err(format!("original: {difference}"));
    }
    if empty_stream_0(&converted) {
        Ok(())
    } else {
        Err("stream 0 is not an empty stream".to_owned())
    }
}

/// Convert the MSFZ container in `bytes` to MSF 7.0 with pages of `page_size` bytes.
fn decode(bytes: &[u8], page_size: u32) -> Result<Vec<u8>, String> {
    let pdb_rs = |error: pdb_rs::Error| format!("pdb-rs: {error}");
    let container = Container::read(bytes).map_err(pdb_rs)?;
    if container.msfz().is_none() {
        return Err("the PDZ fixture is not an MSFZ container".to_owned());
    }
    container
        .to_msf(MsfWriteOptions::default().with_page_size(page_size), POLICY)
        .and_then(|plan| plan.to_vec())
        .map_err(pdb_rs)
}

/// Return the page size of the MSF 7.0 file at `path`.
fn page_size(path: &Path) -> Result<u32, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    Msf::read(bytes.as_slice())
        .map(|msf| msf.block_size())
        .map_err(|error| format!("pdb-rs: {error}"))
}
