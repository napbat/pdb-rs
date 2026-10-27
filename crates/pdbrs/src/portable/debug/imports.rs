//! The `ImportScope` table: import blobs and the import scope record.
//!
//! An imports blob is a sequence of imports. Each import is a compressed kind (1 to 9) and the
//! fields of that kind in this order: alias, target assembly, target namespace, target type.
//! An alias and a namespace are compressed `#Blob` offsets of UTF-8 text. A target assembly is
//! an `AssemblyRef` row. A target type is a `TypeDefOrRefOrSpecEncoded` value.

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::rows::{TypedRows, each_row, in_blob};
use super::type_handle;
use crate::Result;
use crate::bytes::Bytes;
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::custom::blob::{Reader, Writer, invalid};
use crate::portable::metadata::{
    AssemblyRefId, BlobOffset, ImportScopeId, ImportScopeRow, MetadataToken, TableId,
};

/// One import of an imports blob, with the `#Blob` offsets of its names.
///
/// This is the table layer of an import. [`Import`] is the record with the names.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum RawImport {
    /// Kind 1: import the members of a namespace.
    Namespace {
        /// The namespace.
        namespace: BlobOffset,
    },
    /// Kind 2: import the members of a namespace in an assembly.
    AssemblyNamespace {
        /// The assembly.
        assembly: AssemblyRefId,
        /// The namespace.
        namespace: BlobOffset,
    },
    /// Kind 3: import the members of a type.
    Type {
        /// The type: a `TypeDef`, `TypeRef` or `TypeSpec` token.
        target_type: MetadataToken,
    },
    /// Kind 4: import an XML namespace with a prefix.
    XmlNamespace {
        /// The prefix.
        alias: BlobOffset,
        /// The XML namespace.
        namespace: BlobOffset,
    },
    /// Kind 5: import an assembly reference alias of an ancestor scope.
    AssemblyReferenceAlias {
        /// The alias.
        alias: BlobOffset,
    },
    /// Kind 6: define an alias for an assembly.
    AliasAssemblyReference {
        /// The alias.
        alias: BlobOffset,
        /// The assembly.
        assembly: AssemblyRefId,
    },
    /// Kind 7: define an alias for a namespace.
    AliasNamespace {
        /// The alias.
        alias: BlobOffset,
        /// The namespace.
        namespace: BlobOffset,
    },
    /// Kind 8: define an alias for a namespace in an assembly.
    AliasAssemblyNamespace {
        /// The alias.
        alias: BlobOffset,
        /// The assembly.
        assembly: AssemblyRefId,
        /// The namespace.
        namespace: BlobOffset,
    },
    /// Kind 9: define an alias for a type.
    AliasType {
        /// The alias.
        alias: BlobOffset,
        /// The type: a `TypeDef`, `TypeRef` or `TypeSpec` token.
        target_type: MetadataToken,
    },
}

impl RawImport {
    /// Return the import kind (1 to 9).
    #[must_use]
    pub const fn kind(&self) -> u32 {
        match self {
            Self::Namespace { .. } => 1,
            Self::AssemblyNamespace { .. } => 2,
            Self::Type { .. } => 3,
            Self::XmlNamespace { .. } => 4,
            Self::AssemblyReferenceAlias { .. } => 5,
            Self::AliasAssemblyReference { .. } => 6,
            Self::AliasNamespace { .. } => 7,
            Self::AliasAssemblyNamespace { .. } => 8,
            Self::AliasType { .. } => 9,
        }
    }

    /// Return the alias, if the kind has one.
    #[must_use]
    pub const fn alias(&self) -> Option<BlobOffset> {
        match *self {
            Self::XmlNamespace { alias, .. }
            | Self::AssemblyReferenceAlias { alias }
            | Self::AliasAssemblyReference { alias, .. }
            | Self::AliasNamespace { alias, .. }
            | Self::AliasAssemblyNamespace { alias, .. }
            | Self::AliasType { alias, .. } => Some(alias),
            _ => None,
        }
    }

    /// Return the target assembly, if the kind has one.
    #[must_use]
    pub const fn assembly(&self) -> Option<AssemblyRefId> {
        match *self {
            Self::AssemblyNamespace { assembly, .. }
            | Self::AliasAssemblyReference { assembly, .. }
            | Self::AliasAssemblyNamespace { assembly, .. } => Some(assembly),
            _ => None,
        }
    }

    /// Return the target namespace, if the kind has one.
    #[must_use]
    pub const fn namespace(&self) -> Option<BlobOffset> {
        match *self {
            Self::Namespace { namespace }
            | Self::AssemblyNamespace { namespace, .. }
            | Self::XmlNamespace { namespace, .. }
            | Self::AliasNamespace { namespace, .. }
            | Self::AliasAssemblyNamespace { namespace, .. } => Some(namespace),
            _ => None,
        }
    }

    /// Return the target type, if the kind has one.
    #[must_use]
    pub const fn target_type(&self) -> Option<MetadataToken> {
        match *self {
            Self::Type { target_type } | Self::AliasType { target_type, .. } => Some(target_type),
            _ => None,
        }
    }

    /// Read one import.
    fn read(reader: &mut Reader<'_>) -> Result<Self> {
        let kind = reader.unsigned()?;
        let blob = |reader: &mut Reader<'_>| reader.unsigned().map(BlobOffset);
        Ok(match kind {
            1 => Self::Namespace {
                namespace: blob(reader)?,
            },
            2 => Self::AssemblyNamespace {
                assembly: read_assembly(reader)?,
                namespace: blob(reader)?,
            },
            3 => Self::Type {
                target_type: type_handle::read(reader)?,
            },
            4 => Self::XmlNamespace {
                alias: blob(reader)?,
                namespace: blob(reader)?,
            },
            5 => Self::AssemblyReferenceAlias {
                alias: blob(reader)?,
            },
            6 => Self::AliasAssemblyReference {
                alias: blob(reader)?,
                assembly: read_assembly(reader)?,
            },
            7 => Self::AliasNamespace {
                alias: blob(reader)?,
                namespace: blob(reader)?,
            },
            8 => Self::AliasAssemblyNamespace {
                alias: blob(reader)?,
                assembly: read_assembly(reader)?,
                namespace: blob(reader)?,
            },
            9 => Self::AliasType {
                alias: blob(reader)?,
                target_type: type_handle::read(reader)?,
            },
            _ => return Err(invalid("import has an unknown kind")),
        })
    }

    /// Write one import.
    fn write(&self, writer: &mut Writer<'_>) -> Result<()> {
        writer.unsigned(self.kind())?;
        if let Some(alias) = self.alias() {
            writer.unsigned(alias.0)?;
        }
        if let Some(assembly) = self.assembly() {
            if assembly.is_nil() {
                return Err(invalid("import target assembly is nil"));
            }
            writer.unsigned(assembly.0)?;
        }
        if let Some(namespace) = self.namespace() {
            writer.unsigned(namespace.0)?;
        }
        if let Some(target_type) = self.target_type() {
            type_handle::write(writer, target_type)?;
        }
        Ok(())
    }
}

/// Read a non-nil `AssemblyRef` row.
fn read_assembly(reader: &mut Reader<'_>) -> Result<AssemblyRefId> {
    let row = reader.unsigned()?;
    if row == 0 {
        return Err(invalid("import target assembly is nil"));
    }
    Ok(AssemblyRefId(row))
}

/// A lazy, checked iterator over the imports of a blob.
///
/// Each item is an import or the first error. An unknown kind gives an error, because the
/// fields of the kind are unknown. The iterator stops after an error.
#[derive(Clone, Debug)]
pub struct Imports<'a> {
    reader: Reader<'a>,
    failed: bool,
}

impl<'a> Imports<'a> {
    /// Start to decode an imports blob. An empty blob has no imports.
    #[must_use]
    pub const fn parse(blob: &'a [u8]) -> Self {
        Self {
            reader: Reader::new(blob),
            failed: false,
        }
    }

    /// Encode an imports blob, and append it to `out`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) for a nil target
    ///   assembly or a target type that is not a non-nil `TypeDef`, `TypeRef` or `TypeSpec`
    ///   token.
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if a value does not fit
    ///   in a compressed integer.
    ///
    /// On error, `out` does not change.
    pub fn encode_parts(imports: &[RawImport], out: &mut Vec<u8>) -> Result<()> {
        let mut writer = Writer::append(out);
        for import in imports {
            import.write(&mut writer)?;
        }
        writer.finish();
        Ok(())
    }
}

impl Iterator for Imports<'_> {
    type Item = Result<RawImport>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.reader.is_empty() {
            return None;
        }
        let import = RawImport::read(&mut self.reader);
        self.failed = import.is_err();
        Some(import)
    }
}

impl FusedIterator for Imports<'_> {}

/// One import of an import scope, with its names.
///
/// The variants are the import kinds 1 to 9. The plan of an editor adds each name to the
/// `#Blob` heap and encodes the imports as a [`RawImport`] list.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Import<'a> {
    /// Kind 1: import the members of a namespace.
    Namespace {
        /// The namespace.
        namespace: Bytes<'a>,
    },
    /// Kind 2: import the members of a namespace in an assembly.
    AssemblyNamespace {
        /// The assembly.
        assembly: AssemblyRefId,
        /// The namespace.
        namespace: Bytes<'a>,
    },
    /// Kind 3: import the members of a type.
    Type {
        /// The type: a `TypeDef`, `TypeRef` or `TypeSpec` token.
        target_type: MetadataToken,
    },
    /// Kind 4: import an XML namespace with a prefix.
    XmlNamespace {
        /// The prefix.
        alias: Bytes<'a>,
        /// The XML namespace.
        namespace: Bytes<'a>,
    },
    /// Kind 5: import an assembly reference alias of an ancestor scope.
    AssemblyReferenceAlias {
        /// The alias.
        alias: Bytes<'a>,
    },
    /// Kind 6: define an alias for an assembly.
    AliasAssemblyReference {
        /// The alias.
        alias: Bytes<'a>,
        /// The assembly.
        assembly: AssemblyRefId,
    },
    /// Kind 7: define an alias for a namespace.
    AliasNamespace {
        /// The alias.
        alias: Bytes<'a>,
        /// The namespace.
        namespace: Bytes<'a>,
    },
    /// Kind 8: define an alias for a namespace in an assembly.
    AliasAssemblyNamespace {
        /// The alias.
        alias: Bytes<'a>,
        /// The assembly.
        assembly: AssemblyRefId,
        /// The namespace.
        namespace: Bytes<'a>,
    },
    /// Kind 9: define an alias for a type.
    AliasType {
        /// The alias.
        alias: Bytes<'a>,
        /// The type: a `TypeDef`, `TypeRef` or `TypeSpec` token.
        target_type: MetadataToken,
    },
}

impl<'a> Import<'a> {
    /// Return the import kind (1 to 9).
    #[must_use]
    pub const fn kind(&self) -> u32 {
        match self {
            Self::Namespace { .. } => 1,
            Self::AssemblyNamespace { .. } => 2,
            Self::Type { .. } => 3,
            Self::XmlNamespace { .. } => 4,
            Self::AssemblyReferenceAlias { .. } => 5,
            Self::AliasAssemblyReference { .. } => 6,
            Self::AliasNamespace { .. } => 7,
            Self::AliasAssemblyNamespace { .. } => 8,
            Self::AliasType { .. } => 9,
        }
    }

    /// Return an import that owns its bytes. Copy borrowed bytes.
    #[must_use]
    pub fn into_owned(self) -> Import<'static> {
        self.map_names(Bytes::into_owned)
    }

    /// Return the import with each name changed by `name`.
    fn map_names<'b>(self, mut name: impl FnMut(Bytes<'a>) -> Bytes<'b>) -> Import<'b> {
        match self {
            Self::Namespace { namespace } => Import::Namespace {
                namespace: name(namespace),
            },
            Self::AssemblyNamespace {
                assembly,
                namespace,
            } => Import::AssemblyNamespace {
                assembly,
                namespace: name(namespace),
            },
            Self::Type { target_type } => Import::Type { target_type },
            Self::XmlNamespace { alias, namespace } => Import::XmlNamespace {
                alias: name(alias),
                namespace: name(namespace),
            },
            Self::AssemblyReferenceAlias { alias } => {
                Import::AssemblyReferenceAlias { alias: name(alias) }
            }
            Self::AliasAssemblyReference { alias, assembly } => Import::AliasAssemblyReference {
                alias: name(alias),
                assembly,
            },
            Self::AliasNamespace { alias, namespace } => Import::AliasNamespace {
                alias: name(alias),
                namespace: name(namespace),
            },
            Self::AliasAssemblyNamespace {
                alias,
                assembly,
                namespace,
            } => Import::AliasAssemblyNamespace {
                alias: name(alias),
                assembly,
                namespace: name(namespace),
            },
            Self::AliasType { alias, target_type } => Import::AliasType {
                alias: name(alias),
                target_type,
            },
        }
    }

    /// Return the raw import. `blob` gives the `#Blob` offset of each name.
    pub(crate) fn to_raw(
        &self,
        mut blob: impl FnMut(&[u8]) -> Result<BlobOffset>,
    ) -> Result<RawImport> {
        Ok(match self {
            Self::Namespace { namespace } => RawImport::Namespace {
                namespace: blob(namespace)?,
            },
            Self::AssemblyNamespace {
                assembly,
                namespace,
            } => RawImport::AssemblyNamespace {
                assembly: *assembly,
                namespace: blob(namespace)?,
            },
            Self::Type { target_type } => RawImport::Type {
                target_type: *target_type,
            },
            Self::XmlNamespace { alias, namespace } => RawImport::XmlNamespace {
                alias: blob(alias)?,
                namespace: blob(namespace)?,
            },
            Self::AssemblyReferenceAlias { alias } => RawImport::AssemblyReferenceAlias {
                alias: blob(alias)?,
            },
            Self::AliasAssemblyReference { alias, assembly } => RawImport::AliasAssemblyReference {
                alias: blob(alias)?,
                assembly: *assembly,
            },
            Self::AliasNamespace { alias, namespace } => RawImport::AliasNamespace {
                alias: blob(alias)?,
                namespace: blob(namespace)?,
            },
            Self::AliasAssemblyNamespace {
                alias,
                assembly,
                namespace,
            } => RawImport::AliasAssemblyNamespace {
                alias: blob(alias)?,
                assembly: *assembly,
                namespace: blob(namespace)?,
            },
            Self::AliasType { alias, target_type } => RawImport::AliasType {
                alias: blob(alias)?,
                target_type: *target_type,
            },
        })
    }
}

impl crate::bytes::IntoOwned for Import<'_> {
    type Owned = Import<'static>;

    fn to_static(self) -> Import<'static> {
        self.into_owned()
    }
}

impl RawImport {
    /// Return the import with the names. `blob` reads the name at a `#Blob` offset.
    fn resolve(
        self,
        mut blob: impl FnMut(BlobOffset) -> Result<Vec<u8>>,
    ) -> Result<Import<'static>> {
        let mut name = |offset| blob(offset).map(Bytes::from);
        Ok(match self {
            Self::Namespace { namespace } => Import::Namespace {
                namespace: name(namespace)?,
            },
            Self::AssemblyNamespace {
                assembly,
                namespace,
            } => Import::AssemblyNamespace {
                assembly,
                namespace: name(namespace)?,
            },
            Self::Type { target_type } => Import::Type { target_type },
            Self::XmlNamespace { alias, namespace } => Import::XmlNamespace {
                alias: name(alias)?,
                namespace: name(namespace)?,
            },
            Self::AssemblyReferenceAlias { alias } => Import::AssemblyReferenceAlias {
                alias: name(alias)?,
            },
            Self::AliasAssemblyReference { alias, assembly } => Import::AliasAssemblyReference {
                alias: name(alias)?,
                assembly,
            },
            Self::AliasNamespace { alias, namespace } => Import::AliasNamespace {
                alias: name(alias)?,
                namespace: name(namespace)?,
            },
            Self::AliasAssemblyNamespace {
                alias,
                assembly,
                namespace,
            } => Import::AliasAssemblyNamespace {
                alias: name(alias)?,
                assembly,
                namespace: name(namespace)?,
            },
            Self::AliasType { alias, target_type } => Import::AliasType {
                alias: name(alias)?,
                target_type,
            },
        })
    }
}

/// An import scope: its parent and its imports.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ImportScope<'a> {
    /// The parent import scope, or nil.
    pub parent: ImportScopeId,
    /// The imports in their order.
    pub imports: Vec<Import<'a>>,
}

crate::bytes::into_owned!(struct ImportScope { imports; parent });

impl<S: ReadAt> Pdb<S> {
    /// Read an import scope and decode its imports with their names.
    ///
    /// # Errors
    ///
    /// - The errors of [`Pdb::row`] and [`Pdb::blob`].
    /// - The errors of [`Imports`], with the location of the imports blob: for example an
    ///   import of an unknown kind.
    pub fn import_scope(&self, id: ImportScopeId) -> Result<ImportScope<'static>> {
        let row: ImportScopeRow = self.row(id)?;
        let blob = self.blob(row.imports)?;
        let imports = Imports::parse(&blob)
            .map(|import| {
                import
                    .map_err(|error| {
                        in_blob(error, TableId::IMPORT_SCOPE, id.0, "Imports", row.imports)
                    })?
                    .resolve(|offset| self.blob(offset))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ImportScope {
            parent: row.parent,
            imports,
        })
    }

    /// Read all import scopes in row order, each with its row ID.
    #[must_use]
    pub fn import_scopes(&self) -> TypedRows<'_, S, (ImportScopeId, ImportScope<'static>)> {
        each_row(self, TableId::IMPORT_SCOPE, |pdb, row| {
            let id = ImportScopeId(row);
            pdb.import_scope(id).map(|scope| (id, scope))
        })
    }
}
