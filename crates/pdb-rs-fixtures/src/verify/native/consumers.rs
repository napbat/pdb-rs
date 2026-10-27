//! Find and run the consumer tools, and normalize their output.
//!
//! The commands find `pdbtool` and `DIA2Dump` through the tools manifest of the workspace. If
//! the workspace has no built tool, they use the `consumer_tools` records of the fixture
//! manifest. They find `llvm-pdbutil` and `llvm-symbolizer` in the `LLVM\bin` directory below
//! the `ProgramFiles` directory.

use std::ffi::OsStr;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

use crate::json::{self, Value};
use crate::tools::{self, dia2dump, pdbtool};

/// The file name of `llvm-pdbutil`.
const PDBUTIL: &str = "llvm-pdbutil.exe";

/// The file name of `llvm-symbolizer`.
const SYMBOLIZER: &str = "llvm-symbolizer.exe";

/// The text that replaces a stream number in the normalized `llvm-pdbutil` dump.
const STREAM_NUMBER: &str = "<n>";

/// The text that replaces a 64-bit pointer in the masked `DIA2Dump` output.
const POINTER: &str = "<ptr>";

/// The paths of the consumer tools. A missing tool has the reason as its error.
#[derive(Debug)]
pub(crate) struct Consumers {
    /// `pdbtool.exe`.
    pub pdbtool: Result<PathBuf, String>,
    /// `DIA2Dump.exe`, with `msdia140.dll` next to it.
    pub dia2dump: Result<PathBuf, String>,
    /// `llvm-pdbutil.exe`.
    pub pdbutil: Result<PathBuf, String>,
    /// `llvm-symbolizer.exe`.
    pub symbolizer: Result<PathBuf, String>,
}

impl Consumers {
    /// Find the consumer tools for the fixtures in `fixtures`.
    pub(crate) fn locate(workspace: &Path, fixtures: &Path) -> Self {
        let llvm = std::env::var_os("ProgramFiles")
            .map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from)
            .join("LLVM")
            .join("bin");
        Self {
            pdbtool: built_tool(workspace, fixtures, pdbtool::NAME),
            dia2dump: built_tool(workspace, fixtures, dia2dump::NAME),
            pdbutil: llvm_tool(&llvm, PDBUTIL),
            symbolizer: llvm_tool(&llvm, SYMBOLIZER),
        }
    }

    /// Return the name and the error of each missing tool.
    pub(crate) fn missing(&self) -> Vec<(&'static str, &str)> {
        [
            (pdbtool::NAME, &self.pdbtool),
            (dia2dump::NAME, &self.dia2dump),
            ("llvm-pdbutil", &self.pdbutil),
            ("llvm-symbolizer", &self.symbolizer),
        ]
        .into_iter()
        .filter_map(|(name, path)| path.as_ref().err().map(|error| (name, error.as_str())))
        .collect()
    }
}

/// Find a tool of the `tools` command: first in the tools manifest of the workspace, then in
/// the `consumer_tools` section of the fixture manifest.
fn built_tool(workspace: &Path, fixtures: &Path, name: &str) -> Result<PathBuf, String> {
    tools::built_tool(workspace, name).or_else(|workspace_error| {
        fixture_tool(fixtures, name)
            .map_err(|fixture_error| format!("{workspace_error}; {fixture_error}"))
    })
}

/// Find a built tool in the `consumer_tools` section of the fixture manifest.
fn fixture_tool(fixtures: &Path, name: &str) -> Result<PathBuf, String> {
    let path = fixtures.join("manifest.json");
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let manifest = json::parse(&text)?;
    let Some(Value::Array(records)) = manifest
        .get("consumer_tools")
        .and_then(|tools| tools.get("tools"))
    else {
        return Err(format!("{} has no consumer tools", path.display()));
    };
    let record = records
        .iter()
        .find(|record| record.get("name").and_then(Value::as_str) == Some(name))
        .ok_or_else(|| format!("{} has no record of {name}", path.display()))?;
    if record.get("status").and_then(Value::as_str) != Some("built") {
        return Err(format!("{} records {name} as not built", path.display()));
    }
    let tool = record
        .get("path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| format!("the record of {name} has no path"))?;
    if tool.is_file() {
        Ok(tool)
    } else {
        Err(format!("{} does not exist", tool.display()))
    }
}

/// Find an LLVM tool in `directory`.
fn llvm_tool(directory: &Path, file: &str) -> Result<PathBuf, String> {
    let path = directory.join(file);
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("{} does not exist", path.display()))
    }
}

/// The result of one tool run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Run {
    /// The exit code, or `None` if a signal stopped the process.
    pub code: Option<i32>,
    /// The standard output and then the standard error, as lossy UTF-8.
    pub text: String,
}

impl Run {
    /// Return `true` if the exit code is 0.
    pub(crate) fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Run a tool with arguments and with `input` on its standard input.
pub(crate) fn run<I, S>(program: &Path, arguments: I, input: Option<&[u8]>) -> Result<Run, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_in(program, arguments, input, None)
}

/// Run a tool with arguments and with `input` on its standard input, in `directory` if it is
/// given.
///
/// A second thread writes the input, so a tool that writes much output before it reads all its
/// input does not block.
pub(crate) fn run_in<I, S>(
    program: &Path,
    arguments: I,
    input: Option<&[u8]>,
    directory: Option<&Path>,
) -> Result<Run, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command
        .args(arguments)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot run {}: {error}", program.display()))?;
    let stdin = child.stdin.take();
    let output = thread::scope(|scope| {
        let writer = input.zip(stdin).map(|(input, mut stdin)| {
            scope.spawn(move || {
                stdin
                    .write_all(input)
                    .map_err(|error| format!("cannot write to {}: {error}", program.display()))
            })
        });
        let output = child
            .wait_with_output()
            .map_err(|error| format!("cannot run {}: {error}", program.display()))?;
        if let Some(writer) = writer {
            writer
                .join()
                .map_err(|_| format!("the input thread of {} failed", program.display()))??;
        }
        Ok::<_, String>(output)
    })?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(Run {
        code: output.status.code(),
        text,
    })
}

/// Run `pdbtool check` and return the number of PDBs with errors.
///
/// The exit code of `pdbtool check` is 0 also for a PDB with errors, so the function reads the
/// line `Number of PDBs with errors`.
pub(crate) fn pdbtool_check(pdbtool: &Path, pdb: &Path) -> Result<u32, String> {
    let run = run(pdbtool, [OsStr::new("check"), pdb.as_os_str()], None)?;
    check_errors(&run.text).ok_or_else(|| {
        format!(
            "pdbtool check has no error count (exit {:?}): {}",
            run.code,
            crate::step::last_lines(&run.text, 3)
        )
    })
}

/// Read the count of the line `Number of PDBs with errors : N`.
fn check_errors(text: &str) -> Option<u32> {
    text.lines().find_map(|line| {
        let (_, rest) = line.split_once("Number of PDBs with errors")?;
        rest.trim().strip_prefix(':')?.trim().parse().ok()
    })
}

/// The streams that differ between two PDBs, from `pdbtool compare`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct StreamDifferences {
    /// The stream counts of the first and the second PDB, if they differ.
    pub counts: Option<(u32, u32)>,
    /// The streams with different sizes or contents, in the order of the output.
    pub streams: Vec<u32>,
}

/// Run `pdbtool compare --continue-on-differences` and return the streams that differ.
pub(crate) fn pdbtool_compare(
    pdbtool: &Path,
    first: &Path,
    second: &Path,
) -> Result<StreamDifferences, String> {
    let run = run(
        pdbtool,
        [
            OsStr::new("compare"),
            OsStr::new("--continue-on-differences"),
            first.as_os_str(),
            second.as_os_str(),
        ],
        None,
    )?;
    let differences = stream_differences(&run.text);
    let same = differences.counts.is_none() && differences.streams.is_empty();
    if run.success() == same {
        Ok(differences)
    } else {
        Err(format!(
            "pdbtool compare exit {:?} does not agree with its output: {}",
            run.code,
            crate::step::last_lines(&run.text, 3)
        ))
    }
}

/// Read the stream differences of the output of `pdbtool compare`.
fn stream_differences(text: &str) -> StreamDifferences {
    let mut differences = StreamDifferences::default();
    for line in text.lines() {
        if let Some((_, rest)) = line.split_once("Files have different number of streams. First: ")
        {
            let counts = rest.split_once(", Second: ").and_then(|(first, second)| {
                Some((first.trim().parse().ok()?, second.trim().parse().ok()?))
            });
            differences.counts = differences.counts.or(counts);
        } else if let Some((_, rest)) = line.split_once("ERROR pdbtool::compare: Stream ") {
            let (number, rest) = rest.split_once(' ').unwrap_or((rest, ""));
            if (rest.starts_with("has different") || rest.starts_with("differs"))
                && let Ok(number) = number.parse()
            {
                differences.streams.push(number);
            }
        }
    }
    differences
}

/// Run `llvm-pdbutil dump --all`.
pub(crate) fn pdbutil_dump(pdbutil: &Path, pdb: &Path) -> Result<Run, String> {
    run(
        pdbutil,
        [OsStr::new("dump"), OsStr::new("--all"), pdb.as_os_str()],
        None,
    )
}

/// Remove the container layout from an `llvm-pdbutil dump --all` output.
///
/// The function removes the lines `Number of blocks`, the `Blocks:` lists and the stream
/// directory entries `Stream N (...)`. It replaces the stream numbers of `Stream N, M bytes`,
/// `debug stream: N` and `Stream Index: N` with `<n>`. A stream index of 65535 means "no
/// stream", so it stays.
pub(crate) fn normalize_pdbutil(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("Number of blocks:")
            || trimmed.starts_with("Blocks:")
            || is_stream_entry(trimmed)
        {
            continue;
        }
        let mut line = line.to_owned();
        if trimmed.starts_with("Stream ") {
            line = replace_number(&line, "Stream ", |rest| rest.starts_with(", "));
        }
        line = replace_number(&line, "debug stream: ", |_| true);
        line = replace_number(&line, "Stream Index: ", |_| true);
        normalized.push_str(&line);
        normalized.push('\n');
    }
    normalized
}

/// Return `true` for a stream directory entry: `Stream`, spaces, a number, spaces and `(`.
fn is_stream_entry(trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix("Stream ") else {
        return false;
    };
    let rest = rest.trim_start();
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    digits > 0 && rest[digits..].trim_start().starts_with('(')
}

/// Replace the decimal number after the first `marker` with [`STREAM_NUMBER`] if `follows`
/// accepts the text after the number. Keep the number 65535.
fn replace_number(line: &str, marker: &str, follows: impl Fn(&str) -> bool) -> String {
    let Some(position) = line.find(marker) else {
        return line.to_owned();
    };
    let start = position + marker.len();
    let rest = &line[start..];
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 || &rest[..digits] == "65535" || !follows(&rest[digits..]) {
        return line.to_owned();
    }
    format!("{}{STREAM_NUMBER}{}", &line[..start], &rest[digits..])
}

/// Run `DIA2Dump` with `arguments` and then the PDB, and mask the pointers of the output.
pub(crate) fn dia2dump<I, S>(dia2dump: &Path, arguments: I, pdb: &Path) -> Result<Run, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut all: Vec<std::ffi::OsString> = arguments
        .into_iter()
        .map(|argument| argument.as_ref().to_owned())
        .collect();
    all.push(pdb.as_os_str().to_owned());
    let mut run = run(dia2dump, all, None)?;
    run.text = mask_pointers(&run.text);
    Ok(run)
}

/// Replace each word of exactly 16 uppercase hexadecimal digits with `<ptr>`.
///
/// `DIA2Dump` prints the addresses of its objects in this form, so they change from run to run.
pub(crate) fn mask_pointers(text: &str) -> String {
    let bytes = text.as_bytes();
    let is_word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let is_digit = |byte: u8| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte);
    let mut masked = String::with_capacity(text.len());
    let mut copied = 0;
    let mut index = 0;
    while index < bytes.len() {
        let starts_word = index == 0 || !is_word(bytes[index - 1]);
        if starts_word && is_digit(bytes[index]) {
            let length = bytes[index..]
                .iter()
                .take_while(|&&byte| is_digit(byte))
                .count();
            let end = index + length;
            if length == 16 && bytes.get(end).is_none_or(|&byte| !is_word(byte)) {
                masked.push_str(&text[copied..index]);
                masked.push_str(POINTER);
                copied = end;
            }
            index = end.max(index + 1);
        } else {
            index += 1;
        }
    }
    masked.push_str(&text[copied..]);
    masked
}

#[cfg(test)]
mod tests {
    use super::{
        StreamDifferences, check_errors, mask_pointers, normalize_pdbutil, stream_differences,
    };

    #[test]
    fn normalization_removes_the_container_layout() {
        let dump = "\
  Block Size: 4096
  Number of blocks: 385
  Number of streams: 117
  Stream   0 (    48 bytes): [Old MSF Directory]
              Blocks: [7]
  Stream  13 (  1284 bytes): [Module \"c_api.obj\"]
  Stream 18, 1932 bytes
  Stream 18 has no colon
debug stream: 33, # files: 26, has ec info: true
  Hash Stream Index: 114
  Aux Hash Stream Index: 65535
   25964 | S_UDT [size = 24] `std::streamoff`
";
        let expected = "\
  Block Size: 4096
  Number of streams: 117
  Stream <n>, 1932 bytes
  Stream 18 has no colon
debug stream: <n>, # files: 26, has ec info: true
  Hash Stream Index: <n>
  Aux Hash Stream Index: 65535
   25964 | S_UDT [size = 24] `std::streamoff`
";
        assert_eq!(normalize_pdbutil(dump), expected);
    }

    #[test]
    fn masks_only_whole_16_digit_words() {
        assert_eq!(
            mask_pointers("at 000001F2A3B4C5D6, 0000000000000001x, 00000001F2A3B4C5D6\n"),
            "at <ptr>, 0000000000000001x, 00000001F2A3B4C5D6\n"
        );
        assert_eq!(
            mask_pointers("__real@4000000000000000(abcdef0123456789)"),
            "__real@<ptr>(abcdef0123456789)"
        );
        assert_eq!(mask_pointers("FFFFFFFFFFFFFFFF"), "<ptr>");
    }

    #[test]
    fn reads_the_pdbtool_results() {
        let check = " INFO pdbtool::check: Number of PDBs checked                   :        1\n \
                     INFO pdbtool::check: Number of PDBs with errors               :        2\n";
        assert_eq!(check_errors(check), Some(2));
        assert_eq!(check_errors("Error: cannot open"), None);

        let compare = "\
ERROR pdbtool::compare: Files have different number of streams. First: 74, Second: 75
ERROR pdbtool::compare: Stream 1 has different sizes. First: 161 bytes, Second: 176 bytes
ERROR pdbtool::compare: Stream 7 differs at byte offset 8 (0x8)
 INFO pdbtool::compare:   Streams with differences: 2
";
        assert_eq!(
            stream_differences(compare),
            StreamDifferences {
                counts: Some((74, 75)),
                streams: vec![1, 7],
            }
        );
        assert_eq!(stream_differences(""), StreamDifferences::default());
    }
}
