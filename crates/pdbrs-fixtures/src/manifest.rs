//! Build records and the `manifest.json` file.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::classify::Kind;
use crate::json::Value;
use crate::matrix::Fixture;
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

/// Render the fixture manifest with the consumer tools section.
pub fn render(header: &Header, records: &[Record], consumer_tools: Value) -> String {
    header.render(vec![
        ("consumer_tools".to_owned(), consumer_tools),
        (
            "fixtures".to_owned(),
            Value::Array(records.iter().map(Record::to_value).collect()),
        ),
    ])
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
    use super::{Header, relative_path, render};
    use crate::json::{self, Value};
    use std::path::{Path, PathBuf};

    #[test]
    fn renders_the_header_then_the_consumer_tools_then_the_fixtures() {
        let header = Header {
            commit: Some("abc".to_owned()),
            out: PathBuf::from("out"),
            unix_seconds: 7,
        };
        let tools = Value::object([("tools", Value::Array(Vec::new()))]);
        let manifest = json::parse(&render(&header, &[], tools.clone())).unwrap();
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
