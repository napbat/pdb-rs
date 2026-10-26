//! Portable PDB writing: new layouts from raw parts.
//!
//! # Write operations
//!
//! | Operation | Guarantee | Not guaranteed |
//! | --- | --- | --- |
//! | [`io::copy_exact`](crate::io::copy_exact) with [`Pdb::source`](crate::portable::Pdb::source) | Every source byte, including padding and unreferenced heap data | Any validation of the source beyond the open checks |
//! | [`PdbParts::plan`] | A Portable PDB with the given rows and heaps, in a checked and deterministic layout | Byte-identical output for input from other producers |
//!
//! [`Pdb::to_parts`](crate::portable::Pdb::to_parts) reads all rows and heap entries of a Portable PDB into new heaps. The
//! row IDs and the row order do not change. A plan of these parts writes the same records.
//!
//! # Layout
//!
//! [`PdbParts::plan`] checks all parts and computes the full layout before it writes a byte.
//! It uses the layout of `System.Reflection.Metadata` (`PortablePdbBuilder`):
//!
//! 1. The metadata root (version 1.1) has six stream headers in this order: `#Pdb`, `#~`,
//!    `#Strings`, `#US`, `#GUID` and `#Blob`. Each stream starts at a 4-byte boundary.
//!    Zero bytes pad each stream to a multiple of 4.
//! 2. `#Pdb` has the PDB ID, the entry point and the external tables with a row count other
//!    than 0.
//! 3. `#~` has version 2.0 and the reserved byte 1. `Valid` has the debug tables with rows.
//!    `Sorted` has `LocalScope`, `StateMachineMethod` and `CustomDebugInformation` if they
//!    have rows. One zero byte follows the rows.
//! 4. A heap index has 4 bytes if the heap has more than `0xFFFF` bytes without padding. A
//!    table index has 4 bytes if the table has `0x10000` rows or more. The row count of a
//!    type-system table is its external row count. A `HasCustomDebugInformation` coded index
//!    has 4 bytes if one of its tables has `0x800` rows or more.
//!
//! The same parts and the same [`IdentityPolicy`](crate::portable::IdentityPolicy) thus give the same bytes.
//!
//! # Checks
//!
//! The plan checks the metadata version string, the external row counts, the entry point and
//! each row:
//!
//! - Each heap reference: string offsets inside `#Strings`, blob offsets at the start of a
//!   `#Blob` entry, GUID indexes inside `#GUID`.
//! - Each row reference, list start and coded parent. Nil is valid only where the
//!   specification allows it. A document name is not nil.
//! - `MethodDebugInformation` has no rows or one row for each external `MethodDef` row.
//! - The order of `LocalScope` (method, start offset, then length with the longest first),
//!   non-decreasing variable and constant lists, `StateMachineMethod` (unique `MoveNext`
//!   methods in ascending order, unique kickoff methods) and `CustomDebugInformation`
//!   (coded parent in ascending order).
//! - Import scope parents do not form a cycle.
//!
//! The plan does not decode blobs.
//!
//! # Identity
//!
//! The plan sets the PDB ID from its [`IdentityPolicy`](crate::portable::IdentityPolicy). [`PdbWritePlan::identity`](crate::portable::PdbWritePlan::identity) and
//! [`PdbWritePlan::codeview`](crate::portable::PdbWritePlan::codeview) return the values for the debug directory of the assembly.
//! [`PdbWritePlan::checksum`](crate::portable::PdbWritePlan::checksum) returns the PDB checksum. Updating the assembly is the task of
//! the caller.
//!
//! # Errors and partial output
//!
//! A planning error writes nothing. [`PdbWritePlan::write_to`](crate::portable::PdbWritePlan::write_to) writes in file order. An output
//! error can leave a partial output in the writer. Under `std`, `PdbWritePlan::write_file` and
//! `io::copy_exact_file` write a temporary file and rename it. On error, they delete the
//! temporary file and do not change the destination. Writing over the source file is not
//! supported.

mod check;
mod heaps;
mod identity;
mod layout;
mod parts;
pub(super) mod plan;
mod rebuild;

pub use heaps::{BlobHeap, GuidHeap, StringHeap, StringHeapBuilder, UserStringHeap};
pub use identity::{ChecksumAlgorithm, CodeViewFields, PdbChecksum};
pub use parts::{DebugTables, ExternalRowCounts, PDB_VERSION, PdbParts};
