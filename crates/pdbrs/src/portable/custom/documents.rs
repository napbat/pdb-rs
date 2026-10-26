//! The type definition documents of a type.
//!
//! The layout follows the "Type Definition Document" section of the Portable
//! PDB specification and `EmbedTypeDefinitionDocumentInformation` in Roslyn
//! `MetadataWriter.PortablePdb.cs` (commit
//! `e3c31f15b79e226656e3b082a921d4d60127b1fa`).

use alloc::vec::Vec;

use super::blob::{Reader, Writer, invalid};
use super::{Entries, count_to_end};
use crate::Result;
use crate::portable::metadata::DocumentId;

/// The documents of a type that has no other document information, for
/// example an enumeration or a type without methods.
///
/// The blob is a `Document` row ID for each document, as a compressed unsigned
/// integer, to the end of the blob.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TypeDefinitionDocuments<'a> {
    /// The row IDs. [`TypeDefinitionDocuments::parse`] checked them.
    bytes: &'a [u8],
    /// The number of row IDs.
    len: usize,
}

impl<'a> TypeDefinitionDocuments<'a> {
    /// Decode a type definition documents blob.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`](crate::ErrorKind::UnexpectedEof) if the
    ///   blob ends inside a compressed integer.
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a
    ///   compressed integer is not valid or a row ID is 0.
    pub fn parse(blob: &'a [u8]) -> Result<Self> {
        let len = count_to_end(blob, 0, read_document)?;
        Ok(Self { bytes: blob, len })
    }

    /// Return the blob bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Return the number of documents.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Return `true` if there are no documents.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Return an iterator over the documents.
    #[must_use]
    pub const fn iter(&self) -> Entries<'a, DocumentId> {
        Entries::new(self.bytes, self.len, 0, read_document)
    }

    /// Encode a type definition documents blob.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a
    ///   document is nil.
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if a
    ///   row ID does not fit in a compressed integer, or if an allocation fails.
    pub fn encode(documents: impl IntoIterator<Item = DocumentId>) -> Result<Vec<u8>> {
        let mut writer = Writer::new();
        for document in documents {
            if document.is_nil() {
                return Err(invalid("type definition document row ID is 0"));
            }
            writer.unsigned(document.get())?;
        }
        Ok(writer.finish())
    }
}

iterable!(TypeDefinitionDocuments => Entries<'a, DocumentId>);

/// Read one `Document` row ID.
fn read_document(reader: &mut Reader<'_>, _: i32) -> Result<DocumentId> {
    let document = DocumentId::new(reader.unsigned()?);
    if document.is_nil() {
        return Err(invalid("type definition document row ID is 0"));
    }
    Ok(document)
}
