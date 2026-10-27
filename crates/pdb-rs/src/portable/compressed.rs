//! Compressed integers of ECMA-335 metadata blobs (partition II, section 23.2).
//!
//! A compressed integer uses 1, 2 or 4 bytes in big-endian order. The high
//! bits of the first byte give the length:
//!
//! | First byte | Length | Value bits |
//! | --- | --- | --- |
//! | `0b0xxx_xxxx` | 1 | 7 |
//! | `0b10xx_xxxx` | 2 | 14 |
//! | `0b110x_xxxx` | 4 | 29 |
//!
//! A signed value uses the same lengths. Its two's complement bits are
//! rotated left by one bit within the value bits, so bit 0 holds the sign.
//! The length of a signed value comes from its range, not from the rotated
//! bits: -64 is `0x01` and -8192 is `0x80 0x01`.
//!
//! The decoders accept an encoding that is longer than necessary, as
//! `System.Reflection.Metadata` does. The encoders always write the shortest
//! encoding.

use crate::{Error, ErrorKind, Result};

/// The largest value that a compressed unsigned integer can hold.
pub const MAX_UNSIGNED: u32 = 0x1fff_ffff;
/// The smallest value that a compressed signed integer can hold.
pub const MIN_SIGNED: i32 = -0x1000_0000;
/// The largest value that a compressed signed integer can hold.
pub const MAX_SIGNED: i32 = 0x0fff_ffff;

/// The error context for input that ends inside a compressed integer.
const TRUNCATED: &str = "compressed integer is truncated";

/// The bytes of one encoded compressed integer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Encoded {
    bytes: [u8; 4],
    len: usize,
}

impl Encoded {
    /// Return the encoded bytes (1, 2 or 4 bytes).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// Decode a compressed unsigned integer at the start of `bytes`.
///
/// Return the value and the bytes after it.
///
/// # Errors
///
/// Return [`ErrorKind::UnexpectedEof`] if `bytes` ends inside the integer, and
/// [`ErrorKind::InvalidFormat`] if the first byte starts with `0b111`.
pub fn decode_unsigned(bytes: &[u8]) -> Result<(u32, &[u8])> {
    let (value, _, rest) = decode_raw(bytes)?;
    Ok((value, rest))
}

/// Decode a compressed signed integer at the start of `bytes`.
///
/// Return the value and the bytes after it.
///
/// # Errors
///
/// Return the same errors as [`decode_unsigned`].
pub fn decode_signed(bytes: &[u8]) -> Result<(i32, &[u8])> {
    let (raw, bits, rest) = decode_raw(bytes)?;
    // `raw` has at most 29 bits, so the shifted value fits in an `i32`.
    let magnitude = (raw >> 1).cast_signed();
    let value = if raw & 1 == 0 {
        magnitude
    } else {
        magnitude - (1 << (bits - 1))
    };
    Ok((value, rest))
}

/// Encode a compressed unsigned integer.
///
/// # Errors
///
/// Return [`ErrorKind::LimitExceeded`] if `value` is larger than
/// [`MAX_UNSIGNED`].
pub fn encode_unsigned(value: u32) -> Result<Encoded> {
    let bits = match value {
        0..0x80 => 7,
        0x80..0x4000 => 14,
        0x4000..=MAX_UNSIGNED => 29,
        _ => return Err(too_large()),
    };
    Ok(encode_raw(value, bits))
}

/// Encode a compressed signed integer.
///
/// # Errors
///
/// Return [`ErrorKind::LimitExceeded`] if `value` is outside
/// [`MIN_SIGNED`]..=[`MAX_SIGNED`].
pub fn encode_signed(value: i32) -> Result<Encoded> {
    let bits = match value {
        -0x40..0x40 => 7,
        -0x2000..0x2000 => 14,
        MIN_SIGNED..=MAX_SIGNED => 29,
        _ => return Err(too_large()),
    };
    let mask = (1_u32 << bits) - 1;
    let sign = u32::from(value < 0);
    let raw = ((value.cast_unsigned() << 1) | sign) & mask;
    Ok(encode_raw(raw, bits))
}

/// Decode the raw bits of a compressed integer.
///
/// Return the bits, the number of value bits of the encoding and the rest.
fn decode_raw(bytes: &[u8]) -> Result<(u32, u32, &[u8])> {
    let truncated = || Error::new(ErrorKind::UnexpectedEof, None).with_context(TRUNCATED);
    let (&first, rest) = bytes.split_first().ok_or_else(truncated)?;
    match first {
        0x00..=0x7f => Ok((u32::from(first), 7, rest)),
        0x80..=0xbf => {
            let [second, rest @ ..] = rest else {
                return Err(truncated());
            };
            let value = u32::from_be_bytes([0, 0, first & 0x3f, *second]);
            Ok((value, 14, rest))
        }
        0xc0..=0xdf => {
            let [b1, b2, b3, rest @ ..] = rest else {
                return Err(truncated());
            };
            let value = u32::from_be_bytes([first & 0x1f, *b1, *b2, *b3]);
            Ok((value, 29, rest))
        }
        _ => Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("compressed integer has an invalid first byte")),
    }
}

/// Encode raw bits with the length of a `bits`-bit encoding.
fn encode_raw(raw: u32, bits: u32) -> Encoded {
    let [b0, b1, b2, b3] = raw.to_be_bytes();
    match bits {
        7 => Encoded {
            bytes: [b3, 0, 0, 0],
            len: 1,
        },
        14 => Encoded {
            bytes: [0x80 | b2, b3, 0, 0],
            len: 2,
        },
        _ => Encoded {
            bytes: [0xc0 | b0, b1, b2, b3],
            len: 4,
        },
    }
}

/// The error for a value that does not fit in a compressed integer.
fn too_large() -> Error {
    Error::new(ErrorKind::LimitExceeded, None)
        .with_context("value does not fit in a compressed integer")
}
