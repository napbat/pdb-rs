//! Build steps: one child process each, with a recorded command line and a log.

use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::environment::Environment;
use crate::json::Value;

/// The environment of a step.
#[derive(Debug, Clone)]
pub enum StepEnvironment {
    /// Inherit the environment of the generator.
    Inherit,
    /// Replace the environment with a captured block. The label names its source.
    Replace {
        /// A description of the source, for example `vcvarsall.bat x64`.
        label: String,
        /// The complete block.
        environment: Environment,
    },
}

/// One child process of a build.
#[derive(Debug, Clone)]
pub struct Step {
    /// The program, as an absolute path or as a name to find on `PATH`.
    pub program: PathBuf,
    /// The arguments.
    pub arguments: Vec<String>,
    /// The working directory.
    pub directory: PathBuf,
    /// The base environment.
    pub environment: StepEnvironment,
    /// Variables to set on top of the base environment.
    pub variables: Vec<(String, String)>,
    /// An output marker that shows that the tool does not support a requested feature,
    /// and the reason to record.
    pub unsupported: Option<(String, String)>,
}

/// Why a step did not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepError {
    /// The process did not start or exited with a failure code.
    Failed(String),
    /// The output shows that the tool does not support a requested feature.
    Unsupported(String),
}

impl Step {
    /// Make a step that inherits the environment of the generator.
    pub fn new(program: impl Into<PathBuf>, directory: &Path) -> Self {
        Self {
            program: program.into(),
            arguments: Vec::new(),
            directory: directory.to_owned(),
            environment: StepEnvironment::Inherit,
            variables: Vec::new(),
            unsupported: None,
        }
    }

    /// Append arguments.
    #[must_use]
    pub fn args<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.arguments.extend(arguments.into_iter().map(Into::into));
        self
    }

    /// Replace the base environment with a captured block.
    #[must_use]
    pub fn environment(mut self, label: &str, environment: &Environment) -> Self {
        self.environment = StepEnvironment::Replace {
            label: label.to_owned(),
            environment: environment.clone(),
        };
        self
    }

    /// Set one variable on top of the base environment.
    #[must_use]
    pub fn variable(mut self, name: &str, value: &str) -> Self {
        self.variables.push((name.to_owned(), value.to_owned()));
        self
    }

    /// Treat output that contains `marker` as an unsupported feature.
    #[must_use]
    pub fn unsupported_when(mut self, marker: &str, reason: &str) -> Self {
        self.unsupported = Some((marker.to_owned(), reason.to_owned()));
        self
    }

    /// The command line, with each argument quoted where needed.
    pub fn command_line(&self) -> String {
        let mut line = quote(&self.program.to_string_lossy());
        for argument in &self.arguments {
            line.push(' ');
            line.push_str(&quote(argument));
        }
        line
    }

    /// The manifest entry of the step.
    pub fn to_value(&self) -> Value {
        let environment = match &self.environment {
            StepEnvironment::Inherit => "inherited".to_owned(),
            StepEnvironment::Replace { label, .. } => label.clone(),
        };
        let variables = self
            .variables
            .iter()
            .map(|(name, value)| (name.clone(), Value::string(value)))
            .collect();
        Value::object([
            ("directory", Value::string(self.directory.to_string_lossy())),
            ("environment", Value::String(environment)),
            ("variables", Value::Object(variables)),
            ("command", Value::String(self.command_line())),
        ])
    }

    /// Run the step and append its command line and output to `log`.
    ///
    /// A failure error includes the last lines of the output.
    pub fn run(&self, log: &mut File) -> Result<(), StepError> {
        let mut command = Command::new(&self.program);
        command.args(&self.arguments).current_dir(&self.directory);
        if let StepEnvironment::Replace { environment, .. } = &self.environment {
            command.env_clear().envs(environment.iter());
        }
        command.envs(self.variables.iter().map(|(name, value)| (name, value)));
        let line = self.command_line();
        let _ = writeln!(log, "> {line}");
        let output = command
            .output()
            .map_err(|error| StepError::Failed(format!("cannot start `{line}`: {error}")))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let _ = writeln!(log, "{text}");
        if let Some((marker, reason)) = &self.unsupported
            && text.contains(marker.as_str())
        {
            return Err(StepError::Unsupported(reason.clone()));
        }
        if output.status.success() {
            Ok(())
        } else {
            Err(StepError::Failed(format!(
                "`{}` failed with {}: {}",
                self.program
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
                output.status,
                last_lines(&text, 3)
            )))
        }
    }
}

/// Quote an argument like the Microsoft C runtime expects, if it needs quotes.
pub fn quote(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '"']) {
        return argument.to_owned();
    }
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                // The pushed backslashes stay literal only if each one gets a partner.
                quoted.extend(std::iter::repeat_n('\\', backslashes + 1));
                backslashes = 0;
            }
            _ => backslashes = 0,
        }
        quoted.push(character);
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes));
    quoted.push('"');
    quoted
}

/// Join the last `count` non-empty lines of `text` with ` | `.
pub fn last_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    lines[lines.len().saturating_sub(count)..].join(" | ")
}

#[cfg(test)]
mod tests {
    use super::{Step, last_lines, quote};
    use std::path::Path;

    #[test]
    fn quotes_only_where_needed() {
        assert_eq!(quote("/Zi"), "/Zi");
        assert_eq!(quote(""), "\"\"");
        assert_eq!(quote(r"C:\Program Files\x"), r#""C:\Program Files\x""#);
        assert_eq!(quote(r"C:\out dir\"), r#""C:\out dir\\""#);
        assert_eq!(quote(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote(r#"a\"b c"#), r#""a\\\"b c""#);
    }

    #[test]
    fn command_line_quotes_program_and_arguments() {
        let step = Step::new(r"C:\Program Files\cl.exe", Path::new(".")).args(["/c", "a b.cpp"]);
        assert_eq!(
            step.command_line(),
            r#""C:\Program Files\cl.exe" /c "a b.cpp""#
        );
    }

    #[test]
    fn keeps_the_last_non_empty_lines() {
        assert_eq!(last_lines("a\n\n b \nc\n\n", 2), "b | c");
        assert_eq!(last_lines("only", 3), "only");
        assert_eq!(last_lines("", 3), "");
    }
}
