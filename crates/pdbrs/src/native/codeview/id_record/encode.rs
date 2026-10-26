//! Encoders of IPI records.

use alloc::vec::Vec;

use super::{
    ID_SIZE, IdList, IdRecord, LF_BUILDINFO, LF_FUNC_ID, LF_MFUNC_ID, LF_STRING_ID, LF_SUBSTR_LIST,
    LF_UDT_MOD_SRC_LINE, LF_UDT_SRC_LINE,
};
use crate::native::codeview::index::IdIndex;
use crate::native::codeview::writer::{FieldWriter, Padding};
use crate::{Error, ErrorKind, Result};

/// Return the error for a list that is longer than its count field allows.
fn too_long() -> Error {
    Error::new(ErrorKind::LimitExceeded, None)
        .with_context("IPI record list is longer than its count field allows")
}

impl<'a> IdList<'a> {
    /// Make a list from serialized little-endian `u32` ID indices.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the length is not a multiple
    /// of 4.
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self> {
        let (entries, rest) = bytes.as_chunks::<ID_SIZE>();
        if rest.is_empty() {
            Ok(Self { entries })
        } else {
            Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("ID index list has a partial index"))
        }
    }

    /// Return the serialized indices.
    #[must_use]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.entries.as_flattened()
    }
}

impl IdRecord<'_> {
    /// Return the leaf kind of the record.
    #[must_use]
    pub const fn kind(&self) -> u16 {
        match self {
            Self::Function(_) => LF_FUNC_ID,
            Self::MemberFunction(_) => LF_MFUNC_ID,
            Self::BuildInfo(_) => LF_BUILDINFO,
            Self::SubstringList(_) => LF_SUBSTR_LIST,
            Self::String(_) => LF_STRING_ID,
            Self::UdtSourceLine(_) => LF_UDT_SRC_LINE,
            Self::UdtModuleSourceLine(_) => LF_UDT_MOD_SRC_LINE,
            Self::Unsupported(record) => record.kind,
        }
    }

    /// Append the complete record to `out`: the length field, the kind, the
    /// payload and the `LF_PAD` padding to a multiple of 4 bytes.
    ///
    /// For a decoded record of a stream that pads its records, the output is
    /// the input record. [`IdRecord::Unsupported`] keeps its payload bytes
    /// and gets no padding.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`] if `out` cannot grow, if the record is
    ///   longer than its `u16` length field allows, or if a list is longer
    ///   than its count field allows.
    /// - [`ErrorKind::InvalidFormat`] if a name contains a NUL byte.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut writer = FieldWriter::new(out);
        let start = writer.begin_record(self.kind())?;
        let padding = match self {
            Self::Function(value) => {
                writer.id_index(value.scope.unwrap_or(IdIndex::NONE))?;
                writer.type_index(value.function_type)?;
                writer.name(value.name)?;
                if let Some(hash) = value.decorated_name_hash {
                    writer.u64(hash)?;
                }
                Padding::Type
            }
            Self::MemberFunction(value) => {
                writer.type_index(value.parent_type)?;
                writer.type_index(value.function_type)?;
                writer.name(value.name)?;
                if let Some(hash) = value.decorated_name_hash {
                    writer.u64(hash)?;
                }
                Padding::Type
            }
            Self::BuildInfo(value) => {
                writer.u16(u16::try_from(value.arguments.len()).map_err(|_| too_long())?)?;
                writer.bytes(value.arguments.as_bytes())?;
                Padding::Type
            }
            Self::SubstringList(list) => {
                writer.u32(u32::try_from(list.len()).map_err(|_| too_long())?)?;
                writer.bytes(list.as_bytes())?;
                Padding::Type
            }
            Self::String(value) => {
                writer.id_index(value.substrings.unwrap_or(IdIndex::NONE))?;
                writer.name(value.string)?;
                Padding::Type
            }
            Self::UdtSourceLine(value) => {
                writer.type_index(value.udt)?;
                writer.id_index(value.source_file)?;
                writer.u32(value.line)?;
                Padding::Type
            }
            Self::UdtModuleSourceLine(value) => {
                writer.type_index(value.udt)?;
                writer.u32(value.name_offset)?;
                writer.u32(value.line)?;
                writer.u16(value.module)?;
                if value.padded {
                    Padding::Type
                } else {
                    Padding::None
                }
            }
            Self::Unsupported(record) => {
                writer.bytes(record.data)?;
                Padding::None
            }
        };
        writer.finish_record(start, padding)
    }
}
