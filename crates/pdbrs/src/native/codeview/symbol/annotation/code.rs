//! The code ranges of the binary annotations of an inline site.
//!
//! The code opcodes of the annotations are a small program with this state:
//! a code offset, a code chunk and an open range. [`CodeState`] runs the
//! program. The locator ([`InlineRanges`]) and the symbol validator use it,
//! so that both have the same rules.
//!
//! - `CodeOffset` sets the code offset.
//! - `ChangeCodeOffset` and `ChangeCodeOffsetAndLineOffset` add a delta to
//!   the code offset and start a range there.
//! - `ChangeCodeLengthAndCodeOffset` adds a delta and starts a range with a
//!   length.
//! - `ChangeCodeLength` gives the length of the open range, if it has no
//!   length, and moves the code offset to the end of that range. Thus a
//!   later range can start after a gap.
//! - `ChangeCodeOffsetBase` selects a code chunk of the procedure
//!   (`cvinfo.h`: "nth separated code chunk (main code chunk == 0)"). It
//!   ends the open range and sets the code offset to 0. Thus the next ranges
//!   are relative to the start of the chunk.
//!
//! A new range ends the open range. If the open range has no length, it
//! ends where the new range starts. A range without a length that the
//! program does not end otherwise ends with its chunk.
//!
//! Chunk 0 is the code of the procedure. Chunk `n` is the code of the `n`-th
//! `S_SEPCODE` record of the module whose parent address is the start of the
//! procedure, in stream order ([`separated_chunks`]). MSVC writes these
//! records at the top level, after the end of the procedure. The MSVC PGO
//! fixture `msvc-x64-pgo` follows these rules: DIA gives the inline
//! frames of its separated code from the ranges of chunk 1.
//!
//! The rules follow `InlineeLineIterator` in the `pdb` crate (version
//! 0.8.0) and `MCCodeView::encodeInlineLineTable` in LLVM (revision
//! `b9f2235f`).
//!
//! [`InlineRanges`]: crate::native::locate::InlineRanges

use super::Annotation;
use crate::native::address::SectionOffset;

/// A code range of an inline site, relative to the start of a code chunk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ChunkRange {
    /// The code chunk: 0 for the procedure, `n` for its `n`-th separated
    /// code chunk.
    pub(crate) chunk: u32,
    /// The offset of the first byte from the start of the chunk.
    pub(crate) start: u32,
    /// The length, or `None` for a range that ends with its chunk.
    pub(crate) length: Option<u32>,
}

impl ChunkRange {
    /// Return the first value and the value after the last byte of the
    /// range, for a chunk from `chunk_start` to `chunk_end`.
    ///
    /// The values are in the address space of the chunk bounds, for example
    /// RVAs or section offsets. A range without a length ends at
    /// `chunk_end`, or at its start if `chunk_end` is before the start.
    /// Returns `None` if a value does not fit in 32 bits.
    pub(crate) fn bounds(&self, chunk_start: u32, chunk_end: u32) -> Option<(u32, u32)> {
        let start = chunk_start.checked_add(self.start)?;
        let end = match self.length {
            Some(length) => start.checked_add(length)?,
            None => chunk_end.max(start),
        };
        Some((start, end))
    }
}

/// An error of the code opcodes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CodeError {
    /// The code offset does not fit in 32 bits.
    Overflow,
    /// A range without a length is followed by a range of the same chunk
    /// that starts before it.
    Decreasing,
}

/// The effect of one annotation on the code ranges.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CodeStep {
    /// The range that the annotation ends.
    pub(crate) finished: Option<ChunkRange>,
    /// `true` if the annotation starts a new range.
    pub(crate) started: bool,
}

/// The state of the code opcodes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CodeState {
    /// The code offset relative to the start of the chunk.
    offset: u32,
    /// The current code chunk.
    chunk: u32,
    /// The started range that is not ended yet.
    open: Option<ChunkRange>,
}

impl CodeState {
    /// Run one annotation. Annotations without a code operand do nothing.
    ///
    /// # Errors
    ///
    /// Returns a [`CodeError`] if the annotation does not give a valid
    /// range. The state is then not defined.
    pub(crate) fn step(&mut self, annotation: Annotation) -> Result<CodeStep, CodeError> {
        let mut step = CodeStep::default();
        match annotation {
            Annotation::CodeOffset(offset) => self.offset = offset,
            Annotation::ChangeCodeOffsetBase(chunk) => {
                step.finished = self.open.take();
                self.chunk = chunk;
                self.offset = 0;
            }
            Annotation::ChangeCodeLength(length) => {
                if let Some(open @ ChunkRange { length: None, .. }) = &mut self.open {
                    open.length = Some(length);
                }
                self.advance(length)?;
            }
            Annotation::ChangeCodeOffset(delta)
            | Annotation::ChangeCodeOffsetAndLineOffset {
                code_delta: delta, ..
            } => {
                self.advance(delta)?;
                step = self.start(None)?;
            }
            Annotation::ChangeCodeLengthAndCodeOffset {
                code_length,
                code_delta,
            } => {
                self.advance(code_delta)?;
                step = self.start(Some(code_length))?;
            }
            _ => {}
        }
        Ok(step)
    }

    /// End the program. Return the open range: it ends with its chunk if it
    /// has no length.
    pub(crate) fn finish(&mut self) -> Option<ChunkRange> {
        self.open.take()
    }

    /// Add `delta` to the code offset.
    fn advance(&mut self, delta: u32) -> Result<(), CodeError> {
        self.offset = self.offset.checked_add(delta).ok_or(CodeError::Overflow)?;
        Ok(())
    }

    /// Start a range at the code offset. Return the open range as finished.
    fn start(&mut self, length: Option<u32>) -> Result<CodeStep, CodeError> {
        let next = ChunkRange {
            chunk: self.chunk,
            start: self.offset,
            length,
        };
        let mut finished = self.open.replace(next);
        // `ChangeCodeOffsetBase` ends the open range, so it is in the same
        // chunk as the new range.
        if let Some(previous @ ChunkRange { length: None, .. }) = &mut finished {
            let length = next
                .start
                .checked_sub(previous.start)
                .ok_or(CodeError::Decreasing)?;
            previous.length = Some(length);
        }
        Ok(CodeStep {
            finished,
            started: true,
        })
    }
}

/// Return the separated code chunks of the procedure at `procedure`, in
/// chunk order from chunk 1.
///
/// `separated` has the parent address and a value for each `S_SEPCODE`
/// record of the module, in stream order.
pub(crate) fn separated_chunks<T>(
    separated: &[(SectionOffset, T)],
    procedure: SectionOffset,
) -> impl Iterator<Item = &T> {
    separated
        .iter()
        .filter(move |(parent, _)| *parent == procedure)
        .map(|(_, value)| value)
}
