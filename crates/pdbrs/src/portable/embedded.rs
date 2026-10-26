//! The embedded-source blob of Portable PDBs and .NET document records.
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

use alloc::borrow::Cow;
use alloc::vec::Vec;

use crate::compression::deflate;
use crate::{Error, ErrorKind, Result};

/// The size of the format field at the start of a blob.
const FORMAT_FIELD: usize = 4;

/// The storage format of an embedded source.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
}

/// Encode `source` as an embedded-source blob in `format`.
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
pub fn encode(source: &[u8], format: EmbeddedFormat) -> Result<Vec<u8>> {
    let limit = || {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("embedded source blob allocation failed")
    };
    let (size, body) = match format {
        EmbeddedFormat::Raw => (0, Cow::Borrowed(source)),
        EmbeddedFormat::Deflate if source.is_empty() => (0, Cow::Borrowed(source)),
        EmbeddedFormat::Deflate => {
            let size = i32::try_from(source.len()).map_err(|_| {
                Error::new(ErrorKind::LimitExceeded, None)
                    .with_context("embedded source is larger than i32::MAX bytes")
            })?;
            (size, Cow::Owned(deflate::encode(source)?))
        }
    };
    let total = body.len().checked_add(FORMAT_FIELD).ok_or_else(limit)?;
    let mut blob = Vec::new();
    blob.try_reserve_exact(total).map_err(|_| limit())?;
    blob.extend_from_slice(&i32::to_le_bytes(size));
    blob.extend_from_slice(&body);
    Ok(blob)
}
