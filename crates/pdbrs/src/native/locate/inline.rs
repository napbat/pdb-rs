//! Source line ranges from the binary annotations of an inline site.
//!
//! An `S_INLINESITE` record does not store code ranges directly. Its binary
//! annotations are a small program. [`InlineRanges`] runs the program and
//! returns one [`InlineRange`] for each range that the program emits.
//!
//! The program has this state: a code offset, a code offset base, a file, a
//! line, a line count, columns and a range kind. The start values come from
//! the `DEBUG_S_INLINEELINES` entry of the inlinee. These opcodes start a
//! range at the current state:
//!
//! - `ChangeCodeOffset`.
//! - `ChangeCodeOffsetAndLineOffset`.
//! - `ChangeCodeLengthAndCodeOffset`. It also gives the length of the range.
//!
//! A range without an explicit length ends where the next range starts.
//! `ChangeCodeLength` gives the length of the open range and moves the code
//! offset to the end of that range. Thus a later range can start after a gap.
//!
//! The code offset of a range is relative to the start of the procedure that
//! contains the inline site, also for nested inline sites. The range offset
//! is the code offset plus the code offset base. `ChangeCodeOffsetBase` sets
//! the base for separated code chunks. No fixture contains it.
//!
//! The rules follow `InlineeLineIterator` in the `pdb` crate (version
//! 0.8.0) and `MCCodeView::encodeInlineLineTable` in LLVM (revision
//! `b9f2235f`). The opcodes come from `BinaryAnnotationOpcode` in `cvinfo.h`.

use core::iter::FusedIterator;

use crate::native::codeview::symbol::{Annotation, Annotations, BinaryAnnotations};
use crate::{Error, ErrorKind, Result};

/// One source line range of an inline site.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct InlineRange {
    /// The offset of the first byte from the start of the procedure.
    ///
    /// The value includes the code offset base.
    pub code_offset: u32,
    /// The number of bytes in the range, or `None` if the annotations do not
    /// give the end of the last range.
    pub code_length: Option<u32>,
    /// The byte offset of the file entry in the `DEBUG_S_FILECHKSMS`
    /// subsection of the module.
    pub file: u32,
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

impl InlineRange {
    /// Return the offset after the last byte, if the range has a length.
    #[must_use]
    pub fn code_end(&self) -> Option<u64> {
        self.code_length
            .map(|length| u64::from(self.code_offset) + u64::from(length))
    }

    /// Return `true` if the range holds `offset`, an offset from the start of
    /// the procedure.
    ///
    /// A range without a length holds no offset.
    #[must_use]
    pub fn contains(&self, offset: u32) -> bool {
        offset >= self.code_offset && self.code_end().is_some_and(|end| u64::from(offset) < end)
    }
}

/// An iterator that runs the binary annotations of an inline site.
///
/// The iterator returns the ranges in annotation order. It returns one error
/// for a range that cannot be represented and then stops.
#[derive(Clone, Debug)]
pub struct InlineRanges<'a> {
    /// The annotations that are not run yet.
    annotations: Annotations<'a>,
    /// The code offset relative to the procedure start, without the base.
    code_offset: u32,
    /// The code offset base.
    base: u32,
    /// The file checksum offset.
    file: u32,
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
    /// The last range that was started and is not returned yet.
    open: Option<InlineRange>,
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
    pub fn new(annotations: &BinaryAnnotations<'a>, file: u32, line: u32) -> Self {
        Self {
            annotations: annotations.iter(),
            code_offset: 0,
            base: 0,
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

    /// Run one annotation. Return a finished range, if the annotation
    /// starts a new range.
    fn step(&mut self, annotation: Annotation) -> Result<Option<InlineRange>> {
        match annotation {
            Annotation::CodeOffset(offset) => self.code_offset = offset,
            Annotation::ChangeCodeOffsetBase(base) => self.base = base,
            Annotation::ChangeCodeOffset(delta) => {
                self.advance(delta)?;
                return self.start(None);
            }
            Annotation::ChangeCodeLength(length) => {
                if let Some(open) = &mut self.open
                    && open.code_length.is_none()
                {
                    open.code_length = Some(length);
                }
                self.advance(length)?;
            }
            Annotation::ChangeFile(file) => self.file = file,
            Annotation::ChangeLineOffset(delta) => self.line += i64::from(delta),
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
            Annotation::ChangeCodeOffsetAndLineOffset {
                code_delta,
                line_delta,
            } => {
                self.advance(code_delta)?;
                self.line += i64::from(line_delta);
                return self.start(None);
            }
            Annotation::ChangeCodeLengthAndCodeOffset {
                code_length,
                code_delta,
            } => {
                self.advance(code_delta)?;
                return self.start(Some(code_length));
            }
        }
        Ok(None)
    }

    /// Add `delta` to the code offset.
    fn advance(&mut self, delta: u32) -> Result<()> {
        self.code_offset = self.code_offset.checked_add(delta).ok_or_else(|| {
            Error::new(ErrorKind::OffsetOverflow, None)
                .with_context("inline site code offset overflows 32 bits")
        })?;
        Ok(())
    }

    /// Start a range at the current state. Return the range that it ends.
    fn start(&mut self, code_length: Option<u32>) -> Result<Option<InlineRange>> {
        let code_offset = self.code_offset.checked_add(self.base).ok_or_else(|| {
            Error::new(ErrorKind::OffsetOverflow, None)
                .with_context("inline site code offset overflows 32 bits")
        })?;
        let line = u32::try_from(self.line)
            .map_err(|_| invalid("inline site line is outside the 32-bit range"))?;
        let next = InlineRange {
            code_offset,
            code_length,
            file: self.file,
            line,
            line_count: self.line_count,
            column_start: self.column_start,
            column_end: self.column_end,
            is_statement: self.is_statement,
        };
        let Some(mut previous) = self.open.replace(next) else {
            return Ok(None);
        };
        if previous.code_length.is_none() {
            let length = code_offset
                .checked_sub(previous.code_offset)
                .ok_or_else(|| invalid("inline site code offsets decrease"))?;
            previous.code_length = Some(length);
        }
        Ok(Some(previous))
    }
}

impl Iterator for InlineRanges<'_> {
    type Item = Result<InlineRange>;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.done {
            let Some(annotation) = self.annotations.next() else {
                self.done = true;
                return self.open.take().map(Ok);
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
