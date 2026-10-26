//! The `ImportScope` table: import blobs and the typed row.
//!
//! An imports blob is a sequence of imports. Each import is a compressed kind (1 to 9) and the
//! fields of that kind in this order: alias, target assembly, target namespace, target type.
//! An alias and a namespace are compressed `#Blob` offsets of UTF-8 text. A target assembly is
//! an `AssemblyRef` row. A target type is a `TypeDefOrRefOrSpecEncoded` value.

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::rows::{each_row, in_blob};
use super::type_handle;
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::custom::blob::{Reader, Writer, invalid};
use crate::portable::metadata::{
    AssemblyRefId, BlobOffset, ImportScopeId, ImportScopeRow, MetadataToken, TableId,
};
use crate::{Error, Result};

/// One import of an import scope.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Import {
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

impl Import {
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
    pub fn encode_parts(imports: &[Import], out: &mut Vec<u8>) -> Result<()> {
        let mut writer = Writer::append(out);
        for import in imports {
            import.write(&mut writer)?;
        }
        writer.finish();
        Ok(())
    }
}

impl Iterator for Imports<'_> {
    type Item = Result<Import>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.reader.is_empty() {
            return None;
        }
        let import = Import::read(&mut self.reader);
        self.failed = import.is_err();
        Some(import)
    }
}

impl FusedIterator for Imports<'_> {}

/// A typed row of the `ImportScope` table with its imports blob.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ImportScope {
    id: ImportScopeId,
    row: ImportScopeRow,
    imports: Vec<u8>,
}

impl ImportScope {
    /// Return the row ID.
    #[must_use]
    pub const fn id(&self) -> ImportScopeId {
        self.id
    }

    /// Return the raw row.
    #[must_use]
    pub const fn row(&self) -> &ImportScopeRow {
        &self.row
    }

    /// Return the parent import scope, or nil.
    #[must_use]
    pub const fn parent(&self) -> ImportScopeId {
        self.row.parent
    }

    /// Return the bytes of the imports blob.
    #[must_use]
    pub fn imports_blob(&self) -> &[u8] {
        &self.imports
    }

    /// Return the imports. Add the blob location to an error with
    /// [`ImportScope::blob_error`].
    #[must_use]
    pub fn imports(&self) -> Imports<'_> {
        Imports::parse(&self.imports)
    }

    /// Add the location of the imports blob to a codec error.
    #[must_use]
    pub fn blob_error(&self, error: Error) -> Error {
        in_blob(
            error,
            TableId::IMPORT_SCOPE,
            self.id.0,
            "Imports",
            self.row.imports,
        )
    }
}

impl<S: ReadAt> Pdb<S> {
    /// Read an `ImportScope` row with its imports blob.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Pdb::row`] and [`Pdb::blob`].
    pub fn import_scope(&self, id: ImportScopeId) -> Result<ImportScope> {
        let row: ImportScopeRow = self.row(id)?;
        Ok(ImportScope {
            id,
            row,
            imports: self.blob(row.imports)?,
        })
    }

    /// Read all `ImportScope` rows in row order.
    pub fn import_scopes(&self) -> impl ExactSizeIterator<Item = Result<ImportScope>> + '_ {
        each_row(self, TableId::IMPORT_SCOPE, |pdb, row| {
            pdb.import_scope(ImportScopeId(row))
        })
    }
}
