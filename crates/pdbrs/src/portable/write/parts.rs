//! The parts of a Portable PDB before layout: the `#Pdb` fields, the heaps and the rows.

use alloc::vec::Vec;

use super::heaps::{BlobHeap, GuidHeap, StringHeap, UserStringHeap};
use crate::portable::metadata::{
    CustomDebugInformationRow, DocumentRow, ImportScopeRow, LocalConstantRow, LocalScopeRow,
    LocalVariableRow, MethodDebugInformationRow, MethodDefId, PdbStream, StateMachineMethodRow,
    TableId,
};

/// The metadata version string of a Portable PDB.
pub const PDB_VERSION: &[u8] = b"PDB v1.0";

/// The raw rows of the eight debug tables, in row order.
///
/// The row ID of a row is its index in the vector plus 1. Each row refers to the heaps of its
/// [`PdbParts`] and to rows of these tables or of the external type-system tables.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DebugTables {
    /// The `Document` table (0x30).
    pub documents: Vec<DocumentRow>,
    /// The `MethodDebugInformation` table (0x31): no rows, or one row for each external
    /// `MethodDef` row.
    pub method_debug_information: Vec<MethodDebugInformationRow>,
    /// The `LocalScope` table (0x32), sorted by method, start offset and length (longest
    /// first).
    pub local_scopes: Vec<LocalScopeRow>,
    /// The `LocalVariable` table (0x33).
    pub local_variables: Vec<LocalVariableRow>,
    /// The `LocalConstant` table (0x34).
    pub local_constants: Vec<LocalConstantRow>,
    /// The `ImportScope` table (0x35).
    pub import_scopes: Vec<ImportScopeRow>,
    /// The `StateMachineMethod` table (0x36), sorted by `MoveNext` method.
    pub state_machine_methods: Vec<StateMachineMethodRow>,
    /// The `CustomDebugInformation` table (0x37), sorted by the coded parent.
    pub custom_debug_information: Vec<CustomDebugInformationRow>,
}

impl DebugTables {
    /// Return the row count of a debug table as `usize`, or 0 for another table.
    pub(crate) fn len(&self, table: TableId) -> usize {
        match table {
            TableId::DOCUMENT => self.documents.len(),
            TableId::METHOD_DEBUG_INFORMATION => self.method_debug_information.len(),
            TableId::LOCAL_SCOPE => self.local_scopes.len(),
            TableId::LOCAL_VARIABLE => self.local_variables.len(),
            TableId::LOCAL_CONSTANT => self.local_constants.len(),
            TableId::IMPORT_SCOPE => self.import_scopes.len(),
            TableId::STATE_MACHINE_METHOD => self.state_machine_methods.len(),
            TableId::CUSTOM_DEBUG_INFORMATION => self.custom_debug_information.len(),
            _ => 0,
        }
    }
}

/// The row counts of the external type-system tables of the assembly.
///
/// The counts set the widths of references to these tables. The `#Pdb` stream lists each
/// table with a count other than 0.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExternalRowCounts([u32; 64]);

impl Default for ExternalRowCounts {
    fn default() -> Self {
        Self([0; 64])
    }
}

impl ExternalRowCounts {
    /// Make a set of counts that are all 0.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Copy the external row counts of a `#Pdb` stream.
    #[must_use]
    pub fn from_pdb_stream(stream: &PdbStream) -> Self {
        Self(*stream.row_counts())
    }

    /// Return the row count of `table`.
    #[must_use]
    pub const fn get(&self, table: TableId) -> u32 {
        self.0[table.index()]
    }

    /// Set the row count of `table`. The write plan accepts counts only for type-system
    /// tables (0x00 to 0x2C).
    pub const fn set(&mut self, table: TableId, count: u32) {
        self.0[table.index()] = count;
    }

    /// Return the counts, indexed by table number.
    pub(crate) const fn as_array(&self) -> &[u32; 64] {
        &self.0
    }
}

/// The parts of a Portable PDB before layout.
///
/// Fill the heaps, then the rows with the offsets and indexes of the heaps. Then call
/// [`PdbParts::plan`](crate::portable::write::PdbParts::plan) to check the parts and freeze
/// the layout. [`Pdb::to_parts`](crate::portable::Pdb::to_parts) makes the parts of an existing
/// Portable PDB.
#[derive(Clone, Debug)]
pub struct PdbParts {
    /// The metadata version string without its terminator. The default is
    /// [`PDB_VERSION`] (`PDB v1.0`).
    pub version: Vec<u8>,
    /// The entry point method of the assembly, or `None`.
    pub entry_point: Option<MethodDefId>,
    /// The row counts of the external type-system tables.
    pub external: ExternalRowCounts,
    /// The `#Strings` heap.
    pub strings: StringHeap,
    /// The `#US` heap.
    pub user_strings: UserStringHeap,
    /// The `#Blob` heap.
    pub blobs: BlobHeap,
    /// The `#GUID` heap.
    pub guids: GuidHeap,
    /// The rows of the debug tables.
    pub tables: DebugTables,
}

impl Default for PdbParts {
    fn default() -> Self {
        Self {
            version: PDB_VERSION.to_vec(),
            entry_point: None,
            external: ExternalRowCounts::new(),
            strings: StringHeap::default(),
            user_strings: UserStringHeap::new(),
            blobs: BlobHeap::new(),
            guids: GuidHeap::new(),
            tables: DebugTables::default(),
        }
    }
}
