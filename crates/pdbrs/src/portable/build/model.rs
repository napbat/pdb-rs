//! The records of a [`PdbBuilder`](super::PdbBuilder): owned values with typed handles.

use alloc::vec::Vec;

use crate::portable::custom::CustomDebugKind;
use crate::portable::debug::{LocalSignatureForm, SequencePoint};
use crate::portable::metadata::{
    AssemblyRefId, DocumentId, IlOffset, ImportScopeId, MetadataToken, MethodDefId, StandAloneSigId,
};

/// Define an opaque handle of a record that the plan can move to another row.
macro_rules! handle {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        ///
        /// The handle stays valid when other records are added. The plan gives the record its
        /// row ID.
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(pub(super) u32);

        impl $name {
            /// Return the index of the record in the order of addition.
            pub(super) fn index(self) -> usize {
                // A `u32` always fits in `usize` on the supported targets.
                usize::try_from(self.0).unwrap_or(usize::MAX)
            }
        }
    };
}

handle!(
    /// A handle of a local scope.
    LocalScopeHandle
);
handle!(
    /// A handle of a local variable.
    LocalVariableHandle
);
handle!(
    /// A handle of a local constant.
    LocalConstantHandle
);
handle!(
    /// A handle of a custom debug information record.
    CustomDebugHandle
);

/// A source document.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DocumentInfo {
    /// The document name, for example a path. The plan splits it into parts as Roslyn does.
    pub name: Vec<u8>,
    /// The GUID of the hash algorithm, or `None`.
    pub hash_algorithm: Option<[u8; 16]>,
    /// The hash of the document content, or an empty value.
    pub hash: Vec<u8>,
    /// The GUID of the source language, or `None`.
    pub language: Option<[u8; 16]>,
}

/// The sequence points of a method.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MethodInfo {
    /// The local signature of the method, or nil.
    pub local_signature: StandAloneSigId,
    /// How the blob header stores the local signature.
    pub local_signature_form: LocalSignatureForm,
    /// The sequence points in IL offset order. Each document is a [`DocumentId`] of the
    /// builder.
    pub sequence_points: Vec<SequencePoint>,
}

/// A local scope of a method.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LocalScopeInfo {
    /// The method.
    pub method: MethodDefId,
    /// The import scope, or nil.
    pub import_scope: ImportScopeId,
    /// The IL offset of the scope start.
    pub start_offset: IlOffset,
    /// The scope length in IL bytes.
    pub length: u32,
}

/// A local variable of a local scope.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LocalVariableInfo {
    /// The `LocalVariableAttributes` flags.
    pub attributes: u16,
    /// The slot index in the local signature of the method.
    pub index: u16,
    /// The name.
    pub name: Vec<u8>,
}

/// A local constant of a local scope.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LocalConstantInfo {
    /// The name.
    pub name: Vec<u8>,
    /// The local constant signature blob. Encode it with
    /// [`LocalConstantSignature::encode`](crate::portable::debug::LocalConstantSignature::encode).
    pub signature: Vec<u8>,
}

/// One import of an import scope, with names as bytes.
///
/// The variants are the import kinds 1 to 9 of
/// [`Import`](crate::portable::debug::Import). The plan adds each name to the `#Blob` heap.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ImportInfo {
    /// Kind 1: import the members of a namespace.
    Namespace {
        /// The namespace.
        namespace: Vec<u8>,
    },
    /// Kind 2: import the members of a namespace in an assembly.
    AssemblyNamespace {
        /// The assembly.
        assembly: AssemblyRefId,
        /// The namespace.
        namespace: Vec<u8>,
    },
    /// Kind 3: import the members of a type.
    Type {
        /// The type: a `TypeDef`, `TypeRef` or `TypeSpec` token.
        target_type: MetadataToken,
    },
    /// Kind 4: import an XML namespace with a prefix.
    XmlNamespace {
        /// The prefix.
        alias: Vec<u8>,
        /// The XML namespace.
        namespace: Vec<u8>,
    },
    /// Kind 5: import an assembly reference alias of an ancestor scope.
    AssemblyReferenceAlias {
        /// The alias.
        alias: Vec<u8>,
    },
    /// Kind 6: define an alias for an assembly.
    AliasAssemblyReference {
        /// The alias.
        alias: Vec<u8>,
        /// The assembly.
        assembly: AssemblyRefId,
    },
    /// Kind 7: define an alias for a namespace.
    AliasNamespace {
        /// The alias.
        alias: Vec<u8>,
        /// The namespace.
        namespace: Vec<u8>,
    },
    /// Kind 8: define an alias for a namespace in an assembly.
    AliasAssemblyNamespace {
        /// The alias.
        alias: Vec<u8>,
        /// The assembly.
        assembly: AssemblyRefId,
        /// The namespace.
        namespace: Vec<u8>,
    },
    /// Kind 9: define an alias for a type.
    AliasType {
        /// The alias.
        alias: Vec<u8>,
        /// The type: a `TypeDef`, `TypeRef` or `TypeSpec` token.
        target_type: MetadataToken,
    },
}

/// An import scope.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImportScopeInfo {
    /// The parent import scope, or nil.
    pub parent: ImportScopeId,
    /// The imports in their order.
    pub imports: Vec<ImportInfo>,
}

/// The parent of a custom debug information record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CustomDebugParent {
    /// A row of a type-system table of the assembly, for example a `MethodDef` or the
    /// `Module` row. The table must be in the `HasCustomDebugInformation` coded index.
    Metadata(MetadataToken),
    /// A document of the builder.
    Document(DocumentId),
    /// A local scope of the builder.
    LocalScope(LocalScopeHandle),
    /// A local variable of the builder.
    LocalVariable(LocalVariableHandle),
    /// A local constant of the builder.
    LocalConstant(LocalConstantHandle),
    /// An import scope of the builder.
    ImportScope(ImportScopeId),
}

/// A custom debug information record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CustomDebugInfo {
    /// The parent.
    pub parent: CustomDebugParent,
    /// The kind GUID.
    pub kind: CustomDebugKind,
    /// The value blob. Encode a known kind with its codec in
    /// [`custom`](crate::portable::custom).
    pub value: Vec<u8>,
}
