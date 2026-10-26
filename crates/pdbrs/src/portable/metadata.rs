//! Portable PDB metadata: the metadata root, the heaps, the `#Pdb` stream and the `#~` tables.
//!
//! A Portable PDB uses the physical layout of ECMA-335 partition II, chapter 24, with the
//! debug tables of the Portable PDB specification (`PortablePdb-Metadata.md` in
//! `dotnet/runtime`, commit `736475ca`). The `#~` stream of a standalone Portable PDB holds
//! only the eight debug tables. The `#Pdb` stream gives the row counts of the type-system
//! tables of the assembly. These external counts set the widths of references to those
//! tables, so the reader needs no assembly.
//!
//! The types of this module are the validated headers that [`Pdb`](super::Pdb) reads at
//! open, the typed IDs and the raw rows of the debug tables.
//!
//! - [`MetadataRoot`]: the version string and the stream headers.
//! - [`PdbStream`]: the PDB ID, the entry point and the external row counts.
//! - [`TableStream`]: the `#~` header, the table masks, the row counts, the index widths
//!   and the table layout.
//! - [`TableRow`] and the row types, for example [`DocumentRow`].
//!
//! Heap offsets, GUID indexes, row IDs and metadata tokens have distinct types.

mod coded;
pub(crate) mod heaps;
mod ids;
mod pdb_stream;
mod root;
mod rows;
mod tables;

pub use coded::CodedIndex;
pub use ids::{
    AssemblyRefId, BlobOffset, CustomDebugInformationId, DocumentId, GuidIndex, IlOffset,
    ImportScopeId, LocalConstantId, LocalScopeId, LocalVariableId, MAX_ROW, MetadataToken,
    MethodDebugInformationId, MethodDefId, StandAloneSigId, StateMachineMethodId, StringOffset,
    TableId, UserStringOffset,
};
pub use pdb_stream::PdbStream;
pub use root::{METADATA_SIGNATURE, MetadataRoot, MetadataStream, StreamHeader};
pub use rows::{
    CustomDebugInformationRow, DocumentRow, ImportScopeRow, LocalConstantRow, LocalScopeRow,
    LocalVariableRow, MethodDebugInformationRow, StateMachineMethodRow, TableRow,
};
pub(crate) use rows::{RowReader, decode_row};
pub use tables::TableStream;
