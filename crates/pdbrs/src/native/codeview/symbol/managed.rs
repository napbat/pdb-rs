//! Managed procedure, slot, constant and OEM records.
//!
//! The .NET symbol writers write these records in Windows PDBs of managed
//! assemblies. The layouts follow `MANPROCSYM`, `ATTRSLOTSYM`, `CONSTSYM` and
//! `OEMSYMBOL` in Microsoft `cvinfo.h`. That header packs the structures to
//! one byte, so no padding separates the fields. `S_TOKENREF` has the
//! `REFSYM2` layout of `S_PROCREF` and decodes as a
//! [`ProcedureReference`](super::ProcedureReference).
//!
//! A metadata token is a raw `u32`: the high byte is the metadata table and
//! the low 24 bits are the row number. For example, `0x0600_0001` is the first
//! `MethodDef` row.

use super::super::Fields;
use super::super::index::TypeIndex;
use super::super::numeric::Integer;
use super::{LocalFlags, ProcedureFlags, SectionOffset};
use crate::Result;

/// The kind of an OEM-defined record (`S_OEM`).
pub const S_OEM: u16 = 0x0404;
/// The kind of a managed local variable in an IL slot (`S_MANSLOT`).
pub const S_MANSLOT: u16 = 0x1120;
/// The kind of a reference to a managed procedure (`S_TOKENREF`).
pub const S_TOKENREF: u16 = 0x1129;
/// The kind of a global managed procedure start (`S_GMANPROC`).
pub const S_GMANPROC: u16 = 0x112a;
/// The kind of a module-local managed procedure start (`S_LMANPROC`).
pub const S_LMANPROC: u16 = 0x112b;
/// The kind of a managed constant (`S_MANCONSTANT`).
pub const S_MANCONSTANT: u16 = 0x112d;

/// The OEM ID of a symbol attribute from the Microsoft native symbol writer.
///
/// The writer stores each `ISymUnmanagedWriter::SetSymAttribute` value in an
/// [`S_OEM`] record with this ID. The bytes are the GUID
/// `c6ea3fc9-59b3-49d6-bc25-0902bbabb460` in serialized order. Roslyn uses
/// such attributes for its custom debug information (`MD2`) and for async
/// method information (`asyncMethodInfo`).
pub const MANAGED_ATTRIBUTE_OEM_ID: [u8; 16] = [
    0xc9, 0x3f, 0xea, 0xc6, 0xb3, 0x59, 0xd6, 0x49, 0xbc, 0x25, 0x09, 0x02, 0xbb, 0xab, 0xb4, 0x60,
];

/// The numeric leaf of a counted byte string (`LF_VARSTRING`).
const LF_VARSTRING: u16 = 0x8010;

/// A managed procedure start record (`S_GMANPROC` and `S_LMANPROC`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ManagedProcedure<'a> {
    /// The module stream offset of the enclosing scope record, or 0.
    pub parent: u32,
    /// The module stream offset of the record that ends this scope.
    pub end: u32,
    /// The module stream offset of the next procedure record, or 0. The
    /// scope check does not use this value.
    pub next: u32,
    /// The length of the procedure code in bytes.
    pub code_length: u32,
    /// The offset of the end of the prologue, relative to the procedure start.
    pub debug_start: u32,
    /// The offset of the start of the epilogue, relative to the procedure start.
    pub debug_end: u32,
    /// The metadata token of the method, usually a `MethodDef` token (table `0x06`).
    pub token: u32,
    /// The start address. For IL code, the offset is an IL offset.
    pub address: SectionOffset,
    /// The flags.
    pub flags: ProcedureFlags,
    /// The register of the return value. It can be 0.
    pub return_register: u16,
    /// The name. It can be empty: the metadata has the method name.
    pub name: &'a [u8],
}

impl<'a> ManagedProcedure<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let parent = fields.u32()?;
        let end = fields.u32()?;
        let next = fields.u32()?;
        let code_length = fields.u32()?;
        let debug_start = fields.u32()?;
        let debug_end = fields.u32()?;
        let token = fields.u32()?;
        let address = SectionOffset::read(fields)?;
        let flags = ProcedureFlags(fields.u8()?);
        let return_register = fields.u16()?;
        let name = fields.name()?;
        Ok(Self {
            parent,
            end,
            next,
            code_length,
            debug_start,
            debug_end,
            token,
            address,
            flags,
            return_register,
            name,
        })
    }
}

/// A managed local variable in an IL slot (`S_MANSLOT`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ManagedSlot<'a> {
    /// The index of the local variable slot in the method body.
    pub slot: u32,
    /// A type index or a metadata token. Roslyn writes the token of the local
    /// signature (`StandAloneSig`, table `0x11`), so
    /// [`Symbol::type_index`](super::Symbol::type_index) does not give it.
    pub type_or_token: u32,
    /// The first code address where the variable is live. Roslyn writes 0:0.
    pub address: SectionOffset,
    /// The flags.
    pub flags: LocalFlags,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> ManagedSlot<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let slot = fields.u32()?;
        let type_or_token = fields.u32()?;
        let address = SectionOffset::read(fields)?;
        let flags = LocalFlags(fields.u16()?);
        let name = fields.name()?;
        Ok(Self {
            slot,
            type_or_token,
            address,
            flags,
            name,
        })
    }
}

/// The value of a managed constant.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ManagedConstantValue<'a> {
    /// An immediate value or an integer leaf.
    Integer(Integer),
    /// An `LF_VARSTRING` leaf: the counted bytes of a string. The bytes are
    /// not decoded as text.
    String(&'a [u8]),
    /// A fixed-size leaf that is not an integer: a real, a complex value,
    /// `LF_DECIMAL` or `LF_DATE`. The bytes are the raw value after the leaf.
    Fixed {
        /// The leaf, for example `0x8006` (`LF_REAL64`).
        leaf: u16,
        /// The value bytes.
        bytes: &'a [u8],
    },
}

impl<'a> ManagedConstantValue<'a> {
    /// Read a numeric leaf.
    fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let leaf = fields
            .rest()
            .first_chunk::<2>()
            .map(|bytes| u16::from_le_bytes(*bytes));
        if leaf == Some(LF_VARSTRING) {
            fields.u16()?;
            let length = fields.u16()?;
            return Ok(Self::String(fields.bytes(usize::from(length))?));
        }
        if let Some((leaf, size)) = leaf.and_then(|leaf| Some((leaf, fixed_leaf_size(leaf)?))) {
            fields.u16()?;
            return Ok(Self::Fixed {
                leaf,
                bytes: fields.bytes(size)?,
            });
        }
        Ok(Self::Integer(fields.integer()?))
    }
}

/// Return the value size of a fixed-size numeric leaf that is not an integer.
///
/// The sizes come from the `lfReal*`, `lfCmplx*`, `lfDecimal` and `lfDate`
/// structures in `cvinfo.h`.
const fn fixed_leaf_size(leaf: u16) -> Option<usize> {
    Some(match leaf {
        // LF_REAL16
        0x801c => 2,
        // LF_REAL32
        0x8005 => 4,
        // LF_REAL48
        0x800b => 6,
        // LF_REAL64, LF_COMPLEX32 and LF_DATE
        0x8006 | 0x800c | 0x801a => 8,
        // LF_REAL80
        0x8007 => 10,
        // LF_REAL128, LF_COMPLEX64 and LF_DECIMAL
        0x8008 | 0x800d | 0x8019 => 16,
        // LF_COMPLEX80
        0x800e => 20,
        // LF_COMPLEX128
        0x800f => 32,
        _ => return None,
    })
}

/// A managed constant record (`S_MANCONSTANT`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ManagedConstant<'a> {
    /// A type index or a metadata token. Roslyn writes the token of the
    /// constant signature (`StandAloneSig`, table `0x11`).
    pub type_or_token: u32,
    /// The value.
    pub value: ManagedConstantValue<'a>,
    /// The name.
    pub name: &'a [u8],
}

impl<'a> ManagedConstant<'a> {
    /// Read the fields.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let type_or_token = fields.u32()?;
        let value = ManagedConstantValue::read(fields)?;
        let name = fields.name()?;
        Ok(Self {
            type_or_token,
            value,
            name,
        })
    }
}

/// An OEM-defined record (`S_OEM`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Oem<'a> {
    /// The OEM ID: GUID bytes in serialized order, for example
    /// [`MANAGED_ATTRIBUTE_OEM_ID`].
    pub id: [u8; 16],
    /// The type index in the TPI stream.
    pub type_index: TypeIndex,
    /// The OEM data after the type index, raw, with the record padding.
    ///
    /// For [`MANAGED_ATTRIBUTE_OEM_ID`], the fixtures show a NUL-terminated
    /// UTF-16 attribute name and then the attribute value. No public document
    /// describes this layout, so the bytes stay raw.
    pub data: &'a [u8],
}

impl<'a> Oem<'a> {
    /// Read the fields and all remaining bytes.
    pub(super) fn read(fields: &mut Fields<'a>) -> Result<Self> {
        let mut id = [0; 16];
        id.copy_from_slice(fields.bytes(16)?);
        let type_index = fields.type_index()?;
        let data = fields.bytes(fields.rest().len())?;
        Ok(Self {
            id,
            type_index,
            data,
        })
    }
}
