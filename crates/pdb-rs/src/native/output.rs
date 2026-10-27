//! Checked little-endian output for native stream encoders.
//!
//! [`Output`] is the only field writer of the native encoders. It appends little-endian fields
//! to a caller `Vec<u8>` with fallible reservation. Encoders do not truncate values. A value
//! that does not fit its field gives an error.
//!
//! The `codeview::writer` module adds the `CodeView` methods to [`Output`]. These methods write
//! type indexes and ID indexes, and frame records with padding and a `u16` length field.

use alloc::vec::Vec;

use crate::{Error, ErrorKind, Result};

/// Return the error for an output allocation that failed.
pub(crate) fn no_memory() -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context("stream output cannot be allocated")
}

/// Convert a length or an offset to a `u32` field.
///
/// # Errors
///
/// Returns [`ErrorKind::LimitExceeded`] with `context` if the value does not fit.
pub(crate) fn to_u32(value: usize, context: &'static str) -> Result<u32> {
    u32::try_from(value)
        .map_err(|_| Error::new(ErrorKind::LimitExceeded, None).with_context(context))
}

/// An appender of little-endian fields.
pub(crate) struct Output<'v> {
    /// The output.
    out: &'v mut Vec<u8>,
}

impl<'v> Output<'v> {
    /// Create a writer that appends to `out`.
    pub(crate) const fn new(out: &'v mut Vec<u8>) -> Self {
        Self { out }
    }

    /// Return the output, for encoders of nested values.
    pub(crate) const fn out(&mut self) -> &mut Vec<u8> {
        self.out
    }

    /// Return the number of bytes in the output.
    pub(crate) const fn len(&self) -> usize {
        self.out.len()
    }

    /// Reserve space for `additional` more bytes.
    pub(crate) fn reserve(&mut self, additional: usize) -> Result<()> {
        self.out.try_reserve(additional).map_err(|_| no_memory())
    }

    /// Append bytes.
    pub(crate) fn bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.reserve(bytes.len())?;
        self.out.extend_from_slice(bytes);
        Ok(())
    }

    /// Append `count` zero bytes.
    pub(crate) fn zeros(&mut self, count: usize) -> Result<()> {
        self.reserve(count)?;
        self.out.resize(self.out.len() + count, 0);
        Ok(())
    }

    /// Append a `u8`.
    pub(crate) fn u8(&mut self, value: u8) -> Result<()> {
        self.bytes(&[value])
    }

    /// Append a little-endian `u16`.
    pub(crate) fn u16(&mut self, value: u16) -> Result<()> {
        self.bytes(&value.to_le_bytes())
    }

    /// Append a little-endian `u32`.
    pub(crate) fn u32(&mut self, value: u32) -> Result<()> {
        self.bytes(&value.to_le_bytes())
    }

    /// Append a little-endian `i32`.
    pub(crate) fn i32(&mut self, value: i32) -> Result<()> {
        self.bytes(&value.to_le_bytes())
    }

    /// Append a little-endian `u64`.
    pub(crate) fn u64(&mut self, value: u64) -> Result<()> {
        self.bytes(&value.to_le_bytes())
    }

    /// Append a name and its NUL terminator.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the name contains a NUL byte, because a reader
    /// would end the name at that byte.
    pub(crate) fn c_string(&mut self, name: &[u8]) -> Result<()> {
        if name.contains(&0) {
            return Err(
                Error::new(ErrorKind::InvalidFormat, None).with_context("name contains a NUL byte")
            );
        }
        self.reserve(name.len().saturating_add(1))?;
        self.out.extend_from_slice(name);
        self.out.push(0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::{Output, to_u32};
    use crate::ErrorKind;

    #[test]
    fn writes_little_endian_fields_and_names() {
        let mut bytes = Vec::new();
        let mut out = Output::new(&mut bytes);
        out.u8(1).unwrap();
        out.u16(0x0302).unwrap();
        out.u32(0x0706_0504).unwrap();
        out.c_string(b"ab").unwrap();
        out.zeros(2).unwrap();
        assert_eq!(bytes, vec![1, 2, 3, 4, 5, 6, 7, b'a', b'b', 0, 0, 0]);
    }

    #[test]
    fn rejects_values_that_do_not_fit() {
        let mut bytes = Vec::new();
        let error = Output::new(&mut bytes).c_string(b"a\0b").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat);
        assert_eq!(bytes.len(), 0);
        assert_eq!(to_u32(7, "size").unwrap(), 7);
        assert_eq!(
            to_u32(usize::MAX, "size").unwrap_err().kind(),
            ErrorKind::LimitExceeded
        );
    }
}
