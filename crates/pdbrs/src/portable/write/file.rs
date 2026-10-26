//! File output through a temporary sibling file.
//!
//! Caution: Do not give the path of the source file as the destination. The functions do not
//! detect this case, and this design does not support it.

use std::path::Path;

use super::WritePlan;
use crate::Result;
use crate::io::{ReadAt, write_new_file};
use crate::portable::Pdb;

impl WritePlan {
    /// Write the Portable PDB to the file `path`.
    ///
    /// The function writes a temporary file in the same directory and renames it to `path`.
    /// The rename replaces an existing file.
    ///
    /// Caution: Do not give the path of the source file. This operation is not supported.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Io`](crate::ErrorKind::Io) if the temporary file does not open, a write
    ///   fails, or the rename fails.
    /// - The errors of [`WritePlan::write_to`].
    ///
    /// After an error, the function deletes the temporary file. The file at `path` does not
    /// change.
    pub fn write_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        write_new_file(path.as_ref(), |output| self.write_to(output))
    }
}

impl<S: ReadAt> Pdb<S> {
    /// Copy all bytes of the source to the file `path` without a change.
    ///
    /// The function uses [`Pdb::copy_exact`] to write a temporary file in the same directory.
    /// Then it renames the temporary file to `path`. The rename replaces an existing file. The
    /// function returns the number of bytes.
    ///
    /// Caution: Do not give the path of the source file. This operation is not supported.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Io`](crate::ErrorKind::Io) if the temporary file does not open, a write
    ///   fails, or the rename fails.
    /// - The errors of [`Pdb::copy_exact`].
    ///
    /// After an error, the function deletes the temporary file. The file at `path` does not
    /// change.
    pub fn copy_exact_file<P: AsRef<Path>>(&self, path: P) -> Result<u64> {
        let mut length = 0;
        write_new_file(path.as_ref(), |output| {
            length = self.copy_exact(output)?;
            Ok(())
        })?;
        Ok(length)
    }
}
