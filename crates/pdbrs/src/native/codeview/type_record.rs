//! Typed decoding and encoding of TPI type records (leaves).
//!
//! [`TypeLeaf::decode`] decodes one framed [`Record`] from a TPI stream, and
//! [`TypeLeaf::encode`] gives the complete record bytes again. The kind
//! values come from `cvinfo.h` in the Microsoft PDB source and
//! `CodeViewTypes.def` in LLVM (revision `b9f2235f`). The layouts come from
//! `cvinfo.h` and `TypeRecordMapping.cpp` in LLVM.
//!
//! Each decoder reads all fixed fields, numeric leaves and names. The bytes
//! after the last field must be valid `LF_PAD` padding. UDT records also
//! accept the padding form of Microsoft Macro Assembler and keep it (see
//! [`TypeLeaf::trailing`]). Names stay as
//! bytes. Kinds without a decoder give [`TypeLeaf::Unsupported`], which keeps
//! the kind and the payload. The encoders pad each record to a multiple of
//! 4 bytes with `LF_PAD` bytes, as Microsoft and LLVM do. A UDT record with
//! trailing bytes gets these bytes again.

use alloc::vec::Vec;

use super::field_list::FieldList;
use super::index::TypeIndex;
use super::writer::Padding;
use super::{Fields, LeafKind, Record};
use crate::native::output::Output;
use crate::{ErrorKind, Result};

mod alias;
mod array;
mod attributes;
mod basic;
mod hlsl;
mod list;
mod oem;
mod pointer;
mod references;
mod server;
mod udt;

pub use alias::{Alias, DefaultArgument};
pub use array::{
    ConstantDimensions, DimensionBound, DimensionBounds, DimensionedArray, StridedArray,
    VariableDimensions,
};
pub use attributes::{
    CallingConvention, ClassOptions, FunctionOptions, HfaKind, LabelMode, MemberAccess,
    MemberAttributes, MemberPointerRepresentation, MethodKind, MocomKind, ModifierOptions,
    PointerAttributes, PointerKind, PointerMode, VtableSlotKind,
};
pub use basic::{Array, Bitfield, MemberFunction, Modifier, Procedure};
pub use hlsl::{Hlsl, Matrix, ModifierEx, NumericList, Vector};
pub use list::{
    ArgumentList, MethodList, MethodListEntry, MethodListIter, TypeIndexList, VirtualFunctionTable,
    VtableShape,
};
pub use oem::{Cobol0, Oem, Oem2, Skip};
pub use pointer::{BasedPointer, MemberPointer, Pointer};
pub use references::{ReferenceRole, TypeReference};
pub use server::{Precompiled, TypeServer, TypeServer2};
pub use udt::{Class, ClassKind, Enum, Tag, Union, WideClass, WideUnion};

pub(crate) use list::encode_vtable_offset;

/// The kind of a virtual function table shape (`LF_VTSHAPE`).
pub const LF_VTSHAPE: LeafKind = LeafKind(0x000a);
/// The kind of a code label type (`LF_LABEL`).
pub const LF_LABEL: LeafKind = LeafKind(0x000e);
/// The kind of the end of precompiled types (`LF_ENDPRECOMP`).
pub const LF_ENDPRECOMP: LeafKind = LeafKind(0x0014);
/// The kind of a type with modifiers (`LF_MODIFIER`).
pub const LF_MODIFIER: LeafKind = LeafKind(0x1001);
/// The kind of a pointer type (`LF_POINTER`).
pub const LF_POINTER: LeafKind = LeafKind(0x1002);
/// The kind of a procedure type (`LF_PROCEDURE`).
pub const LF_PROCEDURE: LeafKind = LeafKind(0x1008);
/// The kind of a member function type (`LF_MFUNCTION`).
pub const LF_MFUNCTION: LeafKind = LeafKind(0x1009);
/// The kind of a COBOL type (`LF_COBOL0`).
pub const LF_COBOL0: LeafKind = LeafKind(0x100a);
/// The kind of a basic array (`LF_BARRAY`).
pub const LF_BARRAY: LeafKind = LeafKind(0x100b);
/// The kind of a virtual function table path (`LF_VFTPATH`).
pub const LF_VFTPATH: LeafKind = LeafKind(0x100d);
/// The kind of an OEM type (`LF_OEM`).
pub const LF_OEM: LeafKind = LeafKind(0x100f);
/// The kind of an OEM type with a GUID (`LF_OEM2`).
pub const LF_OEM2: LeafKind = LeafKind(0x1011);
/// The kind of a record that reserves type indices (`LF_SKIP`).
pub const LF_SKIP: LeafKind = LeafKind(0x1200);
/// The kind of an argument list (`LF_ARGLIST`).
pub const LF_ARGLIST: LeafKind = LeafKind(0x1201);
/// The kind of a field list (`LF_FIELDLIST`).
pub const LF_FIELDLIST: LeafKind = LeafKind(0x1203);
/// The kind of a derived class list (`LF_DERIVED`).
pub const LF_DERIVED: LeafKind = LeafKind(0x1204);
/// The kind of a bit field type (`LF_BITFIELD`).
pub const LF_BITFIELD: LeafKind = LeafKind(0x1205);
/// The kind of a method overload list (`LF_METHODLIST`).
pub const LF_METHODLIST: LeafKind = LeafKind(0x1206);
/// The kind of constant upper dimension bounds (`LF_DIMCONU`).
pub const LF_DIMCONU: LeafKind = LeafKind(0x1207);
/// The kind of constant lower and upper dimension bounds (`LF_DIMCONLU`).
pub const LF_DIMCONLU: LeafKind = LeafKind(0x1208);
/// The kind of variable upper dimension bounds (`LF_DIMVARU`).
pub const LF_DIMVARU: LeafKind = LeafKind(0x1209);
/// The kind of variable lower and upper dimension bounds (`LF_DIMVARLU`).
pub const LF_DIMVARLU: LeafKind = LeafKind(0x120a);
/// The kind of a type server reference (`LF_TYPESERVER`).
pub const LF_TYPESERVER: LeafKind = LeafKind(0x1501);
/// The kind of an array type (`LF_ARRAY`).
pub const LF_ARRAY: LeafKind = LeafKind(0x1503);
/// The kind of a class type (`LF_CLASS`).
pub const LF_CLASS: LeafKind = LeafKind(0x1504);
/// The kind of a structure type (`LF_STRUCTURE`).
pub const LF_STRUCTURE: LeafKind = LeafKind(0x1505);
/// The kind of a union type (`LF_UNION`).
pub const LF_UNION: LeafKind = LeafKind(0x1506);
/// The kind of an enumeration type (`LF_ENUM`).
pub const LF_ENUM: LeafKind = LeafKind(0x1507);
/// The kind of a dimensioned array (`LF_DIMARRAY`).
pub const LF_DIMARRAY: LeafKind = LeafKind(0x1508);
/// The kind of a precompiled type inclusion (`LF_PRECOMP`).
pub const LF_PRECOMP: LeafKind = LeafKind(0x1509);
/// The kind of a `typedef` (`LF_ALIAS`).
pub const LF_ALIAS: LeafKind = LeafKind(0x150a);
/// The kind of a default argument (`LF_DEFARG`).
pub const LF_DEFARG: LeafKind = LeafKind(0x150b);
/// The kind of a managed type (`LF_MANAGED`).
pub const LF_MANAGED: LeafKind = LeafKind(0x1514);
/// The kind of a type server reference with a GUID (`LF_TYPESERVER2`).
pub const LF_TYPESERVER2: LeafKind = LeafKind(0x1515);
/// The kind of a strided array (`LF_STRIDED_ARRAY`).
pub const LF_STRIDED_ARRAY: LeafKind = LeafKind(0x1516);
/// The kind of an HLSL built-in type (`LF_HLSL`).
pub const LF_HLSL: LeafKind = LeafKind(0x1517);
/// The kind of a type with a modifier list (`LF_MODIFIER_EX`).
pub const LF_MODIFIER_EX: LeafKind = LeafKind(0x1518);
/// The kind of an interface type (`LF_INTERFACE`).
pub const LF_INTERFACE: LeafKind = LeafKind(0x1519);
/// The kind of a vector type (`LF_VECTOR`).
pub const LF_VECTOR: LeafKind = LeafKind(0x151b);
/// The kind of a matrix type (`LF_MATRIX`).
pub const LF_MATRIX: LeafKind = LeafKind(0x151c);
/// The kind of a virtual function table with method names (`LF_VFTABLE`).
pub const LF_VFTABLE: LeafKind = LeafKind(0x151d);
/// The kind of a class with a 32-bit property word (`LF_CLASS2`).
pub const LF_CLASS2: LeafKind = LeafKind(0x1608);
/// The kind of a structure with a 32-bit property word (`LF_STRUCTURE2`).
pub const LF_STRUCTURE2: LeafKind = LeafKind(0x1609);
/// The kind of a union with a 32-bit property word (`LF_UNION2`).
pub const LF_UNION2: LeafKind = LeafKind(0x160a);
/// The kind of an interface with a 32-bit property word (`LF_INTERFACE2`).
pub const LF_INTERFACE2: LeafKind = LeafKind(0x160b);

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
    /// `LF_CLASS2`, `LF_STRUCTURE2` or `LF_INTERFACE2`.
    WideClass(WideClass<'a>),
    /// `LF_UNION`.
    Union(Union<'a>),
    /// `LF_UNION2`.
    WideUnion(WideUnion<'a>),
    /// `LF_ENUM`.
    Enum(Enum<'a>),
    /// `LF_VTSHAPE`.
    VtableShape(VtableShape<'a>),
    /// `LF_VFTABLE`.
    VirtualFunctionTable(VirtualFunctionTable<'a>),
    /// `LF_LABEL`: the addressing mode of a code label.
    Label(LabelMode),
    /// `LF_COBOL0`.
    Cobol0(Cobol0<'a>),
    /// `LF_BARRAY`: a basic array of the element type.
    BasicArray(TypeIndex),
    /// `LF_VFTPATH`: the base classes from the root to the leaf.
    VirtualFunctionTablePath(TypeIndexList<'a>),
    /// `LF_OEM`.
    Oem(Oem<'a>),
    /// `LF_OEM2`.
    Oem2(Oem2<'a>),
    /// `LF_SKIP`.
    Skip(Skip<'a>),
    /// `LF_DERIVED`: the classes that derive from a class.
    DerivedClasses(TypeIndexList<'a>),
    /// `LF_DIMCONU` or `LF_DIMCONLU`.
    ConstantDimensions(ConstantDimensions<'a>),
    /// `LF_DIMVARU` or `LF_DIMVARLU`.
    VariableDimensions(VariableDimensions<'a>),
    /// `LF_TYPESERVER`.
    TypeServer(TypeServer<'a>),
    /// `LF_TYPESERVER2`.
    TypeServer2(TypeServer2<'a>),
    /// `LF_DIMARRAY`.
    DimensionedArray(DimensionedArray<'a>),
    /// `LF_PRECOMP`.
    Precompiled(Precompiled<'a>),
    /// `LF_ENDPRECOMP`: the signature of the precompiled types.
    EndPrecompiled(u32),
    /// `LF_ALIAS`.
    Alias(Alias<'a>),
    /// `LF_DEFARG`.
    DefaultArgument(DefaultArgument<'a>),
    /// `LF_MANAGED`: the UTF-8 name of a managed type.
    Managed(&'a [u8]),
    /// `LF_STRIDED_ARRAY`.
    StridedArray(StridedArray<'a>),
    /// `LF_HLSL`.
    Hlsl(Hlsl<'a>),
    /// `LF_MODIFIER_EX`.
    ModifierEx(ModifierEx<'a>),
    /// `LF_VECTOR`.
    Vector(Vector<'a>),
    /// `LF_MATRIX`.
    Matrix(Matrix<'a>),
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
    ///   are not valid padding. UDT records also accept the MASM form of
    ///   [`TypeLeaf::trailing`].
    /// - [`ErrorKind::UnsupportedVersion`] for a numeric leaf with an unknown
    ///   tag, for a field-list member kind without a decoder, or for constant
    ///   dimension bounds of a type without a known size.
    pub fn decode(record: Record<'a>) -> Result<Self> {
        let data = record.data;
        Ok(match LeafKind(record.kind) {
            LF_MODIFIER => Self::Modifier(Modifier::decode(data)?),
            LF_POINTER => Self::Pointer(Pointer::decode(data)?),
            LF_PROCEDURE => Self::Procedure(Procedure::decode(data)?),
            LF_MFUNCTION => Self::MemberFunction(MemberFunction::decode(data)?),
            LF_ARGLIST => Self::ArgumentList(ArgumentList::decode(data)?),
            LF_FIELDLIST => Self::FieldList(FieldList::parse(data)?),
            LF_BITFIELD => Self::Bitfield(Bitfield::decode(data)?),
            LF_METHODLIST => Self::MethodList(MethodList::parse(data)?),
            LF_ARRAY => Self::Array(Array::decode(data)?),
            LF_CLASS => Self::Class(Class::decode(ClassKind::Class, data)?),
            LF_STRUCTURE => Self::Class(Class::decode(ClassKind::Structure, data)?),
            LF_INTERFACE => Self::Class(Class::decode(ClassKind::Interface, data)?),
            LF_CLASS2 => Self::WideClass(WideClass::decode(ClassKind::Class, data)?),
            LF_STRUCTURE2 => Self::WideClass(WideClass::decode(ClassKind::Structure, data)?),
            LF_INTERFACE2 => Self::WideClass(WideClass::decode(ClassKind::Interface, data)?),
            LF_UNION => Self::Union(Union::decode(data)?),
            LF_UNION2 => Self::WideUnion(WideUnion::decode(data)?),
            LF_ENUM => Self::Enum(Enum::decode(data)?),
            LF_VTSHAPE => Self::VtableShape(VtableShape::decode(data)?),
            LF_VFTABLE => Self::VirtualFunctionTable(VirtualFunctionTable::decode(data)?),
            LF_LABEL => Self::Label(decode_label(data)?),
            _ => decode_other(record)?,
        })
    }

    /// Return the leaf kind of the record.
    #[must_use]
    pub const fn kind(&self) -> LeafKind {
        match self {
            Self::Modifier(_) => LF_MODIFIER,
            Self::Pointer(_) => LF_POINTER,
            Self::Procedure(_) => LF_PROCEDURE,
            Self::MemberFunction(_) => LF_MFUNCTION,
            Self::ArgumentList(_) => LF_ARGLIST,
            Self::FieldList(_) => LF_FIELDLIST,
            Self::Bitfield(_) => LF_BITFIELD,
            Self::MethodList(_) => LF_METHODLIST,
            Self::Array(_) => LF_ARRAY,
            Self::Class(class) => match class.kind {
                ClassKind::Class => LF_CLASS,
                ClassKind::Structure => LF_STRUCTURE,
                ClassKind::Interface => LF_INTERFACE,
            },
            Self::WideClass(class) => match class.kind {
                ClassKind::Class => LF_CLASS2,
                ClassKind::Structure => LF_STRUCTURE2,
                ClassKind::Interface => LF_INTERFACE2,
            },
            Self::Union(_) => LF_UNION,
            Self::WideUnion(_) => LF_UNION2,
            Self::Enum(_) => LF_ENUM,
            Self::VtableShape(_) => LF_VTSHAPE,
            Self::VirtualFunctionTable(_) => LF_VFTABLE,
            Self::Label(_) => LF_LABEL,
            Self::Cobol0(_) => LF_COBOL0,
            Self::BasicArray(_) => LF_BARRAY,
            Self::VirtualFunctionTablePath(_) => LF_VFTPATH,
            Self::Oem(_) => LF_OEM,
            Self::Oem2(_) => LF_OEM2,
            Self::Skip(_) => LF_SKIP,
            Self::DerivedClasses(_) => LF_DERIVED,
            Self::ConstantDimensions(value) => {
                if value.lower_bounds {
                    LF_DIMCONLU
                } else {
                    LF_DIMCONU
                }
            }
            Self::VariableDimensions(value) => {
                if value.lower_bounds {
                    LF_DIMVARLU
                } else {
                    LF_DIMVARU
                }
            }
            Self::TypeServer(_) => LF_TYPESERVER,
            Self::TypeServer2(_) => LF_TYPESERVER2,
            Self::DimensionedArray(_) => LF_DIMARRAY,
            Self::Precompiled(_) => LF_PRECOMP,
            Self::EndPrecompiled(_) => LF_ENDPRECOMP,
            Self::Alias(_) => LF_ALIAS,
            Self::DefaultArgument(_) => LF_DEFARG,
            Self::Managed(_) => LF_MANAGED,
            Self::StridedArray(_) => LF_STRIDED_ARRAY,
            Self::Hlsl(_) => LF_HLSL,
            Self::ModifierEx(_) => LF_MODIFIER_EX,
            Self::Vector(_) => LF_VECTOR,
            Self::Matrix(_) => LF_MATRIX,
            Self::Unsupported(record) => LeafKind(record.kind),
        }
    }

    /// Append the complete record to `out`: the length field, the kind, the
    /// payload and the `LF_PAD` padding to a multiple of 4 bytes.
    ///
    /// For a decoded record of a stream that pads its records, the output is
    /// the input record. [`TypeLeaf::Unsupported`] and the records with
    /// producer-defined data ([`TypeLeaf::Oem`], [`TypeLeaf::Oem2`],
    /// [`TypeLeaf::Cobol0`] and [`TypeLeaf::Skip`]) keep their payload bytes
    /// and get no padding.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`] if `out` cannot grow, if the record is
    ///   longer than its `u16` length field allows, or if a list is longer
    ///   than its count field allows. `out` keeps its previous bytes for a
    ///   record that is too long.
    /// - [`ErrorKind::InvalidFormat`] if a name contains a NUL byte, or if
    ///   fields do not match each other, for example a unique name without
    ///   the unique-name flag, or trailing bytes outside the MASM form or
    ///   that does not end the record on a 4-byte boundary.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut writer = Output::new(out);
        let start = writer.begin_record(self.kind())?;
        let padding = self.encode_payload(&mut writer)?;
        writer.finish_record(start, padding)
    }

    /// Append the payload fields and return the padding rule of the record.
    fn encode_payload(&self, writer: &mut Output<'_>) -> Result<Padding> {
        match self {
            Self::Modifier(value) => value.encode(writer)?,
            Self::Pointer(value) => value.encode(writer)?,
            Self::Procedure(value) => value.encode(writer)?,
            Self::MemberFunction(value) => value.encode(writer)?,
            Self::ArgumentList(value) => value.encode(writer)?,
            Self::FieldList(value) => value.encode_members(writer)?,
            Self::Bitfield(value) => value.encode(writer)?,
            Self::MethodList(value) => value.encode(writer)?,
            Self::Array(value) => value.encode(writer)?,
            Self::Class(value) => return value.encode(writer),
            Self::WideClass(value) => return value.encode(writer),
            Self::Union(value) => return value.encode(writer),
            Self::WideUnion(value) => return value.encode(writer),
            Self::Enum(value) => return value.encode(writer),
            Self::VtableShape(value) => value.encode(writer)?,
            Self::VirtualFunctionTable(value) => value.encode(writer)?,
            Self::Label(mode) => writer.u16(mode.raw())?,
            Self::BasicArray(index) => writer.type_index(*index)?,
            Self::VirtualFunctionTablePath(list) | Self::DerivedClasses(list) => {
                list.encode_counted(writer)?;
            }
            Self::ConstantDimensions(value) => value.encode(writer)?,
            Self::VariableDimensions(value) => value.encode(writer)?,
            Self::TypeServer(value) => value.encode(writer)?,
            Self::TypeServer2(value) => value.encode(writer)?,
            Self::DimensionedArray(value) => value.encode(writer)?,
            Self::Precompiled(value) => value.encode(writer)?,
            Self::EndPrecompiled(signature) => writer.u32(*signature)?,
            Self::Alias(value) => value.encode(writer)?,
            Self::DefaultArgument(value) => value.encode(writer)?,
            Self::Managed(name) => writer.c_string(name)?,
            Self::StridedArray(value) => value.encode(writer)?,
            Self::Hlsl(value) => value.encode(writer)?,
            Self::ModifierEx(value) => value.encode(writer)?,
            Self::Vector(value) => value.encode(writer)?,
            Self::Matrix(value) => value.encode(writer)?,
            Self::Cobol0(value) => {
                value.encode(writer)?;
                return Ok(Padding::None);
            }
            Self::Oem(value) => {
                value.encode(writer)?;
                return Ok(Padding::None);
            }
            Self::Oem2(value) => {
                value.encode(writer)?;
                return Ok(Padding::None);
            }
            Self::Skip(value) => {
                value.encode(writer)?;
                return Ok(Padding::None);
            }
            Self::Unsupported(record) => {
                writer.bytes(record.data)?;
                return Ok(Padding::None);
            }
        }
        Ok(Padding::Type)
    }

    /// Return the bytes after the last field of a UDT record if they are not the standard
    /// `LF_PAD` run, or an empty slice.
    ///
    /// Microsoft Macro Assembler writes these bytes after some `LF_STRUCTURE` names, for
    /// example `f1 6d 65`. Other kinds always give an empty slice.
    #[must_use]
    pub const fn trailing(&self) -> &'a [u8] {
        match self {
            Self::Class(value) => value.trailing,
            Self::WideClass(value) => value.trailing,
            Self::Union(value) => value.trailing,
            Self::WideUnion(value) => value.trailing,
            Self::Enum(value) => value.trailing,
            _ => &[],
        }
    }

    /// Return the options, the name and the unique name of a user-defined type.
    ///
    /// Returns `None` if the record is not a class, a structure, an
    /// interface, a union or an enumeration, in the normal or the wide form.
    #[must_use]
    pub const fn tag(&self) -> Option<Tag<'a>> {
        match self {
            Self::Class(value) => Some(value.tag()),
            Self::WideClass(value) => Some(value.tag()),
            Self::Union(value) => Some(value.tag()),
            Self::WideUnion(value) => Some(value.tag()),
            Self::Enum(value) => Some(value.tag()),
            _ => None,
        }
    }
}

/// Decode a record of a kind that is not in the common dispatch.
fn decode_other(record: Record<'_>) -> Result<TypeLeaf<'_>> {
    let data = record.data;
    Ok(match LeafKind(record.kind) {
        LF_COBOL0 => TypeLeaf::Cobol0(Cobol0::decode(data)?),
        LF_BARRAY => TypeLeaf::BasicArray(single_index(data)?),
        LF_VFTPATH => TypeLeaf::VirtualFunctionTablePath(counted_indices(data)?),
        LF_OEM => TypeLeaf::Oem(Oem::decode(data)?),
        LF_OEM2 => TypeLeaf::Oem2(Oem2::decode(data)?),
        LF_SKIP => TypeLeaf::Skip(Skip::decode(data)?),
        LF_DERIVED => TypeLeaf::DerivedClasses(counted_indices(data)?),
        LF_DIMCONU => TypeLeaf::ConstantDimensions(ConstantDimensions::decode(false, data)?),
        LF_DIMCONLU => TypeLeaf::ConstantDimensions(ConstantDimensions::decode(true, data)?),
        LF_DIMVARU => TypeLeaf::VariableDimensions(VariableDimensions::decode(false, data)?),
        LF_DIMVARLU => TypeLeaf::VariableDimensions(VariableDimensions::decode(true, data)?),
        LF_TYPESERVER => TypeLeaf::TypeServer(TypeServer::decode(data)?),
        LF_TYPESERVER2 => TypeLeaf::TypeServer2(TypeServer2::decode(data)?),
        LF_DIMARRAY => TypeLeaf::DimensionedArray(DimensionedArray::decode(data)?),
        LF_PRECOMP => TypeLeaf::Precompiled(Precompiled::decode(data)?),
        LF_ENDPRECOMP => TypeLeaf::EndPrecompiled(server::decode_end_precompiled(data)?),
        LF_ALIAS => TypeLeaf::Alias(Alias::decode(data)?),
        LF_DEFARG => TypeLeaf::DefaultArgument(DefaultArgument::decode(data)?),
        LF_MANAGED => TypeLeaf::Managed(alias::decode_managed(data)?),
        LF_STRIDED_ARRAY => TypeLeaf::StridedArray(StridedArray::decode(data)?),
        LF_HLSL => TypeLeaf::Hlsl(Hlsl::decode(data)?),
        LF_MODIFIER_EX => TypeLeaf::ModifierEx(ModifierEx::decode(data)?),
        LF_VECTOR => TypeLeaf::Vector(Vector::decode(data)?),
        LF_MATRIX => TypeLeaf::Matrix(Matrix::decode(data)?),
        _ => TypeLeaf::Unsupported(record),
    })
}

/// Decode a payload with one type index.
fn single_index(data: &[u8]) -> Result<TypeIndex> {
    let mut fields = fields(data);
    let index = fields.type_index()?;
    fields.finish_type_padding()?;
    Ok(index)
}

/// Decode a payload with a `u32` count and that many type indices.
fn counted_indices(data: &[u8]) -> Result<TypeIndexList<'_>> {
    let mut fields = fields(data);
    let list = TypeIndexList::decode_counted(&mut fields)?;
    fields.finish_type_padding()?;
    Ok(list)
}

/// Decode the payload of an `LF_LABEL` record: a `u16` addressing mode.
fn decode_label(data: &[u8]) -> Result<LabelMode> {
    let mut fields = fields(data);
    let mode = LabelMode::from_raw(fields.u16()?)?;
    fields.finish_type_padding()?;
    Ok(mode)
}
