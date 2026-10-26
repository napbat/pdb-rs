//! The `CodeView` methods of the native field writer.
//!
//! This module adds methods to [`Output`], the counterpart of [`super::Fields`]. The methods
//! append type indexes and ID indexes. They also frame a record with [`Output::begin_record`]
//! and [`Output::finish_record`]. These methods pad the record and set its `u16` length field.

use super::index::{IdIndex, TypeIndex};
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// The alignment of records and field-list members.
const ALIGNMENT: usize = 4;

/// The `LF_PAD` bytes of the longest padding run. A shorter run is a suffix.
const TYPE_PADDING: [u8; 3] = [0xf3, 0xf2, 0xf1];

/// The zero bytes of the longest padding run.
const ZERO_PADDING: [u8; 3] = [0; 3];

/// The padding rule at the end of a record or a field-list member.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Padding {
    /// No padding.
    None,
    /// `LF_PAD` bytes that count down to `0xf1`, for type records.
    Type,
    /// Zero bytes, for symbol records.
    Zero,
}

/// The start of a record or a field-list member in the output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RecordStart(usize);

impl Output<'_> {
    /// Append a 32-bit type index.
    pub(crate) fn type_index(&mut self, index: TypeIndex) -> Result<()> {
        self.u32(index.0)
    }

    /// Append a 32-bit ID index.
    pub(crate) fn id_index(&mut self, index: IdIndex) -> Result<()> {
        self.u32(index.0)
    }

    /// Return the current end of the output as a start position.
    ///
    /// Field-list members align relative to the start of their record.
    pub(crate) const fn position(&self) -> RecordStart {
        RecordStart(self.len())
    }

    /// Append padding until the output length from `start` is a multiple of 4.
    pub(crate) fn pad(&mut self, start: RecordStart, padding: Padding) -> Result<()> {
        let used = (self.len() - start.0) % ALIGNMENT;
        let run = match padding {
            Padding::None => return Ok(()),
            Padding::Type => &TYPE_PADDING,
            Padding::Zero => &ZERO_PADDING,
        };
        if used == 0 {
            return Ok(());
        }
        // `used` is 1 to 3, so the run has `4 - used` bytes.
        self.bytes(&run[used - 1..])
    }

    /// Append the length field and the kind field of a record.
    ///
    /// The length field is a placeholder until [`Output::finish_record`].
    pub(crate) fn begin_record(&mut self, kind: u16) -> Result<RecordStart> {
        let start = self.position();
        self.u16(0)?;
        self.u16(kind)?;
        Ok(start)
    }

    /// Pad the record that starts at `start` and set its length field.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the record is longer than the
    /// `u16` length field allows. The output then ends at `start`.
    pub(crate) fn finish_record(&mut self, start: RecordStart, padding: Padding) -> Result<()> {
        self.pad(start, padding)?;
        // The length field does not count itself.
        let Ok(length) = u16::try_from(self.len() - start.0 - 2) else {
            self.out().truncate(start.0);
            return Err(Error::new(ErrorKind::LimitExceeded, None)
                .with_context("CodeView record is longer than its u16 length field allows"));
        };
        self.out()[start.0..start.0 + 2].copy_from_slice(&length.to_le_bytes());
        Ok(())
    }
}
