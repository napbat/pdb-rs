//! Container signature identification and format dispatch.
//!
//! [`probe`] identifies a container from the signature at the start of an input. It does not
//! parse or validate the container contents.
//!
//! [`Pdb`] detects the PDB format from the contents and opens the input with the matching
//! backend: [`native::Pdb`] or [`portable::Pdb`]. It does not use the file name.
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
//!
//! [`Pdb`] reports a Portable PDB only if the metadata has valid `#Pdb` and `#~` streams.

use alloc::boxed::Box;

use crate::io::ReadAt;
use crate::{Error, ErrorKind, Result, native, portable};

/// A PDB format that [`Pdb`] detects from the contents of its input.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Format {
    /// A native PDB in an MSF 7.00 or an MSFZ container.
    Native,
    /// A standalone Portable PDB.
    Portable,
}

/// A PDB of either format, opened with the backend that its contents select.
///
/// Use the root facade to detect the format. Then match the backend for format-specific
/// operations. Each variant boxes its reader, so the two readers can have different sizes.
#[derive(Debug)]
#[non_exhaustive]
pub enum Pdb<S> {
    /// A native PDB.
    Native(Box<native::Pdb<S>>),
    /// A standalone Portable PDB.
    Portable(Box<portable::Pdb<S>>),
}

impl<S: ReadAt> Pdb<S> {
    /// Detect the format of a stable positional source and open it with default limits.
    ///
    /// # Errors
    ///
    /// - The errors of [`probe`] if the input has no known signature.
    /// - [`ErrorKind::UnsupportedVersion`] for a legacy MSF 2.00 PDB.
    /// - [`ErrorKind::InvalidFormat`] for an `MPDB` payload. Decode it with
    ///   [`portable::embedded::decode_embedded`] first.
    /// - [`ErrorKind::MissingStream`] for CLI metadata without a `#Pdb` stream, for example
    ///   the metadata of an assembly. The error context tells that it is not a Portable PDB.
    /// - The errors of [`native::Pdb::read`] or [`portable::Pdb::read`].
    pub fn read(source: S) -> Result<Self> {
        match probe(&source)? {
            ContainerKind::NativeMsf7 | ContainerKind::NativeMsfz => native::Pdb::read(source)
                .map(|pdb| Self::Native(Box::new(pdb))),
            ContainerKind::CliMetadata => portable::Pdb::read(source)
                .map(|pdb| Self::Portable(Box::new(pdb))),
            ContainerKind::LegacyMsf2 => Err(Error::new(ErrorKind::UnsupportedVersion, Some(0))
                .with_context("legacy MSF 2.00 PDBs are recognized but not supported")),
            ContainerKind::EmbeddedPortablePdb => Err(Error::new(ErrorKind::InvalidFormat, Some(0))
                .with_context(
                    "an MPDB payload is an embedded Portable PDB; decode it with portable::embedded::decode_embedded",
                )),
        }
    }

    /// Return the detected format.
    #[must_use]
    pub const fn format(&self) -> Format {
        match self {
            Self::Native(_) => Format::Native,
            Self::Portable(_) => Format::Portable,
        }
    }
}

impl<'a> Pdb<&'a [u8]> {
    /// Detect the format of borrowed bytes and open them.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Pdb::read`].
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::read(bytes)
    }
}

#[cfg(all(feature = "std", any(unix, windows)))]
impl Pdb<crate::io::FileReader> {
    /// Open a file and detect its format from the contents.
    ///
    /// Requires the `std` feature and Unix or Windows.
    ///
    /// # Errors
    ///
    /// Returns file I/O errors and the errors of [`Pdb::read`].
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Self::read(crate::io::FileReader::open(path)?)
    }
}

/// The signature of a native MSF 7.00 container (32 bytes).
const NATIVE_MSF7_SIGNATURE: &[u8] = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0";

/// The signature of a native MSFZ container (32 bytes).
const NATIVE_MSFZ_SIGNATURE: &[u8] = b"Microsoft MSFZ Container\r\n\x1aALD\0\0";

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
const KINDS: [ContainerKind; 5] = [
    ContainerKind::NativeMsf7,
    ContainerKind::NativeMsfz,
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
    /// A native PDB in the MSFZ container (a PDZ file).
    ///
    /// The signature is `b"Microsoft MSFZ Container\r\n\x1aALD\0\0"` (32 bytes).
    NativeMsfz,
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
            Self::NativeMsfz => NATIVE_MSFZ_SIGNATURE,
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
