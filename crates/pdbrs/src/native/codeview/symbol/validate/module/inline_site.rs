//! The code ranges of an inline site and the def-range check against them.
//!
//! The binary annotations of an inline site give its code ranges. The rules
//! are the code offset rules of [`InlineRanges`]; this module does not run
//! the line and column opcodes, because the check needs only the code.
//!
//! `BA_OP_ChangeCodeOffsetBase` selects a code chunk of the procedure
//! (`cvinfo.h`: "nth separated code chunk (main code chunk == 0)"). Chunk 0
//! is the code of the procedure. Chunk `n` is the code of the `n`-th
//! `S_SEPCODE` record of the procedure, in stream order. The opcode sets the
//! code offset to 0, so that the next ranges are relative to the start of
//! the chunk. MSVC PGO output follows these rules (`msvc-x64-pgo` of #27):
//! the sites put 40 `ChangeCodeOffsetBase` opcodes after other code
//! opcodes, and the next range then starts at offset 0 of the chunk.
//!
//! [`InlineRanges`]: crate::native::locate::InlineRanges

use alloc::vec::Vec;

use super::super::super::{AddressGaps, AddressRange, Annotation, BinaryAnnotations};
use super::{CodeRange, push};
use crate::Result;

/// A code range of an inline site, relative to the start of a code chunk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SiteRange {
    /// The code chunk: 0 for the procedure, `n` for its `n`-th separated
    /// code chunk.
    pub(super) chunk: u32,
    /// The offset of the first byte from the start of the chunk.
    pub(super) start: u32,
    /// The length, or `None` for a last range that ends with its chunk.
    pub(super) length: Option<u32>,
}

/// The state of the code opcodes.
#[derive(Default)]
struct State {
    /// The code offset relative to the start of the chunk.
    offset: u32,
    /// The current code chunk.
    chunk: u32,
    /// The started range that is not appended yet.
    open: Option<SiteRange>,
}

impl State {
    /// Add `delta` to the code offset. Returns `false` on overflow.
    fn advance(&mut self, delta: u32) -> bool {
        self.offset
            .checked_add(delta)
            .inspect(|&next| self.offset = next)
            .is_some()
    }

    /// Start a range at the code offset and append the open range to `out`.
    ///
    /// Returns `Ok(false)` if the new range starts before the open range of
    /// the same chunk, and the open range has no length.
    fn start(&mut self, length: Option<u32>, out: &mut Vec<SiteRange>) -> Result<bool> {
        let next = SiteRange {
            chunk: self.chunk,
            start: self.offset,
            length,
        };
        self.close(Some(next), out)
    }

    /// Append the open range to `out` and replace it with `next`.
    ///
    /// The open range without a length ends at the start of `next`, if both
    /// are in the same chunk; else it ends with its chunk.
    fn close(&mut self, next: Option<SiteRange>, out: &mut Vec<SiteRange>) -> Result<bool> {
        let Some(mut previous) = core::mem::replace(&mut self.open, next) else {
            return Ok(true);
        };
        if previous.length.is_none()
            && let Some(next) = next.filter(|next| next.chunk == previous.chunk)
        {
            let Some(length) = next.start.checked_sub(previous.start) else {
                return Ok(false);
            };
            previous.length = Some(length);
        }
        push(out, previous)?;
        Ok(true)
    }
}

/// Append the code ranges of an inline site to `out`.
///
/// Returns `Ok(false)` if the annotations do not give valid ranges: a code
/// offset overflows 32 bits, or a range without a length is followed by a
/// range that starts before it. Then `out` keeps its old length.
///
/// # Errors
///
/// Returns [`crate::ErrorKind::LimitExceeded`] if an allocation fails.
pub(super) fn site_ranges(
    annotations: BinaryAnnotations<'_>,
    out: &mut Vec<SiteRange>,
) -> Result<bool> {
    let first = out.len();
    if collect(annotations, out)? {
        Ok(true)
    } else {
        out.truncate(first);
        Ok(false)
    }
}

/// Run the code opcodes and append the ranges to `out`.
fn collect(annotations: BinaryAnnotations<'_>, out: &mut Vec<SiteRange>) -> Result<bool> {
    let mut state = State::default();
    for annotation in &annotations {
        let valid = match annotation {
            Annotation::CodeOffset(value) => {
                state.offset = value;
                true
            }
            Annotation::ChangeCodeOffsetBase(chunk) => {
                let valid = state.close(None, out)?;
                state.chunk = chunk;
                state.offset = 0;
                valid
            }
            Annotation::ChangeCodeLength(length) => {
                if let Some(open @ SiteRange { length: None, .. }) = &mut state.open {
                    open.length = Some(length);
                }
                state.advance(length)
            }
            Annotation::ChangeCodeOffset(delta)
            | Annotation::ChangeCodeOffsetAndLineOffset {
                code_delta: delta, ..
            } => state.advance(delta) && state.start(None, out)?,
            Annotation::ChangeCodeLengthAndCodeOffset {
                code_length,
                code_delta,
            } => state.advance(code_delta) && state.start(Some(code_length), out)?,
            _ => true,
        };
        if !valid {
            return Ok(false);
        }
    }
    state.close(None, out)
}

/// Convert site ranges to absolute code ranges in `out`.
///
/// `chunk` gives the code range of a chunk number. Returns `Ok(false)` if a
/// chunk is not known, or if a range does not fit in 32 bits.
///
/// # Errors
///
/// Returns [`crate::ErrorKind::LimitExceeded`] if an allocation fails.
pub(super) fn absolute(
    ranges: &[SiteRange],
    chunk: impl Fn(u32) -> Option<CodeRange>,
    out: &mut Vec<CodeRange>,
) -> Result<bool> {
    out.clear();
    for range in ranges {
        let Some(code) = chunk(range.chunk) else {
            return Ok(false);
        };
        let Some(start) = code.start.checked_add(range.start) else {
            return Ok(false);
        };
        let end = match range.length {
            Some(length) => start.checked_add(length),
            None => Some(code.end.max(start)),
        };
        let Some(end) = end else {
            return Ok(false);
        };
        push(
            out,
            CodeRange {
                section: code.section,
                start,
                end,
            },
        )?;
    }
    Ok(true)
}

/// Append the parts of a def-range outside its gaps to `out`, as absolute
/// code ranges. Parts of length 0 are left out.
///
/// # Errors
///
/// Returns [`crate::ErrorKind::LimitExceeded`] if an allocation fails.
pub(super) fn parts(
    range: AddressRange,
    gaps: Option<AddressGaps<'_>>,
    out: &mut Vec<CodeRange>,
) -> Result<()> {
    let section = range.start.section;
    let start = u64::from(range.start.offset);
    let end = start + u64::from(range.length);
    let mut add = |from: u64, to: u64| -> Result<()> {
        // The caller checked that the def-range fits in 32 bits.
        match (u32::try_from(from), u32::try_from(to)) {
            (Ok(from), Ok(to)) if from < to => push(
                out,
                CodeRange {
                    section,
                    start: from,
                    end: to,
                },
            ),
            _ => Ok(()),
        }
    };
    let mut cursor = start;
    for gap in gaps.into_iter().flatten() {
        let gap_start = start + u64::from(gap.start);
        add(cursor, gap_start.min(end))?;
        cursor = cursor.max(gap_start + u64::from(gap.length));
    }
    add(cursor, end)
}

/// Return `true` if one of the `parts` of a def-range overlaps or touches
/// one of the `ranges` of its inline site.
///
/// A part touches a range if it starts at the end of the range or ends at
/// its start. MSVC writes such def-ranges, for example for the result of an
/// inlined function after the inlined code (`msvc-x64-release`, #30).
pub(super) fn touches(ranges: &[CodeRange], parts: &[CodeRange]) -> bool {
    parts.iter().any(|part| {
        ranges.iter().any(|code| {
            code.section == part.section && code.start <= part.end && part.start <= code.end
        })
    })
}
