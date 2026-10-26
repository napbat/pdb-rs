//! File output through a temporary sibling file.
//!
//! The function writes the output to a new temporary file in the directory of the destination.
//! After a complete write, it flushes the file to storage and renames it to the destination.
//! The rename replaces an existing destination. On error, it deletes the temporary file.
//!
//! Caution: Do not give the path of the source file as the destination. The function does not
//! detect this case, and this design does not support it.

use std::path::Path;

use super::WritePlan;
use crate::Result;
use crate::io::write_new_file;

impl WritePlan<'_> {
    /// Write the container to the file `path`.
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
