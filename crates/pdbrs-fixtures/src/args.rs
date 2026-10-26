//! Command-line arguments.

use std::path::PathBuf;

/// The usage text.
pub const USAGE: &str = "\
usage: cargo run -p pdbrs-fixtures -- [--out <dir>] [--only <name>]... [--list]
       cargo run -p pdbrs-fixtures -- tools
       cargo run -p pdbrs-fixtures -- verify-portable [--fixtures <dir>]
       cargo run -p pdbrs-fixtures -- verify-rebuild [--fixtures <dir>]
       cargo run -p pdbrs-fixtures -- verify-edit [--fixtures <dir>]
       cargo run -p pdbrs-fixtures -- verify-native-build [--fixtures <dir>]
       cargo run -p pdbrs-fixtures -- verify-native-edit [--fixtures <dir>]
       cargo run -p pdbrs-fixtures -- verify-msfz [--fixtures <dir>]
       cargo run -p pdbrs-fixtures -- detect [--fixtures <dir>]

  --out <dir>    Write the fixtures and manifest.json to <dir>.
                 The default is target/fixtures/ below the workspace root.
  --only <name>  Build only the named fixture. Give the option again for more fixtures.
                 The manifest keeps the entries of the other fixtures.
  --list         Print the fixture names and stop.
  --help         Print this text and stop.

  tools          Install pdbtool, build DIA2Dump and build the Portable PDB verifier into
                 target/tools/bin/ below the workspace root, and write
                 target/tools/manifest.json. The command takes no other arguments.

  verify-portable
                 Print the canonical dump of each Portable PDB fixture with pdbrs and with
                 the verifier, write both to target/verify-portable/ and compare them.

  verify-rebuild Rebuild each Portable PDB fixture with the pdbrs writer, write it to
                 target/verify-rebuild/ and compare its bytes, its ID and its verifier dump
                 with the original.

  verify-edit    Edit and build each Portable PDB fixture with the pdbrs editor, compare
                 the verifier dumps with the expected change, and run each standalone fixture
                 program with the built and the edited PDB. Write the results to
                 target/verify-edit/.

  verify-native-build
                 Rebuild each native PDB fixture with the pdbrs builder, write it to
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
                 pdbtool pdz-encode. Convert each pdz-* fixture to MSF 7.0 and check it with
                 pdbtool check, llvm-pdbutil and DIA2Dump. Write the results to
                 target/verify-msfz/.

  detect         Open each fixture output with pdbrs::Pdb and compare the detected format
                 with the manifest. The CLI metadata of each image must be rejected.

  --fixtures <dir>
                 Read manifest.json and the outputs in <dir>. The default is
                 target/fixtures/ below the workspace root.";

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
    /// Compare the pdbrs and verifier dumps of the Portable PDB fixtures. The value is the
    /// fixture root, if the user gave one.
    VerifyPortable(Option<PathBuf>),
    /// Open each fixture output with the format facade. The value is the fixture root, if
    /// the user gave one.
    Detect(Option<PathBuf>),
    /// Rebuild each Portable PDB fixture with the pdbrs writer and compare it with the
    /// original. The value is the fixture root, if the user gave one.
    VerifyRebuild(Option<PathBuf>),
    /// Edit and build each Portable PDB fixture and compare the results with the verifier.
    /// The value is the fixture root, if the user gave one.
    VerifyEdit(Option<PathBuf>),
    /// Rebuild each native PDB fixture with the pdbrs builder and compare it with the
    /// original through the consumer tools. The value is the fixture root, if the user gave
    /// one.
    VerifyNativeBuild(Option<PathBuf>),
    /// Apply each native edit to each native PDB fixture and check the results through the
    /// consumer tools. The value is the fixture root, if the user gave one.
    VerifyNativeEdit(Option<PathBuf>),
    /// Write MSFZ files from the native PDB fixtures, convert the PDZ fixtures to MSF 7.0 and
    /// check both through the consumer tools. The value is the fixture root, if the user gave
    /// one.
    VerifyMsfz(Option<PathBuf>),
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
    ] {
        if arguments.next_if(|argument| argument == command).is_some() {
            return fixture_root(command, &mut arguments).map(action);
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

/// Parse the options of a command that reads the fixture manifest: `[--fixtures <dir>]`.
fn fixture_root(
    command: &str,
    mut arguments: impl Iterator<Item = String>,
) -> Result<Option<PathBuf>, String> {
    let mut fixtures = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--fixtures" if fixtures.is_none() => {
                fixtures = Some(PathBuf::from(value(&mut arguments, "--fixtures")?));
            }
            "--fixtures" => return Err("--fixtures is given more than once".to_owned()),
            other => return Err(format!("`{command}` does not take `{other}`")),
        }
    }
    Ok(fixtures)
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

    #[test]
    fn fixture_commands_take_an_optional_fixture_root() {
        assert_eq!(run(&["verify-portable"]), Ok(Action::VerifyPortable(None)));
        assert_eq!(run(&["verify-rebuild"]), Ok(Action::VerifyRebuild(None)));
        assert_eq!(run(&["verify-edit"]), Ok(Action::VerifyEdit(None)));
        assert_eq!(
            run(&["verify-native-build", "--fixtures", "D:/f"]),
            Ok(Action::VerifyNativeBuild(Some(PathBuf::from("D:/f"))))
        );
        assert_eq!(
            run(&["verify-native-edit"]),
            Ok(Action::VerifyNativeEdit(None))
        );
        assert_eq!(run(&["verify-msfz"]), Ok(Action::VerifyMsfz(None)));
        assert_eq!(
            run(&["detect", "--fixtures", "D:/f"]),
            Ok(Action::Detect(Some(PathBuf::from("D:/f"))))
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
