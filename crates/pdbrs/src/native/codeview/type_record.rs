//! Typed decoding of TPI type records (leaves).
//!
//! [`TypeLeaf::decode`] decodes one framed [`Record`] from a TPI stream. The
//! kind values come from `CodeViewTypes.def` and the layouts come from
//! `TypeRecordMapping.cpp` in LLVM (revision `b9f2235f`). The bit layouts of
//! flag words come from `cvinfo.h` in the Microsoft PDB source.
//!
//! Each decoder reads all fixed fields, numeric leaves and names. The bytes
//! after the last field must be valid `LF_PAD` padding. Names stay as
//! bytes. Kinds without a decoder give [`TypeLeaf::Unsupported`], which keeps
//! the kind and the payload.

use super::index::TypeIndex;
use super::numeric::Integer;
use super::{Fields, Record};
use crate::{ErrorKind, Result};

mod attributes;
mod list;
mod udt;

pub use super::field_list::FieldList;
pub use attributes::{
    CallingConvention, ClassOptions, FunctionOptions, HfaKind, LabelMode, MemberAccess,
    MemberAttributes, MemberPointerRepresentation, MethodKind, MocomKind, ModifierOptions,
    PointerAttributes, PointerKind, PointerMode, VtableSlotKind,
};
pub use list::{
    ArgumentList, MethodList, MethodListEntry, MethodListIter, VirtualFunctionTable, VtableShape,
};
pub use udt::{Class, ClassKind, Enum, Tag, Union};

/// The kind of a virtual function table shape (`LF_VTSHAPE`).
pub const LF_VTSHAPE: u16 = 0x000a;
/// The kind of a code label type (`LF_LABEL`).
pub const LF_LABEL: u16 = 0x000e;
/// The kind of a type with modifiers (`LF_MODIFIER`).
pub const LF_MODIFIER: u16 = 0x1001;
/// The kind of a pointer type (`LF_POINTER`).
pub const LF_POINTER: u16 = 0x1002;
/// The kind of a procedure type (`LF_PROCEDURE`).
pub const LF_PROCEDURE: u16 = 0x1008;
/// The kind of a member function type (`LF_MFUNCTION`).
pub const LF_MFUNCTION: u16 = 0x1009;
/// The kind of an argument list (`LF_ARGLIST`).
pub const LF_ARGLIST: u16 = 0x1201;
/// The kind of a field list (`LF_FIELDLIST`).
pub const LF_FIELDLIST: u16 = 0x1203;
/// The kind of a bit field type (`LF_BITFIELD`).
pub const LF_BITFIELD: u16 = 0x1205;
/// The kind of a method overload list (`LF_METHODLIST`).
pub const LF_METHODLIST: u16 = 0x1206;
/// The kind of an array type (`LF_ARRAY`).
pub const LF_ARRAY: u16 = 0x1503;
/// The kind of a class type (`LF_CLASS`).
pub const LF_CLASS: u16 = 0x1504;
/// The kind of a structure type (`LF_STRUCTURE`).
pub const LF_STRUCTURE: u16 = 0x1505;
/// The kind of a union type (`LF_UNION`).
pub const LF_UNION: u16 = 0x1506;
/// The kind of an enumeration type (`LF_ENUM`).
pub const LF_ENUM: u16 = 0x1507;
/// The kind of an interface type (`LF_INTERFACE`).
pub const LF_INTERFACE: u16 = 0x1519;
/// The kind of a virtual function table with method names (`LF_VFTABLE`).
pub const LF_VFTABLE: u16 = 0x151d;

/// The error context for a payload that ends before its last field.
pub(crate) const TRUNCATED: &str = "type record payload is truncated";

/// Return a field reader over a type record payload.
pub(crate) const fn fields(data: &[u8]) -> Fields<'_> {
    Fields::new(data, ErrorKind::InvalidFormat, TRUNCATED)
}

/// A decoded TPI type record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum TypeLeaf<'a> {
    /// `LF_MODIFIER`.
    Modifier(Modifier),
    /// `LF_POINTER`.
    Pointer(Pointer<'a>),
    /// `LF_PROCEDURE`.
    Procedure(Procedure),
    /// `LF_MFUNCTION`.
    MemberFunction(MemberFunction),
    /// `LF_ARGLIST`.
    ArgumentList(ArgumentList<'a>),
    /// `LF_FIELDLIST`.
    FieldList(FieldList<'a>),
    /// `LF_BITFIELD`.
    Bitfield(Bitfield),
    /// `LF_METHODLIST`.
    MethodList(MethodList<'a>),
    /// `LF_ARRAY`.
    Array(Array<'a>),
    /// `LF_CLASS`, `LF_STRUCTURE` or `LF_INTERFACE`.
    Class(Class<'a>),
    /// `LF_UNION`.
    Union(Union<'a>),
    /// `LF_ENUM`.
    Enum(Enum<'a>),
    /// `LF_VTSHAPE`.
    VtableShape(VtableShape<'a>),
    /// `LF_VFTABLE`.
    VirtualFunctionTable(VirtualFunctionTable<'a>),
    /// `LF_LABEL`: the addressing mode of a code label.
    Label(LabelMode),
    /// A kind without a decoder in this crate.
    ///
    /// The record keeps the kind and the payload, including padding. ID
    /// records of the IPI stream also give this variant.
    Unsupported(Record<'a>),
}

impl<'a> TypeLeaf<'a> {
    /// Decode one type record.
    ///
    /// # Errors
    ///
    /// The errors do not have a file offset or a stream location.
    ///
    /// - [`ErrorKind::InvalidFormat`] if the payload is truncated, if a name
    ///   has no terminator, if an enumeration field has an unknown value, if
    ///   a field-list member is malformed, or if bytes after the last field
    ///   are not valid padding.
    /// - [`ErrorKind::UnsupportedVersion`] for a numeric leaf that is not an
    ///   integer, or for a field-list member kind without a decoder.
    pub fn decode(record: Record<'a>) -> Result<Self> {
        let data = record.data;
        Ok(match record.kind {
            LF_MODIFIER => Self::Modifier(Modifier::decode(data)?),
            LF_POINTER => Self::Pointer(Pointer::decode(data)?),
            LF_PROCEDURE => Self::Procedure(Procedure::decode(data)?),
            LF_MFUNCTION => Self::MemberFunction(MemberFunction::decode(data)?),
            LF_ARGLIST => Self::ArgumentList(ArgumentList::decode(data)?),
            LF_FIELDLIST => Self::FieldList(FieldList::decode(data)?),
            LF_BITFIELD => Self::Bitfield(Bitfield::decode(data)?),
            LF_METHODLIST => Self::MethodList(MethodList::decode(data)?),
            LF_ARRAY => Self::Array(Array::decode(data)?),
            LF_CLASS => Self::Class(Class::decode(ClassKind::Class, data)?),
            LF_STRUCTURE => Self::Class(Class::decode(ClassKind::Structure, data)?),
            LF_INTERFACE => Self::Class(Class::decode(ClassKind::Interface, data)?),
            LF_UNION => Self::Union(Union::decode(data)?),
            LF_ENUM => Self::Enum(Enum::decode(data)?),
            LF_VTSHAPE => Self::VtableShape(VtableShape::decode(data)?),
            LF_VFTABLE => Self::VirtualFunctionTable(VirtualFunctionTable::decode(data)?),
            LF_LABEL => Self::Label(decode_label(data)?),
            _ => Self::Unsupported(record),
        })
    }

    /// Return the options, the name and the unique name of a user-defined type.
    ///
    /// Returns `None` if the record is not a class, a structure, an
    /// interface, a union or an enumeration.
    #[must_use]
    pub const fn tag(&self) -> Option<Tag<'a>> {
        match self {
            Self::Class(value) => Some(value.tag()),
            Self::Union(value) => Some(value.tag()),
            Self::Enum(value) => Some(value.tag()),
            _ => None,
        }
    }
}

/// Decode the payload of an `LF_LABEL` record: a `u16` addressing mode.
fn decode_label(data: &[u8]) -> Result<LabelMode> {
    let mut fields = fields(data);
    let mode = LabelMode::from_raw(fields.u16()?)?;
    fields.finish_type_padding()?;
    Ok(mode)
}

/// An `LF_MODIFIER` record: a type with `const`, `volatile` or `__unaligned`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Modifier {
    /// The modified type.
    pub modified_type: TypeIndex,
    /// The modifiers.
    pub options: ModifierOptions,
}

impl Modifier {
    /// Decode the payload of an `LF_MODIFIER` record.
    fn decode(data: &[u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            modified_type: fields.type_index()?,
            options: ModifierOptions(fields.u16()?),
        };
        fields.finish_type_padding()?;
        Ok(value)
    }
}

/// The containing class of a pointer to member.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MemberPointer {
    /// The class that contains the member.
    pub containing_type: TypeIndex,
    /// The representation of the pointer.
    pub representation: MemberPointerRepresentation,
}

/// An `LF_POINTER` record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Pointer<'a> {
    /// The type that the pointer refers to.
    pub referent_type: TypeIndex,
    /// The kind, the mode, the size and the flags.
    pub attributes: PointerAttributes,
    /// The member-pointer information, present if
    /// [`PointerMode::is_member_pointer`] is `true` for the mode.
    pub member: Option<MemberPointer>,
    /// The raw base data of a based pointer, including the record padding.
    ///
    /// This value is present if [`PointerKind::is_based`] is `true` for the
    /// kind and the mode is not a member-pointer mode. The layout depends on
    /// the kind (`lfPointer::pbase` in `cvinfo.h`). This crate does not
    /// decode it, because LLVM does not decode it and no sample has it.
    pub based: Option<&'a [u8]>,
}

impl<'a> Pointer<'a> {
    /// Decode the payload of an `LF_POINTER` record.
    fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let referent_type = fields.type_index()?;
        let attributes = PointerAttributes::from_raw(fields.u32()?)?;
        let mut member = None;
        let mut based = None;
        if attributes.mode().is_member_pointer() {
            member = Some(MemberPointer {
                containing_type: fields.type_index()?,
                representation: MemberPointerRepresentation::from_raw(fields.u16()?)?,
            });
        } else if attributes.kind().is_based() {
            based = Some(fields.rest());
        }
        if based.is_none() {
            fields.finish_type_padding()?;
        }
        Ok(Self {
            referent_type,
            attributes,
            member,
            based,
        })
    }
}

/// An `LF_PROCEDURE` record: the type of a function that is not a member.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Procedure {
    /// The return type.
    pub return_type: TypeIndex,
    /// The calling convention.
    pub calling_convention: CallingConvention,
    /// The function options.
    pub options: FunctionOptions,
    /// The number of parameters.
    pub parameter_count: u16,
    /// The `LF_ARGLIST` record with the parameter types.
    pub argument_list: TypeIndex,
}

impl Procedure {
    /// Decode the payload of an `LF_PROCEDURE` record.
    fn decode(data: &[u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            return_type: fields.type_index()?,
            calling_convention: CallingConvention::from_raw(fields.u8()?)?,
            options: FunctionOptions(fields.u8()?),
            parameter_count: fields.u16()?,
            argument_list: fields.type_index()?,
        };
        fields.finish_type_padding()?;
        Ok(value)
    }
}

/// An `LF_MFUNCTION` record: the type of a member function.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MemberFunction {
    /// The return type.
    pub return_type: TypeIndex,
    /// The class that contains the function.
    pub class_type: TypeIndex,
    /// The type of `this`, or [`TypeIndex::NONE`] for a static function.
    pub this_type: TypeIndex,
    /// The calling convention.
    pub calling_convention: CallingConvention,
    /// The function options.
    pub options: FunctionOptions,
    /// The number of parameters.
    pub parameter_count: u16,
    /// The `LF_ARGLIST` record with the parameter types.
    pub argument_list: TypeIndex,
    /// The adjustment that the function applies to `this`.
    pub this_adjustment: i32,
}

impl MemberFunction {
    /// Decode the payload of an `LF_MFUNCTION` record.
    fn decode(data: &[u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            return_type: fields.type_index()?,
            class_type: fields.type_index()?,
            this_type: fields.type_index()?,
            calling_convention: CallingConvention::from_raw(fields.u8()?)?,
            options: FunctionOptions(fields.u8()?),
            parameter_count: fields.u16()?,
            argument_list: fields.type_index()?,
            this_adjustment: fields.i32()?,
        };
        fields.finish_type_padding()?;
        Ok(value)
    }
}

/// An `LF_BITFIELD` record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Bitfield {
    /// The type that holds the bits.
    pub underlying_type: TypeIndex,
    /// The number of bits.
    pub bit_size: u8,
    /// The position of the first bit.
    pub bit_offset: u8,
}

impl Bitfield {
    /// Decode the payload of an `LF_BITFIELD` record.
    fn decode(data: &[u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            underlying_type: fields.type_index()?,
            bit_size: fields.u8()?,
            bit_offset: fields.u8()?,
        };
        fields.finish_type_padding()?;
        Ok(value)
    }
}

/// An `LF_ARRAY` record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Array<'a> {
    /// The element type.
    pub element_type: TypeIndex,
    /// The type of the index.
    pub index_type: TypeIndex,
    /// The size of the array in bytes.
    pub size: Integer,
    /// The name bytes. The name is usually empty.
    pub name: &'a [u8],
}

impl<'a> Array<'a> {
    /// Decode the payload of an `LF_ARRAY` record.
    fn decode(data: &'a [u8]) -> Result<Self> {
        let mut fields = fields(data);
        let value = Self {
            element_type: fields.type_index()?,
            index_type: fields.type_index()?,
            size: fields.integer()?,
            name: fields.name()?,
        };
        fields.finish_type_padding()?;
        Ok(value)
    }
}
