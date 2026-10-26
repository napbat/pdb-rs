//! Command-line arguments.

use std::path::PathBuf;

/// The usage text.
pub const USAGE: &str = "\
usage: cargo run -p pdbrs-fixtures -- [--out <dir>] [--only <name>]... [--list]

  --out <dir>    Write the fixtures and manifest.json to <dir>.
                 The default is target/fixtures/ below the workspace root.
  --only <name>  Build only the named fixture. Give the option again for more fixtures.
  --list         Print the fixture names and stop.
  --help         Print this text and stop.";

/// What the program must do.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Print the usage text.
    Help,
    /// Print the fixture names.
    List,
    /// Build fixtures.
    Build(Options),
}

/// The options of a build.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// The output root, if the user gave one.
    pub out: Option<PathBuf>,
    /// The fixture names to build. An empty list selects all fixtures.
    pub only: Vec<String>,
}

/// Parse the arguments that follow the program name.
pub fn parse<I>(arguments: I) -> Result<Action, String>
where
    I: IntoIterator<Item = String>,
{
    let mut options = Options::default();
    let mut list = false;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--help" | "-h" => return Ok(Action::Help),
            "--list" => list = true,
            "--out" => {
                if options.out.is_some() {
                    return Err("--out is given more than once".to_owned());
                }
                options.out = Some(PathBuf::from(value(&mut arguments, "--out")?));
            }
            "--only" => options.only.push(value(&mut arguments, "--only")?),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    if list {
        Ok(Action::List)
    } else {
        Ok(Action::Build(options))
    }
}

fn value(arguments: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    match arguments.next() {
        Some(value) if !value.starts_with("--") => Ok(value),
        _ => Err(format!("{option} needs a value")),
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, Options, parse};
    use std::path::PathBuf;

    fn run(arguments: &[&str]) -> Result<Action, String> {
        parse(arguments.iter().map(|argument| (*argument).to_owned()))
    }

    #[test]
    fn no_arguments_build_everything() {
        assert_eq!(run(&[]), Ok(Action::Build(Options::default())));
    }

    #[test]
    fn collects_out_and_repeated_only() {
        let expected = Options {
            out: Some(PathBuf::from("C:/out dir")),
            only: vec!["msvc-x64-debug".to_owned(), "rust-lld-x64-debug".to_owned()],
        };
        assert_eq!(
            run(&[
                "--only",
                "msvc-x64-debug",
                "--out",
                "C:/out dir",
                "--only",
                "rust-lld-x64-debug"
            ]),
            Ok(Action::Build(expected))
        );
    }

    #[test]
    fn list_and_help_stop_the_build() {
        assert_eq!(run(&["--only", "a", "--list"]), Ok(Action::List));
        assert_eq!(run(&["--list", "--help"]), Ok(Action::Help));
    }

    #[test]
    fn rejects_missing_values_and_unknown_arguments() {
        assert_eq!(run(&["--out"]), Err("--out needs a value".to_owned()));
        assert_eq!(
            run(&["--only", "--list"]),
            Err("--only needs a value".to_owned())
        );
        assert_eq!(
            run(&["--fast"]),
            Err("unknown argument `--fast`".to_owned())
        );
        assert_eq!(
            run(&["--out", "a", "--out", "b"]),
            Err("--out is given more than once".to_owned())
        );
    }
}
