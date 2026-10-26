//! Identity types for modern and legacy native PDB files.
//!
//! Container versions, stream versions and record kinds are independent.
//! These identity values do not imply that a corresponding codec is available.

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
