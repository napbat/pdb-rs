//! The content-derived ID, the PDB checksum and the PE matching fields.

use alloc::vec::Vec;

use crate::crypto::{Sha256Hasher, Sha384Hasher, Sha512Hasher};
use crate::portable::Identity;

/// Make the content-derived ID from the SHA-256 hash of the output with a zero ID.
pub(super) fn content_id(hash: &[u8; 32]) -> Identity {
    let mut guid = [0; 16];
    guid.copy_from_slice(&hash[..16]);
    guid[7] = (guid[7] & 0x0F) | 0x40;
    guid[8] = (guid[8] & 0x3F) | 0x80;
    let stamp = u32::from_le_bytes([hash[16], hash[17], hash[18], hash[19]]) | 0x8000_0000;
    Identity::new(guid, stamp)
}

/// The fields of the portable `CodeView` debug directory entry of the assembly.
///
/// The entry has the type 2 and the minor version `0x504D`. Its data is `RSDS`, the GUID, the
/// age and the PDB path. The stamp goes into the `TimeDateStamp` field of the debug directory
/// entry. The embedded Portable PDB entry (type 17) has the stamp 0, not this stamp.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct CodeViewFields {
    /// The GUID in its serialized byte order.
    pub guid: [u8; 16],
    /// The age. It is always 1 for a Portable PDB.
    pub age: u32,
    /// The `TimeDateStamp` of the debug directory entry.
    pub stamp: u32,
}

impl From<Identity> for CodeViewFields {
    fn from(identity: Identity) -> Self {
        Self {
            guid: *identity.guid(),
            age: 1,
            stamp: identity.stamp(),
        }
    }
}

/// A hash algorithm of the PDB checksum debug directory entry (type 19).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ChecksumAlgorithm {
    /// SHA-256 (32 bytes).
    Sha256,
    /// SHA-384 (48 bytes).
    Sha384,
    /// SHA-512 (64 bytes).
    Sha512,
}

impl ChecksumAlgorithm {
    /// Return the case-sensitive algorithm name of the entry, for example `SHA256`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sha256 => "SHA256",
            Self::Sha384 => "SHA384",
            Self::Sha512 => "SHA512",
        }
    }

    /// Return the digest size in bytes.
    #[must_use]
    pub const fn size(self) -> usize {
        match self {
            Self::Sha256 => 32,
            Self::Sha384 => 48,
            Self::Sha512 => 64,
        }
    }
}

/// The checksum of a Portable PDB: a digest of all bytes with the 20 ID bytes set to zero.
///
/// The CLI PE/COFF addendum specifies this input for the PDB checksum debug directory entry
/// (type 19, version 1.0).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdbChecksum {
    algorithm: ChecksumAlgorithm,
    digest: [u8; 64],
}

impl PdbChecksum {
    /// Return the hash algorithm.
    #[must_use]
    pub const fn algorithm(&self) -> ChecksumAlgorithm {
        self.algorithm
    }

    /// Return the digest bytes.
    #[must_use]
    pub fn digest(&self) -> &[u8] {
        &self.digest[..self.algorithm.size()]
    }

    /// Return the data of the debug directory entry: the algorithm name, a zero byte and the
    /// digest.
    #[must_use]
    pub fn entry_data(&self) -> Vec<u8> {
        let name = self.algorithm.name().as_bytes();
        let mut data = Vec::with_capacity(name.len() + 1 + self.algorithm.size());
        data.extend_from_slice(name);
        data.push(0);
        data.extend_from_slice(self.digest());
        data
    }
}

/// Compute a checksum over `pieces` in order.
pub(super) fn checksum<'a>(
    algorithm: ChecksumAlgorithm,
    pieces: impl IntoIterator<Item = &'a [u8]>,
) -> PdbChecksum {
    let mut digest = [0; 64];
    match algorithm {
        ChecksumAlgorithm::Sha256 => {
            digest[..32].copy_from_slice(&sha256(pieces));
        }
        ChecksumAlgorithm::Sha384 => {
            let mut hasher = Sha384Hasher::new();
            pieces.into_iter().for_each(|piece| hasher.update(piece));
            digest[..48].copy_from_slice(&hasher.finalize());
        }
        ChecksumAlgorithm::Sha512 => {
            let mut hasher = Sha512Hasher::new();
            pieces.into_iter().for_each(|piece| hasher.update(piece));
            digest = hasher.finalize();
        }
    }
    PdbChecksum { algorithm, digest }
}

/// Compute the SHA-256 digest of `pieces` in order.
pub(super) fn sha256<'a>(pieces: impl IntoIterator<Item = &'a [u8]>) -> [u8; 32] {
    let mut hasher = Sha256Hasher::new();
    pieces.into_iter().for_each(|piece| hasher.update(piece));
    hasher.finalize()
}
