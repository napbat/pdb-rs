//! The type indices that a type record refers to.
//!
//! [`TypeLeaf::for_each_reference`] visits each type index of a record with
//! its role. A role names the record kinds that the index can refer to.
//! [`crate::native::types::validate_type_references`] uses the roles to
//! check the references of a TPI stream.

use super::{
    LF_ARGLIST, LF_DIMCONLU, LF_DIMCONU, LF_DIMVARLU, LF_DIMVARU, LF_FIELDLIST, LF_METHODLIST,
    LF_MFUNCTION, LF_VTSHAPE, TypeLeaf,
};
use crate::Result;
use crate::native::codeview::field_list::Member;
use crate::native::codeview::index::TypeIndex;

/// The role of a type index in a type record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ReferenceRole {
    /// Any type: a primitive type or a record.
    Type,
    /// The field list of a user-defined type: an `LF_FIELDLIST` record, or
    /// [`TypeIndex::NONE`] for a forward reference.
    FieldList,
    /// The continuation of a field list: an `LF_FIELDLIST` record.
    Continuation,
    /// The parameter list of a procedure type: an `LF_ARGLIST` record.
    ArgumentList,
    /// The overloads of a method: an `LF_METHODLIST` record.
    MethodList,
    /// The type of a method: an `LF_MFUNCTION` record.
    MethodType,
    /// The virtual table shape of a class: an `LF_VTSHAPE` record, or
    /// [`TypeIndex::NONE`].
    VtableShape,
    /// The dimensions of an `LF_DIMARRAY` record: an `LF_DIMCONU`,
    /// `LF_DIMCONLU`, `LF_DIMVARU` or `LF_DIMVARLU` record.
    Dimensions,
}

impl ReferenceRole {
    /// Return `true` if [`TypeIndex::NONE`] is a valid value for the role.
    #[must_use]
    pub const fn allows_none(self) -> bool {
        matches!(self, Self::Type | Self::FieldList | Self::VtableShape)
    }

    /// Return the record kinds that a record reference of this role can
    /// refer to, or `None` for any kind.
    #[must_use]
    pub const fn kinds(self) -> Option<&'static [u16]> {
        Some(match self {
            Self::Type => return None,
            Self::FieldList | Self::Continuation => &[LF_FIELDLIST],
            Self::ArgumentList => &[LF_ARGLIST],
            Self::MethodList => &[LF_METHODLIST],
            Self::MethodType => &[LF_MFUNCTION],
            Self::VtableShape => &[LF_VTSHAPE],
            Self::Dimensions => &[LF_DIMCONU, LF_DIMCONLU, LF_DIMVARU, LF_DIMVARLU],
        })
    }
}

/// One type index in a type record, with its role.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TypeReference {
    /// The type index.
    pub index: TypeIndex,
    /// The role of the index.
    pub role: ReferenceRole,
}

/// Visit a list of type indices with one role.
fn each<F>(
    indices: impl Iterator<Item = TypeIndex>,
    role: ReferenceRole,
    visit: &mut F,
) -> Result<()>
where
    F: FnMut(TypeReference) -> Result<()>,
{
    for index in indices {
        visit(TypeReference { index, role })?;
    }
    Ok(())
}

impl TypeLeaf<'_> {
    /// Call `visit` for each type index of the record, in field order.
    ///
    /// A field list gives the indices of its members, including an
    /// `LF_INDEX` continuation. The function does not follow references.
    /// [`TypeLeaf::Unsupported`] has no known references.
    ///
    /// # Errors
    ///
    /// Returns the first error of `visit`.
    pub fn for_each_reference<F>(&self, mut visit: F) -> Result<()>
    where
        F: FnMut(TypeReference) -> Result<()>,
    {
        let visit = &mut visit;
        let mut one = |index, role| visit(TypeReference { index, role });
        match self {
            Self::Modifier(value) => one(value.modified_type, ReferenceRole::Type),
            Self::Pointer(value) => {
                one(value.referent_type, ReferenceRole::Type)?;
                if let Some(member) = value.member {
                    one(member.containing_type, ReferenceRole::Type)?;
                }
                match value.based {
                    Some(super::BasedPointer::Type { base_type, .. }) => {
                        one(base_type, ReferenceRole::Type)
                    }
                    _ => Ok(()),
                }
            }
            Self::Procedure(value) => {
                one(value.return_type, ReferenceRole::Type)?;
                one(value.argument_list, ReferenceRole::ArgumentList)
            }
            Self::MemberFunction(value) => {
                one(value.return_type, ReferenceRole::Type)?;
                one(value.class_type, ReferenceRole::Type)?;
                one(value.this_type, ReferenceRole::Type)?;
                one(value.argument_list, ReferenceRole::ArgumentList)
            }
            Self::ArgumentList(list) => each(list.iter(), ReferenceRole::Type, visit),
            Self::VirtualFunctionTablePath(list) | Self::DerivedClasses(list) => {
                each(list.iter(), ReferenceRole::Type, visit)
            }
            Self::FieldList(list) => {
                for member in list.members() {
                    member?.for_each_reference(&mut *visit)?;
                }
                Ok(())
            }
            Self::Bitfield(value) => one(value.underlying_type, ReferenceRole::Type),
            Self::MethodList(list) => {
                for entry in list.entries() {
                    one(entry?.method_type, ReferenceRole::MethodType)?;
                }
                Ok(())
            }
            Self::Array(value) => {
                one(value.element_type, ReferenceRole::Type)?;
                one(value.index_type, ReferenceRole::Type)
            }
            Self::Class(value) => {
                one(value.field_list, ReferenceRole::FieldList)?;
                one(value.derivation_list, ReferenceRole::Type)?;
                one(value.vtable_shape, ReferenceRole::VtableShape)
            }
            Self::WideClass(value) => {
                one(value.field_list, ReferenceRole::FieldList)?;
                one(value.derivation_list, ReferenceRole::Type)?;
                one(value.vtable_shape, ReferenceRole::VtableShape)
            }
            Self::Union(value) => one(value.field_list, ReferenceRole::FieldList),
            Self::WideUnion(value) => one(value.field_list, ReferenceRole::FieldList),
            Self::Enum(value) => {
                one(value.underlying_type, ReferenceRole::Type)?;
                one(value.field_list, ReferenceRole::FieldList)
            }
            Self::VirtualFunctionTable(value) => {
                one(value.complete_class, ReferenceRole::Type)?;
                match value.overridden_table {
                    Some(table) => one(table, ReferenceRole::Type),
                    None => Ok(()),
                }
            }
            Self::Cobol0(value) => one(value.parent_type, ReferenceRole::Type),
            Self::BasicArray(index) => one(*index, ReferenceRole::Type),
            Self::Oem(value) => each(value.indices.iter(), ReferenceRole::Type, visit),
            Self::Oem2(value) => each(value.indices.iter(), ReferenceRole::Type, visit),
            Self::ConstantDimensions(value) => one(value.index_type, ReferenceRole::Type),
            Self::VariableDimensions(value) => {
                one(value.index_type, ReferenceRole::Type)?;
                each(value.bounds.iter(), ReferenceRole::Type, visit)
            }
            Self::DimensionedArray(value) => {
                one(value.element_type, ReferenceRole::Type)?;
                one(value.dimensions, ReferenceRole::Dimensions)
            }
            Self::Alias(value) => one(value.underlying_type, ReferenceRole::Type),
            Self::DefaultArgument(value) => one(value.argument_type, ReferenceRole::Type),
            Self::StridedArray(value) => {
                one(value.element_type, ReferenceRole::Type)?;
                one(value.index_type, ReferenceRole::Type)
            }
            Self::Hlsl(value) => one(value.subtype, ReferenceRole::Type),
            Self::ModifierEx(value) => one(value.modified_type, ReferenceRole::Type),
            Self::Vector(value) => one(value.element_type, ReferenceRole::Type),
            Self::Matrix(value) => one(value.element_type, ReferenceRole::Type),
            Self::VtableShape(_)
            | Self::Label(_)
            | Self::Skip(_)
            | Self::TypeServer(_)
            | Self::TypeServer2(_)
            | Self::Precompiled(_)
            | Self::EndPrecompiled(_)
            | Self::Managed(_)
            | Self::Unsupported(_) => Ok(()),
        }
    }
}

impl Member<'_> {
    /// Call `visit` for each type index of the member, in field order.
    ///
    /// # Errors
    ///
    /// Returns the first error of `visit`.
    pub fn for_each_reference<F>(&self, mut visit: F) -> Result<()>
    where
        F: FnMut(TypeReference) -> Result<()>,
    {
        let mut one = |index, role| visit(TypeReference { index, role });
        match self {
            Self::BaseClass(base) | Self::BaseInterface(base) => {
                one(base.base_type, ReferenceRole::Type)
            }
            Self::VirtualBaseClass(base) => {
                one(base.base_type, ReferenceRole::Type)?;
                one(base.vbptr_type, ReferenceRole::Type)
            }
            Self::Continuation(index) => one(*index, ReferenceRole::Continuation),
            Self::VirtualFunctionTable(index) | Self::FriendClass(index) => {
                one(*index, ReferenceRole::Type)
            }
            Self::VirtualFunctionOffset(value) => one(value.pointer_type, ReferenceRole::Type),
            Self::FriendFunction(value) => one(value.function_type, ReferenceRole::Type),
            Self::Enumerator(_) => Ok(()),
            Self::DataMember(value) => one(value.member_type, ReferenceRole::Type),
            Self::StaticMember(value) => one(value.member_type, ReferenceRole::Type),
            Self::OverloadedMethod(value) => one(value.method_list, ReferenceRole::MethodList),
            Self::NestedType(value) => one(value.nested_type, ReferenceRole::Type),
            Self::NestedTypeEx(value) => one(value.nested_type, ReferenceRole::Type),
            Self::MemberModify(value) => one(value.base_type, ReferenceRole::Type),
            Self::OneMethod(value) => one(value.method_type, ReferenceRole::MethodType),
        }
    }
}
