//! Type records with producer-defined data: `LF_OEM`, `LF_OEM2`,
//! `LF_COBOL0` and `LF_SKIP`.
//!
//! The layouts follow `lfOEM`, `lfOEM2`, `lfCobol0` and `lfSkip` in
//! `cvinfo.h`. The producer-defined data runs to the end of the record.
//! The record has no field that separates this data from the record
//! padding, so [`Oem::data`] and the other data fields keep the padding,
//! and the encoders add no padding.

use super::{TypeIndexList, fields};
use crate::Result;
use crate::native::codeview::index::TypeIndex;
use crate::native::output::Output;

/// An `LF_OEM` record: a type that an OEM defines.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Oem<'a> {
    /// The OEM identifier that Microsoft assigned.
    pub oem: u16,
    /// The type identifier that the OEM assigned.
    pub record: u16,
    /// The type indices that the record refers to.
    pub indices: TypeIndexList<'a>,
    /// The OEM-defined data and the record padding.
    pub data: &'a [u8],
}

impl<'a> Oem<'a> {
    /// Decode the payload of an `LF_OEM` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let oem = fields.u16()?;
        let record = fields.u16()?;
        let indices = TypeIndexList::decode_counted(&mut fields)?;
        Ok(Self {
            oem,
            record,
            indices,
            data: fields.rest(),
        })
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.u16(self.oem)?;
        writer.u16(self.record)?;
        self.indices.encode_counted(writer)?;
        writer.bytes(self.data)
    }
}

/// An `LF_OEM2` record: a type that an OEM with a GUID defines.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Oem2<'a> {
    /// The OEM GUID, in its serialized byte order.
    pub oem: [u8; 16],
    /// The type indices that the record refers to.
    pub indices: TypeIndexList<'a>,
    /// The OEM-defined data and the record padding.
    pub data: &'a [u8],
}

impl<'a> Oem2<'a> {
    /// Decode the payload of an `LF_OEM2` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let oem = fields.array()?;
        let indices = TypeIndexList::decode_counted(&mut fields)?;
        Ok(Self {
            oem,
            indices,
            data: fields.rest(),
        })
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.bytes(&self.oem)?;
        self.indices.encode_counted(writer)?;
        writer.bytes(self.data)
    }
}

/// An `LF_COBOL0` record: a COBOL type.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Cobol0<'a> {
    /// The parent type.
    pub parent_type: TypeIndex,
    /// The COBOL data and the record padding.
    pub data: &'a [u8],
}

impl<'a> Cobol0<'a> {
    /// Decode the payload of an `LF_COBOL0` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        Ok(Self {
            parent_type: fields.type_index()?,
            data: fields.rest(),
        })
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.type_index(self.parent_type)?;
        writer.bytes(self.data)
    }
}

/// An `LF_SKIP` record: a record that reserves type indices.
///
/// The next valid type index follows the record. This crate decodes the
/// record. It does not change the indices of the next records.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Skip<'a> {
    /// The next valid type index.
    pub next_index: TypeIndex,
    /// The pad data.
    pub data: &'a [u8],
}

impl<'a> Skip<'a> {
    /// Decode the payload of an `LF_SKIP` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        Ok(Self {
            next_index: fields.type_index()?,
            data: fields.rest(),
        })
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.type_index(self.next_index)?;
        writer.bytes(self.data)
    }
}
