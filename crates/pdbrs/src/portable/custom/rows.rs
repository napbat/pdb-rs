//! The custom debug information record: its parent, its kind and its value blob.

use super::{CustomDebugKind, CustomDebugValue};
use crate::PortableLocation;
use crate::bytes::{Bytes, into_owned};
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::debug::rows::each_row;
use crate::portable::debug::{
    LocalConstantHandle, LocalScopeHandle, LocalVariableHandle, TypedRows,
};
use crate::portable::metadata::{
    CustomDebugInformationId, CustomDebugInformationRow, DocumentId, ImportScopeId, MetadataStream,
    MetadataToken, TableId,
};
use crate::{Error, Result};

/// A handle of a custom debug information record: its index in the order of addition.
///
/// The plan of a [`PdbEditor`](crate::portable::PdbEditor) gives the record its row ID. A
/// removed record leaves a gap, so the other handles stay valid.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CustomDebugHandle(u32);

impl CustomDebugHandle {
    /// Make the handle of the record at the zero-based index `index`.
    pub(crate) const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Return the zero-based index of the record.
    pub(crate) fn index(self) -> usize {
        // A `u32` always fits in `usize` on the supported targets.
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

/// The parent of a custom debug information record.
///
/// Documents and import scopes keep their row IDs in an editor, so they are IDs. The plan
/// gives local scopes, local variables and local constants their rows, so they are handles.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum CustomDebugParent {
    /// A row of a type-system table of the assembly, for example a `MethodDef` or the
    /// `Module` row. The table must be in the `HasCustomDebugInformation` coded index.
    Metadata(MetadataToken),
    /// A document.
    Document(DocumentId),
    /// A local scope.
    LocalScope(LocalScopeHandle),
    /// A local variable.
    LocalVariable(LocalVariableHandle),
    /// A local constant.
    LocalConstant(LocalConstantHandle),
    /// An import scope.
    ImportScope(ImportScopeId),
}

/// A custom debug information record.
///
/// The value stays encoded. [`CustomDebugInformation::decode`] decodes it through the codec
/// of its kind; [`CustomDebugInformation::value`] keeps the raw bytes of every kind.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct CustomDebugInformation<'a> {
    /// The parent.
    pub parent: CustomDebugParent,
    /// The kind GUID. The nil GUID index gives the GUID with zero bytes.
    pub kind: CustomDebugKind,
    /// The value blob. Encode a known kind with its codec in
    /// [`custom`](crate::portable::custom).
    pub value: Bytes<'a>,
}

into_owned!(struct CustomDebugInformation { value; parent, kind });

impl CustomDebugInformation<'_> {
    /// Make a record of the kind `kind` for `parent` with an empty value blob.
    ///
    /// Set the value with the codec of the kind, for example
    /// [`CustomDebugValue::encode`].
    #[must_use]
    pub const fn new(parent: CustomDebugParent, kind: CustomDebugKind) -> Self {
        Self {
            parent,
            kind,
            value: Bytes::new(&[]),
        }
    }

    /// Decode the value through the codec of its kind.
    ///
    /// # Errors
    ///
    /// Return the error of the codec. The error has no location. An unknown kind does not
    /// give an error.
    pub fn decode(&self) -> Result<CustomDebugValue<'_>> {
        CustomDebugValue::parse(self.kind, &self.value)
    }
}

/// Return the location of a column of the `CustomDebugInformation` row `id`.
const fn column_location(id: CustomDebugInformationId, column: &'static str) -> PortableLocation {
    PortableLocation::new()
        .with_stream(MetadataStream::Tables)
        .with_table(TableId::CUSTOM_DEBUG_INFORMATION)
        .with_row(id.0)
        .with_column(column)
}

impl<S: ReadAt> Pdb<S> {
    /// Read a custom debug information record: the parent, the kind GUID and the value blob.
    ///
    /// The parent of a local variable or a local constant gives the scope whose run holds the
    /// row, and the position in the run.
    ///
    /// # Errors
    ///
    /// - The errors of [`Pdb::row`], [`Pdb::guid`] and [`Pdb::blob`], with the row and the
    ///   column as their location.
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if the parent is a
    ///   local variable or a local constant that is not in the run of a local scope.
    pub fn custom_debug_information(
        &self,
        id: CustomDebugInformationId,
    ) -> Result<CustomDebugInformation<'static>> {
        let row: CustomDebugInformationRow = self.row(id)?;
        let at = |column| move |error: Error| error.with_portable(column_location(id, column));
        let kind = self.guid(row.kind).map_err(at("Kind"))?.unwrap_or([0; 16]);
        let value = self.blob(row.value).map_err(at("Value"))?;
        Ok(CustomDebugInformation {
            parent: self.custom_parent(row.parent).map_err(at("Parent"))?,
            kind: CustomDebugKind(kind),
            value: value.into(),
        })
    }

    /// Return the parent of a token of the `Parent` column.
    fn custom_parent(&self, token: MetadataToken) -> Result<CustomDebugParent> {
        let row = token.row();
        let index = row.saturating_sub(1);
        Ok(match token.table() {
            Some(TableId::DOCUMENT) => CustomDebugParent::Document(DocumentId(row)),
            Some(TableId::IMPORT_SCOPE) => CustomDebugParent::ImportScope(ImportScopeId(row)),
            Some(TableId::LOCAL_SCOPE) => {
                CustomDebugParent::LocalScope(LocalScopeHandle::new(index))
            }
            Some(TableId::LOCAL_VARIABLE) => {
                let (scope, index) = self.local_owner(TableId::LOCAL_VARIABLE, row)?;
                CustomDebugParent::LocalVariable(LocalVariableHandle { scope, index })
            }
            Some(TableId::LOCAL_CONSTANT) => {
                let (scope, index) = self.local_owner(TableId::LOCAL_CONSTANT, row)?;
                CustomDebugParent::LocalConstant(LocalConstantHandle { scope, index })
            }
            _ => CustomDebugParent::Metadata(token),
        })
    }

    /// Read all custom debug information records in row order, each with its row ID.
    #[must_use]
    pub fn custom_debug_informations(
        &self,
    ) -> TypedRows<'_, S, (CustomDebugInformationId, CustomDebugInformation<'static>)> {
        each_row(self, TableId::CUSTOM_DEBUG_INFORMATION, |pdb, row| {
            let id = CustomDebugInformationId(row);
            pdb.custom_debug_information(id)
                .map(|information| (id, information))
        })
    }
}
