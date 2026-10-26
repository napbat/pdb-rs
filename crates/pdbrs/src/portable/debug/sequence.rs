//! The `MethodDebugInformation` table: sequence point blobs and the typed row.
//!
//! A sequence point blob has a header (the `LocalSignature` row and, for a method with
//! several documents, the initial document) and then records. A record is a sequence point,
//! a hidden sequence point or a document change. The IL offset is a delta from the previous
//! sequence point. The start line and column of a visible sequence point are signed deltas
//! from the previous visible sequence point.

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::rows::{each_row, in_blob};
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::custom::blob::{Reader, Writer, invalid};
use crate::portable::metadata::{
    BlobOffset, DocumentId, IlOffset, MAX_ROW, MethodDebugInformationId, MethodDebugInformationRow,
    StandAloneSigId, TableId,
};
use crate::{Error, Result};

/// The start line and end line of a hidden sequence point.
pub const HIDDEN_LINE: u32 = 0x00fe_efee;
/// The end of the IL offset range (exclusive).
pub const MAX_IL_OFFSET: u32 = 0x2000_0000;
/// The end of the line range (exclusive).
pub const MAX_LINE: u32 = 0x2000_0000;
/// The end of the column range (exclusive).
pub const MAX_COLUMN: u32 = 0x1_0000;

/// The source span of a visible sequence point.
///
/// A valid span has lines below [`MAX_LINE`] and not equal to [`HIDDEN_LINE`], and an end
/// line at or after the start line. If the lines are equal, the end column is after the start
/// column.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct SourceSpan {
    /// The start line.
    pub start_line: u32,
    /// The start column.
    pub start_column: u16,
    /// The end line.
    pub end_line: u32,
    /// The end column.
    pub end_column: u16,
}

impl SourceSpan {
    /// Check the line and column bounds of the span.
    fn check(self) -> Result<()> {
        let line = |line: u32| line < MAX_LINE && line != HIDDEN_LINE;
        if !line(self.start_line) || !line(self.end_line) {
            return Err(invalid(
                "sequence point line is out of range or the hidden line",
            ));
        }
        if self.end_line < self.start_line
            || (self.end_line == self.start_line && self.end_column <= self.start_column)
        {
            return Err(invalid("sequence point ends before it starts"));
        }
        Ok(())
    }
}

/// One sequence point.
///
/// The default value is a hidden sequence point at IL offset 0 with the nil document.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct SequencePoint {
    /// The IL offset.
    pub il_offset: IlOffset,
    /// The document.
    pub document: DocumentId,
    /// The source span, or `None` for a hidden sequence point.
    pub span: Option<SourceSpan>,
}

impl SequencePoint {
    /// Return `true` for a hidden sequence point.
    #[must_use]
    pub const fn is_hidden(&self) -> bool {
        self.span.is_none()
    }
}

/// A lazy, checked iterator over the sequence points of a blob.
///
/// Each item is a sequence point or the first error. The iterator stops after an error.
#[derive(Clone, Debug)]
pub struct SequencePoints<'a> {
    reader: Reader<'a>,
    local_signature: u32,
    initial_document: DocumentId,
    document: DocumentId,
    previous_offset: Option<u32>,
    previous_start: Option<(u32, u16)>,
    failed: bool,
}

impl<'a> SequencePoints<'a> {
    /// Read the header of a sequence point blob.
    ///
    /// `document` is the `Document` column of the `MethodDebugInformation` row. If it is nil,
    /// the header has the initial document.
    ///
    /// The `LocalSignature` field is a `StandAloneSig` row. The F# compiler writes a full
    /// `StandAloneSig` token (`0x11` in the high byte) instead. The decoder accepts both forms.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`](crate::ErrorKind::UnexpectedEof) if the header is
    ///   truncated.
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if the initial
    ///   document is nil, the `LocalSignature` field is not a row or a `StandAloneSig` token,
    ///   a header field is not a compressed integer, or the blob has no sequence point.
    pub fn parse(blob: &'a [u8], document: DocumentId) -> Result<Self> {
        let mut reader = Reader::new(blob);
        let local_signature = reader.unsigned()?;
        let table = local_signature >> 24;
        if table != 0 && table != u32::from(TableId::STAND_ALONE_SIG.raw()) {
            return Err(invalid(
                "sequence point LocalSignature is not a row or a StandAloneSig token",
            ));
        }
        let document = if document.is_nil() {
            read_document(&mut reader)?
        } else {
            document
        };
        if reader.is_empty() {
            return Err(invalid("sequence point blob has no sequence point"));
        }
        Ok(Self {
            reader,
            local_signature,
            initial_document: document,
            document,
            previous_offset: None,
            previous_start: None,
            failed: false,
        })
    }

    /// Return the `LocalSignature` row of the header. It is nil if the method has no locals.
    #[must_use]
    pub const fn local_signature(&self) -> StandAloneSigId {
        StandAloneSigId(self.local_signature & MAX_ROW)
    }

    /// Return the form of the `LocalSignature` field in the header. A nil row has the row
    /// form, also if the header has the token `0x11000000`, because the encoder writes a nil
    /// row as 0 in both forms.
    #[must_use]
    pub const fn local_signature_form(&self) -> LocalSignatureForm {
        if self.local_signature > MAX_ROW && self.local_signature & MAX_ROW != 0 {
            LocalSignatureForm::Token
        } else {
            LocalSignatureForm::Row
        }
    }

    /// Return the document of the first sequence point.
    #[must_use]
    pub const fn initial_document(&self) -> DocumentId {
        self.initial_document
    }

    /// Read the next record that is a sequence point.
    fn read(&mut self) -> Result<SequencePoint> {
        let il_offset = loop {
            let delta = self.reader.unsigned()?;
            let Some(previous) = self.previous_offset else {
                break delta;
            };
            if delta != 0 {
                break previous
                    .checked_add(delta)
                    .ok_or_else(|| invalid("sequence point IL offset is out of range"))?;
            }
            self.document = read_document(&mut self.reader)?;
            if self.reader.is_empty() {
                return Err(invalid("sequence point blob ends with a document record"));
            }
        };
        if il_offset >= MAX_IL_OFFSET {
            return Err(invalid("sequence point IL offset is out of range"));
        }
        self.previous_offset = Some(il_offset);
        let delta_lines = self.reader.unsigned()?;
        let delta_columns = if delta_lines == 0 {
            i64::from(self.reader.unsigned()?)
        } else {
            i64::from(self.reader.signed()?)
        };
        let span = if delta_lines == 0 && delta_columns == 0 {
            None
        } else {
            Some(self.read_span(delta_lines, delta_columns)?)
        };
        Ok(SequencePoint {
            il_offset: IlOffset(il_offset),
            document: self.document,
            span,
        })
    }

    /// Read the start of a visible sequence point and compute its span.
    fn read_span(&mut self, delta_lines: u32, delta_columns: i64) -> Result<SourceSpan> {
        let (start_line, start_column) = match self.previous_start {
            None => (
                i64::from(self.reader.unsigned()?),
                i64::from(self.reader.unsigned()?),
            ),
            Some((line, column)) => (
                i64::from(line) + i64::from(self.reader.signed()?),
                i64::from(column) + i64::from(self.reader.signed()?),
            ),
        };
        let line = |value: i64| {
            u32::try_from(value).map_err(|_| invalid("sequence point line is out of range"))
        };
        let column = |value: i64| {
            u16::try_from(value).map_err(|_| invalid("sequence point column is out of range"))
        };
        let span = SourceSpan {
            start_line: line(start_line)?,
            start_column: column(start_column)?,
            end_line: line(start_line + i64::from(delta_lines))?,
            end_column: column(start_column + delta_columns)?,
        };
        span.check()?;
        self.previous_start = Some((span.start_line, span.start_column));
        Ok(span)
    }
}

/// Read a non-nil document row ID.
fn read_document(reader: &mut Reader<'_>) -> Result<DocumentId> {
    let document = reader.unsigned()?;
    if document == 0 {
        return Err(invalid("sequence point document is nil"));
    }
    Ok(DocumentId(document))
}

impl Iterator for SequencePoints<'_> {
    type Item = Result<SequencePoint>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.reader.is_empty() {
            return None;
        }
        let point = self.read();
        self.failed = point.is_err();
        Some(point)
    }
}

impl FusedIterator for SequencePoints<'_> {}

/// The form of the `LocalSignature` field in a sequence point header.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum LocalSignatureForm {
    /// The `StandAloneSig` row, as the specification and Roslyn write it.
    #[default]
    Row,
    /// The full `StandAloneSig` token (`0x11` in the high byte), as the F# compiler writes it.
    /// A nil row is 0 in both forms.
    Token,
}

/// The result of [`SequencePoints::encode_parts`].
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct EncodedSequencePoints {
    /// The value of the `Document` column: the single document of all sequence points, or nil
    /// if the method has several documents or no sequence points.
    pub document: DocumentId,
    /// The blob, or an empty blob if the method has no sequence points.
    pub blob: Vec<u8>,
}

impl SequencePoints<'_> {
    /// Encode the sequence points of a method in the form that Roslyn writes.
    ///
    /// If all points have the same document, the blob has no document and the result gives
    /// the document for the `Document` column. Otherwise, the header has the initial document
    /// and a document record comes before each point with a different document. `form`
    /// selects how the header stores `local_signature`; use
    /// [`SequencePoints::local_signature_form`] to keep the form of a decoded blob.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a document is nil,
    ///   the IL offsets do not increase or are out of range, or a span is not valid.
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if a value does not
    ///   fit in a compressed integer.
    pub fn encode_parts(
        local_signature: StandAloneSigId,
        form: LocalSignatureForm,
        points: &[SequencePoint],
    ) -> Result<EncodedSequencePoints> {
        let Some(first) = points.first() else {
            return Ok(EncodedSequencePoints {
                document: DocumentId(0),
                blob: Vec::new(),
            });
        };
        let single = points.iter().all(|point| point.document == first.document);
        let mut blob = Vec::new();
        let mut writer = Writer::append(&mut blob);
        let header = match form {
            LocalSignatureForm::Token if !local_signature.is_nil() => local_signature
                .token()
                .ok_or_else(|| invalid("LocalSignature row does not fit in a token"))?
                .raw(),
            _ => local_signature.0,
        };
        writer.unsigned(header)?;
        let mut document = if single { Some(first.document) } else { None };
        let mut previous_offset: Option<u32> = None;
        let mut previous_start: Option<(u32, u16)> = None;
        for point in points {
            if point.document.is_nil() {
                return Err(invalid("sequence point document is nil"));
            }
            if document != Some(point.document) {
                if document.is_some() {
                    writer.unsigned(0)?;
                }
                writer.unsigned(point.document.0)?;
                document = Some(point.document);
            }
            let offset = point.il_offset.0;
            if offset >= MAX_IL_OFFSET || previous_offset.is_some_and(|previous| offset <= previous)
            {
                return Err(invalid(
                    "sequence point IL offsets do not increase or are out of range",
                ));
            }
            writer.unsigned(offset - previous_offset.unwrap_or(0))?;
            previous_offset = Some(offset);
            let Some(span) = point.span else {
                writer.u16(0)?;
                continue;
            };
            span.check()?;
            write_span(&mut writer, span, previous_start)?;
            previous_start = Some((span.start_line, span.start_column));
        }
        writer.finish();
        Ok(EncodedSequencePoints {
            document: if single {
                first.document
            } else {
                DocumentId(0)
            },
            blob,
        })
    }
}

/// Write the line and column deltas of a visible sequence point.
fn write_span(
    writer: &mut Writer<'_>,
    span: SourceSpan,
    previous: Option<(u32, u16)>,
) -> Result<()> {
    let delta_lines = span.end_line - span.start_line;
    let delta_columns = i32::from(span.end_column) - i32::from(span.start_column);
    writer.unsigned(delta_lines)?;
    if delta_lines == 0 {
        // `SourceSpan::check` makes the end column larger than the start column.
        writer.unsigned(delta_columns.unsigned_abs())?;
    } else {
        writer.signed(delta_columns)?;
    }
    match previous {
        None => {
            writer.unsigned(span.start_line)?;
            writer.unsigned(u32::from(span.start_column))
        }
        Some((line, column)) => {
            let delta = i64::from(span.start_line) - i64::from(line);
            // Lines are below `MAX_LINE`, so the delta fits in an `i32`.
            writer.signed(i32::try_from(delta).map_err(|_| invalid("line delta overflow"))?)?;
            writer.signed(i32::from(span.start_column) - i32::from(column))
        }
    }
}

/// A typed row of the `MethodDebugInformation` table with its sequence point blob.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MethodDebugInformation {
    id: MethodDebugInformationId,
    row: MethodDebugInformationRow,
    sequence_points: Vec<u8>,
}

impl MethodDebugInformation {
    /// Return the row ID. It is the row number of the `MethodDef` row that the row extends.
    #[must_use]
    pub const fn id(&self) -> MethodDebugInformationId {
        self.id
    }

    /// Return the raw row.
    #[must_use]
    pub const fn row(&self) -> &MethodDebugInformationRow {
        &self.row
    }

    /// Return the `Document` column: the single document, or nil.
    #[must_use]
    pub const fn document(&self) -> DocumentId {
        self.row.document
    }

    /// Return the bytes of the sequence point blob. They are empty if the method has no
    /// sequence points.
    #[must_use]
    pub fn sequence_points_blob(&self) -> &[u8] {
        &self.sequence_points
    }

    /// Return the sequence points, or `None` if the method has no sequence points.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`SequencePoints::parse`] with the location of the blob.
    pub fn sequence_points(&self) -> Result<Option<SequencePoints<'_>>> {
        if self.sequence_points.is_empty() {
            return Ok(None);
        }
        SequencePoints::parse(&self.sequence_points, self.row.document)
            .map(Some)
            .map_err(|error| self.blob_error(error))
    }

    /// Add the location of the sequence point blob to a codec error.
    #[must_use]
    pub fn blob_error(&self, error: Error) -> Error {
        in_blob(
            error,
            TableId::METHOD_DEBUG_INFORMATION,
            self.id.0,
            "SequencePoints",
            self.row.sequence_points,
        )
    }

    /// Return the `#Blob` offset of the sequence point blob.
    #[must_use]
    pub const fn sequence_points_offset(&self) -> BlobOffset {
        self.row.sequence_points
    }
}

impl<S: ReadAt> Pdb<S> {
    /// Read a `MethodDebugInformation` row with its sequence point blob.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::row`] and [`Pdb::blob`].
    pub fn method_debug_information(
        &self,
        id: MethodDebugInformationId,
    ) -> Result<MethodDebugInformation> {
        let row: MethodDebugInformationRow = self.row(id)?;
        Ok(MethodDebugInformation {
            id,
            row,
            sequence_points: self.blob(row.sequence_points)?,
        })
    }

    /// Read all `MethodDebugInformation` rows in row order.
    pub fn method_debug_informations(
        &self,
    ) -> impl ExactSizeIterator<Item = Result<MethodDebugInformation>> + '_ {
        each_row(self, TableId::METHOD_DEBUG_INFORMATION, |pdb, row| {
            pdb.method_debug_information(MethodDebugInformationId(row))
        })
    }
}
