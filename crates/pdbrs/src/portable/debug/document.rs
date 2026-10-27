//! The `Document` table: name blobs, languages, hash algorithms and the document record.
//!
//! A document name blob is `separator part+`. The separator is one UTF-8 character, or byte 0
//! for no separator. Each part is a compressed `#Blob` offset of UTF-8 text. The name is the
//! parts joined with the separator.

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::rows::{TypedRows, each_row, in_blob, row_error};
use crate::bytes::{Bytes, into_owned};
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::custom::blob::{Reader, Writer, invalid, too_large, truncated};
use crate::portable::metadata::{BlobOffset, DocumentId, DocumentRow, TableId};
use crate::{ErrorKind, Result};

/// Make GUID bytes in serialized order from the fields of the GUID text form.
const fn guid(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> [u8; 16] {
    let [a0, a1, a2, a3] = data1.to_le_bytes();
    let [b0, b1] = data2.to_le_bytes();
    let [c0, c1] = data3.to_le_bytes();
    let [d0, d1, d2, d3, d4, d5, d6, d7] = data4;
    [
        a0, a1, a2, a3, b0, b1, c0, c1, d0, d1, d2, d3, d4, d5, d6, d7,
    ]
}

/// A source language that the specification defines for the `Language` column.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum Language {
    /// `3f5162f8-07c6-11d3-9053-00c04fa302a1`: Visual C#.
    CSharp,
    /// `3a12d0b8-c26c-11d0-b442-00a0244a1dd2`: Visual Basic.
    VisualBasic,
    /// `ab4f38c9-b6e6-43ba-be3b-58080b2ccce3`: Visual F#.
    FSharp,
}

impl Language {
    /// The defined languages.
    const ALL: [Self; 3] = [Self::CSharp, Self::VisualBasic, Self::FSharp];

    /// Return the GUID bytes in serialized order.
    #[must_use]
    pub const fn guid(self) -> [u8; 16] {
        match self {
            Self::CSharp => guid(
                0x3f51_62f8,
                0x07c6,
                0x11d3,
                [0x90, 0x53, 0x00, 0xc0, 0x4f, 0xa3, 0x02, 0xa1],
            ),
            Self::VisualBasic => guid(
                0x3a12_d0b8,
                0xc26c,
                0x11d0,
                [0xb4, 0x42, 0x00, 0xa0, 0x24, 0x4a, 0x1d, 0xd2],
            ),
            Self::FSharp => guid(
                0xab4f_38c9,
                0xb6e6,
                0x43ba,
                [0xbe, 0x3b, 0x58, 0x08, 0x0b, 0x2c, 0xcc, 0xe3],
            ),
        }
    }

    /// Return the language of GUID bytes, or `None` for a GUID that the specification does not
    /// define.
    #[must_use]
    pub fn from_guid(bytes: &[u8; 16]) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|language| language.guid() == *bytes)
    }
}

/// A hash algorithm that the specification defines for the `HashAlgorithm` column.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum HashAlgorithm {
    /// `ff1816ec-aa5e-4d10-87f7-6f4963833460`: SHA-1.
    Sha1,
    /// `8829d00f-11b8-4213-878b-770e8597ac16`: SHA-256.
    Sha256,
    /// `d99cfeb1-8c43-444a-8a6c-b61269d2a0bf`: SHA-384.
    Sha384,
    /// `ef2d1afc-6550-46d6-b14b-d70afe9a5566`: SHA-512.
    Sha512,
}

impl HashAlgorithm {
    /// The defined hash algorithms.
    const ALL: [Self; 4] = [Self::Sha1, Self::Sha256, Self::Sha384, Self::Sha512];

    /// Return the GUID bytes in serialized order.
    #[must_use]
    pub const fn guid(self) -> [u8; 16] {
        match self {
            Self::Sha1 => guid(
                0xff18_16ec,
                0xaa5e,
                0x4d10,
                [0x87, 0xf7, 0x6f, 0x49, 0x63, 0x83, 0x34, 0x60],
            ),
            Self::Sha256 => guid(
                0x8829_d00f,
                0x11b8,
                0x4213,
                [0x87, 0x8b, 0x77, 0x0e, 0x85, 0x97, 0xac, 0x16],
            ),
            Self::Sha384 => guid(
                0xd99c_feb1,
                0x8c43,
                0x444a,
                [0x8a, 0x6c, 0xb6, 0x12, 0x69, 0xd2, 0xa0, 0xbf],
            ),
            Self::Sha512 => guid(
                0xef2d_1afc,
                0x6550,
                0x46d6,
                [0xb1, 0x4b, 0xd7, 0x0a, 0xfe, 0x9a, 0x55, 0x66],
            ),
        }
    }

    /// Return the hash size in bytes.
    #[must_use]
    pub const fn hash_len(self) -> usize {
        match self {
            Self::Sha1 => 20,
            Self::Sha256 => 32,
            Self::Sha384 => 48,
            Self::Sha512 => 64,
        }
    }

    /// Return the algorithm of GUID bytes, or `None` for a GUID that the specification does
    /// not define.
    #[must_use]
    pub fn from_guid(bytes: &[u8; 16]) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|algorithm| algorithm.guid() == *bytes)
    }
}

/// A checked document name blob. It borrows the blob bytes until
/// [`DocumentName::into_owned`] copies them.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DocumentName<'a> {
    separator: Option<char>,
    parts: Bytes<'a>,
}

into_owned!(struct DocumentName { parts; separator });

impl<'a> DocumentName<'a> {
    /// Decode and check a document name blob.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] if the blob has no separator, no part or ends inside a
    ///   part offset.
    /// - [`ErrorKind::InvalidFormat`] if the separator is not one UTF-8 character or a part
    ///   offset is not a compressed integer.
    pub fn parse(blob: &'a [u8]) -> Result<Self> {
        let first = *blob
            .first()
            .ok_or_else(|| truncated("document name blob has no separator"))?;
        let width = match first {
            0x00..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return Err(invalid("document name separator is not a UTF-8 character")),
        };
        let (separator, parts) = blob
            .split_at_checked(width)
            .ok_or_else(|| truncated("document name separator is truncated"))?;
        let separator = if first == 0 {
            None
        } else {
            let text = core::str::from_utf8(separator)
                .map_err(|_| invalid("document name separator is not a UTF-8 character"))?;
            text.chars().next()
        };
        if parts.is_empty() {
            return Err(truncated("document name blob has no part"));
        }
        let mut reader = Reader::new(parts);
        while !reader.is_empty() {
            reader.unsigned()?;
        }
        Ok(Self {
            separator,
            parts: Bytes::new(parts),
        })
    }

    /// Return the separator, or `None` for the empty separator (byte 0).
    #[must_use]
    pub const fn separator(&self) -> Option<char> {
        self.separator
    }

    /// Return the `#Blob` offsets of the name parts. Offset 0 is an empty part.
    #[must_use]
    pub fn parts(&self) -> DocumentNameParts<'_> {
        DocumentNameParts {
            reader: Reader::new(&self.parts),
        }
    }

    /// Encode a document name blob from a separator and the `#Blob` offsets of the parts, and
    /// append it to `out`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `parts` is empty or the separator is U+0000.
    /// - [`ErrorKind::LimitExceeded`] if an offset does not fit in a compressed integer.
    ///
    /// On error, `out` does not change.
    pub fn encode_parts(
        separator: Option<char>,
        parts: &[BlobOffset],
        out: &mut Vec<u8>,
    ) -> Result<()> {
        if parts.is_empty() {
            return Err(invalid("document name has no part"));
        }
        let mut writer = Writer::append(out);
        match separator {
            None => writer.u8(0)?,
            Some('\0') => return Err(invalid("document name separator is U+0000")),
            Some(separator) => {
                let mut buffer = [0; 4];
                writer.bytes(separator.encode_utf8(&mut buffer).as_bytes())?;
            }
        }
        for part in parts {
            writer.unsigned(part.0)?;
        }
        writer.finish();
        Ok(())
    }
}

/// An iterator over the `#Blob` offsets of the parts of a [`DocumentName`].
#[derive(Clone, Debug)]
pub struct DocumentNameParts<'a> {
    reader: Reader<'a>,
}

impl Iterator for DocumentNameParts<'_> {
    type Item = BlobOffset;

    fn next(&mut self) -> Option<BlobOffset> {
        // `DocumentName::parse` checked all part offsets.
        if self.reader.is_empty() {
            return None;
        }
        self.reader.unsigned().ok().map(BlobOffset)
    }
}

impl FusedIterator for DocumentNameParts<'_> {}

/// Split a document name into the separator and the parts that Roslyn writes.
///
/// Roslyn uses `/` if the name has at least as many `/` as `\`, else `\`. It splits the name
/// at each separator. Each part needs a `#Blob` entry; the writer assigns the offsets and calls
/// [`DocumentName::encode_parts`].
#[must_use]
pub fn split_document_name(name: &[u8]) -> SplitDocumentName<'_> {
    let count = |byte| {
        name.iter()
            .fold(0_usize, |count, &value| count + usize::from(value == byte))
    };
    let separator = if count(b'/') >= count(b'\\') {
        b'/'
    } else {
        b'\\'
    };
    SplitDocumentName {
        separator,
        rest: Some(name),
    }
}

/// The separator and the parts of a document name from [`split_document_name`].
#[derive(Clone, Debug)]
pub struct SplitDocumentName<'a> {
    separator: u8,
    rest: Option<&'a [u8]>,
}

impl SplitDocumentName<'_> {
    /// Return the separator: `/` or `\`.
    #[must_use]
    pub fn separator(&self) -> char {
        char::from(self.separator)
    }
}

impl<'a> Iterator for SplitDocumentName<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        let rest = self.rest?;
        if let Some(end) = rest.iter().position(|&byte| byte == self.separator) {
            self.rest = Some(&rest[end + 1..]);
            Some(&rest[..end])
        } else {
            self.rest = None;
            Some(rest)
        }
    }
}

impl FusedIterator for SplitDocumentName<'_> {}

/// A source document.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Document<'a> {
    /// The name, for example a path: the parts of the name blob joined with the separator.
    /// The bytes are UTF-8 in valid metadata. The plan of an editor splits the name into
    /// parts as Roslyn does.
    pub name: Bytes<'a>,
    /// The GUID bytes of the hash algorithm, or `None` for the nil GUID index.
    /// [`HashAlgorithm::from_guid`] gives a defined algorithm.
    pub hash_algorithm: Option<[u8; 16]>,
    /// The hash of the document content, or an empty value.
    pub hash: Bytes<'a>,
    /// The GUID bytes of the source language, or `None` for the nil GUID index.
    /// [`Language::from_guid`] gives a defined language.
    pub language: Option<[u8; 16]>,
}

into_owned!(struct Document { name, hash; hash_algorithm, language });

impl<S: ReadAt> Pdb<S> {
    /// Read a document with its name, hash and GUIDs.
    ///
    /// # Errors
    ///
    /// - The errors of [`Pdb::row`] and of the heap accessors.
    /// - The errors of [`Pdb::document_name`].
    pub fn document(&self, id: DocumentId) -> Result<Document<'static>> {
        let row: DocumentRow = self.row(id)?;
        let name = self
            .document_name(row.name)
            .map_err(|error| in_blob(error, TableId::DOCUMENT, id.0, "Name", row.name))?;
        Ok(Document {
            name: name.into(),
            hash_algorithm: self.guid(row.hash_algorithm)?,
            hash: self.blob(row.hash)?.into(),
            language: self.guid(row.language)?,
        })
    }

    /// Read all documents in row order, each with its row ID.
    #[must_use]
    pub fn documents(&self) -> TypedRows<'_, S, (DocumentId, Document<'static>)> {
        each_row(self, TableId::DOCUMENT, |pdb, row| {
            let id = DocumentId(row);
            pdb.document(id).map(|document| (id, document))
        })
    }

    /// Read a document name blob and join its parts.
    ///
    /// # Errors
    ///
    /// - The errors of [`DocumentName::parse`].
    /// - The errors of [`Pdb::blob`] for the name blob and each part.
    /// - [`ErrorKind::LimitExceeded`] if the name is longer than
    ///   [`ReadOptions::max_value_bytes`](crate::portable::ReadOptions::max_value_bytes).
    pub fn document_name(&self, offset: BlobOffset) -> Result<Vec<u8>> {
        let blob = self.blob(offset)?;
        let name = DocumentName::parse(&blob)?;
        let mut buffer = [0; 4];
        let separator = name.separator().map_or(&[][..], |separator| {
            separator.encode_utf8(&mut buffer).as_bytes()
        });
        let limit = self.options().max_value_bytes();
        let mut text = Vec::new();
        for (index, part) in name.parts().enumerate() {
            let part = self.blob(part)?;
            let extra = if index == 0 { 0 } else { separator.len() };
            if text.len() + extra + part.len() > limit {
                return Err(too_large("document name is larger than the limit"));
            }
            text.try_reserve(extra + part.len())
                .map_err(|_| too_large("document name allocation failed"))?;
            if index > 0 {
                text.extend_from_slice(separator);
            }
            text.extend_from_slice(&part);
        }
        Ok(text)
    }
}

/// Return an error for a document hash with the wrong size for its algorithm.
pub(super) fn check_hash(id: DocumentId, document: &Document<'_>) -> Result<()> {
    let algorithm = document
        .hash_algorithm
        .as_ref()
        .and_then(HashAlgorithm::from_guid);
    match algorithm {
        Some(algorithm) if algorithm.hash_len() != document.hash.len() => Err(row_error(
            ErrorKind::InvalidFormat,
            "document hash size does not match its hash algorithm",
            TableId::DOCUMENT,
            id.0,
            Some("Hash"),
        )),
        _ => Ok(()),
    }
}
