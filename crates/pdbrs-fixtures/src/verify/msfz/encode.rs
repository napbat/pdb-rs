//! The encode part of `verify-msfz`: MSF 7.0 to MSFZ with each option set.
//!
//! Two `DIA2Dump` results are explained and do not make a check fail:
//!
//! - DIA rejects each file with raw DEFLATE chunks (`HRESULT = 806D000C`). DIA reads only
//!   zstd chunks. `pdbtool` and pdbrs read the DEFLATE files.
//! - For some PDBs, DIA gives another output for each MSFZ file than for the MSF 7.0 file, also
//!   for the `pdz-encode` output. If the output for the pdbrs file is the same as for the
//!   `pdz-default` fixture of the same source, the difference comes from DIA.

use std::fs;
use std::path::{Path, PathBuf};

use pdbrs::native::container::Container;
use pdbrs::native::msfz::{Codec, WriteOptions};

use super::super::native::NativeFixture;
use super::super::native::consumers::{
    Run, StreamDifferences, dia2dump, parallel_map, pdbtool_compare,
};
use super::{
    OPTION_SETS, POLICY, Streams, Tools, available, describe, empty_stream_0, native_output,
    not_run, read_streams, same_output, stream_differences, write_file,
};

/// The text of the DIA error for a file that it does not read.
const DIA_FORMAT_ERROR: &str = "HRESULT = 806D000C";

/// The data of one source PDB that all option sets use.
struct Source {
    bytes: Vec<u8>,
    streams: Streams,
    /// The masked `DIA2Dump -all` output.
    dia: Result<Run, String>,
    /// The `native` example output without the container lines.
    native: Result<Run, String>,
    /// The `pdz-default` fixture of the source, if the manifest has it.
    pdz_encode: Option<PathBuf>,
}

/// The result of the `DIA2Dump -all` check.
enum Dia {
    /// The same output as for the source, with the line count.
    Same(usize),
    /// The same output as for the `pdz-encode` file of the source, with the line count, and
    /// the difference from the output for the source.
    SameAsPdzEncode(usize, String),
    /// DIA rejects the file, because it has raw DEFLATE chunks.
    DeflateRejected,
}

/// The results of one source PDB and one option set.
struct Report {
    /// The size and the chunk count of the PDZ file, or the write error.
    written: Result<(u64, u32), String>,
    deterministic: bool,
    compare: Result<StreamDifferences, String>,
    dia: Result<Dia, String>,
    streams: Result<(), String>,
    native: Result<usize, String>,
}

impl Report {
    /// Return `true` if every check passes.
    fn passed(&self) -> bool {
        self.written.is_ok()
            && self.deterministic
            && self
                .compare
                .as_ref()
                .is_ok_and(|differences| *differences == StreamDifferences::default())
            && self.dia.is_ok()
            && self.streams.is_ok()
            && self.native.is_ok()
    }

    /// Describe the results.
    fn summary(&self) -> String {
        let mut parts = vec![describe("written", &self.written, |(size, chunks)| {
            format!(
                "{size} bytes, {chunks} chunks, {}",
                if self.deterministic {
                    "deterministic"
                } else {
                    "NOT DETERMINISTIC"
                }
            )
        })];
        parts.push(describe("pdbtool compare", &self.compare, |differences| {
            if *differences == StreamDifferences::default() {
                "identical".to_owned()
            } else {
                format!(
                    "DIFFERENT: counts {:?}, streams {:?}",
                    differences.counts, differences.streams
                )
            }
        }));
        parts.push(describe("DIA -all", &self.dia, |dia| match dia {
            Dia::Same(lines) => format!("same ({lines} lines)"),
            Dia::SameAsPdzEncode(lines, difference) => format!(
                "same as for the pdz-encode file ({lines} lines); the MSF source differs \
                 for both: {difference}"
            ),
            Dia::DeflateRejected => {
                format!("rejects raw DEFLATE chunks ({DIA_FORMAT_ERROR}), as expected")
            }
        }));
        parts.push(describe("pdbrs streams", &self.streams, |()| {
            "equal".to_owned()
        }));
        parts.push(describe("native example", &self.native, |lines| {
            format!("same ({lines} lines)")
        }));
        parts.join("; ")
    }
}

/// Run the encode checks for each fixture and option set. Print one line for each pair.
///
/// `fixtures` is the fixture root with the `pdz-default` fixture. Return `false` if a check
/// fails.
pub(super) fn run(list: &[NativeFixture], fixtures: &Path, tools: &Tools<'_>, work: &Path) -> bool {
    println!(
        "== encode: {} native PDBs, {} option sets",
        list.len(),
        OPTION_SETS.len()
    );
    let sources = parallel_map(list, |fixture| source(fixture, fixtures, tools));
    let pairs: Vec<(usize, usize)> = (0..list.len())
        .flat_map(|fixture| (0..OPTION_SETS.len()).map(move |set| (fixture, set)))
        .collect();
    let reports = parallel_map(&pairs, |&(fixture, set)| {
        let directory = list[fixture].work_directory(work).join(OPTION_SETS[set].0);
        match &sources[fixture] {
            Ok(source) => check(
                &list[fixture],
                source,
                OPTION_SETS[set].1(),
                tools,
                &directory,
            ),
            Err(error) => Report::failed(error.clone()),
        }
    });
    let mut passed = 0;
    for (&(fixture, set), report) in pairs.iter().zip(&reports) {
        let pass = report.passed();
        passed += usize::from(pass);
        println!(
            "{} [{}]: {}; {}",
            list[fixture].label(),
            OPTION_SETS[set].0,
            if pass { "pass" } else { "FAIL" },
            report.summary()
        );
    }
    let count = |test: fn(&Dia) -> bool| {
        reports
            .iter()
            .filter(|report| report.dia.as_ref().is_ok_and(test))
            .count()
    };
    println!(
        "encode: {passed} of {} pass; DIA same as the source {}, same as pdz-encode {}, \
         DEFLATE rejected {}",
        pairs.len(),
        count(|dia| matches!(dia, Dia::Same(_))),
        count(|dia| matches!(dia, Dia::SameAsPdzEncode(..))),
        count(|dia| matches!(dia, Dia::DeflateRejected)),
    );
    passed == pairs.len()
}

impl Report {
    /// A report for a source that does not read or a PDZ file that does not write.
    fn failed(error: String) -> Self {
        Self {
            written: Err(error),
            deterministic: false,
            compare: not_run(),
            dia: not_run(),
            streams: not_run(),
            native: not_run(),
        }
    }
}

/// Read a source PDB and run the tools on it.
fn source(fixture: &NativeFixture, fixtures: &Path, tools: &Tools<'_>) -> Result<Source, String> {
    let bytes = fs::read(&fixture.pdb)
        .map_err(|error| format!("cannot read {}: {error}", fixture.pdb.display()))?;
    let streams = read_streams(&bytes)?;
    let dia = available(&tools.consumers.dia2dump)
        .and_then(|tool| dia2dump(tool, ["-all"], &fixture.pdb));
    let native = tools
        .example
        .map_err(Clone::clone)
        .and_then(|example| native_output(example, &fixture.pdb));
    let pdz_encode = fixtures
        .join("pdz-default")
        .join(&fixture.fixture)
        .join(fixture.file_name());
    Ok(Source {
        bytes,
        streams,
        dia,
        native,
        pdz_encode: pdz_encode.is_file().then_some(pdz_encode),
    })
}

/// Write the PDZ file of one option set twice and run the checks.
fn check(
    fixture: &NativeFixture,
    source: &Source,
    options: WriteOptions,
    tools: &Tools<'_>,
    directory: &Path,
) -> Report {
    if directory.exists()
        && let Err(error) = fs::remove_dir_all(directory)
    {
        return Report::failed(format!("cannot remove {}: {error}", directory.display()));
    }
    let (first, second) = match (
        encode(&source.bytes, options),
        encode(&source.bytes, options),
    ) {
        (Ok(first), Ok(second)) => (first, second),
        (Err(error), _) | (_, Err(error)) => return Report::failed(error),
    };
    // The file name keeps the `.pdb` extension, because DIA opens only that extension.
    let path: PathBuf = directory.join(fixture.file_name());
    if let Err(error) = write_file(&path, &first.0) {
        return Report::failed(error);
    }
    let compare = available(&tools.consumers.pdbtool)
        .and_then(|tool| pdbtool_compare(tool, &fixture.pdb, &path));
    let dia = match (&source.dia, available(&tools.consumers.dia2dump)) {
        (Ok(original), Ok(tool)) => dia_check(
            tool,
            original,
            &path,
            source.pdz_encode.as_deref(),
            options.codec() == Codec::Deflate,
            directory,
        ),
        (Err(error), _) => Err(error.clone()),
        (_, Err(error)) => Err(error),
    };
    let streams = read_streams(&first.0).and_then(|written| {
        if !empty_stream_0(&written) {
            return Err("stream 0 is not an empty stream".to_owned());
        }
        stream_differences(&source.streams, &written, 1).map_or(Ok(()), Err)
    });
    let native = match (&source.native, tools.example) {
        (Ok(original), Ok(example)) => native_output(example, &path)
            .and_then(|written| same_output([original, &written], directory, "native")),
        (Err(error), _) | (_, Err(error)) => Err(error.clone()),
    };
    Report {
        written: Ok((u64::try_from(first.0.len()).unwrap_or(u64::MAX), first.1)),
        deterministic: first.0 == second.0,
        compare,
        dia,
        streams,
        native,
    }
}

/// Run `DIA2Dump -all` on the PDZ file at `path` and compare the output with `original`.
///
/// If the outputs differ, compare the output with the output for the `pdz-encode` file of
/// the same source. With `deflate`, DIA must reject the file or give the same output.
fn dia_check(
    tool: &Path,
    original: &Run,
    path: &Path,
    pdz_encode: Option<&Path>,
    deflate: bool,
    directory: &Path,
) -> Result<Dia, String> {
    let written = dia2dump(tool, ["-all"], path)?;
    if deflate && written.code != Some(0) && written.text.contains(DIA_FORMAT_ERROR) {
        return Ok(Dia::DeflateRejected);
    }
    let difference = match same_output([original, &written], directory, "dia-all") {
        Ok(lines) => return Ok(Dia::Same(lines)),
        Err(difference) => difference,
    };
    let Some(reference) = pdz_encode else {
        return Err(difference);
    };
    let theirs = dia2dump(tool, ["-all"], reference)?;
    same_output([&theirs, &written], directory, "dia-all-pdz-encode")
        .map(|lines| Dia::SameAsPdzEncode(lines, difference.clone()))
        .map_err(|error| format!("{difference}; for the pdz-encode file: {error}"))
}

/// Write the container in `bytes` as MSFZ. Return the bytes and the chunk count.
pub(super) fn encode(bytes: &[u8], options: WriteOptions) -> Result<(Vec<u8>, u32), String> {
    let pdbrs = |error: pdbrs::Error| format!("pdbrs: {error}");
    let container = Container::read(bytes).map_err(pdbrs)?;
    let plan = container.to_msfz(options, POLICY).map_err(pdbrs)?;
    Ok((plan.to_vec().map_err(pdbrs)?, plan.chunk_count()))
}
