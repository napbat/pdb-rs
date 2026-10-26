//! Integer numeric leaves shared by type records and symbol records.
//!
//! The first two bytes hold an unsigned value below `0x8000`, or a leaf
//! tag followed by a fixed-width integer. Values keep their signedness.
//! This decoder does not allocate or consume bytes after the integer.
//!
//! Floating-point, complex, decimal, date and string leaves need separate
//! representations. This integer decoder rejects them rather than guessing
//! their size or converting their value.

use super::Fields;
use crate::{Error, ErrorKind, Result};

/// An integer decoded from a `CodeView` numeric leaf.
///
/// Values are widened without loss. The original encoding width is not kept.
/// Keep the input bytes when exact re-encoding is required.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Integer {
    /// A signed integer, including signed 128-bit values.
    Signed(i128),
    /// An unsigned integer, including unsigned 128-bit values.
    Unsigned(u128),
}

impl Integer {
    /// Decode one integer and return the bytes after it.
    ///
    /// Supports immediate values and `LF_CHAR`, `LF_SHORT`, `LF_USHORT`,
    /// `LF_LONG`, `LF_ULONG`, `LF_QUADWORD`, `LF_UQUADWORD`, `LF_OCTWORD`
    /// and `LF_UOCTWORD`. The suffix can contain a name, another field or
    /// record padding. No alignment is applied here.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::UnexpectedEof`] for a truncated tag or integer.
    /// Returns [`ErrorKind::UnsupportedVersion`] for any other leaf tag,
    /// including known non-integer tags. Their payloads are not inspected.
    /// Errors have no physical offset; callers can attach record context.
    pub fn parse_prefix(bytes: &[u8]) -> Result<(Self, &[u8])> {
        let mut fields = Fields::new(
            bytes,
            ErrorKind::UnexpectedEof,
            "CodeView integer numeric leaf is truncated",
        );
        let tag = fields.u16()?;
        let value = match tag {
            0..0x8000 => Self::Unsigned(u128::from(tag)),
            0x8000 => Self::Signed(i128::from(i8::from_le_bytes(fields.array()?))),
            0x8001 => Self::Signed(i128::from(i16::from_le_bytes(fields.array()?))),
            0x8002 => Self::Unsigned(u128::from(fields.u16()?)),
            0x8003 => Self::Signed(i128::from(fields.i32()?)),
            0x8004 => Self::Unsigned(u128::from(fields.u32()?)),
            0x8009 => Self::Signed(i128::from(i64::from_le_bytes(fields.array()?))),
            0x800a => Self::Unsigned(u128::from(u64::from_le_bytes(fields.array()?))),
            0x8017 => Self::Signed(i128::from_le_bytes(fields.array()?)),
            0x8018 => Self::Unsigned(u128::from_le_bytes(fields.array()?)),
            _ => {
                return Err(Error::new(ErrorKind::UnsupportedVersion, None)
                    .with_context("CodeView numeric leaf is not a supported integer"));
            }
        };
        Ok((value, fields.rest()))
    }
}
