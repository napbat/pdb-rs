//! The layout of a class, a structure, an interface or a union, and the values of an enum.

use pdbrs::Result;
use pdbrs::native::codeview::field_list::Member;
use pdbrs::native::codeview::index::{PrimitiveMode, TypeIndex};
use pdbrs::native::codeview::numeric::{Integer, Numeric};
use pdbrs::native::codeview::type_record::{
    ClassKind, MemberAccess, MemberAttributes, MethodKind, TypeLeaf,
};

use crate::names::{Types, text};

/// The common fields of a UDT definition.
pub struct Udt<'a> {
    /// The C keyword: `class`, `struct`, `interface` or `union`.
    pub keyword: &'static str,
    /// The name.
    pub name: &'a [u8],
    /// The size in bytes.
    pub size: &'a Numeric<'a>,
    /// The field list, or `T_NOTYPE` if the type has no members.
    pub field_list: TypeIndex,
}

/// Return the UDT fields of a class or union record that is not a forward reference.
pub fn udt<'a>(leaf: &'a TypeLeaf<'_>) -> Option<Udt<'a>> {
    if leaf.tag()?.options.is_forward_reference() {
        return None;
    }
    let keyword = |kind| match kind {
        ClassKind::Class => "class",
        ClassKind::Structure => "struct",
        ClassKind::Interface => "interface",
        _ => "<unknown class kind>",
    };
    Some(match leaf {
        TypeLeaf::Class(class) => Udt {
            keyword: keyword(class.kind),
            name: &class.name,
            size: &class.size,
            field_list: class.field_list,
        },
        TypeLeaf::WideClass(class) => Udt {
            keyword: keyword(class.kind),
            name: &class.name,
            size: &class.size,
            field_list: class.field_list,
        },
        TypeLeaf::Union(union) => Udt {
            keyword: "union",
            name: &union.name,
            size: &union.size,
            field_list: union.field_list,
        },
        TypeLeaf::WideUnion(union) => Udt {
            keyword: "union",
            name: &union.name,
            size: &union.size,
            field_list: union.field_list,
        },
        _ => return None,
    })
}

/// Return the text of a numeric leaf: the integer, or the leaf in debug form.
fn number(value: &Numeric<'_>) -> String {
    match value.integer() {
        Some(Integer::Signed(value)) => value.to_string(),
        Some(Integer::Unsigned(value)) => value.to_string(),
        None => debug(value),
    }
}

/// Return an offset in hexadecimal with a sign, for example `+0x10`.
fn offset(value: &Numeric<'_>) -> String {
    match value.integer() {
        Some(Integer::Signed(value)) if value < 0 => format!("-{:#x}", value.unsigned_abs()),
        Some(Integer::Signed(value)) => format!("+{value:#x}"),
        Some(Integer::Unsigned(value)) => format!("+{value:#x}"),
        None => debug(value),
    }
}

/// Return a numeric leaf that is not an integer in debug form, with string bytes as a list of
/// numbers.
fn debug(value: &Numeric<'_>) -> String {
    match value {
        Numeric::VarString(bytes) => format!("VarString({:?})", bytes.as_bytes()),
        Numeric::Utf8String(bytes) => format!("Utf8String({:?})", bytes.as_bytes()),
        other => format!("{other:?}"),
    }
}

/// Return the access word of a member, with a space after it.
const fn access(attributes: MemberAttributes) -> &'static str {
    match attributes.access() {
        MemberAccess::None => "",
        MemberAccess::Private => "private ",
        MemberAccess::Protected => "protected ",
        MemberAccess::Public => "public ",
        _ => "<unknown access> ",
    }
}

/// Return the method words before the signature, for example `virtual `.
const fn method_prefix(kind: MethodKind) -> &'static str {
    match kind {
        MethodKind::Vanilla => "",
        MethodKind::Virtual | MethodKind::PureVirtual => "virtual ",
        MethodKind::IntroducingVirtual | MethodKind::PureIntroducingVirtual => {
            "virtual (introducing) "
        }
        MethodKind::Static => "static ",
        MethodKind::Friend => "friend ",
        _ => "<unknown method kind> ",
    }
}

/// Print one method: the access, the method kind, the signature and the virtual table offset.
fn method(
    types: &Types<'_, '_>,
    attributes: MemberAttributes,
    method_type: TypeIndex,
    vtable_offset: Option<i32>,
    name: &[u8],
) -> Result<()> {
    let kind = attributes.method_kind();
    let signature = types.signature(method_type, &text(name))?;
    let pure = if matches!(
        kind,
        MethodKind::PureVirtual | MethodKind::PureIntroducingVirtual
    ) {
        " = 0"
    } else {
        ""
    };
    let slot = vtable_offset.map_or_else(String::new, |offset| format!(" [vftable +{offset:#x}]"));
    println!(
        "  method {}{}{signature}{pure}{slot}",
        access(attributes),
        method_prefix(kind)
    );
    Ok(())
}

/// Print a data member. A bit field gets its width and its first bit.
fn data_member(
    types: &Types<'_, '_>,
    attributes: MemberAttributes,
    member_type: TypeIndex,
    at: &Numeric<'_>,
    name: &[u8],
) -> Result<()> {
    let declaration = types.declare(member_type, &text(name), 0)?;
    let bit = if member_type.is_primitive() {
        String::new()
    } else if let TypeLeaf::Bitfield(field) = types.leaf(member_type)? {
        format!(" (bit {})", field.bit_offset)
    } else {
        String::new()
    };
    println!(
        "  data {}{} {declaration}{bit}",
        access(attributes),
        offset(at)
    );
    Ok(())
}

/// Print one field-list member of a UDT.
fn member(types: &Types<'_, '_>, member: &Member<'_>) -> Result<()> {
    match member {
        Member::BaseClass(base) | Member::BaseInterface(base) => println!(
            "  base {}{} {}",
            access(base.attributes),
            offset(&base.offset),
            types.name(base.base_type)?
        ),
        Member::VirtualBaseClass(base) => println!(
            "  {}virtual base {}{} [vbptr {} {}, vbtable index {}]",
            if base.indirect { "indirect " } else { "" },
            access(base.attributes),
            types.name(base.base_type)?,
            offset(&base.vbptr_offset),
            types.name(base.vbptr_type)?,
            number(&base.vbtable_index)
        ),
        Member::VirtualFunctionTable(pointer) => println!("  vfptr {}", types.name(*pointer)?),
        Member::VirtualFunctionOffset(value) => println!(
            "  vfptr offset {:#x} {}",
            value.offset,
            types.name(value.pointer_type)?
        ),
        Member::FriendClass(class) => println!("  friend class {}", types.name(*class)?),
        Member::FriendFunction(function) => println!(
            "  friend {}",
            types.signature(function.function_type, &text(&function.name))?
        ),
        Member::Enumerator(value) => {
            println!("  {} = {}", text(&value.name), number(&value.value));
        }
        Member::DataMember(data) => {
            data_member(
                types,
                data.attributes,
                data.member_type,
                &data.offset,
                &data.name,
            )?;
        }
        Member::StaticMember(data) => println!(
            "  static {}{}",
            access(data.attributes),
            types.declare(data.member_type, &text(&data.name), 0)?
        ),
        Member::OneMethod(one) => {
            method(
                types,
                one.attributes,
                one.method_type,
                one.vtable_offset,
                &one.name,
            )?;
        }
        Member::OverloadedMethod(overloads) => {
            let TypeLeaf::MethodList(list) = types.leaf(overloads.method_list)? else {
                println!("  method {} <no method list>", text(&overloads.name));
                return Ok(());
            };
            for entry in list.entries() {
                let entry = entry?;
                method(
                    types,
                    entry.attributes,
                    entry.method_type,
                    entry.vtable_offset,
                    &overloads.name,
                )?;
            }
        }
        Member::NestedType(nested) => println!(
            "  nested {} = {}",
            text(&nested.name),
            types.name(nested.nested_type)?
        ),
        Member::NestedTypeEx(nested) => println!(
            "  nested {}{} = {}",
            access(nested.attributes),
            text(&nested.name),
            types.name(nested.nested_type)?
        ),
        Member::MemberModify(modify) => println!(
            "  modify {}{}::{}",
            access(modify.attributes),
            types.name(modify.base_type)?,
            text(&modify.name)
        ),
        // The member iterator follows continuations and does not return them.
        Member::Continuation(_) => {}
        other => println!("  <member {other:?}>"),
    }
    Ok(())
}

/// Print each member of a field list and its continuations.
fn members(types: &Types<'_, '_>, field_list: TypeIndex) -> Result<()> {
    if field_list == TypeIndex::NONE {
        return Ok(());
    }
    for item in types.table.members(field_list)? {
        member(types, &item?)?;
    }
    Ok(())
}

/// Print the layout of a UDT definition.
pub fn print_udt(types: &Types<'_, '_>, index: TypeIndex, udt: &Udt<'_>) -> Result<()> {
    println!(
        "{} {} [type {:#x}, size {}] {{",
        udt.keyword,
        text(udt.name),
        index.0,
        number(udt.size)
    );
    members(types, udt.field_list)?;
    println!("}}");
    Ok(())
}

/// Return an enumerator value as the underlying integer type gives it.
///
/// MSVC writes the values of an unsigned 64-bit enum as signed numeric leaves,
/// for example `-1` for `0xffff_ffff_ffff_ffff`. The function reads the low
/// `bits` bits of the value as signed or unsigned, as `signed` selects.
fn enumerator_value(value: Integer, signed: bool, bits: u32) -> String {
    let raw = match value {
        Integer::Signed(value) => u128::from_ne_bytes(value.to_ne_bytes()),
        Integer::Unsigned(value) => value,
    };
    // Move the low `bits` bits to the top, then shift back with the sign or with zeros.
    let unused = 128 - bits.clamp(1, 128);
    let top = raw << unused;
    if signed {
        (i128::from_ne_bytes(top.to_ne_bytes()) >> unused).to_string()
    } else {
        (top >> unused).to_string()
    }
}

/// Print an enum definition: the underlying type and each enumerator with its value.
///
/// An integer underlying type sets the sign and the width of the values. With
/// another underlying type, the values are the numeric leaves.
pub fn print_enum(
    types: &Types<'_, '_>,
    index: TypeIndex,
    name: &[u8],
    underlying: TypeIndex,
    field_list: TypeIndex,
) -> Result<()> {
    println!(
        "enum {} : {} [type {:#x}] {{",
        text(name),
        types.name(underlying)?,
        index.0
    );
    let integer = underlying.primitive()?.and_then(|primitive| {
        let signed = primitive.kind.is_signed_integer()?;
        let size = primitive.size()?;
        (primitive.mode == PrimitiveMode::Direct).then_some((signed, size * 8))
    });
    if field_list != TypeIndex::NONE {
        for item in types.table.members(field_list)? {
            match (item?, integer) {
                (Member::Enumerator(value), Some((signed, bits))) => {
                    let shown = value.value.integer().map_or_else(
                        || number(&value.value),
                        |raw| enumerator_value(raw, signed, bits),
                    );
                    println!("  {} = {shown}", text(&value.name));
                }
                (other, _) => member(types, &other)?,
            }
        }
    }
    println!("}}");
    Ok(())
}
