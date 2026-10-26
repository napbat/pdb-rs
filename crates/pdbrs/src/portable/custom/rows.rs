//! The rows of the `CustomDebugInformation` table with their kind GUIDs and value blobs.

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::{CustomDebugKind, CustomDebugValue};
use crate::PortableLocation;
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::metadata::{
    BlobOffset, CustomDebugInformationId, CustomDebugInformationRow, GuidIndex, MetadataStream,
    MetadataToken, TableId,
};
use crate::{Error, Result};

/// A row of the `CustomDebugInformation` table with its kind GUID and its value blob.
///
/// [`CustomDebugInformation::decode`] decodes the value through the codec of its kind.
/// [`CustomDebugInformation::value`] keeps the raw bytes of every kind.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CustomDebugInformation {
    /// The row ID.
    id: CustomDebugInformationId,
    /// The `Parent` column.
    parent: MetadataToken,
    /// The `Kind` column.
    kind_index: GuidIndex,
    /// The GUID of the `Kind` column.
    kind: CustomDebugKind,
    /// The `Value` column.
    value_offset: BlobOffset,
    /// The blob of the `Value` column.
    value: Vec<u8>,
}

impl CustomDebugInformation {
    /// Return the row ID.
    #[must_use]
    pub const fn id(&self) -> CustomDebugInformationId {
        self.id
    }

    /// Return the parent: a `HasCustomDebugInformation` entity as a metadata token.
    #[must_use]
    pub const fn parent(&self) -> MetadataToken {
        self.parent
    }

    /// Return the `#GUID` index of the kind.
    #[must_use]
    pub const fn kind_index(&self) -> GuidIndex {
        self.kind_index
    }

    /// Return the kind GUID. The nil GUID index gives the GUID with zero bytes.
    #[must_use]
    pub const fn kind(&self) -> CustomDebugKind {
        self.kind
    }

    /// Return the `#Blob` offset of the value.
    #[must_use]
    pub const fn value_offset(&self) -> BlobOffset {
        self.value_offset
    }

    /// Return the raw value blob.
    #[must_use]
    pub fn value(&self) -> &[u8] {
        &self.value
    }

    /// Decode the value through the codec of its kind.
    ///
    /// # Errors
    ///
    /// Return the error of the codec, with the row, the `Value` column and the blob offset
    /// as its location. [`CustomDebugInformation::kind`] gives the kind of the codec. An
    /// unknown kind does not give an error.
    pub fn decode(&self) -> Result<CustomDebugValue<'_>> {
        CustomDebugValue::parse(self.kind, &self.value).map_err(|error| {
            error.with_portable(
                column_location(self.id, "Value").with_heap_offset(self.value_offset.0),
            )
        })
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
    /// Read a `CustomDebugInformation` row with its kind GUID and its value blob.
    ///
    /// # Errors
    ///
    /// Return the errors of [`Pdb::row`], [`Pdb::guid`] and [`Pdb::blob`], with the row and
    /// the column as their location.
    pub fn custom_debug_information(
        &self,
        id: CustomDebugInformationId,
    ) -> Result<CustomDebugInformation> {
        let row: CustomDebugInformationRow = self.row(id)?;
        let at = |column| move |error: Error| error.with_portable(column_location(id, column));
        let kind = self.guid(row.kind).map_err(at("Kind"))?.unwrap_or([0; 16]);
        let value = self.blob(row.value).map_err(at("Value"))?;
        Ok(CustomDebugInformation {
            id,
            parent: row.parent,
            kind_index: row.kind,
            kind: CustomDebugKind(kind),
            value_offset: row.value,
            value,
        })
    }

    /// Iterate over all `CustomDebugInformation` rows in row order.
    #[must_use]
    pub fn custom_debug_informations(&self) -> CustomDebugInformationIter<'_, S> {
        CustomDebugInformationIter {
            pdb: self,
            remaining: self.tables().row_count(TableId::CUSTOM_DEBUG_INFORMATION),
            next: 1,
        }
    }
}

/// An iterator over the `CustomDebugInformation` rows of a [`Pdb`].
///
/// Each item is a row, or the error of that row.
#[derive(Debug)]
pub struct CustomDebugInformationIter<'a, S> {
    /// The Portable PDB.
    pdb: &'a Pdb<S>,
    /// The number of rows after this iterator.
    remaining: u32,
    /// The next row number.
    next: u32,
}

impl<S: ReadAt> Iterator for CustomDebugInformationIter<'_, S> {
    type Item = Result<CustomDebugInformation>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let id = CustomDebugInformationId(self.next);
        self.remaining -= 1;
        self.next = self.next.saturating_add(1);
        Some(self.pdb.custom_debug_information(id))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = usize::try_from(self.remaining).unwrap_or(usize::MAX);
        (remaining, Some(remaining))
    }
}

impl<S: ReadAt> ExactSizeIterator for CustomDebugInformationIter<'_, S> {}

impl<S: ReadAt> FusedIterator for CustomDebugInformationIter<'_, S> {}
