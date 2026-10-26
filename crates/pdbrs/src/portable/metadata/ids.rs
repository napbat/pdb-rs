//! Typed table numbers, row IDs, metadata tokens and heap handles.
//!
//! Each index space has its own type, so a value of one space cannot be used as a value of
//! another space by mistake. Row IDs are 1-based. The raw value 0 is the nil row, which some
//! columns allow.

use core::fmt;

/// An ECMA-335 metadata table number (0x00 to 0x3F).
///
/// Table numbers 0x00 to 0x2C are the ECMA-335 type-system tables. Table numbers 0x30 to 0x37
/// are the Portable PDB debug tables.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TableId(u8);

/// The names of the ECMA-335 type-system tables, in table number order (0x00 to 0x2C).
const TYPE_SYSTEM_NAMES: [&str; 45] = [
    "Module",
    "TypeRef",
    "TypeDef",
    "FieldPtr",
    "Field",
    "MethodPtr",
    "MethodDef",
    "ParamPtr",
    "Param",
    "InterfaceImpl",
    "MemberRef",
    "Constant",
    "CustomAttribute",
    "FieldMarshal",
    "DeclSecurity",
    "ClassLayout",
    "FieldLayout",
    "StandAloneSig",
    "EventMap",
    "EventPtr",
    "Event",
    "PropertyMap",
    "PropertyPtr",
    "Property",
    "MethodSemantics",
    "MethodImpl",
    "ModuleRef",
    "TypeSpec",
    "ImplMap",
    "FieldRVA",
    "EncLog",
    "EncMap",
    "Assembly",
    "AssemblyProcessor",
    "AssemblyOS",
    "AssemblyRef",
    "AssemblyRefProcessor",
    "AssemblyRefOS",
    "File",
    "ExportedType",
    "ManifestResource",
    "NestedClass",
    "GenericParam",
    "MethodSpec",
    "GenericParamConstraint",
];

/// The names of the Portable PDB debug tables, in table number order (0x30 to 0x37).
const DEBUG_NAMES: [&str; 8] = [
    "Document",
    "MethodDebugInformation",
    "LocalScope",
    "LocalVariable",
    "LocalConstant",
    "ImportScope",
    "StateMachineMethod",
    "CustomDebugInformation",
];

impl TableId {
    /// The `Module` table (0x00).
    pub const MODULE: Self = Self(0x00);
    /// The `TypeRef` table (0x01).
    pub const TYPE_REF: Self = Self(0x01);
    /// The `TypeDef` table (0x02).
    pub const TYPE_DEF: Self = Self(0x02);
    /// The `Field` table (0x04).
    pub const FIELD: Self = Self(0x04);
    /// The `MethodDef` table (0x06).
    pub const METHOD_DEF: Self = Self(0x06);
    /// The `Param` table (0x08).
    pub const PARAM: Self = Self(0x08);
    /// The `InterfaceImpl` table (0x09).
    pub const INTERFACE_IMPL: Self = Self(0x09);
    /// The `MemberRef` table (0x0A).
    pub const MEMBER_REF: Self = Self(0x0A);
    /// The `DeclSecurity` table (0x0E).
    pub const DECL_SECURITY: Self = Self(0x0E);
    /// The `StandAloneSig` table (0x11).
    pub const STAND_ALONE_SIG: Self = Self(0x11);
    /// The `Event` table (0x14).
    pub const EVENT: Self = Self(0x14);
    /// The `Property` table (0x17).
    pub const PROPERTY: Self = Self(0x17);
    /// The `ModuleRef` table (0x1A).
    pub const MODULE_REF: Self = Self(0x1A);
    /// The `TypeSpec` table (0x1B).
    pub const TYPE_SPEC: Self = Self(0x1B);
    /// The `Assembly` table (0x20).
    pub const ASSEMBLY: Self = Self(0x20);
    /// The `AssemblyRef` table (0x23).
    pub const ASSEMBLY_REF: Self = Self(0x23);
    /// The `File` table (0x26).
    pub const FILE: Self = Self(0x26);
    /// The `ExportedType` table (0x27).
    pub const EXPORTED_TYPE: Self = Self(0x27);
    /// The `ManifestResource` table (0x28).
    pub const MANIFEST_RESOURCE: Self = Self(0x28);
    /// The `GenericParam` table (0x2A).
    pub const GENERIC_PARAM: Self = Self(0x2A);
    /// The `MethodSpec` table (0x2B).
    pub const METHOD_SPEC: Self = Self(0x2B);
    /// The `GenericParamConstraint` table (0x2C), the last type-system table.
    pub const GENERIC_PARAM_CONSTRAINT: Self = Self(0x2C);
    /// The `Document` debug table (0x30).
    pub const DOCUMENT: Self = Self(0x30);
    /// The `MethodDebugInformation` debug table (0x31).
    pub const METHOD_DEBUG_INFORMATION: Self = Self(0x31);
    /// The `LocalScope` debug table (0x32).
    pub const LOCAL_SCOPE: Self = Self(0x32);
    /// The `LocalVariable` debug table (0x33).
    pub const LOCAL_VARIABLE: Self = Self(0x33);
    /// The `LocalConstant` debug table (0x34).
    pub const LOCAL_CONSTANT: Self = Self(0x34);
    /// The `ImportScope` debug table (0x35).
    pub const IMPORT_SCOPE: Self = Self(0x35);
    /// The `StateMachineMethod` debug table (0x36).
    pub const STATE_MACHINE_METHOD: Self = Self(0x36);
    /// The `CustomDebugInformation` debug table (0x37).
    pub const CUSTOM_DEBUG_INFORMATION: Self = Self(0x37);

    /// The eight Portable PDB debug tables in table number order.
    pub const DEBUG_TABLES: [Self; 8] = [
        Self::DOCUMENT,
        Self::METHOD_DEBUG_INFORMATION,
        Self::LOCAL_SCOPE,
        Self::LOCAL_VARIABLE,
        Self::LOCAL_CONSTANT,
        Self::IMPORT_SCOPE,
        Self::STATE_MACHINE_METHOD,
        Self::CUSTOM_DEBUG_INFORMATION,
    ];

    /// Make a table number. Return `None` if `number` is larger than 0x3F.
    #[must_use]
    pub const fn new(number: u8) -> Option<Self> {
        if number < 64 {
            Some(Self(number))
        } else {
            None
        }
    }

    /// Return the table number.
    #[must_use]
    pub const fn number(self) -> u8 {
        self.0
    }

    /// Return `true` for an ECMA-335 type-system table (0x00 to 0x2C).
    #[must_use]
    pub const fn is_type_system(self) -> bool {
        self.0 <= Self::GENERIC_PARAM_CONSTRAINT.0
    }

    /// Return `true` for a Portable PDB debug table (0x30 to 0x37).
    #[must_use]
    pub const fn is_debug(self) -> bool {
        self.0 >= Self::DOCUMENT.0 && self.0 <= Self::CUSTOM_DEBUG_INFORMATION.0
    }

    /// Return the table name from ECMA-335 or the Portable PDB specification, if the number
    /// has one.
    #[must_use]
    pub const fn name(self) -> Option<&'static str> {
        if self.is_type_system() {
            Some(TYPE_SYSTEM_NAMES[self.0 as usize])
        } else if self.is_debug() {
            Some(DEBUG_NAMES[(self.0 - Self::DOCUMENT.0) as usize])
        } else {
            None
        }
    }

    /// Return the bit of this table in a 64-bit table mask.
    pub(crate) const fn bit(self) -> u64 {
        1 << self.0
    }

    /// Return the table number as an array index.
    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for TableId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => write!(formatter, "{name} ({:#04x})", self.0),
            None => write!(formatter, "{:#04x}", self.0),
        }
    }
}

/// The largest row number that a metadata token can hold (24 bits).
pub const MAX_ROW: u32 = 0x00FF_FFFF;

/// An ECMA-335 metadata token: a table number in the high byte and a row number in the low
/// 24 bits.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MetadataToken(u32);

impl MetadataToken {
    /// Make a token. Return `None` if `row` is larger than [`MAX_ROW`].
    #[must_use]
    pub const fn new(table: TableId, row: u32) -> Option<Self> {
        if row > MAX_ROW {
            return None;
        }
        Some(Self(((table.0 as u32) << 24) | row))
    }

    /// Make a token from its serialized 32-bit value.
    #[must_use]
    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    /// Return the serialized 32-bit value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Return the table number in the high byte.
    #[must_use]
    pub const fn table(self) -> u8 {
        self.0.to_be_bytes()[0]
    }

    /// Return the row number in the low 24 bits. The value 0 is the nil row.
    #[must_use]
    pub const fn row(self) -> u32 {
        self.0 & MAX_ROW
    }
}

/// Define a typed row ID of one table.
macro_rules! row_id {
    ($(#[$doc:meta])* $name:ident, $table:expr) => {
        $(#[$doc])*
        ///
        /// The value is 1-based. The raw value 0 is the nil row.
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u32);

        impl $name {
            /// The table of this row ID.
            pub const TABLE: TableId = $table;

            /// Make a row ID from its 1-based row number, or 0 for nil.
            #[must_use]
            pub const fn new(row: u32) -> Self {
                Self(row)
            }

            /// Return the 1-based row number, or 0 for nil.
            #[must_use]
            pub const fn get(self) -> u32 {
                self.0
            }

            /// Return `true` for the nil row (0).
            #[must_use]
            pub const fn is_nil(self) -> bool {
                self.0 == 0
            }

            /// Return the metadata token of the row, or `None` if the row number does not fit
            /// in a token.
            #[must_use]
            pub const fn token(self) -> Option<MetadataToken> {
                MetadataToken::new(Self::TABLE, self.0)
            }
        }
    };
}

row_id!(
    /// A row of the external `MethodDef` table of the assembly.
    MethodDefId,
    TableId::METHOD_DEF
);
row_id!(
    /// A row of the `Document` table.
    DocumentId,
    TableId::DOCUMENT
);
row_id!(
    /// A row of the `MethodDebugInformation` table. Its number is the number of the
    /// `MethodDef` row that it extends.
    MethodDebugInformationId,
    TableId::METHOD_DEBUG_INFORMATION
);
row_id!(
    /// A row of the `LocalScope` table.
    LocalScopeId,
    TableId::LOCAL_SCOPE
);
row_id!(
    /// A row of the `LocalVariable` table.
    LocalVariableId,
    TableId::LOCAL_VARIABLE
);
row_id!(
    /// A row of the `LocalConstant` table.
    LocalConstantId,
    TableId::LOCAL_CONSTANT
);
row_id!(
    /// A row of the `ImportScope` table.
    ImportScopeId,
    TableId::IMPORT_SCOPE
);
row_id!(
    /// A row of the `StateMachineMethod` table.
    StateMachineMethodId,
    TableId::STATE_MACHINE_METHOD
);
row_id!(
    /// A row of the `CustomDebugInformation` table.
    CustomDebugInformationId,
    TableId::CUSTOM_DEBUG_INFORMATION
);

/// Define a typed 32-bit value: a heap handle or an IL offset.
macro_rules! heap_handle {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u32);

        impl $name {
            /// Make a handle from its serialized value.
            #[must_use]
            pub const fn new(value: u32) -> Self {
                Self(value)
            }

            /// Return the serialized value.
            #[must_use]
            pub const fn get(self) -> u32 {
                self.0
            }
        }
    };
}

heap_handle!(
    /// A byte offset into the `#Strings` heap. Offset 0 is the empty string.
    StringOffset
);
heap_handle!(
    /// A byte offset into the `#Blob` heap. Offset 0 is the empty blob.
    BlobOffset
);
heap_handle!(
    /// A byte offset into the `#US` (user string) heap. Offset 0 is the empty entry.
    UserStringOffset
);
heap_handle!(
    /// A 1-based index into the `#GUID` heap. Index 0 is the nil GUID.
    GuidIndex
);
heap_handle!(
    /// A byte offset in the IL body of a method. It needs its method for context; it is not
    /// an RVA.
    IlOffset
);
