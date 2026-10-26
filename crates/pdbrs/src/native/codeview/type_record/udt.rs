//! User-defined type records: classes, structures, interfaces, unions and
//! enumerations.
//!
//! These records end with a name. If [`ClassOptions::has_unique_name`] is
//! `true`, a unique (decorated) name follows the name (`mapNameAndUniqueName`
//! in LLVM `TypeRecordMapping.cpp`).

use super::{ClassOptions, fields};
use crate::Result;
use crate::native::codeview::Fields;
use crate::native::codeview::index::TypeIndex;
use crate::native::codeview::numeric::Integer;

/// The options and the names of a user-defined type.
///
/// LLVM calls the records with these fields tag records. Type hashing and
/// forward-reference resolution use only these fields.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Tag<'a> {
    /// The properties of the type.
    pub options: ClassOptions,
    /// The name bytes.
    pub name: &'a [u8],
    /// The unique name bytes, present if the options have the unique-name flag.
    pub unique_name: Option<&'a [u8]>,
}

/// Read the name, the optional unique name and the padding at the end of a record.
fn names<'a>(
    fields: &mut Fields<'a>,
    options: ClassOptions,
) -> Result<(&'a [u8], Option<&'a [u8]>)> {
    let name = fields.name()?;
    let unique_name = if options.has_unique_name() {
        Some(fields.name()?)
    } else {
        None
    };
    fields.finish_type_padding()?;
    Ok((name, unique_name))
}

/// The leaf kind of a [`Class`] record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ClassKind {
    /// `LF_CLASS`.
    Class,
    /// `LF_STRUCTURE`.
    Structure,
    /// `LF_INTERFACE`.
    Interface,
}

/// An `LF_CLASS`, `LF_STRUCTURE` or `LF_INTERFACE` record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Class<'a> {
    /// The leaf kind.
    pub kind: ClassKind,
    /// The number of members in the field list.
    pub member_count: u16,
    /// The properties of the type.
    pub options: ClassOptions,
    /// The `LF_FIELDLIST` record, or [`TypeIndex::NONE`] for a forward reference.
    pub field_list: TypeIndex,
    /// The derivation list, usually [`TypeIndex::NONE`].
    pub derivation_list: TypeIndex,
    /// The `LF_VTSHAPE` record, or [`TypeIndex::NONE`].
    pub vtable_shape: TypeIndex,
    /// The size of the type in bytes.
    pub size: Integer,
    /// The name bytes.
    pub name: &'a [u8],
    /// The unique name bytes, present if the options have the unique-name flag.
    pub unique_name: Option<&'a [u8]>,
}

impl<'a> Class<'a> {
    /// Decode the payload of a class, structure or interface record.
    pub(super) fn decode(kind: ClassKind, data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let member_count = fields.u16()?;
        let options = ClassOptions(fields.u16()?);
        let field_list = fields.type_index()?;
        let derivation_list = fields.type_index()?;
        let vtable_shape = fields.type_index()?;
        let size = fields.integer()?;
        let (name, unique_name) = names(&mut fields, options)?;
        Ok(Self {
            kind,
            member_count,
            options,
            field_list,
            derivation_list,
            vtable_shape,
            size,
            name,
            unique_name,
        })
    }

    /// Return the options and the names.
    #[must_use]
    pub const fn tag(&self) -> Tag<'a> {
        Tag {
            options: self.options,
            name: self.name,
            unique_name: self.unique_name,
        }
    }
}

/// An `LF_UNION` record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Union<'a> {
    /// The number of members in the field list.
    pub member_count: u16,
    /// The properties of the type.
    pub options: ClassOptions,
    /// The `LF_FIELDLIST` record, or [`TypeIndex::NONE`] for a forward reference.
    pub field_list: TypeIndex,
    /// The size of the type in bytes.
    pub size: Integer,
    /// The name bytes.
    pub name: &'a [u8],
    /// The unique name bytes, present if the options have the unique-name flag.
    pub unique_name: Option<&'a [u8]>,
}

impl<'a> Union<'a> {
    /// Decode the payload of an `LF_UNION` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let member_count = fields.u16()?;
        let options = ClassOptions(fields.u16()?);
        let field_list = fields.type_index()?;
        let size = fields.integer()?;
        let (name, unique_name) = names(&mut fields, options)?;
        Ok(Self {
            member_count,
            options,
            field_list,
            size,
            name,
            unique_name,
        })
    }

    /// Return the options and the names.
    #[must_use]
    pub const fn tag(&self) -> Tag<'a> {
        Tag {
            options: self.options,
            name: self.name,
            unique_name: self.unique_name,
        }
    }
}

/// An `LF_ENUM` record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Enum<'a> {
    /// The number of enumerators in the field list.
    pub member_count: u16,
    /// The properties of the type.
    pub options: ClassOptions,
    /// The integer type of the enumerator values.
    pub underlying_type: TypeIndex,
    /// The `LF_FIELDLIST` record, or [`TypeIndex::NONE`] for a forward reference.
    pub field_list: TypeIndex,
    /// The name bytes.
    pub name: &'a [u8],
    /// The unique name bytes, present if the options have the unique-name flag.
    pub unique_name: Option<&'a [u8]>,
}

impl<'a> Enum<'a> {
    /// Decode the payload of an `LF_ENUM` record.
    pub(super) fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let member_count = fields.u16()?;
        let options = ClassOptions(fields.u16()?);
        let underlying_type = fields.type_index()?;
        let field_list = fields.type_index()?;
        let (name, unique_name) = names(&mut fields, options)?;
        Ok(Self {
            member_count,
            options,
            underlying_type,
            field_list,
            name,
            unique_name,
        })
    }

    /// Return the options and the names.
    #[must_use]
    pub const fn tag(&self) -> Tag<'a> {
        Tag {
            options: self.options,
            name: self.name,
            unique_name: self.unique_name,
        }
    }
}
