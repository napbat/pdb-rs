//! File output through a temporary sibling file.
//!
//! [`write_new_file`] writes the output to a new temporary file in the directory of the
//! destination. After a complete write, it flushes the file to storage and renames it to the
//! destination. The rename replaces an existing destination. On error, it deletes the
//! temporary file, so the destination does not change.
//!
//! Caution: Do not give the path of the source file as the destination. The function does not
//! detect this case, and this design does not support it.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::process;

use super::Write;
use super::copy::COPY_BUFFER_SIZE;
use crate::{Error, ErrorKind, Result};

/// The number of temporary file names to try before the function fails.
const TEMPORARY_ATTEMPTS: u32 = 64;

/// Write a temporary sibling of `path` with `write`, then rename it to `path`.
pub(crate) fn write_new_file(
    path: &Path,
    write: impl FnOnce(&mut BufWriter<File>) -> Result<()>,
) -> Result<()> {
    let (temporary, file) = create_temporary(path)?;
    let result = finish(path, &temporary, file, write);
    if result.is_err() {
        // The original error is more useful than an error from the cleanup.
        drop(fs::remove_file(&temporary));
    }
    result
}

/// Write and rename the temporary file.
fn finish(
    path: &Path,
    temporary: &Path,
    file: File,
    write: impl FnOnce(&mut BufWriter<File>) -> Result<()>,
) -> Result<()> {
    let mut output = BufWriter::with_capacity(COPY_BUFFER_SIZE, file);
    write(&mut output)?;
    output
        .flush()
        .map_err(|error| Error::from_io(error, None))?;
    let file = output
        .into_inner()
        .map_err(|error| Error::from_io(error.into_error(), None))?;
    file.sync_all()
        .map_err(|error| Error::from_io(error, None))?;
    drop(file);
    fs::rename(temporary, path)
        .map_err(|error| Error::from_io(error, None).with_context("temporary file rename failed"))
}

/// Create a new temporary file in the directory of `path`.
///
/// The name is `.<file name>.<process ID>-<attempt>.tmp`.
fn create_temporary(path: &Path) -> Result<(PathBuf, File)> {
    let name = path.file_name().ok_or_else(|| {
        Error::new(ErrorKind::Io, None).with_context("output path has no file name")
    })?;
    let id = process::id();
    for attempt in 0..TEMPORARY_ATTEMPTS {
        let mut temporary_name = OsString::from(".");
        temporary_name.push(name);
        temporary_name.push(format!(".{id}-{attempt}.tmp"));
        let temporary = path.with_file_name(temporary_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(
                    Error::from_io(error, None).with_context("temporary output file does not open")
                );
            }
        }
    }
    Err(Error::new(ErrorKind::Io, None).with_context("no free temporary output file name"))
}
