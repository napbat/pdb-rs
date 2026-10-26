//! Members of `LF_FIELDLIST` type records.
//!
//! A field list is a sequence of member records without length fields. Each
//! member starts with a `u16` kind. The kind sets the layout, so a decoder
//! must know every kind in the list. An unknown kind is an error.
//!
//! `LF_PAD` bytes (`0xf0` to `0xff`) can follow a member. The low 4 bits of
//! the first padding byte give the number of padding bytes, and each next
//! byte counts down to `0xf1` (`CodeViewRecordIO::skipPadding` in LLVM).
//!
//! A long field list continues in another `LF_FIELDLIST` record. The last
//! member is then an `LF_INDEX` member with the index of that record.
//! [`encode_field_list`] splits a long member sequence in this way.
//! The layouts follow `TypeRecordMapping.cpp` in LLVM (revision `b9f2235f`)
//! and `cvinfo.h`.

use core::iter::FusedIterator;

use super::Fields;
use super::index::TypeIndex;
use super::numeric::Numeric;
use super::type_record::MemberAttributes;
use crate::{Error, ErrorKind, Result};

mod encode;

pub use encode::{FieldListRecords, LLVM_FIELD_LIST_LIMIT, encode_field_list};

/// The kind of a base class member (`LF_BCLASS`).
pub const LF_BCLASS: u16 = 0x1400;
/// The kind of a direct virtual base class member (`LF_VBCLASS`).
pub const LF_VBCLASS: u16 = 0x1401;
/// The kind of an indirect virtual base class member (`LF_IVBCLASS`).
pub const LF_IVBCLASS: u16 = 0x1402;
/// The kind of a field-list continuation member (`LF_INDEX`).
pub const LF_INDEX: u16 = 0x1404;
/// The kind of a virtual function table pointer member (`LF_VFUNCTAB`).
pub const LF_VFUNCTAB: u16 = 0x1409;
/// The kind of a friend class member (`LF_FRIENDCLS`).
pub const LF_FRIENDCLS: u16 = 0x140a;
/// The kind of a virtual function offset member (`LF_VFUNCOFF`).
pub const LF_VFUNCOFF: u16 = 0x140c;
/// The kind of an enumerator member (`LF_ENUMERATE`).
pub const LF_ENUMERATE: u16 = 0x1502;
/// The kind of a friend function member (`LF_FRIENDFCN`).
pub const LF_FRIENDFCN: u16 = 0x150c;
/// The kind of a data member (`LF_MEMBER`).
pub const LF_MEMBER: u16 = 0x150d;
/// The kind of a static data member (`LF_STMEMBER`).
pub const LF_STMEMBER: u16 = 0x150e;
/// The kind of an overloaded method member (`LF_METHOD`).
pub const LF_METHOD: u16 = 0x150f;
/// The kind of a nested type member (`LF_NESTTYPE`).
pub const LF_NESTTYPE: u16 = 0x1510;
/// The kind of a single method member (`LF_ONEMETHOD`).
pub const LF_ONEMETHOD: u16 = 0x1511;
/// The kind of a nested type member with attributes (`LF_NESTTYPEEX`).
pub const LF_NESTTYPEEX: u16 = 0x1512;
/// The kind of a member access change (`LF_MEMBERMODIFY`).
pub const LF_MEMBERMODIFY: u16 = 0x1513;
/// The kind of a base interface member (`LF_BINTERFACE`).
pub const LF_BINTERFACE: u16 = 0x151a;

/// The smallest `LF_PAD` byte.
const LF_PAD0: u8 = 0xf0;

/// Return an invalid-format error with `context`.
fn invalid(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, None).with_context(context)
}

/// An `LF_BCLASS` or `LF_BINTERFACE` member: a non-virtual base class or a
/// base interface.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BaseClass<'a> {
    /// The access of the base class.
    pub attributes: MemberAttributes,
    /// The base class type.
    pub base_type: TypeIndex,
    /// The offset of the base class in the derived class.
    pub offset: Numeric<'a>,
}

/// An `LF_VBCLASS` or `LF_IVBCLASS` member: a virtual base class.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VirtualBaseClass<'a> {
    /// `true` for `LF_IVBCLASS`, `false` for `LF_VBCLASS`.
    pub indirect: bool,
    /// The access of the base class.
    pub attributes: MemberAttributes,
    /// The base class type.
    pub base_type: TypeIndex,
    /// The type of the virtual base pointer.
    pub vbptr_type: TypeIndex,
    /// The offset of the virtual base pointer from the address point.
    pub vbptr_offset: Numeric<'a>,
    /// The index of the base class in the virtual base table.
    pub vbtable_index: Numeric<'a>,
}

/// An `LF_ENUMERATE` member: one enumerator.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Enumerator<'a> {
    /// The access of the enumerator.
    pub attributes: MemberAttributes,
    /// The value.
    pub value: Numeric<'a>,
    /// The name bytes.
    pub name: &'a [u8],
}

/// An `LF_MEMBER` member: a non-static data member.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DataMember<'a> {
    /// The access of the member.
    pub attributes: MemberAttributes,
    /// The member type.
    pub member_type: TypeIndex,
    /// The offset of the member in the type, in bytes.
    pub offset: Numeric<'a>,
    /// The name bytes.
    pub name: &'a [u8],
}

/// An `LF_STMEMBER` member: a static data member.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StaticMember<'a> {
    /// The access of the member.
    pub attributes: MemberAttributes,
    /// The member type.
    pub member_type: TypeIndex,
    /// The name bytes.
    pub name: &'a [u8],
}

/// An `LF_METHOD` member: a method name with an overload list.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OverloadedMethod<'a> {
    /// The number of overloads.
    pub overload_count: u16,
    /// The `LF_METHODLIST` record with the overloads.
    pub method_list: TypeIndex,
    /// The name bytes.
    pub name: &'a [u8],
}

/// An `LF_NESTTYPE` member: a nested type.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NestedType<'a> {
    /// The nested type.
    pub nested_type: TypeIndex,
    /// The name bytes.
    pub name: &'a [u8],
}

/// An `LF_NESTTYPEEX` member: a nested type with member attributes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NestedTypeEx<'a> {
    /// The access of the nested type.
    pub attributes: MemberAttributes,
    /// The nested type.
    pub nested_type: TypeIndex,
    /// The name bytes.
    pub name: &'a [u8],
}

/// An `LF_MEMBERMODIFY` member: new attributes of an inherited member.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MemberModify<'a> {
    /// The new attributes.
    pub attributes: MemberAttributes,
    /// The base class that declares the member.
    pub base_type: TypeIndex,
    /// The member name bytes.
    pub name: &'a [u8],
}

/// An `LF_FRIENDFCN` member: a friend function.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FriendFunction<'a> {
    /// The type of the friend function.
    pub function_type: TypeIndex,
    /// The name bytes.
    pub name: &'a [u8],
}

/// An `LF_VFUNCOFF` member: a virtual function table pointer at an offset.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VirtualFunctionOffset {
    /// The type of the table pointer.
    pub pointer_type: TypeIndex,
    /// The offset of the table pointer.
    pub offset: i32,
}

/// An `LF_ONEMETHOD` member: a method without overloads.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OneMethod<'a> {
    /// The access and the method properties.
    pub attributes: MemberAttributes,
    /// The `LF_MFUNCTION` record of the method.
    pub method_type: TypeIndex,
    /// The offset of the method in the virtual function table.
    ///
    /// This value is present only if the method introduces a virtual slot.
    pub vtable_offset: Option<i32>,
    /// The name bytes.
    pub name: &'a [u8],
}

/// One decoded field-list member.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Member<'a> {
    /// `LF_BCLASS`.
    BaseClass(BaseClass<'a>),
    /// `LF_BINTERFACE`.
    BaseInterface(BaseClass<'a>),
    /// `LF_VBCLASS` or `LF_IVBCLASS`.
    VirtualBaseClass(VirtualBaseClass<'a>),
    /// `LF_INDEX`: the list continues in the `LF_FIELDLIST` record at this index.
    Continuation(TypeIndex),
    /// `LF_VFUNCTAB`: a virtual function table pointer with this pointer type.
    VirtualFunctionTable(TypeIndex),
    /// `LF_VFUNCOFF`.
    VirtualFunctionOffset(VirtualFunctionOffset),
    /// `LF_FRIENDCLS`: a friend class.
    FriendClass(TypeIndex),
    /// `LF_FRIENDFCN`.
    FriendFunction(FriendFunction<'a>),
    /// `LF_ENUMERATE`.
    Enumerator(Enumerator<'a>),
    /// `LF_MEMBER`.
    DataMember(DataMember<'a>),
    /// `LF_STMEMBER`.
    StaticMember(StaticMember<'a>),
    /// `LF_METHOD`.
    OverloadedMethod(OverloadedMethod<'a>),
    /// `LF_NESTTYPE`.
    NestedType(NestedType<'a>),
    /// `LF_NESTTYPEEX`.
    NestedTypeEx(NestedTypeEx<'a>),
    /// `LF_MEMBERMODIFY`.
    MemberModify(MemberModify<'a>),
    /// `LF_ONEMETHOD`.
    OneMethod(OneMethod<'a>),
}

/// The payload of an `LF_FIELDLIST` record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FieldList<'a> {
    /// The serialized members and their padding.
    data: &'a [u8],
    /// The index in the last `LF_INDEX` member, if present.
    continuation: Option<TypeIndex>,
}

impl<'a> FieldList<'a> {
    /// Decode and validate the payload of an `LF_FIELDLIST` record.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if a member is truncated, if a name has
    ///   no terminator, if padding or a reserved field is not valid, or if a
    ///   member follows an `LF_INDEX` member.
    /// - [`ErrorKind::UnsupportedVersion`] for a member kind without a
    ///   decoder or a numeric leaf with an unknown tag.
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        let mut continuation = None;
        for member in Members::new(data) {
            if let Member::Continuation(index) = member? {
                continuation = Some(index);
            }
        }
        Ok(Self { data, continuation })
    }

    /// Return the raw payload.
    #[must_use]
    pub const fn data(&self) -> &'a [u8] {
        self.data
    }

    /// Return the index in the final `LF_INDEX` member, if present.
    #[must_use]
    pub const fn continuation(&self) -> Option<TypeIndex> {
        self.continuation
    }

    /// Return an iterator over the members of this record.
    ///
    /// The iterator includes an `LF_INDEX` member as [`Member::Continuation`].
    /// It does not follow the continuation. [`FieldList::decode`] validates
    /// all members, so the iterator does not give errors for a decoded list.
    #[must_use]
    pub const fn members(&self) -> Members<'a> {
        Members::new(self.data)
    }
}

/// An iterator over the members of one `LF_FIELDLIST` record.
#[derive(Clone, Debug)]
pub struct Members<'a> {
    /// The bytes that are not read yet.
    data: &'a [u8],
    /// `true` after an `LF_INDEX` member.
    continued: bool,
    /// `true` after the end or after an error.
    done: bool,
}

impl<'a> Members<'a> {
    /// Create an iterator over serialized members.
    const fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            continued: false,
            done: false,
        }
    }

    /// Decode the member at the start of the remaining bytes and its padding.
    fn read(&mut self) -> Result<Member<'a>> {
        if self.continued {
            return Err(invalid("field list has a member after its continuation"));
        }
        let mut fields = Fields::new(
            self.data,
            ErrorKind::InvalidFormat,
            "field list member is truncated",
        );
        let member = decode_member(&mut fields)?;
        self.continued = matches!(member, Member::Continuation(_));
        self.data = skip_padding(fields.rest())?;
        Ok(member)
    }
}

impl<'a> Iterator for Members<'a> {
    type Item = Result<Member<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.data.is_empty() {
            return None;
        }
        let result = self.read();
        self.done = result.is_err();
        Some(result)
    }
}

impl FusedIterator for Members<'_> {}

/// Skip the `LF_PAD` bytes at the start of `bytes`, if present.
fn skip_padding(bytes: &[u8]) -> Result<&[u8]> {
    let Some(&first) = bytes.first() else {
        return Ok(bytes);
    };
    if first < LF_PAD0 {
        return Ok(bytes);
    }
    let count = usize::from(first & 0x0f);
    let valid = count != 0
        && bytes.get(..count).is_some_and(|padding| {
            padding
                .iter()
                .zip((1..=count).rev())
                .all(|(&byte, left)| usize::from(byte) == usize::from(LF_PAD0) + left)
        });
    if valid {
        Ok(&bytes[count..])
    } else {
        Err(invalid("field list member has invalid padding"))
    }
}

/// Read a `u16` field that must be zero.
fn reserved(fields: &mut Fields<'_>) -> Result<()> {
    if fields.u16()? == 0 {
        Ok(())
    } else {
        Err(invalid("field list member padding field is not zero"))
    }
}

/// Read a member attribute word.
fn attributes(fields: &mut Fields<'_>) -> Result<MemberAttributes> {
    MemberAttributes::from_raw(fields.u16()?)
}

/// Decode one member, without the padding after it.
fn decode_member<'a>(fields: &mut Fields<'a>) -> Result<Member<'a>> {
    let kind = fields.u16()?;
    Ok(match kind {
        LF_BCLASS | LF_BINTERFACE => {
            let base = BaseClass {
                attributes: attributes(fields)?,
                base_type: fields.type_index()?,
                offset: fields.numeric()?,
            };
            if kind == LF_BCLASS {
                Member::BaseClass(base)
            } else {
                Member::BaseInterface(base)
            }
        }
        LF_VBCLASS | LF_IVBCLASS => Member::VirtualBaseClass(VirtualBaseClass {
            indirect: kind == LF_IVBCLASS,
            attributes: attributes(fields)?,
            base_type: fields.type_index()?,
            vbptr_type: fields.type_index()?,
            vbptr_offset: fields.numeric()?,
            vbtable_index: fields.numeric()?,
        }),
        LF_INDEX | LF_VFUNCTAB | LF_FRIENDCLS => {
            reserved(fields)?;
            let index = fields.type_index()?;
            match kind {
                LF_INDEX => Member::Continuation(index),
                LF_VFUNCTAB => Member::VirtualFunctionTable(index),
                _ => Member::FriendClass(index),
            }
        }
        LF_VFUNCOFF => {
            reserved(fields)?;
            Member::VirtualFunctionOffset(VirtualFunctionOffset {
                pointer_type: fields.type_index()?,
                offset: fields.i32()?,
            })
        }
        LF_FRIENDFCN => {
            reserved(fields)?;
            Member::FriendFunction(FriendFunction {
                function_type: fields.type_index()?,
                name: fields.name()?,
            })
        }
        LF_ENUMERATE => Member::Enumerator(Enumerator {
            attributes: attributes(fields)?,
            value: fields.numeric()?,
            name: fields.name()?,
        }),
        LF_MEMBER => Member::DataMember(DataMember {
            attributes: attributes(fields)?,
            member_type: fields.type_index()?,
            offset: fields.numeric()?,
            name: fields.name()?,
        }),
        LF_STMEMBER => Member::StaticMember(StaticMember {
            attributes: attributes(fields)?,
            member_type: fields.type_index()?,
            name: fields.name()?,
        }),
        LF_METHOD => Member::OverloadedMethod(OverloadedMethod {
            overload_count: fields.u16()?,
            method_list: fields.type_index()?,
            name: fields.name()?,
        }),
        LF_NESTTYPE => {
            reserved(fields)?;
            Member::NestedType(NestedType {
                nested_type: fields.type_index()?,
                name: fields.name()?,
            })
        }
        LF_NESTTYPEEX => Member::NestedTypeEx(NestedTypeEx {
            attributes: attributes(fields)?,
            nested_type: fields.type_index()?,
            name: fields.name()?,
        }),
        LF_MEMBERMODIFY => Member::MemberModify(MemberModify {
            attributes: attributes(fields)?,
            base_type: fields.type_index()?,
            name: fields.name()?,
        }),
        LF_ONEMETHOD => Member::OneMethod(one_method(fields)?),
        _ => {
            return Err(Error::new(ErrorKind::UnsupportedVersion, None)
                .with_context("field list member kind has no decoder"));
        }
    })
}

/// Decode the fields of an `LF_ONEMETHOD` member after its kind.
fn one_method<'a>(fields: &mut Fields<'a>) -> Result<OneMethod<'a>> {
    let attributes = attributes(fields)?;
    let method_type = fields.type_index()?;
    let vtable_offset = if attributes.method_kind().is_introducing_virtual() {
        Some(fields.i32()?)
    } else {
        None
    };
    Ok(OneMethod {
        attributes,
        method_type,
        vtable_offset,
        name: fields.name()?,
    })
}
