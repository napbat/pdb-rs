//! Raw rows of the eight Portable PDB debug tables.
//!
//! Each row type holds the serialized column values with typed IDs. Reading a row checks
//! that each heap offset starts inside its heap, each GUID index is inside the `#GUID` heap
//! and each row reference is inside its table. A list column (for example
//! `LocalScope.VariableList`) can also be one more than the row count. The semantic rules,
//! for example the nil values that a column allows, sorting and scope nesting, are not
//! checked here.

use super::coded::CodedIndex;
use super::heaps::Heap;
use super::ids::{
    BlobOffset, CustomDebugInformationId, DocumentId, GuidIndex, IlOffset, ImportScopeId,
    LocalConstantId, LocalScopeId, LocalVariableId, MetadataToken, MethodDebugInformationId,
    MethodDefId, StateMachineMethodId, StringOffset, TableId,
};
use super::root::MetadataStream;
use super::tables::TableStream;
use crate::error::PortableLocation;
use crate::{Error, ErrorKind, Result};

/// A row type of one debug table.
///
/// [`Pdb::row`](crate::portable::Pdb::row) and [`Pdb::rows`](crate::portable::Pdb::rows)
/// read rows of any type that implements this trait. The trait is sealed.
pub trait TableRow: Sized + sealed::Sealed {
    /// The row ID type of the table.
    type Id: Copy;
    /// The table of the rows.
    const TABLE: TableId;

    /// Make a row ID from a 1-based row number.
    fn row_id(row: u32) -> Self::Id;

    /// Return the 1-based row number of a row ID.
    fn row_number(id: Self::Id) -> u32;
}

/// The private part of [`TableRow`].
mod sealed {
    use super::RowReader;
    use crate::Result;

    /// Decode a row. Only this crate implements the trait.
    pub trait Sealed: Sized {
        /// Decode the columns of one row.
        fn decode(reader: &mut RowReader<'_>) -> Result<Self>;
    }
}

/// Decode one row of type `T`.
pub(crate) fn decode_row<T: TableRow>(reader: &mut RowReader<'_>) -> Result<T> {
    <T as sealed::Sealed>::decode(reader)
}

/// The column decoder of one row.
///
/// The type is public only for the sealed trait. Code outside this crate cannot name it.
pub struct RowReader<'a> {
    bytes: &'a [u8],
    tables: &'a TableStream,
    strings: &'a Heap,
    blob: &'a Heap,
    guid: &'a Heap,
    table: TableId,
    row: u32,
}

impl<'a> RowReader<'a> {
    /// Make a decoder for the bytes of one row.
    pub(crate) const fn new(
        bytes: &'a [u8],
        tables: &'a TableStream,
        heaps: [&'a Heap; 3],
        table: TableId,
        row: u32,
    ) -> Self {
        let [strings, blob, guid] = heaps;
        Self {
            bytes,
            tables,
            strings,
            blob,
            guid,
            table,
            row,
        }
    }

    /// Read a value of `width` bytes (2 or 4).
    fn value(&mut self, width: u8) -> u32 {
        if width == 2 {
            u32::from(self.u16())
        } else {
            self.u32()
        }
    }

    /// Read a `u16` column.
    fn u16(&mut self) -> u16 {
        let (bytes, rest) = self.bytes.split_at(2);
        self.bytes = rest;
        u16::from_le_bytes([bytes[0], bytes[1]])
    }

    /// Read a `u32` column.
    fn u32(&mut self) -> u32 {
        let (bytes, rest) = self.bytes.split_at(4);
        self.bytes = rest;
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    /// Read a `#Strings` offset and check it.
    fn string(&mut self, column: &'static str) -> Result<StringOffset> {
        let offset = self.value(self.tables.string_index_width());
        self.check(
            self.strings.contains(offset),
            column,
            Some(offset),
            "string offset is after the #Strings heap",
        )?;
        Ok(StringOffset::new(offset))
    }

    /// Read a `#Blob` offset and check it.
    fn blob(&mut self, column: &'static str) -> Result<BlobOffset> {
        let offset = self.value(self.tables.blob_index_width());
        self.check(
            self.blob.contains(offset),
            column,
            Some(offset),
            "blob offset is after the #Blob heap",
        )?;
        Ok(BlobOffset::new(offset))
    }

    /// Read a `#GUID` index and check it.
    fn guid(&mut self, column: &'static str) -> Result<GuidIndex> {
        let index = self.value(self.tables.guid_index_width());
        self.check(
            index <= self.guid.guid_count(),
            column,
            Some(index),
            "GUID index is after the #GUID heap",
        )?;
        Ok(GuidIndex::new(index))
    }

    /// Read a simple index into `table` and check that it is at most the row count.
    fn index(&mut self, column: &'static str, table: TableId) -> Result<u32> {
        let row = self.value(self.tables.table_index_width(table));
        let valid = row <= self.tables.width_row_count(table);
        self.check(
            valid,
            column,
            None,
            "row reference is after the end of its table",
        )?;
        Ok(row)
    }

    /// Read a list index into `table` and check that it is at most the row count plus one.
    fn list(&mut self, column: &'static str, table: TableId) -> Result<u32> {
        let row = self.value(self.tables.table_index_width(table));
        let valid = u64::from(row) <= u64::from(self.tables.width_row_count(table)) + 1;
        self.check(
            valid,
            column,
            None,
            "list start is after the end of its table",
        )?;
        Ok(row)
    }

    /// Read a coded index and check its tag and row.
    fn coded(&mut self, column: &'static str, index: CodedIndex) -> Result<MetadataToken> {
        let value = self.value(self.tables.coded_index_width(index));
        let token = index.decode(value);
        let valid = token.is_some_and(|token| {
            TableId::new(token.table())
                .is_some_and(|table| token.row() <= self.tables.width_row_count(table))
        });
        self.check(
            valid,
            column,
            None,
            "coded index has an unknown tag or an out-of-range row",
        )?;
        Ok(token.unwrap_or_default())
    }

    /// Return an [`ErrorKind::InvalidReference`] error if `valid` is `false`.
    fn check(
        &self,
        valid: bool,
        column: &'static str,
        heap_offset: Option<u32>,
        context: &'static str,
    ) -> Result<()> {
        if valid {
            return Ok(());
        }
        let mut location = PortableLocation::new()
            .with_stream(MetadataStream::Tables)
            .with_table(self.table)
            .with_row(self.row)
            .with_column(column);
        if let Some(offset) = heap_offset {
            location = location.with_heap_offset(offset);
        }
        Err(Error::new(ErrorKind::InvalidReference, None)
            .with_context(context)
            .with_portable(location))
    }
}

/// Implement [`TableRow`] for a row type.
macro_rules! table_row {
    ($row:ident, $id:ident) => {
        impl TableRow for $row {
            type Id = $id;
            const TABLE: TableId = $id::TABLE;

            fn row_id(row: u32) -> $id {
                $id::new(row)
            }

            fn row_number(id: $id) -> u32 {
                id.get()
            }
        }
    };
}

/// A row of the `Document` table (0x30).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DocumentRow {
    /// `Name`: the document name blob.
    pub name: BlobOffset,
    /// `HashAlgorithm`: the GUID of the hash algorithm, or nil.
    pub hash_algorithm: GuidIndex,
    /// `Hash`: the hash of the document content.
    pub hash: BlobOffset,
    /// `Language`: the GUID of the source language, or nil.
    pub language: GuidIndex,
}

table_row!(DocumentRow, DocumentId);

impl sealed::Sealed for DocumentRow {
    fn decode(reader: &mut RowReader<'_>) -> Result<Self> {
        Ok(Self {
            name: reader.blob("Name")?,
            hash_algorithm: reader.guid("HashAlgorithm")?,
            hash: reader.blob("Hash")?,
            language: reader.guid("Language")?,
        })
    }
}

/// A row of the `MethodDebugInformation` table (0x31).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MethodDebugInformationRow {
    /// `Document`: the single document of all sequence points, or nil.
    pub document: DocumentId,
    /// `SequencePoints`: the sequence points blob, or 0 if the method has none.
    pub sequence_points: BlobOffset,
}

table_row!(MethodDebugInformationRow, MethodDebugInformationId);

impl sealed::Sealed for MethodDebugInformationRow {
    fn decode(reader: &mut RowReader<'_>) -> Result<Self> {
        Ok(Self {
            document: DocumentId::new(reader.index("Document", TableId::DOCUMENT)?),
            sequence_points: reader.blob("SequencePoints")?,
        })
    }
}

/// A row of the `LocalScope` table (0x32).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LocalScopeRow {
    /// `Method`: the external `MethodDef` row.
    pub method: MethodDefId,
    /// `ImportScope`: the import scope.
    pub import_scope: ImportScopeId,
    /// `VariableList`: the first local variable of the run of this scope.
    pub variable_list: LocalVariableId,
    /// `ConstantList`: the first local constant of the run of this scope.
    pub constant_list: LocalConstantId,
    /// `StartOffset`: the IL offset of the scope start.
    pub start_offset: IlOffset,
    /// `Length`: the scope length in IL bytes.
    pub length: u32,
}

table_row!(LocalScopeRow, LocalScopeId);

impl sealed::Sealed for LocalScopeRow {
    fn decode(reader: &mut RowReader<'_>) -> Result<Self> {
        Ok(Self {
            method: MethodDefId::new(reader.index("Method", TableId::METHOD_DEF)?),
            import_scope: ImportScopeId::new(reader.index("ImportScope", TableId::IMPORT_SCOPE)?),
            variable_list: LocalVariableId::new(
                reader.list("VariableList", TableId::LOCAL_VARIABLE)?,
            ),
            constant_list: LocalConstantId::new(
                reader.list("ConstantList", TableId::LOCAL_CONSTANT)?,
            ),
            start_offset: IlOffset::new(reader.u32()),
            length: reader.u32(),
        })
    }
}

/// A row of the `LocalVariable` table (0x33).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LocalVariableRow {
    /// `Attributes`: the `LocalVariableAttributes` flags (0x0001 is `DebuggerHidden`).
    pub attributes: u16,
    /// `Index`: the slot index in the local signature of the method.
    pub index: u16,
    /// `Name`: the variable name.
    pub name: StringOffset,
}

table_row!(LocalVariableRow, LocalVariableId);

impl sealed::Sealed for LocalVariableRow {
    fn decode(reader: &mut RowReader<'_>) -> Result<Self> {
        Ok(Self {
            attributes: reader.u16(),
            index: reader.u16(),
            name: reader.string("Name")?,
        })
    }
}

/// A row of the `LocalConstant` table (0x34).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LocalConstantRow {
    /// `Name`: the constant name.
    pub name: StringOffset,
    /// `Signature`: the local constant signature blob.
    pub signature: BlobOffset,
}

table_row!(LocalConstantRow, LocalConstantId);

impl sealed::Sealed for LocalConstantRow {
    fn decode(reader: &mut RowReader<'_>) -> Result<Self> {
        Ok(Self {
            name: reader.string("Name")?,
            signature: reader.blob("Signature")?,
        })
    }
}

/// A row of the `ImportScope` table (0x35).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ImportScopeRow {
    /// `Parent`: the parent import scope, or nil.
    pub parent: ImportScopeId,
    /// `Imports`: the imports blob.
    pub imports: BlobOffset,
}

table_row!(ImportScopeRow, ImportScopeId);

impl sealed::Sealed for ImportScopeRow {
    fn decode(reader: &mut RowReader<'_>) -> Result<Self> {
        Ok(Self {
            parent: ImportScopeId::new(reader.index("Parent", TableId::IMPORT_SCOPE)?),
            imports: reader.blob("Imports")?,
        })
    }
}

/// A row of the `StateMachineMethod` table (0x36).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StateMachineMethodRow {
    /// `MoveNextMethod`: the `MoveNext` method of the state machine.
    pub move_next_method: MethodDefId,
    /// `KickoffMethod`: the method that starts the state machine.
    pub kickoff_method: MethodDefId,
}

table_row!(StateMachineMethodRow, StateMachineMethodId);

impl sealed::Sealed for StateMachineMethodRow {
    fn decode(reader: &mut RowReader<'_>) -> Result<Self> {
        Ok(Self {
            move_next_method: MethodDefId::new(
                reader.index("MoveNextMethod", TableId::METHOD_DEF)?,
            ),
            kickoff_method: MethodDefId::new(reader.index("KickoffMethod", TableId::METHOD_DEF)?),
        })
    }
}

/// A row of the `CustomDebugInformation` table (0x37).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CustomDebugInformationRow {
    /// `Parent`: the `HasCustomDebugInformation` coded index as a metadata token.
    pub parent: MetadataToken,
    /// `Kind`: the GUID of the information kind.
    pub kind: GuidIndex,
    /// `Value`: the information blob.
    pub value: BlobOffset,
}

table_row!(CustomDebugInformationRow, CustomDebugInformationId);

impl sealed::Sealed for CustomDebugInformationRow {
    fn decode(reader: &mut RowReader<'_>) -> Result<Self> {
        Ok(Self {
            parent: reader.coded("Parent", CodedIndex::HasCustomDebugInformation)?,
            kind: reader.guid("Kind")?,
            value: reader.blob("Value")?,
        })
    }
}
