//! The `#Pdb` stream of a standalone Portable PDB.
//!
//! The layout comes from the "#Pdb stream" section of the Portable PDB specification
//! (`PortablePdb-Metadata.md` in `dotnet/runtime`, commit `736475ca`):
//!
//! | Offset | Size | Field |
//! | --- | --- | --- |
//! | 0 | 20 | PDB ID |
//! | 20 | 4 | `EntryPoint`: a `MethodDef` token or 0 |
//! | 24 | 8 | `ReferencedTypeSystemTables`: a bit for each external table |
//! | 32 | 4 × n | `TypeSystemTableRows`: one row count for each set bit |

use super::ids::{MAX_ROW, MetadataToken, MethodDefId, TableId};
use super::root::MetadataStream;
use crate::PortableLocation;
use crate::io::ReadAt;
use crate::portable::Identity;
use crate::{Error, ErrorKind, Result};

/// The size of the fixed fields.
const FIXED_LEN: u32 = 32;

/// The bits of the type-system tables (0x00 to 0x2C).
const TYPE_SYSTEM_MASK: u64 = (1 << 0x2D) - 1;

/// The decoded `#Pdb` stream.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PdbStream {
    id: [u8; 20],
    entry_point: MetadataToken,
    referenced_tables: u64,
    rows: [u32; 64],
}

impl PdbStream {
    /// Read and validate the `#Pdb` stream of `size` bytes at `offset`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the stream size does not match the referenced
    ///   tables, a referenced table is not a type-system table, or a row count does not fit
    ///   in a metadata token.
    /// - [`ErrorKind::InvalidReference`] if the entry point is not 0 and not a `MethodDef`
    ///   token of an external row.
    /// - Errors of `source`.
    pub(crate) fn read(source: &(impl ReadAt + ?Sized), offset: u32, size: u32) -> Result<Self> {
        let at = |relative: u32| u64::from(offset) + u64::from(relative);
        if size < FIXED_LEN {
            return Err(invalid(at(0), "#Pdb stream is shorter than 32 bytes"));
        }
        let mut id = [0; 20];
        source.read_exact_at(at(0), &mut id)?;
        let entry_point = MetadataToken::from_raw(source.read_u32_at(at(20))?);
        let referenced_tables = source.read_u64_at(at(24))?;
        if referenced_tables & !TYPE_SYSTEM_MASK != 0 {
            return Err(invalid(
                at(24),
                "#Pdb stream references a table that is not a type-system table",
            ));
        }
        if size != FIXED_LEN + 4 * referenced_tables.count_ones() {
            return Err(invalid(
                at(0),
                "#Pdb stream size does not match the referenced table count",
            ));
        }
        let mut rows = [0; 64];
        let mut position = FIXED_LEN;
        for (number, count) in rows.iter_mut().enumerate() {
            if referenced_tables & (1 << number) == 0 {
                continue;
            }
            *count = source.read_u32_at(at(position))?;
            if *count > MAX_ROW {
                return Err(invalid(
                    at(position),
                    "external row count does not fit in a token",
                ));
            }
            position += 4;
        }
        let stream = Self {
            id,
            entry_point,
            referenced_tables,
            rows,
        };
        stream.check_entry_point(at(20))?;
        Ok(stream)
    }

    /// Check that the entry point is 0 or a token of an external `MethodDef` row.
    fn check_entry_point(&self, offset: u64) -> Result<()> {
        let token = self.entry_point;
        let valid = token.raw() == 0
            || (matches!(token.table(), Some(TableId::METHOD_DEF))
                && token.row() != 0
                && token.row() <= self.external_row_count(TableId::METHOD_DEF));
        if valid {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::InvalidReference, Some(offset))
                .with_context("#Pdb entry point is not an external MethodDef token")
                .with_portable(MetadataStream::Pdb.location()))
        }
    }

    /// Return the 20-byte PDB ID.
    #[must_use]
    pub const fn id(&self) -> &[u8; 20] {
        &self.id
    }

    /// Return the PDB ID as a GUID and a stamp.
    #[must_use]
    pub fn identity(&self) -> Identity {
        Identity::from_bytes(self.id)
    }

    /// Return the serialized entry point token. The value 0 means that there is no entry
    /// point.
    #[must_use]
    pub const fn entry_point_token(&self) -> MetadataToken {
        self.entry_point
    }

    /// Return the entry point method, if the stream has one.
    #[must_use]
    pub const fn entry_point(&self) -> Option<MethodDefId> {
        if self.entry_point.raw() == 0 {
            None
        } else {
            Some(MethodDefId(self.entry_point.row()))
        }
    }

    /// Return the `ReferencedTypeSystemTables` mask.
    #[must_use]
    pub const fn referenced_tables(&self) -> u64 {
        self.referenced_tables
    }

    /// Return the row count of an external type-system table, or 0 if the stream does not
    /// reference the table.
    #[must_use]
    pub const fn external_row_count(&self, table: TableId) -> u32 {
        self.rows[table.index()]
    }

    /// Return each referenced table with its row count, in table number order.
    pub fn external_tables(&self) -> impl Iterator<Item = (TableId, u32)> + '_ {
        (0_u8..64)
            .filter_map(TableId::from_raw)
            .filter(|table| self.referenced_tables & table.bit() != 0)
            .map(|table| (table, self.rows[table.index()]))
    }

    /// Return the external row counts, indexed by table number.
    pub(crate) const fn row_counts(&self) -> &[u32; 64] {
        &self.rows
    }
}

/// Make an [`ErrorKind::InvalidFormat`] error in the `#Pdb` stream.
fn invalid(offset: u64, context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, Some(offset))
        .with_context(context)
        .with_portable(PortableLocation::new().with_stream(MetadataStream::Pdb))
}
