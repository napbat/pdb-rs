//! SHA-2 digests for checksums and content IDs.
//!
//! Only this module names `sha2` and `digest`. The module is crate-private. Its
//! functions return plain byte arrays, so no dependency type reaches the public
//! API. Add another digest only when a fixture needs it.

use sha2::{Digest as _, Sha256, Sha384, Sha512};

/// Return the SHA-256 digest of `data`.
pub(crate) fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// Return the SHA-384 digest of `data`.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "first user: #17 Portable PDB checksum entries")
)]
pub(crate) fn sha384(data: &[u8]) -> [u8; 48] {
    Sha384::digest(data).into()
}

/// Return the SHA-512 digest of `data`.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "first user: #17 Portable PDB checksum entries")
)]
pub(crate) fn sha512(data: &[u8]) -> [u8; 64] {
    Sha512::digest(data).into()
}

/// An incremental SHA-256 digest of data that arrives in pieces.
#[derive(Clone, Default)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "first user: #17 Portable PDB content ID of planned output"
    )
)]
pub(crate) struct Sha256Hasher(Sha256);

impl Sha256Hasher {
    /// Create a hasher with no input.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "first user: #17 Portable PDB content ID of planned output"
        )
    )]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Add `data` to the input.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "first user: #17 Portable PDB content ID of planned output"
        )
    )]
    pub(crate) fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    /// Return the SHA-256 digest of all input.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "first user: #17 Portable PDB content ID of planned output"
        )
    )]
    pub(crate) fn finalize(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

#[cfg(test)]
mod tests;
