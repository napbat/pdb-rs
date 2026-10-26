//! Container signature identification.
//!
//! This module identifies a container from the signature at the start of an input. It does not
//! parse or validate the container contents.
//!
//! # Limits of validation
//!
//! [`probe`] compares only the first bytes of the input with a fixed list of signatures. It reads
//! at most [`PROBE_LEN`] bytes from offset 0. It does not interpret the bytes after the signature.
//!
//! - A [`ContainerKind`] result does not mean that the container is valid or complete.
//! - [`ContainerKind::CliMetadata`] does not mean that the input is a Portable PDB. An ECMA-335
//!   metadata root in a CLI assembly has the same signature.
//! - A match does not look at the remaining file. Thus a file that has a signature but no
//!   payload gets a [`ContainerKind`] result.

use crate::io::ReadAt;
use crate::{Error, ErrorKind, Result};

/// The signature of a native MSF 7.00 container (32 bytes).
const NATIVE_MSF7_SIGNATURE: &[u8] = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0";

/// The signature of a legacy small MSF 2.00 container (44 bytes).
const LEGACY_MSF2_SIGNATURE: &[u8] = b"Microsoft C/C++ program database 2.00\r\n\x1aJG\0\0";

/// The signature of an ECMA-335 metadata root (`BSJB`, 4 bytes).
const CLI_METADATA_SIGNATURE: &[u8] = b"BSJB";

/// The signature of an embedded Portable PDB debug directory entry (`MPDB`, 4 bytes).
const EMBEDDED_PORTABLE_PDB_SIGNATURE: &[u8] = b"MPDB";

/// The maximum number of bytes that [`probe`] reads.
///
/// This value is the length of the longest known signature.
pub const PROBE_LEN: usize = LEGACY_MSF2_SIGNATURE.len();

/// All container kinds that [`probe`] can identify.
const KINDS: [ContainerKind; 4] = [
    ContainerKind::NativeMsf7,
    ContainerKind::LegacyMsf2,
    ContainerKind::CliMetadata,
    ContainerKind::EmbeddedPortablePdb,
];

/// A container kind that a signature at offset 0 identifies.
///
/// Each value tells only which signature matched. It does not tell that the container is valid.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ContainerKind {
    /// A native PDB in the MSF 7.00 ("big MSF") container.
    ///
    /// The signature is `b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0"` (32 bytes).
    NativeMsf7,
    /// A legacy native PDB in the MSF 2.00 ("small MSF") container.
    ///
    /// The signature is `b"Microsoft C/C++ program database 2.00\r\n\x1aJG\0\0"` (44 bytes).
    LegacyMsf2,
    /// An ECMA-335 metadata root with the `BSJB` signature (4 bytes).
    ///
    /// A Portable PDB and the metadata of a CLI assembly both start with this signature.
    /// This value does not mean that the input is a Portable PDB.
    CliMetadata,
    /// The data of an embedded Portable PDB debug directory entry with the `MPDB` signature
    /// (4 bytes).
    ///
    /// In a valid entry, a compressed Portable PDB follows the signature. [`probe`] does not
    /// examine those bytes.
    EmbeddedPortablePdb,
}

impl ContainerKind {
    /// Return the full signature that identifies this container kind.
    #[must_use]
    pub const fn signature(self) -> &'static [u8] {
        match self {
            Self::NativeMsf7 => NATIVE_MSF7_SIGNATURE,
            Self::LegacyMsf2 => LEGACY_MSF2_SIGNATURE,
            Self::CliMetadata => CLI_METADATA_SIGNATURE,
            Self::EmbeddedPortablePdb => EMBEDDED_PORTABLE_PDB_SIGNATURE,
        }
    }
}

/// Identify the container kind from the signature at offset 0 of `source`.
///
/// This function does signature identification only. It does not parse or validate a PDB.
/// It reads `source.len()` once. Then it reads at most [`PROBE_LEN`] bytes from offset 0.
/// It does not allocate memory.
///
/// A full signature match gives a [`ContainerKind`]. The bytes after the signature can have any
/// value, and they can be absent. [`ContainerKind::CliMetadata`] does not mean that the input is
/// a Portable PDB.
///
/// # Errors
///
/// - [`ErrorKind::UnexpectedEof`] if the input is shorter than a signature and all input bytes
///   are a prefix of that signature. This includes empty input. The error offset is the input
///   length.
/// - [`ErrorKind::InvalidSignature`] if the input does not start with a known signature or a
///   prefix of one. The error offset is 0.
/// - An error from `source` if [`ReadAt::len`] or [`ReadAt::read_exact_at`] fails.
pub fn probe(source: &(impl ReadAt + ?Sized)) -> Result<ContainerKind> {
    let length = source.len()?;
    let available = usize::try_from(length).map_or(PROBE_LEN, |length| length.min(PROBE_LEN));

    let mut buffer = [0_u8; PROBE_LEN];
    let prefix = &mut buffer[..available];
    source.read_exact_at(0, prefix)?;
    let prefix = &*prefix;

    if let Some(kind) = KINDS
        .into_iter()
        .find(|kind| prefix.starts_with(kind.signature()))
    {
        return Ok(kind);
    }

    if KINDS
        .into_iter()
        .any(|kind| kind.signature().starts_with(prefix))
    {
        return Err(Error::new(ErrorKind::UnexpectedEof, Some(length)));
    }

    Err(Error::new(ErrorKind::InvalidSignature, Some(0)))
}
