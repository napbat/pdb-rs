//! Environment blocks captured from `set` and program lookup on their `PATH`.

use std::path::{Path, PathBuf};

/// A complete environment block for a child process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment {
    variables: Vec<(String, String)>,
}

impl Environment {
    /// Parse the output of the `cmd` command `set`.
    ///
    /// Each line has the form `NAME=value`. The parser ignores empty lines, lines without
    /// `=` and lines that start with `=`. A later line replaces an earlier line with the same
    /// name. Names compare without case, as on Windows.
    pub fn parse_set_output(output: &str) -> Self {
        let mut environment = Self::default();
        for line in output.lines() {
            let line = line.trim_end_matches('\r');
            if let Some((name, value)) = line.split_once('=')
                && !name.is_empty()
            {
                environment.set(name, value);
            }
        }
        environment
    }

    /// Set a variable. The name compares without case.
    pub fn set(&mut self, name: &str, value: &str) {
        match self.position(name) {
            Some(index) => value.clone_into(&mut self.variables[index].1),
            None => self.variables.push((name.to_owned(), value.to_owned())),
        }
    }

    /// Get the value of a variable. The name compares without case.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.position(name)
            .map(|index| self.variables[index].1.as_str())
    }

    /// Iterate over the variables in the order of the source.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.variables
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// Return `true` if the block has no variables.
    pub fn is_empty(&self) -> bool {
        self.variables.is_empty()
    }

    /// Find `file` in the directories of `PATH`, in order.
    pub fn find_program(&self, file: &str) -> Option<PathBuf> {
        let path = self.get("PATH")?;
        path.split(';')
            .map(|directory| directory.trim().trim_matches('"'))
            .filter(|directory| !directory.is_empty())
            .map(|directory| Path::new(directory).join(file))
            .find(|candidate| candidate.is_file())
    }

    fn position(&self, name: &str) -> Option<usize> {
        self.variables
            .iter()
            .position(|(existing, _)| existing.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::Environment;

    #[test]
    fn parses_set_lines_and_ignores_noise() {
        let output = "ALLUSERSPROFILE=C:\\ProgramData\r\n\r\nnot a variable\r\n=C:=C:\\x\r\n\
                      INCLUDE=C:\\a;C:\\b\r\nEQUALS=a=b\r\n";
        let environment = Environment::parse_set_output(output);
        let variables: Vec<_> = environment.iter().collect();
        assert_eq!(
            variables,
            [
                ("ALLUSERSPROFILE", "C:\\ProgramData"),
                ("INCLUDE", "C:\\a;C:\\b"),
                ("EQUALS", "a=b"),
            ]
        );
    }

    #[test]
    fn names_compare_without_case() {
        let mut environment = Environment::parse_set_output("Path=C:\\one\nPATH=C:\\two\n");
        assert_eq!(environment.get("path"), Some("C:\\two"));
        assert_eq!(environment.iter().count(), 1);
        environment.set("pAtH", "C:\\three");
        assert_eq!(environment.get("PATH"), Some("C:\\three"));
    }

    #[test]
    fn keeps_empty_values() {
        let environment = Environment::parse_set_output("EMPTY=\n");
        assert_eq!(environment.get("EMPTY"), Some(""));
    }

    #[test]
    fn finds_programs_on_path() {
        let directory =
            std::env::temp_dir().join(format!("pdbrs-fixtures-env-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let program = directory.join("tool.exe");
        std::fs::write(&program, b"").unwrap();
        let mut environment = Environment::default();
        environment.set(
            "PATH",
            &format!("C:\\missing-dir;;\"{}\"", directory.display()),
        );
        assert_eq!(environment.find_program("tool.exe"), Some(program));
        assert_eq!(environment.find_program("other.exe"), None);
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
