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

/// Define an incremental digest of data that arrives in pieces.
macro_rules! hasher {
    ($(#[$doc:meta])* $name:ident, $digest:ident, $size:literal) => {
        $(#[$doc])*
        #[derive(Clone, Default)]
        pub(crate) struct $name($digest);

        impl $name {
            /// Create a hasher with no input.
            pub(crate) fn new() -> Self {
                Self::default()
            }

            /// Add `data` to the input.
            pub(crate) fn update(&mut self, data: &[u8]) {
                self.0.update(data);
            }

            /// Return the digest of all input.
            pub(crate) fn finalize(self) -> [u8; $size] {
                self.0.finalize().into()
            }
        }
    };
}

hasher!(
    /// An incremental SHA-256 digest.
    Sha256Hasher,
    Sha256,
    32
);
hasher!(
    /// An incremental SHA-384 digest.
    Sha384Hasher,
    Sha384,
    48
);
hasher!(
    /// An incremental SHA-512 digest.
    Sha512Hasher,
    Sha512,
    64
);

#[cfg(test)]
mod tests;
