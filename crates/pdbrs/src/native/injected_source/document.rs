//! .NET document records in `/src/files/` streams (compression byte 101).
//!
//! The .NET symbol writers store one record for each source document of a
//! Windows PDB. The `/src/headerblock` entry has the compression byte 101
//! (LLVM `PDB_SourceCompression::DotNet`). The stream holds:
//!
//! | Offset | Field |
//! | --- | --- |
//! | 0 | Language GUID |
//! | 16 | Language vendor GUID |
//! | 32 | Document type GUID |
//! | 48 | Checksum algorithm GUID |
//! | 64 | `u32` checksum size |
//! | 68 | `u32` embedded source size; 0 means that no source is embedded |
//! | 72 | Checksum bytes, then the embedded-source blob |
//!
//! The embedded-source blob has the Portable PDB format:
//! [`EmbeddedSource`]. The GUIDs are in serialized order.

use crate::crypto::sha256;
use crate::native::codeview::Fields;
use crate::portable::embedded::EmbeddedSource;
use crate::{Error, ErrorKind, Result};

/// The language GUID of C# (`3f5162f8-07c6-11d3-9053-00c04fa302a1`).
pub const LANGUAGE_CSHARP: [u8; 16] = [
    0xf8, 0x62, 0x51, 0x3f, 0xc6, 0x07, 0xd3, 0x11, 0x90, 0x53, 0x00, 0xc0, 0x4f, 0xa3, 0x02, 0xa1,
];

/// The language vendor GUID of Microsoft (`994b45c4-e6e9-11d2-903f-00c04fa302a1`).
pub const VENDOR_MICROSOFT: [u8; 16] = [
    0xc4, 0x45, 0x4b, 0x99, 0xe9, 0xe6, 0xd2, 0x11, 0x90, 0x3f, 0x00, 0xc0, 0x4f, 0xa3, 0x02, 0xa1,
];

/// The document type GUID of a text file (`5a869d0b-6611-11d3-bd2a-0000f80849bd`).
pub const DOCUMENT_TYPE_TEXT: [u8; 16] = [
    0x0b, 0x9d, 0x86, 0x5a, 0x11, 0x66, 0xd3, 0x11, 0xbd, 0x2a, 0x00, 0x00, 0xf8, 0x08, 0x49, 0xbd,
];

/// The checksum algorithm GUID of SHA-1 (`ff1816ec-aa5e-4d10-87f7-6f4963833460`).
pub const CHECKSUM_SHA1: [u8; 16] = [
    0xec, 0x16, 0x18, 0xff, 0x5e, 0xaa, 0x10, 0x4d, 0x87, 0xf7, 0x6f, 0x49, 0x63, 0x83, 0x34, 0x60,
];

/// The checksum algorithm GUID of SHA-256 (`8829d00f-11b8-4213-878b-770e8597ac16`).
pub const CHECKSUM_SHA256: [u8; 16] = [
    0x0f, 0xd0, 0x29, 0x88, 0xb8, 0x11, 0x13, 0x42, 0x87, 0x8b, 0x77, 0x0e, 0x85, 0x97, 0xac, 0x16,
];

/// The error context for a record that is too short for its fields.
const SHORT_RECORD: &str = ".NET document record is shorter than its fields";

/// The result of a checksum check.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ChecksumCheck {
    /// The algorithm is SHA-256 and the checksum matches the source.
    Verified,
    /// This crate does not compute the checksum algorithm, for example SHA-1.
    NotVerified,
}

/// A decoded .NET document record, borrowed from the stream bytes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DotNetDocument<'a> {
    /// The language GUID, for example [`LANGUAGE_CSHARP`].
    pub language: [u8; 16],
    /// The language vendor GUID, for example [`VENDOR_MICROSOFT`].
    pub language_vendor: [u8; 16],
    /// The document type GUID, for example [`DOCUMENT_TYPE_TEXT`].
    pub document_type: [u8; 16],
    /// The checksum algorithm GUID, for example [`CHECKSUM_SHA256`].
    pub checksum_algorithm: [u8; 16],
    /// The checksum bytes.
    pub checksum: &'a [u8],
    /// The embedded source, if the record has one.
    pub embedded: Option<EmbeddedSource<'a>>,
}

impl<'a> DotNetDocument<'a> {
    /// Decode the record in `bytes`: the full stream contents.
    ///
    /// The stream must end exactly after the checksum and the embedded source.
    ///
    /// # Errors
    ///
    /// The errors have no location; the caller adds it.
    ///
    /// - [`ErrorKind::InvalidFormat`] if the stream is too short for its fields
    ///   or has bytes after the embedded source.
    /// - The errors of [`EmbeddedSource::parse`] for the embedded source.
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut fields = Fields::new(bytes, ErrorKind::InvalidFormat, SHORT_RECORD);
        let mut guid = || -> Result<[u8; 16]> {
            let mut value = [0; 16];
            value.copy_from_slice(fields.bytes(16)?);
            Ok(value)
        };
        let language = guid()?;
        let language_vendor = guid()?;
        let document_type = guid()?;
        let checksum_algorithm = guid()?;
        let checksum_size = fields.u32()?;
        let source_size = fields.u32()?;
        let checksum = fields.bytes(to_usize(checksum_size)?)?;
        let embedded = match source_size {
            0 => None,
            size => Some(EmbeddedSource::parse(fields.bytes(to_usize(size)?)?)?),
        };
        if !fields.is_empty() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("bytes follow the .NET document record"));
        }
        Ok(Self {
            language,
            language_vendor,
            document_type,
            checksum_algorithm,
            checksum,
            embedded,
        })
    }

    /// Check [`DotNetDocument::checksum`] against the decoded `source`.
    ///
    /// Only SHA-256 is computed. For other algorithms, the function returns
    /// [`ChecksumCheck::NotVerified`] and does not report an error.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the algorithm is SHA-256 and
    /// the checksum differs from the digest of `source`. The error has no
    /// location.
    pub fn verify_checksum(&self, source: &[u8]) -> Result<ChecksumCheck> {
        if self.checksum_algorithm != CHECKSUM_SHA256 {
            return Ok(ChecksumCheck::NotVerified);
        }
        if self.checksum != sha256(source).as_slice() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context(".NET document SHA-256 checksum differs from the source"));
        }
        Ok(ChecksumCheck::Verified)
    }
}

/// Convert a size field to `usize`.
fn to_usize(value: u32) -> Result<usize> {
    usize::try_from(value)
        .map_err(|_| Error::new(ErrorKind::InvalidFormat, None).with_context(SHORT_RECORD))
}
