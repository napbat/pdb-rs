//! Embedded Portable PDBs and the embedded-source blob.
//!
//! # Embedded PDB envelope
//!
//! A PE debug directory entry of type 17 holds a Portable PDB in an `MPDB`
//! envelope:
//!
//! 1. The 4-byte signature `MPDB`.
//! 2. A little-endian `u32` uncompressed size. It must be larger than 0, and
//!    its highest bit must be clear. Other values are reserved for future
//!    formats.
//! 3. The Portable PDB as a raw DEFLATE (RFC 1951) stream.
//!
//! [`decode_embedded`] and [`encode_embedded`] use the data of the debug
//! directory entry. They do not parse or change a PE file. The layout comes
//! from the "Embedded Portable PDB Debug Directory Entry" section of the CLI
//! PE/COFF addendum (`PE-COFF.md` in `dotnet/runtime`, commit `561f89a9`).
//!
//! # Embedded-source blob
//!
//! The Portable PDB embedded-source custom debug information
//! (`0e8a571b-6926-466e-b4ad-8ab04611f5fe`) and the .NET document records in
//! Windows PDB `/src/files/` streams store a source file in one format:
//!
//! 1. A little-endian `i32` format field. A positive value is the size of the
//!    decoded source, and a negative value is reserved for future formats.
//! 2. If the format is 0, the raw source bytes. If it is positive, a raw
//!    DEFLATE (RFC 1951) stream that decodes to exactly the declared size.
//!
//! The layout comes from the "Embedded Source" section of the Portable PDB
//! specification (`PortablePdb-Metadata.md` in `dotnet/runtime`).
//!
//! The two formats are different. Each one has its own size checks.
//!
//! # Embedded source of a document
//!
//! [`Pdb::document_source`](crate::portable::Pdb::document_source) finds the
//! embedded-source custom debug information of a `Document` row, decodes it
//! and checks it against the document hash.
//! [`Document::verify_source`](crate::portable::debug::Document::verify_source)
//! does the check for a source from another place. Only SHA-256 is computed;
//! other hash algorithms give [`SourceCheck::NotVerified`].

mod source;

use alloc::borrow::Cow;
use alloc::vec::Vec;

pub use source::{DocumentSource, SourceCheck};

use crate::compression::deflate;
use crate::{Error, ErrorKind, Result};

/// The signature at the start of an embedded Portable PDB payload (`MPDB`).
pub const EMBEDDED_PDB_SIGNATURE: [u8; 4] = *b"MPDB";

/// The size of the embedded PDB header: the signature and the uncompressed size.
const EMBEDDED_PDB_HEADER: usize = 8;

/// The size of the format field at the start of a blob.
const FORMAT_FIELD: usize = 4;

/// Decode the Portable PDB in an `MPDB` payload.
///
/// `payload` is the data of a PE debug directory entry of type 17. The
/// function checks the declared size against `limit` before it allocates.
/// Then it decodes into a buffer of exactly the declared size. The result is
/// the bytes of a standalone Portable PDB. The function does not read them.
///
/// # Errors
///
/// The errors have no location; the caller adds it.
///
/// - [`ErrorKind::InvalidSignature`] if `payload` does not start with
///   [`EMBEDDED_PDB_SIGNATURE`].
/// - [`ErrorKind::UnexpectedEof`] if `payload` is shorter than the 8-byte
///   header, or if the DEFLATE stream ends before its final block.
/// - [`ErrorKind::UnsupportedVersion`] if the uncompressed size is 0 or has
///   its highest bit set. These values are reserved for future formats.
/// - [`ErrorKind::LimitExceeded`] if the uncompressed size is larger than
///   `limit`, or if the allocation fails.
/// - [`ErrorKind::InvalidFormat`] if the DEFLATE stream is corrupt, gives a
///   size other than the declared size, or has bytes after its final block.
pub fn decode_embedded(payload: &[u8], limit: usize) -> Result<Vec<u8>> {
    let available = payload.len().min(EMBEDDED_PDB_SIGNATURE.len());
    if payload[..available] != EMBEDDED_PDB_SIGNATURE[..available] {
        return Err(Error::new(ErrorKind::InvalidSignature, None)
            .with_context("embedded PDB payload does not start with MPDB"));
    }
    let (header, data) = payload
        .split_first_chunk::<EMBEDDED_PDB_HEADER>()
        .ok_or_else(|| {
            Error::new(ErrorKind::UnexpectedEof, None)
                .with_context("embedded PDB payload is shorter than its header")
        })?;
    let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if size == 0 || i32::try_from(size).is_err() {
        return Err(Error::new(ErrorKind::UnsupportedVersion, None)
            .with_context("embedded PDB uncompressed size is reserved for future formats"));
    }
    deflate::decode(data, u64::from(size), limit)
}

/// Encode `pdb` as an `MPDB` payload.
///
/// The result is the data of a PE debug directory entry of type 17. The
/// function does not examine `pdb`. It uses the default DEFLATE compression
/// level. The caller writes the debug directory entry: the addendum sets its
/// minor version to `0x0100` and its stamp to 0.
///
/// # Errors
///
/// - [`ErrorKind::InvalidFormat`] if `pdb` is empty, because the size 0 is
///   reserved, or if the DEFLATE encoder does not finish the stream.
/// - [`ErrorKind::LimitExceeded`] if `pdb` is larger than `i32::MAX` bytes, or
///   if an allocation fails.
pub fn encode_embedded(pdb: &[u8]) -> Result<Vec<u8>> {
    if pdb.is_empty() {
        return Err(Error::new(ErrorKind::InvalidFormat, None)
            .with_context("an embedded PDB payload cannot hold an empty PDB"));
    }
    let size = i32::try_from(pdb.len()).map_err(|_| {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("embedded PDB is larger than i32::MAX bytes")
    })?;
    let body = deflate::encode(pdb)?;
    let limit = || {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("embedded PDB payload allocation failed")
    };
    let total = body
        .len()
        .checked_add(EMBEDDED_PDB_HEADER)
        .ok_or_else(limit)?;
    let mut payload = Vec::new();
    payload.try_reserve_exact(total).map_err(|_| limit())?;
    payload.extend_from_slice(&EMBEDDED_PDB_SIGNATURE);
    payload.extend_from_slice(&size.to_le_bytes());
    payload.extend_from_slice(&body);
    Ok(payload)
}

/// The storage format of an embedded source.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum EmbeddedFormat {
    /// The source bytes without compression.
    Raw,
    /// The source bytes as a raw DEFLATE stream.
    Deflate,
}

/// A parsed embedded-source blob, borrowed from the blob bytes.
///
/// [`EmbeddedSource::parse`] reads only the format field. Call
/// [`EmbeddedSource::decode`] to get the source bytes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum EmbeddedSource<'a> {
    /// The format field is 0. The value is the source bytes.
    Raw(&'a [u8]),
    /// The format field is positive. It is the decoded size.
    Deflate {
        /// The declared size of the decoded source in bytes.
        size: u32,
        /// The raw DEFLATE stream.
        data: &'a [u8],
    },
}

impl<'a> EmbeddedSource<'a> {
    /// Parse the format field of `blob` and borrow the rest.
    ///
    /// # Errors
    ///
    /// The errors have no location; the caller adds it.
    ///
    /// - [`ErrorKind::UnexpectedEof`] if `blob` is shorter than the 4-byte
    ///   format field.
    /// - [`ErrorKind::UnsupportedVersion`] if the format field is negative.
    pub fn parse(blob: &'a [u8]) -> Result<Self> {
        let (format, data) = blob.split_first_chunk::<FORMAT_FIELD>().ok_or_else(|| {
            Error::new(ErrorKind::UnexpectedEof, None)
                .with_context("embedded source blob is shorter than its format field")
        })?;
        let format = i32::from_le_bytes(*format);
        match u32::try_from(format) {
            Ok(0) => Ok(Self::Raw(data)),
            Ok(size) => Ok(Self::Deflate { size, data }),
            Err(_) => Err(Error::new(ErrorKind::UnsupportedVersion, None)
                .with_context("embedded source format is reserved for future use")),
        }
    }

    /// Return the storage format.
    #[must_use]
    pub const fn format(&self) -> EmbeddedFormat {
        match self {
            Self::Raw(_) => EmbeddedFormat::Raw,
            Self::Deflate { .. } => EmbeddedFormat::Deflate,
        }
    }

    /// Return the size of the decoded source in bytes.
    ///
    /// For the DEFLATE form, this is the declared size. [`EmbeddedSource::decode`]
    /// makes sure that the stream gives exactly this size.
    #[must_use]
    pub fn decoded_size(&self) -> u64 {
        match self {
            // A slice length always fits in `u64` on supported targets.
            Self::Raw(data) => u64::try_from(data.len()).unwrap_or(u64::MAX),
            Self::Deflate { size, .. } => u64::from(*size),
        }
    }

    /// Return the source bytes.
    ///
    /// The raw form borrows the blob. The DEFLATE form checks the declared
    /// size against `limit` before it allocates, and then decodes into a
    /// buffer of the declared size.
    ///
    /// # Errors
    ///
    /// The errors have no location; the caller adds it.
    ///
    /// - [`ErrorKind::LimitExceeded`] if the decoded size is larger than
    ///   `limit`, or if the allocation fails.
    /// - [`ErrorKind::UnexpectedEof`] if the DEFLATE stream ends before its
    ///   final block.
    /// - [`ErrorKind::InvalidFormat`] if the DEFLATE stream is corrupt, gives
    ///   a size other than the declared size, or has bytes after its final block.
    pub fn decode(&self, limit: usize) -> Result<Cow<'a, [u8]>> {
        match *self {
            Self::Raw(data) => {
                if data.len() > limit {
                    return Err(Error::new(ErrorKind::LimitExceeded, None)
                        .with_context("embedded source is larger than the decode limit"));
                }
                Ok(Cow::Borrowed(data))
            }
            Self::Deflate { size, data } => {
                Ok(Cow::Owned(deflate::decode(data, u64::from(size), limit)?))
            }
        }
    }

    /// Encode the blob again from its format field and its data, and append it
    /// to `out`. The DEFLATE form keeps its stream without change.
    ///
    /// # Errors
    ///
    /// Return [`ErrorKind::LimitExceeded`] if an allocation fails. On error,
    /// `out` does not change.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        match *self {
            Self::Raw(data) => append_blob(0, data, out),
            Self::Deflate { size, data } => append_blob(size, data, out),
        }
    }

    /// Encode `source` as an embedded-source blob in `format`, and append it to
    /// `out`.
    ///
    /// The caller selects the format. An empty source always gives the raw form,
    /// because the format 0 marks the raw form. The DEFLATE form uses the default
    /// compression level.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`] if `source` is larger than `i32::MAX` bytes
    ///   for the DEFLATE form, or if an allocation fails.
    /// - [`ErrorKind::InvalidFormat`] if the DEFLATE encoder does not finish the stream.
    ///
    /// On error, `out` does not change.
    pub fn encode_parts(source: &[u8], format: EmbeddedFormat, out: &mut Vec<u8>) -> Result<()> {
        match format {
            EmbeddedFormat::Raw => append_blob(0, source, out),
            EmbeddedFormat::Deflate if source.is_empty() => append_blob(0, source, out),
            EmbeddedFormat::Deflate => {
                let size = i32::try_from(source.len()).map_err(|_| {
                    Error::new(ErrorKind::LimitExceeded, None)
                        .with_context("embedded source is larger than i32::MAX bytes")
                })?;
                append_blob(size.cast_unsigned(), &deflate::encode(source)?, out)
            }
        }
    }
}

/// Append an embedded-source blob with the format field `format` and `body` to `out`.
fn append_blob(format: u32, body: &[u8], out: &mut Vec<u8>) -> Result<()> {
    let limit = || {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("embedded source blob allocation failed")
    };
    let total = body.len().checked_add(FORMAT_FIELD).ok_or_else(limit)?;
    out.try_reserve_exact(total).map_err(|_| limit())?;
    out.extend_from_slice(&format.to_le_bytes());
    out.extend_from_slice(body);
    Ok(())
}
