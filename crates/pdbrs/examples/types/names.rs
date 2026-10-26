//! C-like names, sizes and signatures of TPI types.
//!
//! [`Types::declare`] builds a C declarator: the name of a member or a
//! function goes inside the type, for example `int (__cdecl *handler)(int)`
//! or `char name[16]`. The functions follow forward references to their
//! definitions for sizes. They stop at [`MAX_DEPTH`] nested types and print
//! `<...>` there.

use pdbrs::Result;
use pdbrs::native::codeview::index::{Primitive, PrimitiveKind, PrimitiveMode, TypeIndex};
use pdbrs::native::codeview::numeric::{Integer, Numeric};
use pdbrs::native::codeview::type_record::{
    CallingConvention, ModifierOptions, PointerAttributes, PointerMode, TypeLeaf,
};
use pdbrs::native::types::{ForwardRefs, TypeTable};

/// The deepest type nesting that the functions follow.
pub const MAX_DEPTH: usize = 64;

/// The type index that MSVC and LLVM use for `std::nullptr_t`.
const NULLPTR_T: TypeIndex = TypeIndex(0x0103);

/// The TPI records of a PDB and the index of its forward references.
pub struct Types<'t, 'a> {
    /// The TPI records.
    pub table: &'t TypeTable<'a>,
    /// The forward-reference index of the TPI records.
    pub forward: ForwardRefs<'t, 'a>,
}

/// Return the integer value of a numeric leaf as `u64`.
pub fn unsigned(value: &Numeric<'_>) -> Option<u64> {
    value.integer().and_then(Integer::to_u64)
}

/// Return the text of a name. Names stay bytes in the reader.
pub fn text(name: &[u8]) -> String {
    String::from_utf8_lossy(name).into_owned()
}

/// Put a declarator after a base type name.
///
/// A declarator that starts with a space starts with a calling convention.
fn join(base: &str, declarator: &str) -> String {
    if declarator.is_empty() {
        base.to_owned()
    } else if declarator.starts_with('[') {
        format!("{base}{declarator}")
    } else {
        format!("{base} {}", declarator.trim_start())
    }
}

/// Return the C name of a primitive value type.
const fn primitive_kind_name(kind: PrimitiveKind) -> &'static str {
    match kind {
        PrimitiveKind::NoType => "<no type>",
        PrimitiveKind::Absolute => "<absolute>",
        PrimitiveKind::Segment => "<segment>",
        PrimitiveKind::Void => "void",
        PrimitiveKind::Currency => "CURRENCY",
        PrimitiveKind::NearBasicString | PrimitiveKind::FarBasicString => "<BASIC string>",
        PrimitiveKind::NotTranslated => "<not translated>",
        PrimitiveKind::HResult => "HRESULT",
        PrimitiveKind::SignedCharacter => "signed char",
        PrimitiveKind::Int16Short => "short",
        PrimitiveKind::Int32Long => "long",
        PrimitiveKind::Int64Quad | PrimitiveKind::Int64 => "__int64",
        PrimitiveKind::Int128Oct | PrimitiveKind::Int128 => "__int128",
        PrimitiveKind::UnsignedCharacter => "unsigned char",
        PrimitiveKind::UInt16Short => "unsigned short",
        PrimitiveKind::UInt32Long => "unsigned long",
        PrimitiveKind::UInt64Quad | PrimitiveKind::UInt64 => "unsigned __int64",
        PrimitiveKind::UInt128Oct | PrimitiveKind::UInt128 => "unsigned __int128",
        PrimitiveKind::Boolean8 => "bool",
        PrimitiveKind::Boolean16 => "__bool16",
        PrimitiveKind::Boolean32 => "__bool32",
        PrimitiveKind::Boolean64 => "__bool64",
        PrimitiveKind::Boolean128 => "__bool128",
        PrimitiveKind::Float32 | PrimitiveKind::Float32PartialPrecision => "float",
        PrimitiveKind::Float64 => "double",
        PrimitiveKind::Float80 => "long double",
        PrimitiveKind::Float128 => "__float128",
        PrimitiveKind::Float48 => "__float48",
        PrimitiveKind::Float16 => "__half",
        PrimitiveKind::Complex32 | PrimitiveKind::Complex32PartialPrecision => "_Complex float",
        PrimitiveKind::Complex64 => "_Complex double",
        PrimitiveKind::Complex80 => "_Complex long double",
        PrimitiveKind::Complex128 => "_Complex __float128",
        PrimitiveKind::Complex48 => "_Complex __float48",
        PrimitiveKind::Complex16 => "_Complex __half",
        PrimitiveKind::Bit => "<bit>",
        PrimitiveKind::PascalCharacter => "<Pascal char>",
        PrimitiveKind::Boolean32FF => "__bool32ff",
        PrimitiveKind::SByte => "__int8",
        PrimitiveKind::Byte => "unsigned __int8",
        PrimitiveKind::NarrowCharacter => "char",
        PrimitiveKind::WideCharacter => "wchar_t",
        PrimitiveKind::Int16 => "__int16",
        PrimitiveKind::UInt16 => "unsigned __int16",
        PrimitiveKind::Int32 => "int",
        PrimitiveKind::UInt32 => "unsigned int",
        PrimitiveKind::Character16 => "char16_t",
        PrimitiveKind::Character32 => "char32_t",
        PrimitiveKind::Character8 => "char8_t",
        PrimitiveKind::CvPointer => "<CodeView pointer>",
        _ => "<unknown primitive>",
    }
}

/// Return the C keyword of a calling convention.
pub const fn calling_convention(convention: CallingConvention) -> &'static str {
    match convention {
        CallingConvention::NearC | CallingConvention::FarC => "__cdecl",
        CallingConvention::NearPascal | CallingConvention::FarPascal => "__pascal",
        CallingConvention::NearFast | CallingConvention::FarFast => "__fastcall",
        CallingConvention::Skipped => "<skipped>",
        CallingConvention::NearStdCall | CallingConvention::FarStdCall => "__stdcall",
        CallingConvention::NearSysCall | CallingConvention::FarSysCall => "__syscall",
        CallingConvention::ThisCall => "__thiscall",
        CallingConvention::MipsCall => "__mipscall",
        CallingConvention::Generic => "__genericcall",
        CallingConvention::AlphaCall => "__alphacall",
        CallingConvention::PpcCall => "__ppccall",
        CallingConvention::ShCall => "__shcall",
        CallingConvention::ArmCall => "__armcall",
        CallingConvention::Am33Call => "__am33call",
        CallingConvention::TriCall => "__tricall",
        CallingConvention::Sh5Call => "__sh5call",
        CallingConvention::M32rCall => "__m32rcall",
        CallingConvention::ClrCall => "__clrcall",
        CallingConvention::Inline => "__inline",
        CallingConvention::NearVector => "__vectorcall",
        CallingConvention::Swift => "__swiftcall",
        _ => "<unknown calling convention>",
    }
}

/// Return the qualifiers of an `LF_MODIFIER` record, each with a space after it.
fn modifier_qualifiers(options: ModifierOptions) -> String {
    let mut out = String::new();
    for (set, word) in [
        (options.is_const(), "const "),
        (options.is_volatile(), "volatile "),
        (options.is_unaligned(), "__unaligned "),
    ] {
        if set {
            out.push_str(word);
        }
    }
    out
}

/// Return the qualifiers of a pointer, each with a space after it.
fn pointer_qualifiers(attributes: PointerAttributes) -> String {
    let mut out = String::new();
    for (flag, word) in [
        (PointerAttributes::CONST, "const "),
        (PointerAttributes::VOLATILE, "volatile "),
        (PointerAttributes::UNALIGNED, "__unaligned "),
        (PointerAttributes::RESTRICT, "__restrict "),
    ] {
        if attributes.contains(flag) {
            out.push_str(word);
        }
    }
    out
}

/// Put qualifiers before a declarator. Remove the last space if the declarator is empty.
fn qualified(qualifiers: &str, inner: &str) -> String {
    if qualifiers.is_empty() {
        inner.to_owned()
    } else if inner.is_empty() {
        qualifiers.trim_end().to_owned()
    } else {
        format!("{qualifiers}{}", inner.trim_start())
    }
}

impl<'a> Types<'_, 'a> {
    /// Decode the TPI record of a type index.
    pub fn leaf(&self, index: TypeIndex) -> Result<TypeLeaf<'a>> {
        self.table.leaf(index)
    }

    /// Return the size of a value of the type in bytes, if the type has a size.
    ///
    /// A UDT forward reference gives the size of its definition. If no
    /// definition matches, as LLVM can report, the type has no size.
    pub fn size(&self, index: TypeIndex, depth: usize) -> Result<Option<u64>> {
        if depth > MAX_DEPTH {
            return Ok(None);
        }
        if let Some(primitive) = index.primitive()? {
            return Ok(primitive.size().map(u64::from));
        }
        let mut leaf = self.leaf(index)?;
        if leaf
            .tag()
            .is_some_and(|tag| tag.options.is_forward_reference())
            && let Some(definition) = self.forward.resolve(index)?
        {
            leaf = self.leaf(definition)?;
        }
        Ok(match leaf {
            TypeLeaf::Modifier(modifier) => self.size(modifier.modified_type, depth + 1)?,
            TypeLeaf::Pointer(pointer) => Some(u64::from(pointer.attributes.size())),
            TypeLeaf::Array(array) => unsigned(&array.size),
            TypeLeaf::Class(class) if !class.options.is_forward_reference() => {
                unsigned(&class.size)
            }
            TypeLeaf::WideClass(class) if !class.tag().options.is_forward_reference() => {
                unsigned(&class.size)
            }
            TypeLeaf::Union(union) if !union.options.is_forward_reference() => {
                unsigned(&union.size)
            }
            TypeLeaf::WideUnion(union) if !union.tag().options.is_forward_reference() => {
                unsigned(&union.size)
            }
            TypeLeaf::Enum(value) => self.size(value.underlying_type, depth + 1)?,
            TypeLeaf::Bitfield(field) => self.size(field.underlying_type, depth + 1)?,
            TypeLeaf::Alias(alias) => self.size(alias.underlying_type, depth + 1)?,
            _ => None,
        })
    }

    /// Return the C name of a type.
    pub fn name(&self, index: TypeIndex) -> Result<String> {
        self.declare(index, "", 0)
    }

    /// Return the C declaration of `inner` with the type at `index`.
    ///
    /// `inner` is a name or a declarator, for example `*name`. An empty
    /// `inner` gives the name of the type.
    pub fn declare(&self, index: TypeIndex, inner: &str, depth: usize) -> Result<String> {
        if depth > MAX_DEPTH {
            return Ok(join("<...>", inner));
        }
        if let Some(primitive) = index.primitive()? {
            return Ok(primitive_declaration(index, primitive, inner));
        }
        let next = depth + 1;
        Ok(match self.leaf(index)? {
            TypeLeaf::Class(class) => join(&text(class.name), inner),
            TypeLeaf::WideClass(class) => join(&text(class.name), inner),
            TypeLeaf::Union(union) => join(&text(union.name), inner),
            TypeLeaf::WideUnion(union) => join(&text(union.name), inner),
            TypeLeaf::Enum(value) => join(&text(value.name), inner),
            TypeLeaf::Alias(alias) => join(&text(alias.name), inner),
            TypeLeaf::Managed(name) => join(&text(name), inner),
            TypeLeaf::Modifier(modifier) => {
                let qualifiers = modifier_qualifiers(modifier.options);
                if self.is_pointer(modifier.modified_type)? {
                    self.declare(modifier.modified_type, &qualified(&qualifiers, inner), next)?
                } else {
                    let declaration = self.declare(modifier.modified_type, inner, next)?;
                    format!("{qualifiers}{declaration}")
                }
            }
            TypeLeaf::Pointer(pointer) => {
                let mut declarator = match pointer.attributes.mode() {
                    PointerMode::Pointer => "*".to_owned(),
                    PointerMode::LValueReference => "&".to_owned(),
                    PointerMode::RValueReference => "&&".to_owned(),
                    PointerMode::PointerToDataMember | PointerMode::PointerToMemberFunction => {
                        match pointer.member {
                            Some(member) => {
                                format!("{}::*", self.declare(member.containing_type, "", next)?)
                            }
                            None => "::*".to_owned(),
                        }
                    }
                    _ => "<unknown pointer mode>".to_owned(),
                };
                declarator.push_str(&qualified(&pointer_qualifiers(pointer.attributes), inner));
                if self.is_array_or_function(pointer.referent_type)? {
                    declarator = format!("({declarator})");
                }
                self.declare(pointer.referent_type, &declarator, next)?
            }
            TypeLeaf::Array(array) => {
                let element = self.size(array.element_type, next)?;
                let count = match (unsigned(&array.size), element) {
                    (Some(size), Some(element)) if element != 0 => (size / element).to_string(),
                    (Some(0), _) => "0".to_owned(),
                    _ => String::new(),
                };
                // A function that returns an array needs parentheses: `float (f(int))[4]`.
                // Only `function_declarator` gives a declarator with a leading space.
                let declarator = if inner.starts_with(' ') {
                    format!("({})[{count}]", inner.trim_start())
                } else {
                    format!("{inner}[{count}]")
                };
                self.declare(array.element_type, &declarator, next)?
            }
            TypeLeaf::Bitfield(field) => self.declare(
                field.underlying_type,
                &format!("{inner} : {}", field.bit_size),
                next,
            )?,
            TypeLeaf::Procedure(procedure) => {
                let arguments = self.arguments(procedure.argument_list, next)?;
                let convention = calling_convention(procedure.calling_convention);
                let declarator = function_declarator(convention, inner, &arguments, "");
                self.declare(procedure.return_type, &declarator, next)?
            }
            TypeLeaf::MemberFunction(function) => {
                let arguments = self.arguments(function.argument_list, next)?;
                let convention = calling_convention(function.calling_convention);
                let suffix = self.this_qualifiers(function.this_type)?;
                let declarator = function_declarator(convention, inner, &arguments, &suffix);
                self.declare(function.return_type, &declarator, next)?
            }
            TypeLeaf::VtableShape(shape) => {
                join(&format!("<vtable shape, {} slots>", shape.len()), inner)
            }
            other => join(
                &format!("<type {:#x}, leaf {:#06x}>", index.0, other.kind()),
                inner,
            ),
        })
    }

    /// Return the C argument list of an `LF_ARGLIST` record.
    ///
    /// A `T_NOTYPE` argument is the C variable argument list `...`.
    pub fn arguments(&self, list: TypeIndex, depth: usize) -> Result<String> {
        if depth > MAX_DEPTH {
            return Ok("<...>".to_owned());
        }
        if list == TypeIndex::NONE {
            return Ok(String::new());
        }
        let TypeLeaf::ArgumentList(arguments) = self.leaf(list)? else {
            return Ok(format!("<argument list {:#x}>", list.0));
        };
        let mut out = String::new();
        for (position, argument) in arguments.iter().enumerate() {
            if position != 0 {
                out.push_str(", ");
            }
            if argument == TypeIndex::NONE {
                out.push_str("...");
            } else {
                out.push_str(&self.declare(argument, "", depth + 1)?);
            }
        }
        Ok(out)
    }

    /// Return the C declaration of a function with the type at `index`.
    ///
    /// For a member function, add the type of `this`: `[this: <type>]`.
    pub fn signature(&self, index: TypeIndex, name: &str) -> Result<String> {
        let declaration = self.declare(index, name, 0)?;
        if !index.is_primitive()
            && let TypeLeaf::MemberFunction(function) = self.leaf(index)?
            && function.this_type != TypeIndex::NONE
        {
            return Ok(format!(
                "{declaration} [this: {}]",
                self.name(function.this_type)?
            ));
        }
        Ok(declaration)
    }

    /// Return `true` if the type is a pointer, a reference or a primitive pointer.
    fn is_pointer(&self, index: TypeIndex) -> Result<bool> {
        if let Some(primitive) = index.primitive()? {
            return Ok(primitive.mode != PrimitiveMode::Direct);
        }
        Ok(matches!(self.leaf(index)?, TypeLeaf::Pointer(_)))
    }

    /// Return `true` if a pointer to the type needs parentheses in a declarator.
    fn is_array_or_function(&self, index: TypeIndex) -> Result<bool> {
        if index.is_primitive() {
            return Ok(false);
        }
        Ok(matches!(
            self.leaf(index)?,
            TypeLeaf::Array(_) | TypeLeaf::Procedure(_) | TypeLeaf::MemberFunction(_)
        ))
    }

    /// Return the qualifiers of a member function, for example ` const`, from its `this` type.
    fn this_qualifiers(&self, this: TypeIndex) -> Result<String> {
        if this.is_primitive() {
            return Ok(String::new());
        }
        let TypeLeaf::Pointer(pointer) = self.leaf(this)? else {
            return Ok(String::new());
        };
        if pointer.referent_type.is_primitive() {
            return Ok(String::new());
        }
        let TypeLeaf::Modifier(modifier) = self.leaf(pointer.referent_type)? else {
            return Ok(String::new());
        };
        let qualifiers = modifier_qualifiers(modifier.options);
        Ok(if qualifiers.is_empty() {
            String::new()
        } else {
            format!(" {}", qualifiers.trim_end())
        })
    }
}

/// Return the declarator of a function: `inner`, the calling convention and the arguments.
///
/// A pointer declarator in parentheses gets the calling convention inside the
/// parentheses, as MSVC prints it: `(__cdecl *name)(int)`. Other declarators
/// start with a space, so that a pointer to the result keeps a space before
/// the calling convention: `char * __cdecl name(int)`.
fn function_declarator(convention: &str, inner: &str, arguments: &str, suffix: &str) -> String {
    match inner.strip_prefix('(') {
        Some(rest) => format!("({convention} {rest}({arguments}){suffix}"),
        None if inner.is_empty() => format!(" {convention}({arguments}){suffix}"),
        None => format!(" {convention} {inner}({arguments}){suffix}"),
    }
}

/// Return the C declaration of `inner` with a primitive type.
fn primitive_declaration(index: TypeIndex, primitive: Primitive, inner: &str) -> String {
    if index == NULLPTR_T {
        return join("std::nullptr_t", inner);
    }
    let base = primitive_kind_name(primitive.kind);
    if primitive.mode == PrimitiveMode::Direct {
        join(base, inner)
    } else {
        join(base, &format!("*{inner}"))
    }
}
