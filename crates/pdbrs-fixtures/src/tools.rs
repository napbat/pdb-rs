//! The consumer tools: `pdbtool`, `DIA2Dump` and the Portable PDB verifier.
//!
//! The `tools` command installs or builds each tool into `target/tools/bin/` below the
//! workspace root. It writes `target/tools/manifest.json` with one record for each tool. The
//! fixture build copies these records into the `consumer_tools` section of its manifest. A
//! missing tool never changes the result of the fixture build.

mod dia2dump;
pub mod file_version;
mod patch;
mod pdbtool;
pub mod verifier;

use std::fs::{self, File};
use std::path::{Path, PathBuf};

use crate::json::{self, Value};
use crate::manifest::{Header, Status};
use crate::step::{Step, StepError};
use crate::toolchain::Toolchains;

/// A function that installs or builds one tool and returns its record.
type Builder = fn(&Paths, &mut Toolchains) -> Tool;

/// The tool names and their builders, in manifest order.
const TOOLS: [(&str, Builder); 3] = [
    (pdbtool::NAME, pdbtool::build),
    (dia2dump::NAME, dia2dump::build),
    (verifier::NAME, verifier::build),
];

/// The tool names, in manifest order.
const NAMES: [&str; 3] = [TOOLS[0].0, TOOLS[1].0, TOOLS[2].0];

/// The command that makes the tools.
const COMMAND: &str = "cargo run -p pdbrs-fixtures -- tools";

/// The directories of the tools command.
#[derive(Debug)]
pub struct Paths {
    /// The tools root, `target/tools/` below the workspace root.
    pub root: PathBuf,
    /// The directory of the executables, `bin/` below the tools root.
    pub bin: PathBuf,
}

impl Paths {
    /// The paths below a tools root.
    pub fn new(root: PathBuf) -> Self {
        let bin = root.join("bin");
        Self { root, bin }
    }

    /// The build log of a tool.
    pub fn log(&self, name: &str) -> PathBuf {
        self.root.join(format!("{name}.log"))
    }
}

/// Why a tool is not built.
#[derive(Debug)]
pub enum Missing {
    /// A prerequisite is not available, for example Visual Studio or the DIA SDK.
    Unavailable(String),
    /// A step failed, or an expected file is missing after the steps.
    Failed(String),
}

impl From<StepError> for Missing {
    fn from(error: StepError) -> Self {
        match error {
            StepError::Failed(reason) => Self::Failed(reason),
            StepError::Unsupported(reason) => Self::Unavailable(reason),
        }
    }
}

/// The record of one tool.
#[derive(Debug)]
pub struct Tool {
    /// The tool name.
    pub name: &'static str,
    /// The result.
    pub status: Status,
    /// Why the tool failed or is unavailable.
    pub reason: Option<String>,
    /// The executable, if the tool is built.
    pub path: Option<PathBuf>,
    /// Every file that the tool needs, with the executable first.
    pub files: Vec<PathBuf>,
    /// Version facts, for example the crate version or the `msdia140.dll` file version.
    pub version: Vec<(String, String)>,
    /// Source facts, for example the crate and its upstream revision or the DIA SDK paths.
    pub source: Vec<(String, String)>,
    /// The steps that ran.
    pub steps: Vec<Step>,
}

impl Tool {
    /// Make an unavailable record without facts.
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            status: Status::Unavailable,
            reason: None,
            path: None,
            files: Vec::new(),
            version: Vec::new(),
            source: Vec::new(),
            steps: Vec::new(),
        }
    }

    /// Add a version fact.
    pub fn version(&mut self, name: &str, value: impl Into<String>) {
        self.version.push((name.to_owned(), value.into()));
    }

    /// Add a source fact.
    pub fn source(&mut self, name: &str, value: impl Into<String>) {
        self.source.push((name.to_owned(), value.into()));
    }

    /// Record the files of a built tool. The executable comes first.
    ///
    /// Return an error if a file does not exist.
    pub fn finish(&mut self, files: Vec<PathBuf>) -> Result<(), Missing> {
        if let Some(missing) = files.iter().find(|file| !file.is_file()) {
            return Err(Missing::Failed(format!(
                "{} is missing after the build",
                missing.display()
            )));
        }
        self.path = files.first().cloned();
        self.files = files;
        Ok(())
    }

    /// The manifest entry of the tool.
    pub fn to_value(&self) -> Value {
        let facts = |facts: &[(String, String)]| {
            Value::Object(
                facts
                    .iter()
                    .map(|(name, value)| (name.clone(), Value::string(value)))
                    .collect(),
            )
        };
        let path = self.path.as_ref().map(|path| path.to_string_lossy());
        Value::object([
            ("name", Value::string(self.name)),
            ("status", Value::string(self.status.name())),
            ("reason", Value::optional(self.reason.as_deref())),
            ("path", Value::optional(path.as_deref())),
            (
                "files",
                Value::Array(
                    self.files
                        .iter()
                        .map(|file| Value::string(file.to_string_lossy()))
                        .collect(),
                ),
            ),
            ("version", facts(&self.version)),
            ("source", facts(&self.source)),
            (
                "commands",
                Value::Array(self.steps.iter().map(Step::to_value).collect()),
            ),
        ])
    }
}

/// The tools root below the workspace root.
pub fn tools_root(workspace: &Path) -> PathBuf {
    workspace.join("target").join("tools")
}

/// Install and build every tool, and write `manifest.json` in the tools root.
///
/// Return `false` if a tool with its prerequisites failed. An unavailable tool is not a
/// failure.
pub fn run(workspace: &Path) -> Result<bool, String> {
    let paths = Paths::new(tools_root(workspace));
    fs::create_dir_all(&paths.bin)
        .map_err(|error| format!("cannot make {}: {error}", paths.bin.display()))?;
    let mut toolchains = Toolchains::default();
    let mut tools = Vec::new();
    for (name, builder) in TOOLS {
        println!("building {name} ...");
        let tool = builder(&paths, &mut toolchains);
        println!(
            "  {}{}",
            tool.status.name(),
            tool.reason
                .as_deref()
                .map(|reason| format!(": {reason}"))
                .unwrap_or_default()
        );
        tools.push(tool);
    }
    let header = Header::now(workspace, &paths.root);
    let manifest = paths.root.join("manifest.json");
    fs::write(&manifest, render(&header, &tools))
        .map_err(|error| format!("cannot write {}: {error}", manifest.display()))?;
    println!("wrote {}", manifest.display());
    Ok(tools.iter().all(|tool| tool.status != Status::Failed))
}

/// Make the record of a tool with `attempt`.
///
/// `attempt` adds facts and steps, and calls [`Tool::finish`] on success.
fn record(name: &'static str, attempt: impl FnOnce(&mut Tool) -> Result<(), Missing>) -> Tool {
    let mut tool = Tool::new(name);
    match attempt(&mut tool) {
        Ok(()) => tool.status = Status::Built,
        Err(Missing::Unavailable(reason)) => tool.reason = Some(reason),
        Err(Missing::Failed(reason)) => {
            tool.status = Status::Failed;
            tool.reason = Some(reason);
        }
    }
    tool
}

/// Run the steps and write their command lines and output to a new log at `log`.
fn run_steps(steps: &[Step], log: &Path) -> Result<(), Missing> {
    let mut file = File::create(log)
        .map_err(|error| Missing::Failed(format!("cannot make {}: {error}", log.display())))?;
    for step in steps {
        step.run(&mut file).map_err(|error| match error {
            StepError::Failed(reason) => {
                Missing::Failed(format!("{reason} (log: {})", log.display()))
            }
            unsupported @ StepError::Unsupported(_) => unsupported.into(),
        })?;
    }
    Ok(())
}

/// Render the tools manifest.
pub fn render(header: &Header, tools: &[Tool]) -> String {
    header.render(vec![(
        "tools".to_owned(),
        Value::Array(tools.iter().map(Tool::to_value).collect()),
    )])
}

/// The `consumer_tools` section of the fixture manifest.
///
/// The section copies the tool records of the tools manifest. A tool without a record, or
/// with a missing file, shows as `unavailable` with the reason.
pub fn consumer_tools(workspace: &Path) -> Value {
    let manifest = tools_root(workspace).join("manifest.json");
    let text = fs::read_to_string(&manifest)
        .map_err(|error| format!("cannot read {}: {error}", manifest.display()));
    Value::object([
        ("manifest", Value::string(manifest.to_string_lossy())),
        ("tools", Value::Array(tool_records(text, Path::is_file))),
    ])
}

/// Return the main file of a built tool from the tools manifest of the workspace.
///
/// Return an error with the reason if the tool is not built or one of its files is missing.
pub fn built_tool(workspace: &Path, name: &str) -> Result<PathBuf, String> {
    let records = consumer_tools(workspace);
    let Some(Value::Array(records)) = records.get("tools") else {
        return Err("the tools manifest has no tools".to_owned());
    };
    let record = records
        .iter()
        .find(|record| record.get("name").and_then(Value::as_str) == Some(name))
        .ok_or_else(|| format!("the tools manifest has no record of {name}"))?;
    if record.get("status").and_then(Value::as_str) != Some(Status::Built.name()) {
        let reason = record
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("no reason");
        return Err(format!("{name} is not available: {reason}"));
    }
    record
        .get("path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| format!("the record of {name} has no path"))
}

/// Select one record for each tool from the text of a tools manifest.
///
/// `exists` tells if a file exists.
fn tool_records(text: Result<String, String>, exists: impl Fn(&Path) -> bool) -> Vec<Value> {
    let records = text.and_then(|text| {
        let manifest = json::parse(&text)?;
        match manifest.get("tools") {
            Some(Value::Array(records)) => Ok(records.clone()),
            _ => Err("the tools manifest has no `tools` array".to_owned()),
        }
    });
    NAMES
        .iter()
        .map(|&name| {
            let found = records.as_ref().map_err(Clone::clone).and_then(|records| {
                records
                    .iter()
                    .find(|record| record.get("name").and_then(Value::as_str) == Some(name))
                    .cloned()
                    .ok_or_else(|| format!("the tools manifest has no record of {name}"))
            });
            match found {
                Ok(record) => checked(record, &exists),
                Err(reason) => unavailable(name, &reason),
            }
        })
        .collect()
}

/// Mark a built record as unavailable if one of its files does not exist.
fn checked(mut record: Value, exists: &impl Fn(&Path) -> bool) -> Value {
    if record.get("status").and_then(Value::as_str) != Some(Status::Built.name()) {
        return record;
    }
    let missing = match record.get("files") {
        Some(Value::Array(files)) => files
            .iter()
            .filter_map(Value::as_str)
            .find(|file| !exists(Path::new(file)))
            .map(str::to_owned),
        _ => None,
    };
    if let Some(file) = missing {
        record.set("status", Value::string(Status::Unavailable.name()));
        record.set(
            "reason",
            Value::String(format!("{file} is missing; run `{COMMAND}`")),
        );
    }
    record
}

/// The record of a tool that the tools manifest does not describe.
fn unavailable(name: &'static str, reason: &str) -> Value {
    let mut tool = Tool::new(name);
    tool.reason = Some(format!("{reason}; run `{COMMAND}`"));
    tool.to_value()
}

#[cfg(test)]
mod tests {
    use super::{NAMES, Status, Tool, render, tool_records};
    use crate::json::{self, Value};
    use crate::manifest::Header;
    use crate::step::Step;
    use std::path::{Path, PathBuf};

    #[test]
    fn the_fixture_build_reads_the_rendered_tools_manifest() {
        let header = Header {
            commit: None,
            out: PathBuf::from("target/tools"),
            unix_seconds: 1,
        };
        let mut failed = Tool::new("DIA2Dump");
        failed.status = Status::Failed;
        failed.reason = Some("MSBuild failed".to_owned());
        let tools = [
            built("pdbtool", &["bin/pdbtool.exe"]),
            failed,
            built(
                "PortableVerifier",
                &["bin/portable-verifier/PortableVerifier.dll"],
            ),
        ];
        let text = render(&header, &tools);
        let manifest = json::parse(&text).unwrap();
        assert_eq!(
            manifest.get("output_root"),
            Some(&Value::string("target/tools"))
        );
        let expected: Vec<Value> = tools.iter().map(Tool::to_value).collect();
        assert_eq!(manifest.get("tools"), Some(&Value::Array(expected.clone())));
        assert_eq!(tool_records(Ok(text), |_| true), expected);
    }

    fn built(name: &'static str, files: &[&str]) -> Tool {
        let mut tool = Tool::new(name);
        tool.status = Status::Built;
        tool.files = files.iter().map(PathBuf::from).collect();
        tool.path = tool.files.first().cloned();
        tool.version("crate", "1.0");
        tool.source("revision", "abc");
        tool
    }

    fn manifest(tools: &[Tool]) -> String {
        Value::object([
            ("generator", Value::string("x")),
            (
                "tools",
                Value::Array(tools.iter().map(Tool::to_value).collect()),
            ),
        ])
        .render()
    }

    fn status_and_reason(record: &Value) -> (Option<&str>, Option<&str>) {
        (
            record.get("status").and_then(Value::as_str),
            record.get("reason").and_then(Value::as_str),
        )
    }

    #[test]
    fn renders_a_tool_record() {
        let mut tool = built("pdbtool", &["bin/pdbtool.exe", "bin/extra.dll"]);
        tool.steps
            .push(Step::new("cargo", Path::new("root")).args(["install", "pdbtool"]));
        let expected = Value::object([
            ("name", Value::string("pdbtool")),
            ("status", Value::string("built")),
            ("reason", Value::Null),
            ("path", Value::string("bin/pdbtool.exe")),
            (
                "files",
                Value::Array(vec![
                    Value::string("bin/pdbtool.exe"),
                    Value::string("bin/extra.dll"),
                ]),
            ),
            ("version", Value::object([("crate", Value::string("1.0"))])),
            (
                "source",
                Value::object([("revision", Value::string("abc"))]),
            ),
            (
                "commands",
                Value::Array(vec![Value::object([
                    ("directory", Value::string("root")),
                    ("environment", Value::string("inherited")),
                    ("variables", Value::object([])),
                    ("command", Value::string("cargo install pdbtool")),
                ])]),
            ),
        ]);
        assert_eq!(tool.to_value(), expected);
    }

    #[test]
    fn copies_records_whose_files_exist() {
        let tools = [
            built("pdbtool", &["bin/pdbtool.exe"]),
            built("DIA2Dump", &["bin/DIA2Dump.exe", "bin/msdia140.dll"]),
            built(
                "PortableVerifier",
                &["bin/portable-verifier/PortableVerifier.dll"],
            ),
        ];
        let records = tool_records(Ok(manifest(&tools)), |_| true);
        assert_eq!(
            records,
            tools.iter().map(Tool::to_value).collect::<Vec<_>>()
        );
    }

    #[test]
    fn marks_a_built_tool_with_a_missing_file_as_unavailable() {
        let tools = [
            built("pdbtool", &["bin/pdbtool.exe"]),
            built("DIA2Dump", &["bin/DIA2Dump.exe", "bin/msdia140.dll"]),
        ];
        let records = tool_records(Ok(manifest(&tools)), |path| {
            path != Path::new("bin/msdia140.dll")
        });
        assert_eq!(status_and_reason(&records[0]), (Some("built"), None));
        assert_eq!(
            status_and_reason(&records[1]),
            (
                Some("unavailable"),
                Some("bin/msdia140.dll is missing; run `cargo run -p pdbrs-fixtures -- tools`")
            )
        );
        assert_eq!(
            records[1].get("version"),
            tools[1].to_value().get("version")
        );
    }

    #[test]
    fn keeps_failed_records_and_fills_absent_ones() {
        let mut failed = Tool::new("DIA2Dump");
        failed.status = Status::Failed;
        failed.reason = Some("MSBuild failed".to_owned());
        let records = tool_records(Ok(manifest(&[failed])), |_| false);
        assert_eq!(
            status_and_reason(&records[0]),
            (
                Some("unavailable"),
                Some(
                    "the tools manifest has no record of pdbtool; run \
                     `cargo run -p pdbrs-fixtures -- tools`"
                )
            )
        );
        assert_eq!(
            status_and_reason(&records[1]),
            (Some("failed"), Some("MSBuild failed"))
        );
    }

    #[test]
    fn records_every_tool_as_unavailable_without_a_manifest() {
        for text in [
            Err("cannot read target/tools/manifest.json: not found".to_owned()),
            Ok("{\"tools\": 1}".to_owned()),
            Ok("{".to_owned()),
        ] {
            let records = tool_records(text, |_| true);
            assert_eq!(records.len(), NAMES.len());
            for (record, name) in records.iter().zip(NAMES) {
                assert_eq!(record.get("name").and_then(Value::as_str), Some(name));
                let (status, reason) = status_and_reason(record);
                assert_eq!(status, Some("unavailable"));
                assert!(
                    reason
                        .is_some_and(|reason| reason
                            .ends_with("; run `cargo run -p pdbrs-fixtures -- tools`")),
                    "{reason:?}"
                );
            }
        }
    }
}
