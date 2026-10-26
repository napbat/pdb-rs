//! Native PDB containers, streams and record views.
//!
//! Native PDBs use the MSF 7.00 container ([`msf`]) or the MSFZ container ([`msfz`]).
//! [`container::Container`] reads streams from both. The stream and record decoders use only
//! the stream bytes.
//!
//! Container versions, stream versions and record kinds are independent.
//! Typed decoders report unsupported versions rather than guessing a layout.
//!
//! [`PdbBuilder`] builds a new native PDB and [`PdbContents`] reads one for a semantic
//! rebuild. [`Pdb::edit`] makes a [`PdbEditor`] that changes an existing PDB. Both make a
//! plan ([`PdbWritePlan`] or [`PdbEditPlan`]) that checks all changes before it writes a byte.

pub mod address;
pub mod auxiliary;
mod build;
pub mod codeview;
pub mod container;
pub mod dbi;
pub mod debug_stream;
mod edit;
pub mod external;
pub mod fpo;
pub mod hash;
pub mod hash_table;
pub mod info;
pub mod injected_source;
pub mod lines;
pub mod locate;
pub mod msf;
pub mod msfz;
pub mod names;
mod output;
pub mod section;
pub mod symbols;
pub mod types;
mod words;
pub mod write;

mod producer;
mod reader;
pub use build::{BuildOptions, PdbBuilder, PdbContents, PdbWritePlan};
pub use edit::{EditOptions, PdbEditPlan, PdbEditor};
pub use producer::Producer;
pub use reader::{Pdb, ReadOptions};

/// The matching identity of a native PDB and its PE debug reference.
///
/// Compare values only after decoding the matching identity from each source.
/// A matching identity does not prove that file contents are identical.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Identity {
    /// The GUID-and-age identity used by an `RSDS` reference.
    Modern {
        /// The GUID bytes in their serialized PDB order, not UUID text order.
        guid: [u8; 16],
        /// The PDB revision counter.
        age: u32,
    },
    /// The signature-and-age identity used by an `NB10` reference.
    Legacy {
        /// The original 32-bit signature; it is not a GUID.
        signature: u32,
        /// The PDB revision counter.
        age: u32,
    },
}

impl Identity {
    /// Return the revision counter for either native identity scheme.
    #[must_use]
    pub const fn age(self) -> u32 {
        match self {
            Self::Modern { age, .. } | Self::Legacy { age, .. } => age,
        }
    }
}

/// The identity change that a writer applies to a PDB.
///
/// The PDB Info stream and the DBI header both store an age. An image matches a PDB if the
/// GUID and the age of its `RSDS` debug entry equal the GUID and the DBI age.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum IdentityPolicy {
    /// Keep the signature, the GUID and both ages. The image keeps matching.
    Preserve,
    /// Add 1 to the Info age and write the new age to the DBI header too, as an incremental
    /// link does. The image must get the new age.
    IncrementAge,
    /// Write a new identity. The DBI header gets the same age.
    Replace {
        /// The 32-bit signature (often a time stamp).
        signature: u32,
        /// The GUID bytes in their serialized order.
        guid: [u8; 16],
        /// The new age.
        age: u32,
    },
}
