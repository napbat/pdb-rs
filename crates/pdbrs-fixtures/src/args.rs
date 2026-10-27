//! Command-line arguments.

use std::num::NonZeroUsize;
use std::path::PathBuf;

/// The usage text.
pub const USAGE: &str = "\
usage: cargo run -p pdbrs-fixtures -- [--out <dir>] [--only <name>]... [--list]
       cargo run -p pdbrs-fixtures -- tools
       cargo run -p pdbrs-fixtures -- verify-portable [--fixtures <dir>] [--jobs <n>]
       cargo run -p pdbrs-fixtures -- verify-rebuild [--fixtures <dir>] [--jobs <n>]
       cargo run -p pdbrs-fixtures -- verify-edit [--fixtures <dir>] [--jobs <n>]
       cargo run -p pdbrs-fixtures -- verify-native-build [--fixtures <dir>] [--jobs <n>]
       cargo run -p pdbrs-fixtures -- verify-native-edit [--fixtures <dir>] [--jobs <n>]
       cargo run -p pdbrs-fixtures -- verify-msfz [--fixtures <dir>] [--jobs <n>]
       cargo run -p pdbrs-fixtures -- verify-remap [--fixtures <dir>] [--jobs <n>]
       cargo run -p pdbrs-fixtures -- detect [--fixtures <dir>] [--jobs <n>]

  --out <dir>    Write the fixtures and manifest.json to <dir>.
                 The default is target/fixtures/ below the workspace root.
  --only <name>  Build only the named fixture. Give the option again for more fixtures.
                 The manifest keeps the entries of the other fixtures.
  --list         Print the fixture names and stop.
  --help         Print this text and stop.

  tools          Install pdbtool, build DIA2Dump, build the Portable PDB verifier and install
                 msfz-encode into target/tools/bin/ below the workspace root, and write
                 target/tools/manifest.json. The command takes no other arguments.

  verify-portable
                 Print the canonical dump of each Portable PDB fixture with pdbrs and with
                 the verifier, write both to target/verify-portable/ and compare them.

  verify-rebuild Rebuild each Portable PDB fixture with the pdbrs writer, write it to
                 target/verify-rebuild/ and compare its bytes, its ID and its verifier dump
                 with the original.

  verify-edit    Edit and build each Portable PDB fixture with the pdbrs editor, compare
                 the verifier dumps with the expected change, and run each standalone fixture
                 program with the built and the edited PDB. Then apply one field change or
                 one removal in each new edit case and compare the changed streams with the
                 plan report. Write the results to target/verify-edit/.

  verify-native-build
                 Rebuild each native PDB fixture with the pdbrs editor, write it to
                 target/verify-native-build/ and compare the original and the rebuilt PDB
                 with pdbtool, llvm-pdbutil, DIA2Dump and llvm-symbolizer.

  verify-native-edit
                 Apply each supported edit of the pdbrs native editor to each native PDB
                 fixture, write the results to target/verify-native-edit/ and check them with
                 pdbtool, llvm-pdbutil, DIA2Dump and pdbstr. Compare the strip with the
                 link /PDBSTRIPPED output of msvc-x64-stripped.

  verify-msfz    Write each native PDB fixture as MSFZ with the pdbrs writer and six option
                 sets, and check each output with pdbtool compare, DIA2Dump, the pdbrs reader
                 and the native example. Compare the outputs with the pdz-* fixtures of
                 pdbtool pdz-encode. Convert each pdz-* and msfz-* fixture to MSF 7.0 and
                 check it with pdbtool check, llvm-pdbutil and DIA2Dump; check the PDZ file
                 with pdbtool compare and the native example. Write the results to
                 target/verify-msfz/.

  verify-remap   Remap the addresses of each native PDB fixture with the pdbrs editor
                 (OMAP output) for five range sets, write the results to target/verify-remap/
                 and check them with DIA2Dump, pdbtool check and the plan report.

  detect         Open each fixture output with pdbrs::Pdb and compare the detected format
                 with the manifest. The CLI metadata of each image must be rejected.

  --fixtures <dir>
                 Read manifest.json and the outputs in <dir>. The default is
                 target/fixtures/ below the workspace root.

  --jobs <n>     Use <n> worker threads (n > 0) for the fixtures and the consumer tools.
                 The default is the available parallelism. --jobs 1 runs all work on one
                 thread. The command prints the results in fixture order.";

/// What the program must do.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Print the usage text.
    Help,
    /// Print the fixture names.
    List,
    /// Build fixtures.
    Build(Options),
    /// Install and build the consumer tools.
    Tools,
    /// Compare the pdbrs and verifier dumps of the Portable PDB fixtures.
    VerifyPortable(Check),
    /// Open each fixture output with the format facade.
    Detect(Check),
    /// Rebuild each Portable PDB fixture with the pdbrs writer and compare it with the
    /// original.
    VerifyRebuild(Check),
    /// Edit and build each Portable PDB fixture and compare the results with the verifier.
    VerifyEdit(Check),
    /// Rebuild each native PDB fixture with the pdbrs editor and compare it with the
    /// original through the consumer tools.
    VerifyNativeBuild(Check),
    /// Apply each native edit to each native PDB fixture and check the results through the
    /// consumer tools.
    VerifyNativeEdit(Check),
    /// Write MSFZ files from the native PDB fixtures, convert the PDZ fixtures to MSF 7.0 and
    /// check both through the consumer tools.
    VerifyMsfz(Check),
    /// Remap the addresses of each native PDB fixture and check the OMAP output through the
    /// consumer tools.
    VerifyRemap(Check),
}

/// The options of a command that reads the fixture manifest.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Check {
    /// The fixture root, if the user gave one.
    pub fixtures: Option<PathBuf>,
    /// The worker count, if the user gave one.
    pub jobs: Option<NonZeroUsize>,
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
    let mut arguments = arguments.into_iter().peekable();
    if arguments.next_if(|argument| argument == "tools").is_some() {
        return match arguments.next().as_deref() {
            None => Ok(Action::Tools),
            Some("--help" | "-h") => Ok(Action::Help),
            Some(other) => Err(format!("`tools` takes no arguments; found `{other}`")),
        };
    }
    for (command, action) in [
        ("verify-portable", Action::VerifyPortable as fn(_) -> _),
        ("detect", Action::Detect),
        ("verify-rebuild", Action::VerifyRebuild),
        ("verify-edit", Action::VerifyEdit),
        ("verify-native-build", Action::VerifyNativeBuild),
        ("verify-native-edit", Action::VerifyNativeEdit),
        ("verify-msfz", Action::VerifyMsfz),
        ("verify-remap", Action::VerifyRemap),
    ] {
        if arguments.next_if(|argument| argument == command).is_some() {
            return check(command, &mut arguments).map(action);
        }
    }
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

/// Parse the options of a command that reads the fixture manifest:
/// `[--fixtures <dir>] [--jobs <n>]`.
fn check(command: &str, mut arguments: impl Iterator<Item = String>) -> Result<Check, String> {
    let mut check = Check::default();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--fixtures" if check.fixtures.is_none() => {
                check.fixtures = Some(PathBuf::from(value(&mut arguments, "--fixtures")?));
            }
            "--jobs" if check.jobs.is_none() => {
                let text = value(&mut arguments, "--jobs")?;
                let jobs = text.parse().map_err(|_| {
                    format!("--jobs needs a whole number greater than 0; found `{text}`")
                })?;
                check.jobs = Some(jobs);
            }
            option @ ("--fixtures" | "--jobs") => {
                return Err(format!("{option} is given more than once"));
            }
            other => return Err(format!("`{command}` does not take `{other}`")),
        }
    }
    Ok(check)
}

#[cfg(test)]
mod tests {
    use super::{Action, Check, Options, parse};
    use std::num::NonZeroUsize;
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
    fn tools_is_a_command_without_options() {
        assert_eq!(run(&["tools"]), Ok(Action::Tools));
        assert_eq!(run(&["tools", "--help"]), Ok(Action::Help));
        assert_eq!(
            run(&["tools", "--out", "x"]),
            Err("`tools` takes no arguments; found `--out`".to_owned())
        );
        assert_eq!(
            run(&["--list", "tools"]),
            Err("unknown argument `tools`".to_owned())
        );
    }

    fn fixtures(path: &str) -> Check {
        Check {
            fixtures: Some(PathBuf::from(path)),
            jobs: None,
        }
    }

    fn jobs(count: usize) -> Option<NonZeroUsize> {
        NonZeroUsize::new(count)
    }

    #[test]
    fn fixture_commands_take_an_optional_fixture_root() {
        let default = Check::default;
        assert_eq!(
            run(&["verify-portable"]),
            Ok(Action::VerifyPortable(default()))
        );
        assert_eq!(
            run(&["verify-rebuild"]),
            Ok(Action::VerifyRebuild(default()))
        );
        assert_eq!(run(&["verify-edit"]), Ok(Action::VerifyEdit(default())));
        assert_eq!(
            run(&["verify-native-build", "--fixtures", "D:/f"]),
            Ok(Action::VerifyNativeBuild(fixtures("D:/f")))
        );
        assert_eq!(
            run(&["verify-native-edit"]),
            Ok(Action::VerifyNativeEdit(default()))
        );
        assert_eq!(run(&["verify-msfz"]), Ok(Action::VerifyMsfz(default())));
        assert_eq!(run(&["verify-remap"]), Ok(Action::VerifyRemap(default())));
        assert_eq!(
            run(&["detect", "--fixtures", "D:/f"]),
            Ok(Action::Detect(fixtures("D:/f")))
        );
        assert_eq!(
            run(&["detect", "--fixtures", "a", "--fixtures", "b"]),
            Err("--fixtures is given more than once".to_owned())
        );
        assert_eq!(
            run(&["verify-portable", "--out", "x"]),
            Err("`verify-portable` does not take `--out`".to_owned())
        );
    }

    #[test]
    fn fixture_commands_take_a_worker_count() {
        let with = |count| Check {
            fixtures: None,
            jobs: jobs(count),
        };
        for (command, action) in [
            ("verify-portable", Action::VerifyPortable as fn(_) -> _),
            ("detect", Action::Detect),
            ("verify-rebuild", Action::VerifyRebuild),
            ("verify-edit", Action::VerifyEdit),
            ("verify-native-build", Action::VerifyNativeBuild),
            ("verify-native-edit", Action::VerifyNativeEdit),
            ("verify-msfz", Action::VerifyMsfz),
            ("verify-remap", Action::VerifyRemap),
        ] {
            assert_eq!(run(&[command, "--jobs", "1"]), Ok(action(with(1))));
            assert_eq!(run(&[command, "--jobs", "12"]), Ok(action(with(12))));
        }
        assert_eq!(
            run(&["verify-msfz", "--jobs", "3", "--fixtures", "D:/f"]),
            Ok(Action::VerifyMsfz(Check {
                fixtures: Some(PathBuf::from("D:/f")),
                jobs: jobs(3),
            }))
        );
    }

    #[test]
    fn rejects_a_bad_worker_count() {
        let error = |text: &str| {
            Err(format!(
                "--jobs needs a whole number greater than 0; found `{text}`"
            ))
        };
        for text in ["0", "-1", "two", "1.5", ""] {
            assert_eq!(run(&["detect", "--jobs", text]), error(text));
        }
        assert_eq!(
            run(&["detect", "--jobs"]),
            Err("--jobs needs a value".to_owned())
        );
        assert_eq!(
            run(&["detect", "--jobs", "--fixtures", "a"]),
            Err("--jobs needs a value".to_owned())
        );
        assert_eq!(
            run(&["detect", "--jobs", "2", "--jobs", "2"]),
            Err("--jobs is given more than once".to_owned())
        );
        assert_eq!(
            run(&["--jobs", "2"]),
            Err("unknown argument `--jobs`".to_owned())
        );
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
