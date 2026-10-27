//! The Portable PDB backend: the reader, the editor, the identity and shared codecs.
//!
//! Portable PDBs use CLI metadata, not MSF. [`Pdb`] reads a standalone
//! Portable PDB: [`metadata`] has the metadata root, the heaps, the `#Pdb`
//! stream, the `#~` table layout and the raw rows of the eight debug tables.
//! The 20-byte identity combines a GUID with a stamp. The stamp is not a
//! native PDB age.
//!
//! [`compressed`] decodes and encodes the compressed integers of metadata
//! blobs. [`debug`] has the records, the blob codecs and the validation of
//! the debug tables. [`custom`] has the custom debug information record and
//! decodes and encodes custom debug information blobs.
//! [`embedded`] decodes and encodes embedded Portable PDB payloads and
//! embedded-source blobs. Windows PDB document records use the same
//! embedded-source blob format. Decode an `MPDB` payload with
//! [`embedded::decode_embedded`], then read the result with [`Pdb::parse`].
//!
//! [`PdbEditor`] builds and edits Portable PDBs from records: [`PdbEditor::new`] starts
//! without a source, and [`Pdb::edit`] starts from a source. [`PdbEditor::plan`] makes a
//! [`PdbWritePlan`] with the output, the identity and a [`PlanReport`].
//! [`write`](mod@write) has the raw parts and heaps of new layouts: [`write::PdbParts::plan`]
//! makes a plan from raw rows, and [`Pdb::to_parts`] reads a Portable PDB into such parts.
//! For an exact copy, give [`Pdb::source`] to [`io::copy_exact`](crate::io::copy_exact).
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use pdb_rs::portable::debug::{Document, LocalScope};
//! use pdb_rs::portable::metadata::{IlOffset, MethodDefId, TableId};
//! use pdb_rs::portable::write::ExternalRowCounts;
//! use pdb_rs::portable::{EditOptions, IdentityPolicy, Pdb, PdbEditor};
//!
//! // Read: records with their row IDs.
//! let bytes = std::fs::read("app.pdb")?;
//! let pdb = Pdb::parse(&bytes)?;
//! for document in pdb.documents() {
//!     let (id, document) = document?;
//!     println!("{} {:?}", id.0, document.name);
//! }
//!
//! // Edit: the identity policy at the entry.
//! let mut edit = pdb.edit(IdentityPolicy::ContentDerived);
//! for (_, document) in edit.documents_mut()?.iter_mut() {
//!     document.name = document.name.to_ascii_lowercase().into();
//! }
//! let plan = edit.plan(EditOptions::default())?;
//! std::fs::write("app.edited.pdb", plan.to_vec()?)?;
//! // Write the new identity into the CodeView debug entry of the assembly.
//! let identity = plan.identity();
//!
//! // Build: the same type without a source.
//! let mut external = ExternalRowCounts::new();
//! external.set(TableId::METHOD_DEF, 1);
//! let mut build = PdbEditor::new(external, IdentityPolicy::Explicit(identity));
//! let mut document = Document::default();
//! document.name = b"/src/Program.cs".into();
//! build.documents_mut()?.push(document)?;
//! let mut scope = LocalScope::default();
//! scope.method = MethodDefId(1);
//! scope.start_offset = IlOffset(0);
//! scope.length = 10;
//! build.local_scopes_mut()?.push(scope)?;
//! let plan = build.plan(EditOptions::default())?;
//! for (stream, change) in plan.report().streams() {
//!     println!("{stream:?}: {change:?}");
//! }
//! # Ok(())
//! # }
//! ```

pub mod compressed;
pub mod custom;
pub mod debug;
pub mod editor;
pub mod embedded;
pub mod metadata;
mod reader;
pub mod write;

pub use editor::model::{EditOptions, PdbEditor};
pub use reader::{Pdb, ReadOptions, Rows};
pub use write::plan::PdbWritePlan;
pub use write::report::{PlanReport, RebuildReason, StreamChange};

/// How a write plan sets the 20-byte PDB ID.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum IdentityPolicy {
    /// Use the given ID.
    Explicit(Identity),
    /// Derive the ID from the output, as `System.Reflection.Metadata` does for deterministic
    /// builds (`BlobContentId.FromHash`).
    ///
    /// The hash is SHA-256 over all output bytes with the 20 ID bytes set to zero. The GUID is
    /// hash bytes 0 to 15, with the high nibble of byte 7 set to 4 (the RFC 4122 version) and
    /// the two high bits of byte 8 set to `10` (the variant). The stamp is hash bytes 16 to
    /// 19 as a little-endian value with bit 31 set. A change to the layout or to any byte
    /// changes the ID, also if the decoded records stay the same.
    ContentDerived,
}

/// The 20-byte identity stored at the start of a Portable PDB's `#Pdb` stream.
///
/// The GUID and stamp also appear in the portable `CodeView` reference and its
/// PE debug-directory entry. The reference's age must be 1.
/// This value does not compute a content hash or prove content integrity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Identity {
    guid: [u8; 16],
    stamp: u32,
}

impl Identity {
    /// Construct an identity from serialized GUID bytes and a numeric stamp.
    ///
    /// The GUID bytes use PDB serialization order, not UUID text order.
    #[must_use]
    pub const fn new(guid: [u8; 16], stamp: u32) -> Self {
        Self { guid, stamp }
    }

    /// Decode the identity bytes from a `#Pdb` stream.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 20]) -> Self {
        let mut guid = [0; 16];
        guid.copy_from_slice(&bytes[..16]);
        let stamp = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        Self { guid, stamp }
    }

    /// Encode the identity for a `#Pdb` stream without changing its GUID order.
    #[must_use]
    pub fn to_bytes(self) -> [u8; 20] {
        let mut bytes = [0; 20];
        bytes[..16].copy_from_slice(&self.guid);
        bytes[16..].copy_from_slice(&self.stamp.to_le_bytes());
        bytes
    }

    /// Return the GUID bytes in their serialized order.
    #[must_use]
    pub const fn guid(&self) -> &[u8; 16] {
        &self.guid
    }

    /// Return the stamp used by the portable `CodeView` debug-directory entry.
    #[must_use]
    pub const fn stamp(self) -> u32 {
        self.stamp
    }

    /// Check the identity fields from a portable `CodeView` reference.
    ///
    /// Pass the debug-directory stamp, not the embedded PDB entry's zero stamp.
    #[must_use]
    pub fn matches_codeview(self, guid: &[u8; 16], stamp: u32, age: u32) -> bool {
        age == 1 && self.guid == *guid && self.stamp == stamp
    }
}
