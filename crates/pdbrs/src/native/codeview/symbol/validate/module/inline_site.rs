//! The code ranges of an inline site and the def-range check against them.
//!
//! The binary annotations of an inline site give its code ranges. The
//! shared code opcode rules ([`CodeState`]) give the ranges, as for the
//! locator ([`InlineRanges`]). The check needs only the code, so it does not
//! run the line and column opcodes.
//!
//! `BA_OP_ChangeCodeOffsetBase` selects a code chunk of the procedure. Chunk
//! 0 is the code of the procedure. Chunk `n` is the code of the `n`-th
//! `S_SEPCODE` record of the procedure, in stream order. MSVC PGO output
//! follows these rules (`msvc-x64-pgo` of #27): the sites put 40
//! `ChangeCodeOffsetBase` opcodes after other code opcodes, and the next
//! range then starts at offset 0 of the chunk.
//!
//! [`InlineRanges`]: crate::native::locate::InlineRanges

use alloc::vec::Vec;

use super::super::super::{AddressGaps, AddressRange, BinaryAnnotations, ChunkRange, CodeState};
use super::{CodeRange, push};
use crate::Result;

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
    annotations: &BinaryAnnotations<'_>,
    out: &mut Vec<ChunkRange>,
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
fn collect(annotations: &BinaryAnnotations<'_>, out: &mut Vec<ChunkRange>) -> Result<bool> {
    let mut state = CodeState::default();
    for annotation in annotations {
        let Ok(step) = state.step(annotation) else {
            return Ok(false);
        };
        if let Some(range) = step.finished {
            push(out, range)?;
        }
    }
    if let Some(range) = state.finish() {
        push(out, range)?;
    }
    Ok(true)
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
    ranges: &[ChunkRange],
    chunk: impl Fn(u32) -> Option<CodeRange>,
    out: &mut Vec<CodeRange>,
) -> Result<bool> {
    out.clear();
    for range in ranges {
        let Some(code) = chunk(range.chunk) else {
            return Ok(false);
        };
        let Some((start, end)) = range.bounds(code.start, code.end) else {
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
    gaps: Option<&AddressGaps<'_>>,
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
