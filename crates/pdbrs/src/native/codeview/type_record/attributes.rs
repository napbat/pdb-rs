//! Flag words and enumerations in type records and field-list members.
//!
//! The bit layouts follow `CV_prop_t`, `CV_modifier_t`, `CV_funcattr_t`,
//! `CV_fldattr_t` and `lfPointerAttr` in the Microsoft `cvinfo.h` header.
//! The enumeration values follow `CodeView.h` in LLVM. Each flag word keeps
//! its raw value, so bits without a meaning stay available.

use crate::{Error, ErrorKind, Result};

/// Return the error for a field value that has no defined meaning.
fn unknown(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, None).with_context(context)
}

/// The properties of a class, structure, union or enumeration (`CV_prop_t`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ClassOptions(pub u16);

impl ClassOptions {
    /// The type has a packed layout.
    pub const PACKED: u16 = 0x0001;
    /// The type has a constructor or a destructor.
    pub const HAS_CONSTRUCTOR_OR_DESTRUCTOR: u16 = 0x0002;
    /// The type has overloaded operators.
    pub const HAS_OVERLOADED_OPERATOR: u16 = 0x0004;
    /// The type is nested in another type.
    pub const NESTED: u16 = 0x0008;
    /// The type contains nested types.
    pub const CONTAINS_NESTED_CLASS: u16 = 0x0010;
    /// The type has an overloaded assignment operator.
    pub const HAS_OVERLOADED_ASSIGNMENT_OPERATOR: u16 = 0x0020;
    /// The type has conversion operators.
    pub const HAS_CONVERSION_OPERATOR: u16 = 0x0040;
    /// The record is a forward reference to an incomplete type.
    pub const FORWARD_REFERENCE: u16 = 0x0080;
    /// The definition is scoped to a function or a block.
    pub const SCOPED: u16 = 0x0100;
    /// A unique (decorated) name follows the name.
    pub const HAS_UNIQUE_NAME: u16 = 0x0200;
    /// The type cannot be a base class.
    pub const SEALED: u16 = 0x0400;
    /// The type is an intrinsic type, for example `__m128d`.
    pub const INTRINSIC: u16 = 0x2000;

    /// Return the raw flag word.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Return `true` if all bits of `flag` are set.
    #[must_use]
    pub const fn contains(self, flag: u16) -> bool {
        self.0 & flag == flag
    }

    /// Return `true` if the record is a forward reference.
    #[must_use]
    pub const fn is_forward_reference(self) -> bool {
        self.contains(Self::FORWARD_REFERENCE)
    }

    /// Return `true` if the definition is scoped.
    #[must_use]
    pub const fn is_scoped(self) -> bool {
        self.contains(Self::SCOPED)
    }

    /// Return `true` if a unique name follows the name.
    #[must_use]
    pub const fn has_unique_name(self) -> bool {
        self.contains(Self::HAS_UNIQUE_NAME)
    }

    /// Return `true` if the type has a packed layout.
    #[must_use]
    pub const fn is_packed(self) -> bool {
        self.contains(Self::PACKED)
    }

    /// Return `true` if the type is nested in another type.
    #[must_use]
    pub const fn is_nested(self) -> bool {
        self.contains(Self::NESTED)
    }

    /// Return the homogeneous floating-point aggregate kind (bits 11 and 12).
    #[must_use]
    pub const fn hfa(self) -> HfaKind {
        match (self.0 >> 11) & 3 {
            0 => HfaKind::None,
            1 => HfaKind::Float,
            2 => HfaKind::Double,
            _ => HfaKind::Other,
        }
    }

    /// Return the `WinRT` (`MoCOM`) class kind (bits 14 and 15).
    #[must_use]
    pub const fn mocom(self) -> MocomKind {
        match self.0 >> 14 {
            0 => MocomKind::None,
            1 => MocomKind::RefClass,
            2 => MocomKind::ValueClass,
            _ => MocomKind::Interface,
        }
    }
}

/// The homogeneous floating-point aggregate kind of a type (`CV_HFA_e`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HfaKind {
    /// The type is not a homogeneous aggregate.
    None,
    /// An aggregate of `float` values.
    Float,
    /// An aggregate of `double` values.
    Double,
    /// An aggregate of other floating-point values.
    Other,
}

/// The `WinRT` class kind of a type (`CV_MOCOM_UDT_e`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MocomKind {
    /// Not a `WinRT` type.
    None,
    /// A reference class.
    RefClass,
    /// A value class.
    ValueClass,
    /// An interface.
    Interface,
}

/// The modifiers of an `LF_MODIFIER` record (`CV_modifier_t`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ModifierOptions(pub u16);

impl ModifierOptions {
    /// `const`.
    pub const CONST: u16 = 0x0001;
    /// `volatile`.
    pub const VOLATILE: u16 = 0x0002;
    /// `__unaligned`.
    pub const UNALIGNED: u16 = 0x0004;

    /// Return the raw flag word.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Return `true` if the type is `const`.
    #[must_use]
    pub const fn is_const(self) -> bool {
        self.0 & Self::CONST != 0
    }

    /// Return `true` if the type is `volatile`.
    #[must_use]
    pub const fn is_volatile(self) -> bool {
        self.0 & Self::VOLATILE != 0
    }

    /// Return `true` if the type is `__unaligned`.
    #[must_use]
    pub const fn is_unaligned(self) -> bool {
        self.0 & Self::UNALIGNED != 0
    }
}

/// The options of a procedure type (`CV_funcattr_t`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FunctionOptions(pub u8);

impl FunctionOptions {
    /// The function returns a C++ user-defined type.
    pub const CXX_RETURN_UDT: u8 = 0x01;
    /// The function is an instance constructor.
    pub const CONSTRUCTOR: u8 = 0x02;
    /// The function is a constructor of a class with virtual bases.
    pub const CONSTRUCTOR_WITH_VIRTUAL_BASES: u8 = 0x04;

    /// Return the raw flag byte.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Return `true` if all bits of `flag` are set.
    #[must_use]
    pub const fn contains(self, flag: u8) -> bool {
        self.0 & flag == flag
    }
}

/// Define a `u8` or `u16` enumeration and its fallible decoder from one table.
macro_rules! raw_enum {
    (
        $(#[$meta:meta])* $name:ident: $raw:ty, $context:literal {
            $($(#[$vmeta:meta])* $variant:ident = $value:literal,)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum $name {
            $($(#[$vmeta])* $variant,)*
        }

        impl $name {
            /// Return the serialized value.
            #[must_use]
            pub const fn raw(self) -> $raw {
                match self {
                    $(Self::$variant => $value,)*
                }
            }

            /// Decode a serialized value.
            pub(crate) fn from_raw(raw: $raw) -> Result<Self> {
                match raw {
                    $($value => Ok(Self::$variant),)*
                    _ => Err(unknown($context)),
                }
            }
        }
    };
}

raw_enum! {
    /// The calling convention of a procedure type (`CV_call_e`).
    CallingConvention: u8, "procedure type has an unknown calling convention" {
        /// Near C: right-to-left push, the caller pops the stack.
        NearC = 0x00,
        /// Far C.
        FarC = 0x01,
        /// Near Pascal: left-to-right push, the callee pops the stack.
        NearPascal = 0x02,
        /// Far Pascal.
        FarPascal = 0x03,
        /// Near fast call: arguments in registers.
        NearFast = 0x04,
        /// Far fast call.
        FarFast = 0x05,
        /// A skipped (unused) call index (`CV_CALL_SKIPPED` in `cvconst.h`).
        Skipped = 0x06,
        /// Near standard call.
        NearStdCall = 0x07,
        /// Far standard call.
        FarStdCall = 0x08,
        /// Near system call.
        NearSysCall = 0x09,
        /// Far system call.
        FarSysCall = 0x0a,
        /// `this` call: `this` is in a register.
        ThisCall = 0x0b,
        /// MIPS call.
        MipsCall = 0x0c,
        /// A generic call sequence.
        Generic = 0x0d,
        /// Alpha call.
        AlphaCall = 0x0e,
        /// `PowerPC` call.
        PpcCall = 0x0f,
        /// Hitachi `SuperH` call.
        ShCall = 0x10,
        /// ARM call.
        ArmCall = 0x11,
        /// AM33 call.
        Am33Call = 0x12,
        /// `TriCore` call.
        TriCall = 0x13,
        /// Hitachi `SuperH`-5 call.
        Sh5Call = 0x14,
        /// M32R call.
        M32rCall = 0x15,
        /// CLR call.
        ClrCall = 0x16,
        /// The function is always inlined and has no calling convention.
        Inline = 0x17,
        /// Near vector call.
        NearVector = 0x18,
        /// Swift call.
        Swift = 0x19,
    }
}

raw_enum! {
    /// The kind of a pointer (`CV_ptrtype_e`).
    PointerKind: u8, "pointer has an unknown kind" {
        /// A 16-bit near pointer.
        Near16 = 0x00,
        /// A 16:16 far pointer.
        Far16 = 0x01,
        /// A 16:16 huge pointer.
        Huge16 = 0x02,
        /// A pointer based on a segment.
        BasedOnSegment = 0x03,
        /// A pointer based on the value of a base.
        BasedOnValue = 0x04,
        /// A pointer based on the segment value of a base.
        BasedOnSegmentValue = 0x05,
        /// A pointer based on the address of a base.
        BasedOnAddress = 0x06,
        /// A pointer based on the segment address of a base.
        BasedOnSegmentAddress = 0x07,
        /// A pointer based on a type.
        BasedOnType = 0x08,
        /// A pointer based on itself.
        BasedOnSelf = 0x09,
        /// A 32-bit near pointer.
        Near32 = 0x0a,
        /// A 16:32 far pointer.
        Far32 = 0x0b,
        /// A 64-bit pointer.
        Near64 = 0x0c,
    }
}

impl PointerKind {
    /// Return `true` for the based kinds `0x03` to `0x09`.
    #[must_use]
    pub const fn is_based(self) -> bool {
        matches!(
            self,
            Self::BasedOnSegment
                | Self::BasedOnValue
                | Self::BasedOnSegmentValue
                | Self::BasedOnAddress
                | Self::BasedOnSegmentAddress
                | Self::BasedOnType
                | Self::BasedOnSelf
        )
    }
}

raw_enum! {
    /// The mode of a pointer (`CV_ptrmode_e`).
    PointerMode: u8, "pointer has an unknown mode" {
        /// A normal pointer.
        Pointer = 0x00,
        /// An lvalue reference.
        LValueReference = 0x01,
        /// A pointer to a data member.
        PointerToDataMember = 0x02,
        /// A pointer to a member function.
        PointerToMemberFunction = 0x03,
        /// An rvalue reference.
        RValueReference = 0x04,
    }
}

impl PointerMode {
    /// Return `true` if member-pointer information follows the attributes.
    #[must_use]
    pub const fn is_member_pointer(self) -> bool {
        matches!(
            self,
            Self::PointerToDataMember | Self::PointerToMemberFunction
        )
    }
}

raw_enum! {
    /// The representation of a pointer to member (`CV_pmtype_e`).
    MemberPointerRepresentation: u16, "member pointer has an unknown representation" {
        /// Not specified (before Visual C++ 8).
        Unknown = 0x00,
        /// Member data, single inheritance.
        SingleInheritanceData = 0x01,
        /// Member data, multiple inheritance.
        MultipleInheritanceData = 0x02,
        /// Member data, virtual inheritance.
        VirtualInheritanceData = 0x03,
        /// Member data, most general.
        GeneralData = 0x04,
        /// Member function, single inheritance.
        SingleInheritanceFunction = 0x05,
        /// Member function, multiple inheritance.
        MultipleInheritanceFunction = 0x06,
        /// Member function, virtual inheritance.
        VirtualInheritanceFunction = 0x07,
        /// Member function, most general.
        GeneralFunction = 0x08,
    }
}

raw_enum! {
    /// The addressing mode of a code label type (`CV_LABEL_TYPE_e`).
    LabelMode: u16, "label type has an unknown addressing mode" {
        /// A near label.
        Near = 0x00,
        /// A far label.
        Far = 0x04,
    }
}

/// The attribute word of an `LF_POINTER` record (`lfPointerAttr`).
///
/// The layout is: kind in bits 0 to 4, mode in bits 5 to 7, the modifier
/// flags in bits 8 to 12, the size in bytes in bits 13 to 18, and the
/// `WinRT`, `&` and `&&` flags in bits 19 to 21. Bits 22 to 31 are unused and
/// stay available in [`PointerAttributes::bits`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PointerAttributes {
    /// The raw attribute word.
    raw: u32,
    /// The decoded kind.
    kind: PointerKind,
    /// The decoded mode.
    mode: PointerMode,
}

impl PointerAttributes {
    /// The pointer is a flat 0:32 pointer.
    pub const FLAT32: u32 = 0x0000_0100;
    /// The pointer is `volatile`.
    pub const VOLATILE: u32 = 0x0000_0200;
    /// The pointer is `const`.
    pub const CONST: u32 = 0x0000_0400;
    /// The pointer is `__unaligned`.
    pub const UNALIGNED: u32 = 0x0000_0800;
    /// The pointer is `__restrict`.
    pub const RESTRICT: u32 = 0x0000_1000;
    /// The pointer is a `WinRT` smart pointer (`^` or `%`).
    pub const WINRT_SMART_POINTER: u32 = 0x0008_0000;
    /// The pointer is the `this` pointer of a `&`-qualified member function.
    pub const LVALUE_REF_THIS_POINTER: u32 = 0x0010_0000;
    /// The pointer is the `this` pointer of a `&&`-qualified member function.
    pub const RVALUE_REF_THIS_POINTER: u32 = 0x0020_0000;

    /// Decode an attribute word.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] for an unknown kind or mode.
    pub fn from_raw(raw: u32) -> Result<Self> {
        let [low, ..] = raw.to_le_bytes();
        Ok(Self {
            raw,
            kind: PointerKind::from_raw(low & 0x1f)?,
            mode: PointerMode::from_raw(low >> 5)?,
        })
    }

    /// Return the raw attribute word.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.raw
    }

    /// Return the pointer kind.
    #[must_use]
    pub const fn kind(self) -> PointerKind {
        self.kind
    }

    /// Return the pointer mode.
    #[must_use]
    pub const fn mode(self) -> PointerMode {
        self.mode
    }

    /// Return the size of the pointer in bytes (bits 13 to 18).
    #[must_use]
    pub const fn size(self) -> u8 {
        // The mask keeps 6 bits, so the value fits in `u8`.
        ((self.raw >> 13) & 0x3f) as u8
    }

    /// Return `true` if all bits of `flag` are set.
    #[must_use]
    pub const fn contains(self, flag: u32) -> bool {
        self.raw & flag == flag
    }
}

raw_enum! {
    /// The method property of a member (`CV_methodprop_e`).
    MethodKind: u8, "member has an unknown method property" {
        /// A non-virtual method or a data member.
        Vanilla = 0x00,
        /// A virtual method.
        Virtual = 0x01,
        /// A static method.
        Static = 0x02,
        /// A friend method.
        Friend = 0x03,
        /// A virtual method that introduces a new virtual table slot.
        IntroducingVirtual = 0x04,
        /// A pure virtual method.
        PureVirtual = 0x05,
        /// A pure virtual method that introduces a new virtual table slot.
        PureIntroducingVirtual = 0x06,
    }
}

impl MethodKind {
    /// Return `true` if a virtual table offset follows the method type.
    #[must_use]
    pub const fn is_introducing_virtual(self) -> bool {
        matches!(
            self,
            Self::IntroducingVirtual | Self::PureIntroducingVirtual
        )
    }
}

/// The access protection of a member (`CV_access_e`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MemberAccess {
    /// No access protection.
    None,
    /// `private`.
    Private,
    /// `protected`.
    Protected,
    /// `public`.
    Public,
}

/// The attribute word of a member or a method (`CV_fldattr_t`).
///
/// The layout is: access in bits 0 and 1, the method property in bits 2 to
/// 4, and the flags in bits 5 to 9. Bits 10 to 15 are unused and stay
/// available in [`MemberAttributes::bits`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MemberAttributes {
    /// The raw attribute word.
    raw: u16,
    /// The decoded method property.
    method_kind: MethodKind,
}

impl MemberAttributes {
    /// The method is compiler generated and does not exist.
    pub const PSEUDO: u16 = 0x0020;
    /// The class cannot be inherited.
    pub const NO_INHERIT: u16 = 0x0040;
    /// The class cannot be constructed.
    pub const NO_CONSTRUCT: u16 = 0x0080;
    /// The method is compiler generated and exists.
    pub const COMPILER_GENERATED: u16 = 0x0100;
    /// The method cannot be overridden.
    pub const SEALED: u16 = 0x0200;

    /// Decode an attribute word.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] for the unknown method property 7.
    pub fn from_raw(raw: u16) -> Result<Self> {
        let [low, _] = raw.to_le_bytes();
        Ok(Self {
            raw,
            method_kind: MethodKind::from_raw((low >> 2) & 7)?,
        })
    }

    /// Return the raw attribute word.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.raw
    }

    /// Return the access protection.
    #[must_use]
    pub const fn access(self) -> MemberAccess {
        match self.raw & 3 {
            0 => MemberAccess::None,
            1 => MemberAccess::Private,
            2 => MemberAccess::Protected,
            _ => MemberAccess::Public,
        }
    }

    /// Return the method property.
    #[must_use]
    pub const fn method_kind(self) -> MethodKind {
        self.method_kind
    }

    /// Return `true` if all bits of `flag` are set.
    #[must_use]
    pub const fn contains(self, flag: u16) -> bool {
        self.raw & flag == flag
    }
}

raw_enum! {
    /// The kind of one virtual function table slot (`CV_VTS_desc_e`).
    VtableSlotKind: u8, "virtual table shape has an unknown slot kind" {
        /// A 16-bit near pointer.
        Near16 = 0x00,
        /// A 16:16 far pointer.
        Far16 = 0x01,
        /// A thin pointer.
        This = 0x02,
        /// A pointer to an outer virtual table.
        Outer = 0x03,
        /// A pointer to metaclass descriptor.
        Meta = 0x04,
        /// A near pointer.
        Near = 0x05,
        /// A far pointer.
        Far = 0x06,
    }
}
