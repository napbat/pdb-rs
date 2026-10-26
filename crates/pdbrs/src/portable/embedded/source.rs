//! The embedded source of a `Document` row and its check against the document hash.
//!
//! The embedded source of a document is the value of a `CustomDebugInformation` row with
//! the kind [`CustomDebugKind::EMBEDDED_SOURCE`] and the document as its parent. The value
//! is an embedded-source blob ([`EmbeddedSource`]).

use alloc::borrow::Cow;
use alloc::vec::Vec;

use super::{EmbeddedFormat, EmbeddedSource, FORMAT_FIELD};
use crate::PortableLocation;
use crate::crypto::sha256;
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::custom::CustomDebugKind;
use crate::portable::debug::{Document, HashAlgorithm};
use crate::portable::metadata::{
    BlobOffset, CustomDebugInformationId, CustomDebugInformationRow, MetadataStream, TableId,
};
use crate::{Error, ErrorKind, Result};

/// The result of a check of a source against the hash of its document.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SourceCheck {
    /// The hash algorithm is SHA-256, and the SHA-256 digest of the source is equal to the
    /// document hash.
    Verified,
    /// The hash algorithm is SHA-256, and the SHA-256 digest of the source is not equal to the
    /// document hash.
    Mismatch,
    /// This crate does not compute the hash algorithm of the document, for example SHA-1, or
    /// the document has no defined hash algorithm.
    NotVerified,
}

/// The decoded embedded source of a document and the result of its hash check.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DocumentSource {
    /// The `CustomDebugInformation` row of the embedded source.
    custom_debug_information: CustomDebugInformationId,
    /// The storage format of the blob.
    format: EmbeddedFormat,
    /// The decoded source bytes.
    content: Vec<u8>,
    /// The result of the hash check.
    check: SourceCheck,
}

impl DocumentSource {
    /// Return the `CustomDebugInformation` row that holds the embedded source.
    #[must_use]
    pub const fn custom_debug_information(&self) -> CustomDebugInformationId {
        self.custom_debug_information
    }

    /// Return the storage format of the embedded-source blob.
    #[must_use]
    pub const fn format(&self) -> EmbeddedFormat {
        self.format
    }

    /// Return the decoded source bytes.
    #[must_use]
    pub fn content(&self) -> &[u8] {
        &self.content
    }

    /// Return the decoded source bytes and drop the other values.
    #[must_use]
    pub fn into_content(self) -> Vec<u8> {
        self.content
    }

    /// Return the result of the check against the document hash.
    #[must_use]
    pub const fn check(&self) -> SourceCheck {
        self.check
    }
}

impl Document {
    /// Check `source` against the hash of the document.
    ///
    /// Only SHA-256 is computed. For other hash algorithms and for a document without a
    /// defined hash algorithm, the function returns [`SourceCheck::NotVerified`].
    #[must_use]
    pub fn verify_source(&self, source: &[u8]) -> SourceCheck {
        match self.hash_algorithm() {
            Some(HashAlgorithm::Sha256) if self.hash() == sha256(source) => SourceCheck::Verified,
            Some(HashAlgorithm::Sha256) => SourceCheck::Mismatch,
            _ => SourceCheck::NotVerified,
        }
    }
}

/// Return the location of the `Value` column of the `CustomDebugInformation` row `id`.
const fn value_location(id: CustomDebugInformationId) -> PortableLocation {
    PortableLocation::new()
        .with_stream(MetadataStream::Tables)
        .with_table(TableId::CUSTOM_DEBUG_INFORMATION)
        .with_row(id.get())
        .with_column("Value")
}

impl<S: ReadAt> Pdb<S> {
    /// Find the embedded-source row of `document`.
    ///
    /// Return the row ID and the `#Blob` offset of its value, or `None` if the document has
    /// no embedded source.
    ///
    /// # Errors
    ///
    /// - The errors of [`Pdb::rows`] and [`Pdb::guid`] for the `CustomDebugInformation` rows.
    /// - [`ErrorKind::InvalidFormat`] if more than one row holds an embedded source for
    ///   `document`. The location is the second row.
    pub fn embedded_source_row(
        &self,
        document: &Document,
    ) -> Result<Option<(CustomDebugInformationId, BlobOffset)>> {
        let Some(token) = document.id().token() else {
            return Ok(None);
        };
        let mut found = None;
        for row in self.rows::<CustomDebugInformationRow>() {
            let (id, row) = row?;
            if row.parent != token
                || self.guid(row.kind)? != Some(CustomDebugKind::EMBEDDED_SOURCE.0)
            {
                continue;
            }
            if found.is_some() {
                return Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("document has more than one embedded source")
                    .with_portable(value_location(id)));
            }
            found = Some((id, row.value));
        }
        Ok(found)
    }

    /// Decode the embedded source of `document` and check it against the document hash.
    ///
    /// `limit` is the largest decoded source size in bytes, for the raw and the DEFLATE form.
    /// The DEFLATE form checks the declared size against `limit` before it allocates.
    /// Return `None` if the document has no embedded source.
    ///
    /// # Errors
    ///
    /// - The errors of [`Pdb::embedded_source_row`] and [`Pdb::blob`].
    /// - The errors of [`EmbeddedSource::parse`] and [`EmbeddedSource::decode`], with the
    ///   row, the `Value` column and the blob offset as their location. This includes
    ///   [`ErrorKind::LimitExceeded`] for a source larger than `limit`.
    ///
    /// A source that does not match the document hash is not an error. The result has
    /// [`SourceCheck::Mismatch`].
    pub fn document_source(
        &self,
        document: &Document,
        limit: usize,
    ) -> Result<Option<DocumentSource>> {
        let Some((id, offset)) = self.embedded_source_row(document)? else {
            return Ok(None);
        };
        let at =
            |error: Error| error.with_portable(value_location(id).with_heap_offset(offset.get()));
        let blob = self.blob(offset).map_err(at)?;
        let source = EmbeddedSource::parse(&blob).map_err(at)?;
        let format = source.format();
        let content = if let Cow::Owned(content) = source.decode(limit).map_err(at)? {
            content
        } else {
            // The raw form: reuse the blob buffer without its format field.
            let mut content = blob;
            content.drain(..FORMAT_FIELD);
            content
        };
        Ok(Some(DocumentSource {
            custom_debug_information: id,
            format,
            check: document.verify_source(&content),
            content,
        }))
    }
}
