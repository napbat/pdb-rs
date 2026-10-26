//! Decoding of common modern symbol records.
//!
//! The layouts follow `PUBSYM32`, `DATASYM32` and `PROCSYM32` in the
//! Microsoft `cvinfo.h` header. These kinds store the name as a
//! NUL-terminated string. Padding bytes can follow the terminator.

use super::{Fields, Record};
use crate::{Error, ErrorKind, Result};

/// The kind of a public symbol (`S_PUB32`).
pub const S_PUB32: u16 = 0x110e;
/// The kind of a module-local data symbol (`S_LDATA32`).
pub const S_LDATA32: u16 = 0x110c;
/// The kind of a global data symbol (`S_GDATA32`).
pub const S_GDATA32: u16 = 0x110d;
/// The kind of a module-local procedure start (`S_LPROC32`).
pub const S_LPROC32: u16 = 0x110f;
/// The kind of a global procedure start (`S_GPROC32`).
pub const S_GPROC32: u16 = 0x1110;
/// The kind of a module-local procedure start with an ID index (`S_LPROC32_ID`).
pub const S_LPROC32_ID: u16 = 0x1146;
/// The kind of a global procedure start with an ID index (`S_GPROC32_ID`).
pub const S_GPROC32_ID: u16 = 0x1147;

/// The error context for a payload that is too short for its fixed fields.
const SHORT_PAYLOAD: &str = "symbol record payload is shorter than its fixed fields";

/// A decoded symbol with an address and a name.
///
/// The value keeps the original [`Record`]. Use [`Symbol::kind`] to tell
/// global symbols from module-local symbols.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Symbol<'a> {
    /// The original record.
    record: Record<'a>,
    /// The name bytes without the terminator and the padding.
    name: &'a [u8],
    /// The section number of the address.
    section: u16,
    /// The offset of the address in its section.
    offset: u32,
    /// The fields that only some kinds have.
    detail: SymbolDetail,
}

impl<'a> Symbol<'a> {
    /// Return the original record.
    #[must_use]
    pub const fn record(&self) -> Record<'a> {
        self.record
    }

    /// Return the original record kind, for example [`S_GPROC32`].
    #[must_use]
    pub const fn kind(&self) -> u16 {
        self.record.kind
    }

    /// Return the name bytes.
    ///
    /// The bytes do not include the NUL terminator or the padding after it.
    /// The bytes are not demangled. Modern compilers write UTF-8, but this
    /// function does not check the encoding.
    #[must_use]
    pub const fn name(&self) -> &'a [u8] {
        self.name
    }

    /// Return the section number of the address.
    ///
    /// In a PE image, section numbers start at 1. The value 0 can identify an
    /// absolute or discarded symbol.
    #[must_use]
    pub const fn section(&self) -> u16 {
        self.section
    }

    /// Return the offset of the address in its section.
    #[must_use]
    pub const fn offset(&self) -> u32 {
        self.offset
    }

    /// Return the fields that only some kinds have.
    #[must_use]
    pub const fn detail(&self) -> SymbolDetail {
        self.detail
    }

    /// Return the type index or ID index, if the kind has one.
    ///
    /// For [`S_GPROC32_ID`] and [`S_LPROC32_ID`], the value is an index in
    /// the IPI stream. For the other procedure kinds and the data kinds, the
    /// value is an index in the TPI stream. [`S_PUB32`] has no index.
    #[must_use]
    pub const fn type_index(&self) -> Option<u32> {
        match self.detail {
            SymbolDetail::Public { .. } => None,
            SymbolDetail::Data { type_index } => Some(type_index),
            SymbolDetail::Procedure(procedure) => Some(procedure.type_index),
        }
    }

    /// Return the length of the procedure code in bytes, if the symbol is a procedure.
    #[must_use]
    pub const fn code_length(&self) -> Option<u32> {
        match self.detail {
            SymbolDetail::Procedure(procedure) => Some(procedure.code_length),
            SymbolDetail::Public { .. } | SymbolDetail::Data { .. } => None,
        }
    }
}

/// The fields of a [`Symbol`] that depend on its kind.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum SymbolDetail {
    /// The fields of [`S_PUB32`].
    Public {
        /// The `CV_PUBSYMFLAGS` bits (code, function, managed code, MSIL).
        flags: u32,
    },
    /// The fields of [`S_GDATA32`] and [`S_LDATA32`].
    Data {
        /// The type index in the TPI stream.
        type_index: u32,
    },
    /// The fields of [`S_GPROC32`], [`S_LPROC32`], [`S_GPROC32_ID`] and [`S_LPROC32_ID`].
    Procedure(Procedure),
}

/// The fixed fields of a procedure start record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Procedure {
    /// The offset of the parent scope record in the module symbols, or 0.
    pub parent: u32,
    /// The offset of the matching `S_END` record in the module symbols.
    pub end: u32,
    /// The offset of the next procedure record in the module symbols, or 0.
    pub next: u32,
    /// The length of the procedure code in bytes.
    pub code_length: u32,
    /// The offset of the end of the prologue, relative to the procedure start.
    pub debug_start: u32,
    /// The offset of the start of the epilogue, relative to the procedure start.
    pub debug_end: u32,
    /// The type index in the TPI stream, or the ID index in the IPI stream for `_ID` kinds.
    pub type_index: u32,
    /// The `CV_PROCFLAGS` bits.
    pub flags: u8,
}

/// Decode `record` if its kind is supported.
pub(super) fn decode(record: Record<'_>) -> Result<Option<Symbol<'_>>> {
    let mut fields = Fields::new(record.data, ErrorKind::InvalidFormat, SHORT_PAYLOAD);
    let (section, offset, detail) = match record.kind {
        S_PUB32 => {
            let flags = fields.u32()?;
            let offset = fields.u32()?;
            (fields.u16()?, offset, SymbolDetail::Public { flags })
        }
        S_GDATA32 | S_LDATA32 => {
            let type_index = fields.u32()?;
            let offset = fields.u32()?;
            (fields.u16()?, offset, SymbolDetail::Data { type_index })
        }
        S_GPROC32 | S_LPROC32 | S_GPROC32_ID | S_LPROC32_ID => {
            let parent = fields.u32()?;
            let end = fields.u32()?;
            let next = fields.u32()?;
            let code_length = fields.u32()?;
            let debug_start = fields.u32()?;
            let debug_end = fields.u32()?;
            let type_index = fields.u32()?;
            let offset = fields.u32()?;
            let section = fields.u16()?;
            let flags = fields.u8()?;
            let procedure = Procedure {
                parent,
                end,
                next,
                code_length,
                debug_start,
                debug_end,
                type_index,
                flags,
            };
            (section, offset, SymbolDetail::Procedure(procedure))
        }
        _ => return Ok(None),
    };
    let name = terminated(fields.rest())?;
    Ok(Some(Symbol {
        record,
        name,
        section,
        offset,
        detail,
    }))
}

/// Return the bytes before the first NUL byte.
fn terminated(bytes: &[u8]) -> Result<&[u8]> {
    let end = bytes.iter().position(|&byte| byte == 0).ok_or_else(|| {
        Error::new(ErrorKind::InvalidFormat, None).with_context("symbol name has no NUL terminator")
    })?;
    Ok(bytes.split_at(end).0)
}
