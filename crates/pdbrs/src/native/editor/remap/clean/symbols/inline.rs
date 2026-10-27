//! Inline sites of a procedure that splits (#43, decision 4).
//!
//! The code offsets of the binary annotations are relative to a code chunk of the procedure
//! ([`CodeState`]). A site whose ranges all lie in the first part keeps its annotations. A
//! site whose ranges all lie in a later part gets a `ChangeCodeOffsetBase` to the chunk of
//! the new `S_SEPCODE` scope of that part, and code offsets from the part start. A site with
//! ranges in more than one part, or with a `ChangeCodeOffsetBase` in the source, fails.

use alloc::vec::Vec;

use super::Split;
use crate::native::RemapItem;
use crate::native::codeview::symbol::{Annotation, BinaryAnnotations, ChunkRange, CodeState};
use crate::native::editor::remap::clean::{no_split_form, record_error};
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::native::remap::clean::CleanMap;
use crate::{Error, ErrorKind, Result};

/// The rewrite of one inline site.
pub(super) enum Site {
    /// The annotations stay.
    Keep,
    /// All ranges of the site have no remap range. The value is the PDB RVA of the first
    /// range.
    Removed(u32),
    /// The new annotations.
    Annotations(BinaryAnnotations<'static>),
}

/// Rewrite the annotations `annotations` of an inline site `item` of the procedure `split`.
pub(super) fn rewrite(
    map: &CleanMap,
    split: &Split,
    annotations: &BinaryAnnotations<'_>,
    item: RemapItem,
    location: Option<(StreamId, u64)>,
) -> Result<Site> {
    let ranges = ranges(annotations, split, item, location)?;
    let mut part = None;
    let mut removed = 0_usize;
    let mut first_rva = None;
    for range in &ranges {
        let (start, end) = range.bounds(0, split.length).ok_or_else(overflow)?;
        let rva = split.start.checked_add(start).ok_or_else(overflow)?;
        first_rva.get_or_insert(rva);
        let pieces = map.parts(rva, end - start)?;
        let Some(piece) = pieces.parts.first() else {
            removed += 1;
            continue;
        };
        if pieces.parts.len() > 1 {
            return Err(no_split_form(item, rva, location));
        }
        if pieces.removed {
            return Err(partly_removed(item, rva, location));
        }
        // The part of the procedure that holds the new start of the range.
        let index = split
            .parts
            .parts
            .iter()
            .position(|part| {
                part.rva <= piece.rva
                    && (piece.rva < part.rva + part.length
                        || (piece.length == 0 && piece.rva == part.rva + part.length))
            })
            .ok_or_else(|| no_split_form(item, rva, location))?;
        if *part.get_or_insert(index) != index {
            return Err(no_split_form(item, rva, location));
        }
    }
    let Some(rva) = first_rva else {
        return Ok(Site::Keep);
    };
    if removed == ranges.len() {
        return Ok(Site::Removed(rva));
    }
    if removed > 0 {
        return Err(partly_removed(item, rva, location));
    }
    match part {
        None | Some(0) => Ok(Site::Keep),
        Some(index) => {
            let base = split.parts.parts[index].offset;
            let chunk = u32::try_from(index - 1)
                .ok()
                .and_then(|index| split.first_chunk.checked_add(index))
                .ok_or_else(overflow)?;
            rebase(annotations, base, chunk)
                .map(Site::Annotations)
                .map_err(|error| match error {
                    Rebase::Error(error) => error,
                    Rebase::NoForm => no_split_form(item, rva, location),
                })
        }
    }
}

/// Return the code ranges of `annotations`, relative to the procedure start.
fn ranges(
    annotations: &BinaryAnnotations<'_>,
    split: &Split,
    item: RemapItem,
    location: Option<(StreamId, u64)>,
) -> Result<Vec<ChunkRange>> {
    let mut state = CodeState::default();
    let mut ranges = Vec::new();
    let mut add = |range: Option<ChunkRange>| -> Result<()> {
        if let Some(range) = range {
            ranges.try_reserve(1).map_err(|_| no_memory())?;
            ranges.push(range);
        }
        Ok(())
    };
    for annotation in annotations {
        if let Annotation::ChangeCodeOffsetBase(_) = annotation {
            // The site already uses separated code chunks of its procedure.
            return Err(no_split_form(item, split.start, location));
        }
        let step = state.step(annotation).map_err(|_| {
            Error::new(ErrorKind::InvalidFormat, None)
                .with_context("the code offsets of an inline site are not valid")
        })?;
        add(step.finished)?;
    }
    add(state.finish())?;
    Ok(ranges)
}

/// The failure of [`rebase`].
enum Rebase {
    /// An error of the encoder.
    Error(Error),
    /// A code offset lies before the part start.
    NoForm,
}

impl From<Error> for Rebase {
    fn from(error: Error) -> Self {
        Self::Error(error)
    }
}

/// Return `annotations` for code chunk `chunk`, whose start is at offset `base` of the
/// procedure.
///
/// The new annotations start with `ChangeCodeOffsetBase`. Each range then starts at its old
/// offset minus `base`. The other annotations stay.
fn rebase(
    annotations: &BinaryAnnotations<'_>,
    base: u32,
    chunk: u32,
) -> core::result::Result<BinaryAnnotations<'static>, Rebase> {
    let mut out = Vec::new();
    Annotation::ChangeCodeOffsetBase(chunk).encode(&mut out)?;
    // The old code offset from the procedure start, and the new offset from the chunk start.
    let mut old = 0_u32;
    let mut new = 0_u32;
    let advance = |old: &mut u32, delta: u32| -> core::result::Result<u32, Rebase> {
        *old = old.checked_add(delta).ok_or(Rebase::Error(overflow()))?;
        old.checked_sub(base).ok_or(Rebase::NoForm)
    };
    for annotation in annotations {
        match annotation {
            Annotation::CodeOffset(value) => {
                old = value;
                if let Some(offset) = value.checked_sub(base) {
                    Annotation::CodeOffset(offset).encode(&mut out)?;
                    new = offset;
                }
            }
            Annotation::ChangeCodeLength(length) => {
                old = old.checked_add(length).ok_or(Rebase::Error(overflow()))?;
                new = new.checked_add(length).ok_or(Rebase::Error(overflow()))?;
                annotation.encode(&mut out)?;
            }
            Annotation::ChangeCodeOffset(delta) => {
                let target = advance(&mut old, delta)?;
                start(&mut out, &mut new, target)?;
            }
            Annotation::ChangeCodeOffsetAndLineOffset {
                code_delta,
                line_delta,
            } => {
                let target = advance(&mut old, code_delta)?;
                if let Some(code_delta) = target.checked_sub(new).filter(|delta| *delta <= 0xf) {
                    Annotation::ChangeCodeOffsetAndLineOffset {
                        code_delta,
                        line_delta,
                    }
                    .encode(&mut out)?;
                } else {
                    Annotation::ChangeLineOffset(line_delta).encode(&mut out)?;
                    start(&mut out, &mut new, target)?;
                }
                new = target;
            }
            Annotation::ChangeCodeLengthAndCodeOffset {
                code_length,
                code_delta,
            } => {
                let target = advance(&mut old, code_delta)?;
                let code_delta = if let Some(delta) = target.checked_sub(new) {
                    delta
                } else {
                    Annotation::CodeOffset(target).encode(&mut out)?;
                    0
                };
                Annotation::ChangeCodeLengthAndCodeOffset {
                    code_length,
                    code_delta,
                }
                .encode(&mut out)?;
                new = target;
            }
            // `ranges` rejects this opcode, and the other annotations have no code operand.
            _ => annotation.encode(&mut out)?,
        }
    }
    Ok(BinaryAnnotations::parse(out)?)
}

/// Append the annotations that start a range at the new offset `target` of the chunk.
fn start(out: &mut Vec<u8>, new: &mut u32, target: u32) -> Result<()> {
    if let Some(delta) = target.checked_sub(*new) {
        Annotation::ChangeCodeOffset(delta).encode(out)?;
    } else {
        Annotation::CodeOffset(target).encode(out)?;
        Annotation::ChangeCodeOffset(0).encode(out)?;
    }
    *new = target;
    Ok(())
}

/// Return the error for a code offset that does not fit in 32 bits.
fn overflow() -> Error {
    Error::new(ErrorKind::OffsetOverflow, None)
        .with_context("an inline site code offset does not fit in 32 bits")
}

/// Return the error for an inline site with some ranges removed and others kept.
fn partly_removed(item: RemapItem, rva: u32, location: Option<(StreamId, u64)>) -> Error {
    record_error(
        item,
        rva,
        location,
        "some code ranges of an inline site have no remap range, but others have one",
    )
}
