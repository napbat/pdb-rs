//! Field readers and writers for Portable PDB blobs.
//!
//! The custom debug information codecs and the debug table blob codecs of
//! [`crate::portable::debug`] use these helpers. Fixed-size integers are
//! little-endian. Compressed integers use [`crate::portable::compressed`].
//! Strings are UTF-8 bytes with a NUL terminator. The errors have no location;
//! the caller adds it.

use alloc::vec::Vec;

use crate::portable::compressed;
use crate::{Error, ErrorKind, Result};

/// Return an [`ErrorKind::InvalidFormat`] error with `context`.
pub(in crate::portable) const fn invalid(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, None).with_context(context)
}

/// Return an [`ErrorKind::UnexpectedEof`] error with `context`.
pub(in crate::portable) const fn truncated(context: &'static str) -> Error {
    Error::new(ErrorKind::UnexpectedEof, None).with_context(context)
}

/// Return an [`ErrorKind::LimitExceeded`] error with `context`.
pub(in crate::portable) const fn too_large(context: &'static str) -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context(context)
}

/// A cursor over the bytes of a blob.
#[derive(Clone, Debug)]
pub(in crate::portable) struct Reader<'a> {
    /// The bytes that are not read yet.
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    /// Start to read `bytes`.
    pub(in crate::portable) const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// Return the bytes that are not read yet.
    pub(in crate::portable) const fn rest(&self) -> &'a [u8] {
        self.bytes
    }

    /// Return `true` if all bytes are read.
    pub(in crate::portable) const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Read an array of `N` bytes.
    pub(in crate::portable) fn array<const N: usize>(
        &mut self,
        context: &'static str,
    ) -> Result<[u8; N]> {
        let (array, rest) = self
            .bytes
            .split_first_chunk::<N>()
            .ok_or_else(|| truncated(context))?;
        self.bytes = rest;
        Ok(*array)
    }

    /// Read a byte.
    pub(in crate::portable) fn u8(&mut self, context: &'static str) -> Result<u8> {
        Ok(self.array::<1>(context)?[0])
    }

    /// Read a little-endian `u16`.
    pub(in crate::portable) fn u16(&mut self, context: &'static str) -> Result<u16> {
        self.array(context).map(u16::from_le_bytes)
    }

    /// Read a little-endian `u32`.
    pub(in crate::portable) fn u32(&mut self, context: &'static str) -> Result<u32> {
        self.array(context).map(u32::from_le_bytes)
    }

    /// Read a little-endian `u64`.
    pub(in crate::portable) fn u64(&mut self, context: &'static str) -> Result<u64> {
        self.array(context).map(u64::from_le_bytes)
    }

    /// Read a compressed unsigned integer.
    pub(in crate::portable) fn unsigned(&mut self) -> Result<u32> {
        let (value, rest) = compressed::decode_unsigned(self.bytes)?;
        self.bytes = rest;
        Ok(value)
    }

    /// Read a compressed signed integer.
    pub(in crate::portable) fn signed(&mut self) -> Result<i32> {
        let (value, rest) = compressed::decode_signed(self.bytes)?;
        self.bytes = rest;
        Ok(value)
    }

    /// Read a NUL-terminated string and return it without the NUL byte.
    pub(in crate::portable) fn c_string(&mut self, context: &'static str) -> Result<&'a [u8]> {
        let end = self
            .bytes
            .iter()
            .position(|&byte| byte == 0)
            .ok_or_else(|| truncated(context))?;
        let string = &self.bytes[..end];
        self.bytes = &self.bytes[end + 1..];
        Ok(string)
    }
}

/// A growing blob.
#[derive(Debug, Default)]
pub(in crate::portable) struct Writer {
    /// The bytes written so far.
    bytes: Vec<u8>,
}

impl Writer {
    /// Start an empty blob.
    pub(in crate::portable) const fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    /// Append `bytes`.
    pub(in crate::portable) fn bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.bytes
            .try_reserve(bytes.len())
            .map_err(|_| too_large("blob allocation failed"))?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    /// Append a byte.
    pub(in crate::portable) fn u8(&mut self, value: u8) -> Result<()> {
        self.bytes(&[value])
    }

    /// Append a little-endian `u16`.
    pub(in crate::portable) fn u16(&mut self, value: u16) -> Result<()> {
        self.bytes(&value.to_le_bytes())
    }

    /// Append a little-endian `u32`.
    pub(in crate::portable) fn u32(&mut self, value: u32) -> Result<()> {
        self.bytes(&value.to_le_bytes())
    }

    /// Append a little-endian `u64`.
    pub(in crate::portable) fn u64(&mut self, value: u64) -> Result<()> {
        self.bytes(&value.to_le_bytes())
    }

    /// Append a compressed unsigned integer.
    pub(in crate::portable) fn unsigned(&mut self, value: u32) -> Result<()> {
        self.bytes(compressed::encode_unsigned(value)?.as_bytes())
    }

    /// Append a compressed unsigned integer from a wider value.
    pub(in crate::portable) fn unsigned_wide(&mut self, value: i64) -> Result<()> {
        let value = u32::try_from(value)
            .map_err(|_| too_large("value does not fit in a compressed unsigned integer"))?;
        self.unsigned(value)
    }

    /// Append a compressed signed integer.
    pub(in crate::portable) fn signed(&mut self, value: i32) -> Result<()> {
        self.bytes(compressed::encode_signed(value)?.as_bytes())
    }

    /// Append `string` and a NUL terminator.
    ///
    /// Return [`ErrorKind::InvalidFormat`] with `context` if `string` has a
    /// NUL byte.
    pub(in crate::portable) fn c_string(
        &mut self,
        string: &[u8],
        context: &'static str,
    ) -> Result<()> {
        if string.contains(&0) {
            return Err(invalid(context));
        }
        self.bytes(string)?;
        self.u8(0)
    }

    /// Return the blob.
    pub(in crate::portable) fn finish(self) -> Vec<u8> {
        self.bytes
    }
}
