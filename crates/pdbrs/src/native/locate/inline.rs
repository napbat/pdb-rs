//! Source line ranges from the binary annotations of an inline site.
//!
//! An `S_INLINESITE` record does not store code ranges directly. Its binary
//! annotations are a small program. [`InlineRanges`] runs the program and
//! returns one [`InlineRange`] for each range that the program emits.
//!
//! The program has a code state and a line state. The code state is a code
//! offset, a code chunk and the open range. The symbol validator uses the
//! same code rules. The line state is a file, a line, a line count, columns
//! and a range kind. The start values of the line state come from the
//! `DEBUG_S_INLINEELINES` entry of the inlinee. These opcodes start a range
//! at the current state:
//!
//! - `ChangeCodeOffset`.
//! - `ChangeCodeOffsetAndLineOffset`.
//! - `ChangeCodeLengthAndCodeOffset`. It also gives the length of the range.
//!
//! A range without an explicit length ends where the next range starts.
//! `ChangeCodeLength` gives the length of the open range and moves the code
//! offset to the end of that range. Thus a later range can start after a gap.
//!
//! The code offset of a range is relative to the start of a code chunk of
//! the procedure that contains the inline site, also for nested inline
//! sites. Chunk 0 is the code of the procedure. `ChangeCodeOffsetBase`
//! selects chunk `n`: the code of the `n`-th `S_SEPCODE` record of the
//! module whose parent address is the start of the procedure, in stream
//! order (`cvinfo.h`: "nth separated code chunk (main code chunk == 0)").
//! The opcode ends the open range and sets the code offset to 0. A range
//! without a length that no later range ends, ends with its chunk.
//! [`SymbolLocator::code_chunks`](super::SymbolLocator::code_chunks) gives
//! the chunks of a procedure. MSVC writes separated code for profile-guided
//! optimization: the fixture `msvc-x64-pgo` of #27 has 62 inline sites with
//! `ChangeCodeOffsetBase`, and DIA gives the same inline frames in its
//! separated code.
//!
//! The rules follow `InlineeLineIterator` in the `pdb` crate (version
//! 0.8.0) and `MCCodeView::encodeInlineLineTable` in LLVM (revision
//! `b9f2235f`). The opcodes come from `BinaryAnnotationOpcode` in `cvinfo.h`.

use core::iter::FusedIterator;

use super::CodeChunk;
use crate::native::address::Rva;
use crate::native::codeview::index::ChecksumOffset;
use crate::native::codeview::symbol::{
    Annotation, BinaryAnnotations, BinaryAnnotationsIter, ChunkRange, CodeError, CodeState,
};
use crate::{Error, ErrorKind, Result};

/// One source line range of an inline site.
///
/// The default value is a statement range of one line: line 0 of the file checksum offset 0,
/// at code offset 0 of chunk 0, without a length and without columns.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct InlineRange {
    /// The code chunk of the procedure: 0 for the code of the procedure,
    /// `n` for the code of its `n`-th `S_SEPCODE` record.
    ///
    /// The value is the index in the list of
    /// [`SymbolLocator::code_chunks`](super::SymbolLocator::code_chunks).
    pub chunk: u32,
    /// The offset of the first byte from the start of the chunk.
    pub code_offset: u32,
    /// The number of bytes in the range, or `None` if the annotations do not
    /// give the end of the range. Such a range ends with its chunk.
    pub code_length: Option<u32>,
    /// The byte offset of the file entry in the `DEBUG_S_FILECHKSMS`
    /// subsection of the module.
    pub file: ChecksumOffset,
    /// The source line.
    pub line: u32,
    /// The number of source lines in the range. The default is 1.
    pub line_count: u32,
    /// The first column, or `None` if the annotations do not give a column.
    pub column_start: Option<u32>,
    /// The column after the end, or `None` if the annotations do not give it.
    pub column_end: Option<u32>,
    /// `true` for a statement and `false` for an expression.
    pub is_statement: bool,
}

impl Default for InlineRange {
    fn default() -> Self {
        Self {
            chunk: 0,
            code_offset: 0,
            code_length: None,
            file: ChecksumOffset::default(),
            line: 0,
            line_count: 1,
            column_start: None,
            column_end: None,
            is_statement: true,
        }
    }
}

impl InlineRange {
    /// Return the RVA of the first byte and the length of the range.
    ///
    /// `chunks` are the code chunks of the procedure, from
    /// [`SymbolLocator::code_chunks`](super::SymbolLocator::code_chunks). A
    /// range without a length ends at the end of its chunk. Return `None`
    /// if `chunks` does not have the chunk of the range, if the chunk has no
    /// RVA, or if the range does not fit in 32 bits.
    #[must_use]
    pub fn rva_range(&self, chunks: &[CodeChunk]) -> Option<(Rva, u32)> {
        let chunk = chunks.get(usize::try_from(self.chunk).ok()?)?;
        let start = chunk.start?.0;
        let code = ChunkRange {
            chunk: self.chunk,
            start: self.code_offset,
            length: self.code_length,
        };
        let (first, end) = code.bounds(start, start.checked_add(chunk.length)?)?;
        Some((Rva(first), end - first))
    }
}

/// The line state of a range.
#[derive(Clone, Copy, Debug)]
struct LineState {
    /// The file checksum offset.
    file: ChecksumOffset,
    /// The line.
    line: u32,
    /// The line count.
    line_count: u32,
    /// The first column.
    column_start: Option<u32>,
    /// The column after the end.
    column_end: Option<u32>,
    /// The range kind.
    is_statement: bool,
}

/// An iterator that runs the binary annotations of an inline site.
///
/// The iterator returns the ranges in annotation order. It returns one error
/// for a range that cannot be represented and then stops.
#[derive(Clone, Debug)]
pub struct InlineRanges<'a> {
    /// The annotations that are not run yet.
    annotations: BinaryAnnotationsIter<'a>,
    /// The code offset, the code chunk and the open code range.
    code: CodeState,
    /// The file checksum offset.
    file: ChecksumOffset,
    /// The line. It is signed so that a negative intermediate value is not
    /// an overflow; a range with a line outside `u32` is an error.
    line: i64,
    /// The line count.
    line_count: u32,
    /// The first column.
    column_start: Option<u32>,
    /// The column after the end.
    column_end: Option<u32>,
    /// The range kind.
    is_statement: bool,
    /// The line state of the open code range.
    open: Option<LineState>,
    /// `true` after an error or after the last range.
    done: bool,
}

impl<'a> InlineRanges<'a> {
    /// Create an iterator for `annotations`.
    ///
    /// `file` and `line` are the start file and the start line of the
    /// inlinee, from its `DEBUG_S_INLINEELINES` entry
    /// ([`InlineeLine`](crate::native::lines::InlineeLine)).
    #[must_use]
    pub fn new(annotations: &'a BinaryAnnotations<'_>, file: ChecksumOffset, line: u32) -> Self {
        Self {
            annotations: annotations.iter(),
            code: CodeState::default(),
            file,
            line: i64::from(line),
            line_count: 1,
            column_start: None,
            column_end: None,
            is_statement: true,
            open: None,
            done: false,
        }
    }

    /// Run one annotation. Return a finished range, if the annotation ends
    /// one.
    fn step(&mut self, annotation: Annotation) -> Result<Option<InlineRange>> {
        match annotation {
            Annotation::ChangeFile(file) => self.file = ChecksumOffset(file),
            Annotation::ChangeLineOffset(delta)
            | Annotation::ChangeCodeOffsetAndLineOffset {
                line_delta: delta, ..
            } => self.line += i64::from(delta),
            Annotation::ChangeLineEndDelta(count) => self.line_count = count,
            Annotation::ChangeRangeKind(kind) => {
                self.is_statement = match kind {
                    0 => false,
                    1 => true,
                    _ => return Err(invalid("inline site range kind is not 0 or 1")),
                };
            }
            Annotation::ChangeColumnStart(column) => {
                self.column_start = (column != 0).then_some(column);
            }
            Annotation::ChangeColumnEndDelta(delta) => {
                if let Some(end) = self.column_end {
                    self.column_end = Some(end.checked_add_signed(delta).ok_or_else(|| {
                        invalid("inline site end column is outside the 32-bit range")
                    })?);
                }
            }
            Annotation::ChangeColumnEnd(column) => self.column_end = Some(column),
            _ => {}
        }
        let step = self.code.step(annotation).map_err(|error| match error {
            CodeError::Overflow => Error::new(ErrorKind::OffsetOverflow, None)
                .with_context("inline site code offset overflows 32 bits"),
            CodeError::Decreasing => invalid("inline site code offsets decrease"),
        })?;
        let finished = step.finished.map(|code| self.finish(code)).transpose()?;
        if step.started {
            let line = u32::try_from(self.line)
                .map_err(|_| invalid("inline site line is outside the 32-bit range"))?;
            self.open = Some(LineState {
                file: self.file,
                line,
                line_count: self.line_count,
                column_start: self.column_start,
                column_end: self.column_end,
                is_statement: self.is_statement,
            });
        }
        Ok(finished)
    }

    /// Join a finished code range with the line state of the open range.
    fn finish(&mut self, code: ChunkRange) -> Result<InlineRange> {
        let line = self
            .open
            .take()
            .ok_or_else(|| invalid("inline site code range has no line state"))?;
        Ok(InlineRange {
            chunk: code.chunk,
            code_offset: code.start,
            code_length: code.length,
            file: line.file,
            line: line.line,
            line_count: line.line_count,
            column_start: line.column_start,
            column_end: line.column_end,
            is_statement: line.is_statement,
        })
    }
}

impl Iterator for InlineRanges<'_> {
    type Item = Result<InlineRange>;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.done {
            let Some(annotation) = self.annotations.next() else {
                self.done = true;
                return self.code.finish().map(|code| self.finish(code));
            };
            match self.step(annotation) {
                Ok(Some(range)) => return Some(Ok(range)),
                Ok(None) => {}
                Err(error) => {
                    self.done = true;
                    return Some(Err(error));
                }
            }
        }
        None
    }
}

impl FusedIterator for InlineRanges<'_> {}

/// Create an [`ErrorKind::InvalidFormat`] error.
fn invalid(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, None).with_context(context)
}
