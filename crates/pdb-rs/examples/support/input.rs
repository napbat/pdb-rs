//! The input option of the `native` and `portable` examples.
//!
//! The examples include this file with `#[path]`. The option selects how the example gives the
//! PDB bytes to the reader. All modes must give the same output.

use std::ffi::OsString;

/// How an example reads the input file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Input {
    /// `Pdb::open`: positional reads from the file through `pdb_rs::io::FileReader`.
    #[default]
    File,
    /// `Pdb::parse`: the example reads all file bytes, and the reader borrows the slice.
    Slice,
    /// `Pdb::read` with `pdb_rs::io::SeekReader` around `std::fs::File`: seek, then read.
    Seek,
}

impl Input {
    /// The text of the option values.
    pub const VALUES: &str = "file|slice|seek";

    fn parse(value: &str) -> Option<Self> {
        match value {
            "file" => Some(Self::File),
            "slice" => Some(Self::Slice),
            "seek" => Some(Self::Seek),
            _ => None,
        }
    }
}

/// Parse `[--input file|slice|seek] <path>` from the arguments after the program name.
///
/// # Errors
///
/// Returns `usage` if the path is missing or if there are other arguments, and a message if the
/// input mode is not known.
pub fn arguments(
    mut arguments: impl Iterator<Item = OsString>,
    usage: &str,
) -> Result<(Input, OsString), String> {
    let mut input = Input::default();
    let mut path = None;
    while let Some(argument) = arguments.next() {
        if argument == "--input" {
            let value = arguments.next().ok_or_else(|| usage.to_owned())?;
            input = value.to_str().and_then(Input::parse).ok_or_else(|| {
                format!(
                    "unknown input mode `{}`: use {}",
                    value.display(),
                    Input::VALUES
                )
            })?;
        } else if path.is_none() {
            path = Some(argument);
        } else {
            return Err(usage.to_owned());
        }
    }
    path.map(|path| (input, path))
        .ok_or_else(|| usage.to_owned())
}
