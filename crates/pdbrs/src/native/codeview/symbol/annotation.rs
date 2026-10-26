//! Binary annotations of inline sites.
//!
//! An `S_INLINESITE` record ends with a sequence of annotations. Each
//! annotation has an opcode and one or two operands. The opcode and the
//! operands are compressed unsigned integers. The opcodes come from
//! `BinaryAnnotationsOpCode` in LLVM `CodeView.h` and
//! `BinaryAnnotationOpcode` in `cvinfo.h`. The decoding follows
//! `BinaryAnnotationIterator` in LLVM `SymbolRecord.h` and
//! `CVUncompressData` and `DecodeSignedInt32` in `cvinfo.h`.
//!
//! A compressed integer has 1, 2 or 4 bytes. The high bits of the first byte
//! give the length:
//!
//! - `0xxxxxxx`: one byte with 7 value bits.
//! - `10xxxxxx`: two bytes with 14 value bits, big-endian.
//! - `110xxxxx`: four bytes with 29 value bits, big-endian.
//!
//! The opcode 0 (`BA_OP_Invalid`) is not an annotation. `cvinfo.h` says that
//! a linked PDB uses it as padding. The decoder accepts it only as the record
//! padding at the end: fewer than four bytes, all zero.

use core::iter::FusedIterator;

use crate::{Error, ErrorKind, Result};

/// One decoded binary annotation.
///
/// The names follow `BinaryAnnotationsOpCode` in LLVM. Operands are
/// unsigned except for line offsets and column end deltas.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Annotation {
    /// Opcode 1: set the code offset.
    CodeOffset(u32),
    /// Opcode 2: select a separated code chunk. The main chunk is 0.
    ChangeCodeOffsetBase(u32),
    /// Opcode 3: add a delta to the code offset.
    ChangeCodeOffset(u32),
    /// Opcode 4: set the length of the code range.
    ChangeCodeLength(u32),
    /// Opcode 5: set the file. The value is an offset in the file checksum
    /// subsection of the module.
    ChangeFile(u32),
    /// Opcode 6: add a signed delta to the line number.
    ChangeLineOffset(i32),
    /// Opcode 7: set the number of lines in the range. The default is 1.
    ChangeLineEndDelta(u32),
    /// Opcode 8: set the range kind: 1 for a statement, 0 for an expression.
    ChangeRangeKind(u32),
    /// Opcode 9: set the start column. The value 0 means no column information.
    ChangeColumnStart(u32),
    /// Opcode 10: add a signed delta to the end column.
    ChangeColumnEndDelta(i32),
    /// Opcode 11: add a code delta and a line delta.
    ///
    /// The operand holds the code delta in bits 0 to 3 and the signed line
    /// delta above them.
    ChangeCodeOffsetAndLineOffset {
        /// The delta to add to the code offset.
        code_delta: u32,
        /// The delta to add to the line number.
        line_delta: i32,
    },
    /// Opcode 12: set the code length and add a code delta.
    ///
    /// The first operand is the length and the second operand is the delta.
    ChangeCodeLengthAndCodeOffset {
        /// The length of the code range.
        code_length: u32,
        /// The delta to add to the code offset.
        code_delta: u32,
    },
    /// Opcode 13: set the end column.
    ChangeColumnEnd(u32),
}

/// A validated sequence of binary annotations.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct BinaryAnnotations<'a> {
    /// The annotation bytes without the trailing padding.
    bytes: &'a [u8],
}

impl<'a> BinaryAnnotations<'a> {
    /// Validate the annotations in `bytes`.
    ///
    /// The bytes can end with record padding: fewer than four zero bytes.
    /// The returned value does not include the padding.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] for an opcode that is not known,
    /// a compressed integer with an invalid first byte, a truncated
    /// annotation, or zero bytes that are not valid padding.
    pub fn new(bytes: &'a [u8]) -> Result<Self> {
        let mut rest = bytes;
        while let Some(&first) = rest.first() {
            if first == 0 {
                if rest.len() >= 4 || rest.iter().any(|&byte| byte != 0) {
                    return Err(Error::new(ErrorKind::InvalidFormat, None)
                        .with_context("binary annotation padding is not valid"));
                }
                break;
            }
            decode(&mut rest)?;
        }
        Ok(Self {
            bytes: &bytes[..bytes.len() - rest.len()],
        })
    }

    /// Return the annotation bytes without the trailing padding.
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Return an iterator over the annotations.
    #[must_use]
    pub const fn iter(&self) -> Annotations<'a> {
        Annotations { bytes: self.bytes }
    }
}

impl<'a> IntoIterator for BinaryAnnotations<'a> {
    type Item = Annotation;
    type IntoIter = Annotations<'a>;

    fn into_iter(self) -> Annotations<'a> {
        self.iter()
    }
}

impl<'a> IntoIterator for &BinaryAnnotations<'a> {
    type Item = Annotation;
    type IntoIter = Annotations<'a>;

    fn into_iter(self) -> Annotations<'a> {
        self.iter()
    }
}

/// An iterator over [`BinaryAnnotations`].
///
/// [`BinaryAnnotations::new`] validates all bytes, so the iterator does not
/// return errors.
#[derive(Clone, Debug)]
pub struct Annotations<'a> {
    /// The annotation bytes that are not decoded yet.
    bytes: &'a [u8],
}

impl Iterator for Annotations<'_> {
    type Item = Annotation;

    fn next(&mut self) -> Option<Annotation> {
        // The bytes are validated, so a decode error cannot occur here.
        decode(&mut self.bytes).ok()
    }
}

impl FusedIterator for Annotations<'_> {}

/// Decode the annotation at the start of `bytes` and move past it.
fn decode(bytes: &mut &[u8]) -> Result<Annotation> {
    let opcode = compressed(bytes)?;
    Ok(match opcode {
        1 => Annotation::CodeOffset(compressed(bytes)?),
        2 => Annotation::ChangeCodeOffsetBase(compressed(bytes)?),
        3 => Annotation::ChangeCodeOffset(compressed(bytes)?),
        4 => Annotation::ChangeCodeLength(compressed(bytes)?),
        5 => Annotation::ChangeFile(compressed(bytes)?),
        6 => Annotation::ChangeLineOffset(signed(compressed(bytes)?)),
        7 => Annotation::ChangeLineEndDelta(compressed(bytes)?),
        8 => Annotation::ChangeRangeKind(compressed(bytes)?),
        9 => Annotation::ChangeColumnStart(compressed(bytes)?),
        10 => Annotation::ChangeColumnEndDelta(signed(compressed(bytes)?)),
        11 => {
            let operand = compressed(bytes)?;
            Annotation::ChangeCodeOffsetAndLineOffset {
                code_delta: operand & 0xf,
                line_delta: signed(operand >> 4),
            }
        }
        12 => {
            let code_length = compressed(bytes)?;
            let code_delta = compressed(bytes)?;
            Annotation::ChangeCodeLengthAndCodeOffset {
                code_length,
                code_delta,
            }
        }
        13 => Annotation::ChangeColumnEnd(compressed(bytes)?),
        _ => {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("binary annotation opcode is not known"));
        }
    })
}

/// Decode a compressed unsigned integer and move past it.
fn compressed(bytes: &mut &[u8]) -> Result<u32> {
    let truncated = || {
        Error::new(ErrorKind::InvalidFormat, None).with_context("binary annotation is truncated")
    };
    let &first = bytes.first().ok_or_else(truncated)?;
    let length = match first {
        0x00..=0x7f => 1,
        0x80..=0xbf => 2,
        0xc0..=0xdf => 4,
        _ => {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("binary annotation integer has an invalid first byte"));
        }
    };
    let (head, rest) = bytes.split_at_checked(length).ok_or_else(truncated)?;
    *bytes = rest;
    let value = match *head {
        [byte] => u32::from(byte),
        [high, low] => u32::from_be_bytes([0, 0, high & 0x3f, low]),
        [b0, b1, b2, b3] => u32::from_be_bytes([b0 & 0x1f, b1, b2, b3]),
        _ => return Err(truncated()),
    };
    Ok(value)
}

/// Decode a signed operand: bit 0 is the sign and the other bits are the
/// magnitude (`DecodeSignedInt32` in `cvinfo.h`).
const fn signed(value: u32) -> i32 {
    // A compressed integer has at most 29 bits, so the magnitude fits.
    let magnitude = (value >> 1).cast_signed();
    if value & 1 == 0 {
        magnitude
    } else {
        -magnitude
    }
}
