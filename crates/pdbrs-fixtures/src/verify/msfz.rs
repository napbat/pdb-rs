//! The `verify-msfz` command: MSFZ writing and conversion in both directions.
//!
//! 1. Encode (see [`encode`]): the command writes each native MSF 7.0 PDB of the fixture
//!    manifest as MSFZ with each option set of [`OPTION_SETS`], twice. Both outputs must be
//!    the same bytes. Then `pdbtool compare` must report no different stream, `DIA2Dump -all`
//!    must give the same masked output as for the source, the pdbrs reader must give the
//!    same streams (stream 0 is empty), and the `native` example must give the same output
//!    apart from the lines that start with `container:`.
//! 2. Reference (see [`reference`](mod@reference)): the command writes each `pdz-*` fixture source again
//!    with the pdbrs options that match the `pdz-encode` options of the fixture, and reports
//!    whether the bytes are the same. Different bytes do not make the command fail.
//! 3. Decode (see [`decode`]): the command converts each PDZ fixture (`pdz-*` of
//!    `pdbtool pdz-encode` and `msfz-*` of `msfz-encode`) to MSF 7.0, twice. Both outputs
//!    must be the same bytes. `pdbtool check` must report 0 errors, `llvm-pdbutil dump --all`
//!    and `DIA2Dump -all` must give the same result as for the original MSF 7.0 fixture, and
//!    the streams must be equal to the streams of the PDZ file and, from stream 1, of the
//!    original. `pdbtool compare` of the original and the PDZ file must report no different
//!    stream, and the `native` example must give the same output for both, apart from the
//!    lines that start with `container:`.
//!
//! The command writes the outputs and the different dumps to `target/verify-msfz/`.

mod decode;
mod encode;
mod reference;

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use pdbrs::native::container::Container;
use pdbrs::native::msf::{OldDirectoryPolicy, StreamId};
use pdbrs::native::msfz::{Codec, MIN_CHUNK_SIZE, MsfzWriteOptions};

use super::native::consumers::{Consumers, Run, run};
use super::native::diff::{Side, line_diff};
use super::native::native_fixtures;
use crate::jobs::Jobs;
use crate::verify::SHOWN_DIFFERENCES;

/// A named option set: a name and a function that gives the options.
pub(crate) type OptionSet = (&'static str, fn() -> MsfzWriteOptions);

/// The option sets of the encode checks.
///
/// The names match the `pdz-*` fixtures of the same options. `small-chunks` uses the smallest
/// chunk size (4 KiB), as the `pdz-small-chunks` fixture does.
pub(crate) const OPTION_SETS: [OptionSet; 6] = [
    ("default", MsfzWriteOptions::default),
    ("compress-stream-dir", || {
        MsfzWriteOptions::default().with_directory_codec(Some(Codec::Zstd))
    }),
    ("one-stream-per-chunk", || {
        MsfzWriteOptions::default().with_one_stream_per_chunk(true)
    }),
    ("small-chunks", || {
        MsfzWriteOptions::default().with_max_chunk_size(MIN_CHUNK_SIZE)
    }),
    ("pad16k", || MsfzWriteOptions::default().with_pad_16k(true)),
    ("deflate", || {
        MsfzWriteOptions::default().with_codec(Codec::Deflate)
    }),
];

/// The largest stream that the comparisons load (1 GiB).
const MAX_STREAM: usize = 1 << 30;

/// The streams of a container: the bytes of each stream, or `None` for a nil stream.
pub(crate) type Streams = Vec<Option<Vec<u8>>>;

/// Run the three parts of the command.
///
/// Return `false` if a check of the encode or decode part fails.
pub fn verify_msfz(workspace: &Path, fixtures: &Path, jobs: &Jobs) -> Result<bool, String> {
    let work = workspace.join("target").join("verify-msfz");
    fs::create_dir_all(&work)
        .map_err(|error| format!("cannot make {}: {error}", work.display()))?;
    let (list, skipped) = native_fixtures(fixtures)?;
    if list.is_empty() {
        return Err(format!(
            "{} has no native PDB outputs",
            fixtures.join("manifest.json").display()
        ));
    }
    let consumers = Consumers::locate(workspace, fixtures);
    for (name, reason) in consumers.missing() {
        println!("tool {name} is missing, its checks fail: {reason}");
    }
    let example = native_example(workspace);
    if let Err(error) = &example {
        println!("the native example is missing, its checks fail: {error}");
    }
    for entry in &skipped {
        println!("{}: skipped: {}", entry.fixture, entry.reason);
    }
    let tools = Tools {
        consumers: &consumers,
        example: example.as_deref(),
        jobs,
    };
    let encoded = encode::run(&list, fixtures, &tools, &work.join("encode"));
    let pdz = decode::pdz_fixtures(fixtures)?;
    reference::run(&pdz, jobs);
    let decoded = decode::run(&pdz, &tools, &work.join("decode"));
    println!("the results are in {}", work.display());
    Ok(encoded && decoded)
}

/// The tools of the checks.
pub(crate) struct Tools<'a> {
    pub consumers: &'a Consumers,
    /// The `native` example of pdbrs.
    pub example: Result<&'a Path, &'a String>,
    /// The worker budget.
    pub jobs: &'a Jobs,
}

/// Build the `native` example of pdbrs and return its path.
fn native_example(workspace: &Path) -> Result<PathBuf, String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(&cargo)
        .args(["build", "--quiet", "-p", "pdbrs", "--example", "native"])
        .current_dir(workspace)
        .output()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo build of the native example failed: {}",
            crate::step::last_lines(&String::from_utf8_lossy(&output.stderr), 3)
        ));
    }
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| workspace.join("target"), PathBuf::from);
    let path = target
        .join("debug")
        .join("examples")
        .join(format!("native{}", std::env::consts::EXE_SUFFIX));
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("{} does not exist", path.display()))
    }
}

/// Run the `native` example on `pdb` and remove the lines that start with `container:`.
pub(crate) fn native_output(example: &Path, pdb: &Path) -> Result<Run, String> {
    let mut output = run(example, [pdb.as_os_str()], None)?;
    let mut text = String::with_capacity(output.text.len());
    for line in output.text.lines() {
        if !line.starts_with("container:") {
            text.push_str(line);
            text.push('\n');
        }
    }
    output.text = text;
    Ok(output)
}

/// Read all streams of the container in `bytes`.
pub(crate) fn read_streams(bytes: &[u8]) -> Result<Streams, String> {
    let pdbrs = |error: pdbrs::Error| format!("pdbrs: {error}");
    let container = Container::read(bytes).map_err(pdbrs)?;
    (0..container.stream_count())
        .map(|id| {
            container
                .stream(StreamId(id))
                .and_then(|stream| stream.map(|s| s.read_all(MAX_STREAM)).transpose())
                .map_err(pdbrs)
        })
        .collect()
}

/// Describe the streams that differ between `left` and `right`, from stream `first`.
///
/// Return `None` if the stream counts are equal and each stream from `first` is equal.
pub(crate) fn stream_differences(left: &Streams, right: &Streams, first: usize) -> Option<String> {
    if left.len() != right.len() {
        return Some(format!("stream counts {} and {}", left.len(), right.len()));
    }
    let different: Vec<usize> = (first..left.len())
        .filter(|&id| left[id] != right[id])
        .collect();
    (!different.is_empty()).then(|| format!("streams {different:?} differ"))
}

/// Return `true` if stream 0 is present and empty, as [`OldDirectoryPolicy::Empty`] writes it.
pub(crate) fn empty_stream_0(streams: &Streams) -> bool {
    streams.first() == Some(&Some(Vec::new()))
}

/// The stream 0 policy of the conversions of the command.
pub(crate) const POLICY: OldDirectoryPolicy = OldDirectoryPolicy::Empty;

/// Compare the outputs of one tool for the original and the written file.
///
/// Return `Ok(lines)` with the line count if the exit codes and the texts are equal.
/// Otherwise write both outputs and return the differences.
pub(crate) fn same_output(runs: [&Run; 2], directory: &Path, name: &str) -> Result<usize, String> {
    if runs[0] == runs[1] {
        return Ok(runs[0].text.lines().count());
    }
    let changes = line_diff(&runs[0].text, &runs[1].text);
    let mut text = String::new();
    for change in &changes {
        let sign = if change.side == Side::Removed {
            '-'
        } else {
            '+'
        };
        let _ = writeln!(text, "{sign}{}: {}", change.line + 1, change.text);
    }
    for (suffix, content) in [
        ("original.txt", runs[0].text.as_str()),
        ("written.txt", runs[1].text.as_str()),
        ("diff", text.as_str()),
    ] {
        let path = directory.join(format!("{name}.{suffix}"));
        fs::write(&path, content)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    let shown: Vec<String> = changes
        .iter()
        .take(SHOWN_DIFFERENCES)
        .map(|change| change.text.trim().to_owned())
        .collect();
    Err(format!(
        "exit {:?} and {:?}, {} lines differ: {}",
        runs[0].code,
        runs[1].code,
        changes.len(),
        shown.join(" | ")
    ))
}

/// Write `bytes` to `path` and make its directory.
pub(crate) fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot make {}: {error}", parent.display()))?;
    }
    fs::write(path, bytes).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

/// Return the path of a found tool, or the result text of a check without the tool.
pub(crate) fn available(tool: &Result<PathBuf, String>) -> Result<&Path, String> {
    tool.as_ref()
        .map(PathBuf::as_path)
        .map_err(|_| "not run: tool missing".to_owned())
}

/// Return the result text of a check: `name ok-text` or `name error`.
pub(crate) fn describe<T>(
    name: &str,
    result: &Result<T, String>,
    ok: impl Fn(&T) -> String,
) -> String {
    match result {
        Ok(value) => format!("{name} {}", ok(value)),
        Err(error) => format!("{name} FAILED: {error}"),
    }
}

/// Return the file name of `path` as text.
pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

/// Return the result of a check that did not run, because an earlier step failed.
pub(crate) fn not_run<T>() -> Result<T, String> {
    Err("not run".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{OPTION_SETS, stream_differences};

    #[test]
    fn option_sets_have_distinct_names_and_valid_options() {
        let mut names: Vec<&str> = OPTION_SETS.iter().map(|(name, _)| *name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), OPTION_SETS.len());
        let mut builder = pdbrs::native::msfz::MsfzBuilder::new();
        builder.push(&b"stream"[..]).unwrap();
        for (_, options) in OPTION_SETS {
            builder.plan(options()).unwrap();
        }
    }

    #[test]
    fn stream_differences_name_the_streams() {
        let left = vec![Some(vec![1]), None, Some(vec![2])];
        let mut right = left.clone();
        assert_eq!(stream_differences(&left, &right, 0), None);
        right[0] = Some(Vec::new());
        right[2] = None;
        assert_eq!(
            stream_differences(&left, &right, 1),
            Some("streams [2] differ".to_owned())
        );
        assert_eq!(
            stream_differences(&left, &right[..2].to_vec(), 0),
            Some("stream counts 3 and 2".to_owned())
        );
    }
}
