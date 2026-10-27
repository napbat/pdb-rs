//! Type records that refer to types in other files: type servers and
//! precompiled types.
//!
//! The layouts follow `lfTypeServer`, `lfTypeServer2`, `lfPreComp` and
//! `lfEndPreComp` in `cvinfo.h` and `TypeRecordMapping.cpp` in LLVM. The
//! names are NUL-terminated, because these leaf kinds are after
//! `LF_ST_MAX`. Object files contain these records; a linked PDB usually
//! does not.

use super::fields;
use crate::bytes::into_owned;
use crate::native::codeview::index::TypeIndex;
use crate::native::output::Output;
use crate::{Bytes, Result};

/// An `LF_TYPESERVER` record: the types are in another PDB.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct TypeServer<'a> {
    /// The signature of the PDB.
    pub signature: u32,
    /// The age of the PDB.
    pub age: u32,
    /// The path of the PDB.
    pub name: Bytes<'a>,
}

impl<'a> TypeServer<'a> {
    /// Decode the payload of an `LF_TYPESERVER` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            signature: fields.u32()?,
            age: fields.u32()?,
            name: Bytes::new(fields.name()?),
        };
        fields.finish_type_padding()?;
        Ok(value)
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.u32(self.signature)?;
        writer.u32(self.age)?;
        writer.c_string(&self.name)
    }
}

into_owned!(struct TypeServer { name; signature, age });

/// An `LF_TYPESERVER2` record: the types are in another PDB with a GUID.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct TypeServer2<'a> {
    /// The GUID of the PDB, in its serialized byte order.
    pub guid: [u8; 16],
    /// The age of the PDB.
    pub age: u32,
    /// The path of the PDB.
    pub name: Bytes<'a>,
}

impl<'a> TypeServer2<'a> {
    /// Decode the payload of an `LF_TYPESERVER2` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            guid: fields.array()?,
            age: fields.u32()?,
            name: Bytes::new(fields.name()?),
        };
        fields.finish_type_padding()?;
        Ok(value)
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.bytes(&self.guid)?;
        writer.u32(self.age)?;
        writer.c_string(&self.name)
    }
}

into_owned!(struct TypeServer2 { name; guid, age });

/// An `LF_PRECOMP` record: the types include a precompiled type file.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Precompiled<'a> {
    /// The first type index that the inclusion gives.
    pub start_index: TypeIndex,
    /// The number of types in the inclusion.
    pub count: u32,
    /// The signature of the precompiled types.
    pub signature: u32,
    /// The path of the file with the precompiled types.
    pub name: Bytes<'a>,
}

impl<'a> Precompiled<'a> {
    /// Decode the payload of an `LF_PRECOMP` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            start_index: fields.type_index()?,
            count: fields.u32()?,
            signature: fields.u32()?,
            name: Bytes::new(fields.name()?),
        };
        fields.finish_type_padding()?;
        Ok(value)
    }

    /// Append the payload fields.
    pub(super) fn encode(&self, writer: &mut Output<'_>) -> Result<()> {
        writer.type_index(self.start_index)?;
        writer.u32(self.count)?;
        writer.u32(self.signature)?;
        writer.c_string(&self.name)
    }
}

into_owned!(struct Precompiled { name; start_index, count, signature });

/// Decode the payload of an `LF_ENDPRECOMP` record: the `u32` signature.
pub(super) fn decode_end_precompiled(data: &[u8]) -> Result<u32> {
    let mut fields = fields(data);
    let signature = fields.u32()?;
    fields.finish_type_padding()?;
    Ok(signature)
}
