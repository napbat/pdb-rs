//! `DEBUG_S_SYMBOLS` subsections: symbol records.
//!
//! The data is a sequence of `CodeView` symbol records without a signature,
//! as in LLVM `DebugSymbolsSubsection`. Object files keep their symbols in
//! this subsection. `link.exe` moves them to the symbol region of the module
//! stream (`Mod1::AddSymbols2` in `PDB/dbi/mod.cpp`).

use alloc::vec::Vec;

use super::invalid;
use crate::Result;
use crate::native::codeview::{Record, Records};
use crate::native::output::Output;

/// The size of the length field and the kind field of a record.
const PREFIX_SIZE: usize = 4;

/// A decoded `DEBUG_S_SYMBOLS` subsection.
///
/// [`SymbolsSubsection::parse`] frames all records. Use
/// [`Symbol::decode`](crate::native::codeview::symbol::Symbol::decode) to
/// decode the payload of a record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SymbolsSubsection<'a> {
    /// The framed records.
    data: &'a [u8],
    /// The number of records.
    count: usize,
}

impl<'a> SymbolsSubsection<'a> {
    /// Frame the records of a `DEBUG_S_SYMBOLS` subsection.
    ///
    /// The records must fill the data exactly.
    ///
    /// # Errors
    ///
    /// Returns the framing errors of [`Records`]:
    /// [`ErrorKind::UnexpectedEof`](crate::ErrorKind::UnexpectedEof) for a
    /// truncated record and
    /// [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) for a
    /// length that does not cover the kind. The errors do not have a file
    /// offset or a stream location.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let mut count = 0;
        for record in Records::new(data) {
            record?;
            count += 1;
        }
        Ok(Self { data, count })
    }

    /// Return the number of records.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Return `true` if the subsection has no records.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Return the records. Record offsets are relative to the subsection
    /// data.
    ///
    /// [`SymbolsSubsection::parse`] framed the records, so the iterator does
    /// not return an error.
    #[must_use]
    pub const fn records(&self) -> Records<'a> {
        Records::new(self.data)
    }

    /// Append the subsection data to `out`: each record with its length
    /// and kind.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a
    ///   record length does not fit in 16 bits.
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if the
    ///   output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        for record in self.records() {
            encode_symbol_record(&record?, out)?;
        }
        Ok(())
    }
}

/// Append one framed `CodeView` record to `out`: the length, the kind and
/// the payload.
///
/// The payload must include the padding at the end of the record, as in
/// [`Record::data`]. The function does not use [`Record::offset`].
///
/// # Errors
///
/// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if the
///   record length does not fit in 16 bits.
/// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if the
///   output cannot grow.
fn encode_symbol_record(record: &Record<'_>, out: &mut Vec<u8>) -> Result<()> {
    // The length counts the kind and the payload, not itself.
    let length = record
        .data
        .len()
        .checked_add(PREFIX_SIZE - 2)
        .and_then(|length| u16::try_from(length).ok())
        .ok_or_else(|| invalid("symbol record length does not fit in 16 bits"))?;
    Output::new(out).u16(length)?;
    Output::new(out).u16(record.kind)?;
    Output::new(out).bytes(record.data)
}
