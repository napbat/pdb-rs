//! Build records and the `manifest.json` file.

use std::path::Path;

use crate::classify::Kind;
use crate::json::Value;
use crate::matrix::Fixture;
use crate::step::Step;

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
pub struct Header<'a> {
    /// The workspace commit, if `git` could read it.
    pub commit: Option<&'a str>,
    /// The output root.
    pub out: &'a Path,
    /// The time of the run, in seconds since the Unix epoch.
    pub unix_seconds: u64,
}

/// Render the manifest.
pub fn render(header: &Header<'_>, records: &[Record]) -> String {
    Value::object([
        (
            "generator",
            Value::string(concat!("pdbrs-fixtures ", env!("CARGO_PKG_VERSION"))),
        ),
        ("source_commit", Value::optional(header.commit)),
        (
            "host",
            Value::string(format!(
                "{}-{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )),
        ),
        ("generated_unix_seconds", Value::Number(header.unix_seconds)),
        ("output_root", Value::string(header.out.to_string_lossy())),
        (
            "fixtures",
            Value::Array(records.iter().map(Record::to_value).collect()),
        ),
    ])
    .render()
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
    use super::relative_path;
    use std::path::Path;

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
