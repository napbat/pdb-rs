//! A checked writer for the fields of `CodeView` records.
//!
//! [`FieldWriter`] is the counterpart of [`super::Fields`]. It appends
//! little-endian fields to a caller `Vec<u8>` with fallible reservation. It
//! frames a record with [`FieldWriter::begin_record`] and
//! [`FieldWriter::finish_record`], which pad the record and set its `u16`
//! length field. Encoders do not truncate values: a value that does not fit
//! its field gives an error.

use alloc::vec::Vec;

use super::index::{IdIndex, TypeIndex};
use crate::{Error, ErrorKind, Result};

/// The alignment of records and field-list members.
const ALIGNMENT: usize = 4;

/// The `LF_PAD` bytes of the longest padding run. A shorter run is a suffix.
const TYPE_PADDING: [u8; 3] = [0xf3, 0xf2, 0xf1];

/// The padding rule at the end of a record or a field-list member.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Padding {
    /// No padding.
    None,
    /// `LF_PAD` bytes that count down to `0xf1`, for type records.
    Type,
}

/// The start of a record or a field-list member in the output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RecordStart(usize);

/// Return the error for an allocation that failed.
fn no_memory() -> Error {
    Error::new(ErrorKind::LimitExceeded, None)
        .with_context("CodeView record output cannot be allocated")
}

/// An appender of little-endian fields.
pub(crate) struct FieldWriter<'v> {
    /// The output.
    out: &'v mut Vec<u8>,
}

impl<'v> FieldWriter<'v> {
    /// Create a writer that appends to `out`.
    pub(crate) const fn new(out: &'v mut Vec<u8>) -> Self {
        Self { out }
    }

    /// Return the output, for encoders of nested values.
    pub(crate) const fn out(&mut self) -> &mut Vec<u8> {
        self.out
    }

    /// Return the number of bytes in the output.
    pub(crate) const fn len(&self) -> usize {
        self.out.len()
    }

    /// Append bytes.
    pub(crate) fn bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.out.try_reserve(bytes.len()).map_err(|_| no_memory())?;
        self.out.extend_from_slice(bytes);
        Ok(())
    }

    /// Append a `u8`.
    pub(crate) fn u8(&mut self, value: u8) -> Result<()> {
        self.bytes(&[value])
    }

    /// Append a little-endian `u16`.
    pub(crate) fn u16(&mut self, value: u16) -> Result<()> {
        self.bytes(&value.to_le_bytes())
    }

    /// Append a little-endian `u32`.
    pub(crate) fn u32(&mut self, value: u32) -> Result<()> {
        self.bytes(&value.to_le_bytes())
    }

    /// Append a little-endian `i32`.
    pub(crate) fn i32(&mut self, value: i32) -> Result<()> {
        self.bytes(&value.to_le_bytes())
    }

    /// Append a little-endian `u64`.
    pub(crate) fn u64(&mut self, value: u64) -> Result<()> {
        self.bytes(&value.to_le_bytes())
    }

    /// Append a 32-bit type index.
    pub(crate) fn type_index(&mut self, index: TypeIndex) -> Result<()> {
        self.u32(index.0)
    }

    /// Append a 32-bit ID index.
    pub(crate) fn id_index(&mut self, index: IdIndex) -> Result<()> {
        self.u32(index.0)
    }

    /// Append a name and its NUL terminator.
    ///
    /// A name with a NUL byte gives [`ErrorKind::InvalidFormat`], because a
    /// reader would end the name at that byte.
    pub(crate) fn name(&mut self, name: &[u8]) -> Result<()> {
        if name.contains(&0) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("CodeView name contains a NUL byte"));
        }
        self.out
            .try_reserve(name.len().saturating_add(1))
            .map_err(|_| no_memory())?;
        self.out.extend_from_slice(name);
        self.out.push(0);
        Ok(())
    }

    /// Return the current end of the output as a start position.
    ///
    /// Field-list members align relative to the start of their record.
    pub(crate) const fn position(&self) -> RecordStart {
        RecordStart(self.out.len())
    }

    /// Append padding until the output length from `start` is a multiple of 4.
    pub(crate) fn pad(&mut self, start: RecordStart, padding: Padding) -> Result<()> {
        let used = (self.out.len() - start.0) % ALIGNMENT;
        let run = match padding {
            Padding::None => return Ok(()),
            Padding::Type => &TYPE_PADDING,
        };
        if used == 0 {
            return Ok(());
        }
        // `used` is 1 to 3, so the run has `4 - used` bytes.
        self.bytes(&run[used - 1..])
    }

    /// Append the length field and the kind field of a record.
    ///
    /// The length field is a placeholder until [`FieldWriter::finish_record`].
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
        let Ok(length) = u16::try_from(self.out.len() - start.0 - 2) else {
            self.out.truncate(start.0);
            return Err(Error::new(ErrorKind::LimitExceeded, None)
                .with_context("CodeView record is longer than its u16 length field allows"));
        };
        self.out[start.0..start.0 + 2].copy_from_slice(&length.to_le_bytes());
        Ok(())
    }
}
