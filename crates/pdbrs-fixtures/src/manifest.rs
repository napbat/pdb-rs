//! Build records and the `manifest.json` file.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::classify::Kind;
use crate::json::{self, Value};
use crate::matrix::{FIXTURES, Fixture};
use crate::step::Step;
use crate::toolchain::capture;

/// The result of one fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Every step succeeded and every output exists.
    Built,
    /// The toolchain is available, but a step failed or an output is missing.
    Failed,
    /// The toolchain is not available.
    Unavailable,
}

impl Status {
    /// The name in the manifest.
    pub fn name(self) -> &'static str {
        match self {
            Self::Built => "built",
            Self::Failed => "failed",
            Self::Unavailable => "unavailable",
        }
    }
}

/// One output file.
#[derive(Debug)]
pub struct Output {
    /// The kind that the contents show.
    pub kind: Kind,
    /// The path relative to the output root, with `/` separators.
    pub path: String,
    /// The size in bytes.
    pub size: u64,
}

/// The record of one fixture.
#[derive(Debug)]
pub struct Record {
    /// The fixture.
    pub fixture: &'static Fixture,
    /// The result.
    pub status: Status,
    /// Why the fixture failed or is unavailable, or a note about a built fixture.
    pub reason: Option<String>,
    /// Tool names and version lines.
    pub tools: Vec<(String, String)>,
    /// The steps that ran.
    pub steps: Vec<Step>,
    /// The outputs.
    pub outputs: Vec<Output>,
}

impl Record {
    /// Make the record of a fixture without an available toolchain.
    pub fn unavailable(fixture: &'static Fixture, reason: String) -> Self {
        Self {
            fixture,
            status: Status::Unavailable,
            reason: Some(reason),
            tools: Vec::new(),
            steps: Vec::new(),
            outputs: Vec::new(),
        }
    }

    fn to_value(&self) -> Value {
        let fixture = self.fixture;
        let tools = self
            .tools
            .iter()
            .map(|(name, version)| (name.clone(), Value::string(version)))
            .collect();
        let outputs = self
            .outputs
            .iter()
            .map(|output| {
                Value::object([
                    ("kind", Value::string(output.kind.name())),
                    ("path", Value::string(&output.path)),
                    ("size", Value::Number(output.size)),
                ])
            })
            .collect();
        Value::object([
            ("name", Value::string(fixture.name)),
            ("producer", Value::string(fixture.producer)),
            ("language", Value::string(fixture.language)),
            ("architecture", Value::string(fixture.arch)),
            ("configuration", Value::string(fixture.configuration)),
            ("status", Value::string(self.status.name())),
            ("reason", Value::optional(self.reason.as_deref())),
            ("tools", Value::Object(tools)),
            (
                "commands",
                Value::Array(self.steps.iter().map(Step::to_value).collect()),
            ),
            ("outputs", Value::Array(outputs)),
        ])
    }
}

/// Facts about the generator run.
#[derive(Debug)]
pub struct Header {
    /// The workspace commit, if `git` could read it.
    pub commit: Option<String>,
    /// The output root.
    pub out: PathBuf,
    /// The time of the run, in seconds since the Unix epoch.
    pub unix_seconds: u64,
}

impl Header {
    /// Collect the facts of a run now: the commit of `workspace` and the current time.
    pub fn now(workspace: &Path, out: &Path) -> Self {
        let commit = capture(
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(workspace),
        )
        .ok()
        .map(|text| text.trim().to_owned());
        Self {
            commit,
            out: out.to_path_buf(),
            unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs()),
        }
    }

    /// Render a manifest: the header members, then `members`.
    pub fn render(&self, members: Vec<(String, Value)>) -> String {
        let mut all = vec![
            (
                "generator".to_owned(),
                Value::string(concat!("pdbrs-fixtures ", env!("CARGO_PKG_VERSION"))),
            ),
            (
                "source_commit".to_owned(),
                Value::optional(self.commit.as_deref()),
            ),
            (
                "host".to_owned(),
                Value::string(format!(
                    "{}-{}",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                )),
            ),
            (
                "generated_unix_seconds".to_owned(),
                Value::Number(self.unix_seconds),
            ),
            (
                "output_root".to_owned(),
                Value::string(self.out.to_string_lossy()),
            ),
        ];
        all.extend(members);
        Value::Object(all).render()
    }
}

/// Render the fixture manifest with the consumer tools section and the fixture entries.
pub fn render(header: &Header, fixtures: Vec<Value>, consumer_tools: Value) -> String {
    header.render(vec![
        ("consumer_tools".to_owned(), consumer_tools),
        ("fixtures".to_owned(), Value::Array(fixtures)),
    ])
}

/// Read an earlier manifest at `path`.
///
/// Return `None` if the file does not exist. If the file cannot be read or parsed, write a
/// warning and return `None`.
pub fn read_previous(path: &Path) -> Option<Value> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return None,
        Err(error) => {
            eprintln!("warning: cannot read {}: {error}", path.display());
            return None;
        }
    };
    match json::parse(&text) {
        Ok(value) => Some(value),
        Err(error) => {
            eprintln!("warning: cannot parse {}: {error}", path.display());
            None
        }
    }
}

/// Make the fixture entries of a manifest from the records of this run.
///
/// Without `previous`, the entries are the records. With the `previous` manifest, keep each
/// earlier entry whose name has no record in this run. A record replaces the earlier entry
/// with the same name. Put the entries in matrix order. Entries with a name that the matrix
/// does not have come last, in their earlier order.
pub fn fixture_entries(previous: Option<&Value>, records: &[Record]) -> Vec<Value> {
    let mut entries: Vec<(String, Value)> = previous
        .and_then(|manifest| match manifest.get("fixtures") {
            Some(Value::Array(fixtures)) => Some(fixtures.as_slice()),
            _ => None,
        })
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?;
            let replaced = records.iter().any(|record| record.fixture.name == name);
            (!replaced).then(|| (name.to_owned(), entry.clone()))
        })
        .collect();
    entries.extend(
        records
            .iter()
            .map(|record| (record.fixture.name.to_owned(), record.to_value())),
    );
    entries.sort_by_key(|(name, _)| {
        FIXTURES
            .iter()
            .position(|fixture| fixture.name == name)
            .unwrap_or(usize::MAX)
    });
    entries.into_iter().map(|(_, entry)| entry).collect()
}

/// The path of `path` relative to `root`, with `/` separators.
pub fn relative_path(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::{Header, Record, Status, fixture_entries, relative_path, render};
    use crate::json::{self, Value};
    use crate::matrix::FIXTURES;
    use std::path::{Path, PathBuf};

    fn record(index: usize, status: Status) -> Record {
        Record {
            fixture: &FIXTURES[index],
            status,
            reason: None,
            tools: Vec::new(),
            steps: Vec::new(),
            outputs: Vec::new(),
        }
    }

    fn entry(name: &str, status: &str) -> Value {
        Value::object([
            ("name", Value::string(name)),
            ("status", Value::string(status)),
        ])
    }

    fn names_and_status(entries: &[Value]) -> Vec<(&str, &str)> {
        entries
            .iter()
            .map(|entry| {
                (
                    entry.get("name").and_then(Value::as_str).unwrap(),
                    entry.get("status").and_then(Value::as_str).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn entries_without_an_earlier_manifest_are_the_records() {
        let records = [record(1, Status::Built), record(0, Status::Failed)];
        let entries = fixture_entries(None, &records);
        assert_eq!(
            names_and_status(&entries),
            [(FIXTURES[0].name, "failed"), (FIXTURES[1].name, "built")]
        );
    }

    #[test]
    fn a_partial_run_replaces_and_adds_entries_and_keeps_the_others() {
        let previous = Value::object([(
            "fixtures",
            Value::Array(vec![
                entry("retired-fixture", "built"),
                entry(FIXTURES[2].name, "failed"),
                entry(FIXTURES[0].name, "built"),
            ]),
        )]);
        let records = [record(2, Status::Built), record(1, Status::Unavailable)];
        let entries = fixture_entries(Some(&previous), &records);
        assert_eq!(
            names_and_status(&entries),
            [
                (FIXTURES[0].name, "built"),
                (FIXTURES[1].name, "unavailable"),
                (FIXTURES[2].name, "built"),
                ("retired-fixture", "built"),
            ]
        );
        assert_eq!(entries[0], entry(FIXTURES[0].name, "built"));
        assert_eq!(entries[2], records[0].to_value());
    }

    #[test]
    fn an_earlier_manifest_without_fixtures_adds_nothing() {
        let previous = Value::object([("fixtures", Value::Null)]);
        let records = [record(3, Status::Built)];
        let entries = fixture_entries(Some(&previous), &records);
        assert_eq!(names_and_status(&entries), [(FIXTURES[3].name, "built")]);
    }

    #[test]
    fn renders_the_header_then_the_consumer_tools_then_the_fixtures() {
        let header = Header {
            commit: Some("abc".to_owned()),
            out: PathBuf::from("out"),
            unix_seconds: 7,
        };
        let tools = Value::object([("tools", Value::Array(Vec::new()))]);
        let manifest = json::parse(&render(&header, Vec::new(), tools.clone())).unwrap();
        let Value::Object(members) = &manifest else {
            panic!("the manifest is not an object");
        };
        let names: Vec<&str> = members.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "generator",
                "source_commit",
                "host",
                "generated_unix_seconds",
                "output_root",
                "consumer_tools",
                "fixtures"
            ]
        );
        assert_eq!(manifest.get("source_commit"), Some(&Value::string("abc")));
        assert_eq!(
            manifest.get("generated_unix_seconds"),
            Some(&Value::Number(7))
        );
        assert_eq!(manifest.get("output_root"), Some(&Value::string("out")));
        assert_eq!(manifest.get("consumer_tools"), Some(&tools));
        assert_eq!(manifest.get("fixtures"), Some(&Value::Array(Vec::new())));
    }

    #[test]
    fn relative_paths_use_forward_slashes() {
        let root = Path::new("out");
        assert_eq!(
            relative_path(root, &root.join("a").join("b.pdb")),
            "a/b.pdb"
        );
        assert_eq!(relative_path(root, Path::new("elsewhere")), "elsewhere");
    }
}
