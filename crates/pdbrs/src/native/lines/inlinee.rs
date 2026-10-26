//! `DEBUG_S_INLINEELINES` subsections: source lines of inlined functions.

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::{entry_list, invalid, read_u32, to_u32, too_large, truncated};
use crate::native::codeview::index::{ChecksumOffset, IdIndex};
use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

/// The size of the signature at the start of the subsection.
const SIGNATURE_SIZE: usize = 4;

/// The size of an entry header: an inlinee, a checksum offset and a line.
const ENTRY_HEADER_SIZE: usize = 12;

/// The size of one checksum offset.
const OFFSET_SIZE: usize = 4;

/// The layout of the entries of a `DEBUG_S_INLINEELINES` subsection.
///
/// The values come from `InlineeLinesSignature` in LLVM `CodeView.h`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum InlineeSignature {
    /// Signature 0 (`CV_INLINEE_SOURCE_LINE_SIGNATURE`): entries without
    /// extra files.
    Normal,
    /// Signature 1 (`CV_INLINEE_SOURCE_LINE_SIGNATURE_EX`): each entry has
    /// a list of extra files.
    ExtraFiles,
}

impl InlineeSignature {
    /// Return the raw signature value.
    #[must_use]
    pub const fn raw(self) -> u32 {
        match self {
            Self::Normal => 0,
            Self::ExtraFiles => 1,
        }
    }

    /// Append the 4-byte signature at the start of the subsection to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(self, out: &mut Vec<u8>) -> Result<()> {
        Output::new(out).u32(self.raw())
    }
}

/// A decoded `DEBUG_S_INLINEELINES` subsection.
///
/// [`InlineeLines::parse`] validates all entries. The entry iterator then
/// cannot fail.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InlineeLines<'a> {
    /// The entry layout.
    signature: InlineeSignature,
    /// The validated entries after the signature.
    entries: &'a [u8],
    /// The number of entries.
    count: usize,
}

impl<'a> InlineeLines<'a> {
    /// Parse and validate the data of a `DEBUG_S_INLINEELINES` subsection.
    ///
    /// The entries must fill the data exactly.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] if the signature, an entry or an
    ///   extra file list is truncated.
    /// - [`ErrorKind::UnsupportedVersion`] if the signature is not 0 or 1.
    /// - [`ErrorKind::LimitExceeded`] if an extra file count does not fit in
    ///   memory.
    ///
    /// The errors do not have a file offset or a stream location.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let signature = match read_u32(data, 0) {
            None => return Err(truncated("inlinee line signature is truncated")),
            Some(0) => InlineeSignature::Normal,
            Some(1) => InlineeSignature::ExtraFiles,
            Some(_) => {
                return Err(Error::new(ErrorKind::UnsupportedVersion, None)
                    .with_context("inlinee line signature is not supported"));
            }
        };
        let entries = &data[SIGNATURE_SIZE..];
        let mut rest = entries;
        let mut count = 0usize;
        while !rest.is_empty() {
            rest = split_entry(rest, signature)?.1;
            count += 1;
        }
        Ok(Self {
            signature,
            entries,
            count,
        })
    }

    /// Return the entry layout.
    #[must_use]
    pub const fn signature(&self) -> InlineeSignature {
        self.signature
    }

    /// Return the number of entries.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Return `true` if the subsection has no entries.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Return an iterator over the entries.
    #[must_use]
    pub const fn iter(&self) -> InlineeLinesIter<'a> {
        InlineeLinesIter {
            bytes: self.entries,
            signature: self.signature,
            left: self.count,
        }
    }

    /// Append the subsection data to `out`: the signature and all entries.
    ///
    /// The output is equal to the data that [`InlineeLines::parse`] read.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`InlineeLine::encode`].
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        self.signature.encode(out)?;
        for entry in self {
            entry.encode(self.signature, out)?;
        }
        Ok(())
    }
}

impl<'a> IntoIterator for InlineeLines<'a> {
    type Item = InlineeLine<'a>;
    type IntoIter = InlineeLinesIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &InlineeLines<'a> {
    type Item = InlineeLine<'a>;
    type IntoIter = InlineeLinesIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// The source location of one inlined function (`InlineeSourceLine`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct InlineeLine<'a> {
    /// The `LF_FUNC_ID` or `LF_MFUNC_ID` record of the inlined function.
    pub inlinee: IdIndex,
    /// The byte offset of the file entry in the `DEBUG_S_FILECHKSMS`
    /// subsection of the same module.
    pub checksum_offset: ChecksumOffset,
    /// The line number of the function in the source file.
    pub source_line: u32,
    /// The checksum offsets of extra files.
    ///
    /// The list is empty for [`InlineeSignature::Normal`].
    pub extra_files: ChecksumOffsets<'a>,
}

impl InlineeLine<'_> {
    /// Append the entry to `out` in the layout of `signature`.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`InlineeLine::encode_parts`].
    pub fn encode(&self, signature: InlineeSignature, out: &mut Vec<u8>) -> Result<()> {
        write_entry(
            signature,
            [self.inlinee.0, self.checksum_offset.0, self.source_line],
            self.extra_files.iter(),
            out,
        )
    }

    /// Append one entry of a `DEBUG_S_INLINEELINES` subsection to `out`.
    ///
    /// Use the signature of the subsection ([`InlineeSignature::encode`]).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `extra_files` is not empty and the
    ///   signature is [`InlineeSignature::Normal`].
    /// - [`ErrorKind::LimitExceeded`] if the extra file count does not fit
    ///   in 32 bits, or if the output cannot grow.
    pub fn encode_parts(
        signature: InlineeSignature,
        inlinee: IdIndex,
        checksum_offset: ChecksumOffset,
        source_line: u32,
        extra_files: &[ChecksumOffset],
        out: &mut Vec<u8>,
    ) -> Result<()> {
        write_entry(
            signature,
            [inlinee.0, checksum_offset.0, source_line],
            extra_files.iter().copied(),
            out,
        )
    }
}

/// Append an entry with the header `fields` and `extra_files` to `out`.
fn write_entry(
    signature: InlineeSignature,
    fields: [u32; 3],
    extra_files: impl ExactSizeIterator<Item = ChecksumOffset>,
    out: &mut Vec<u8>,
) -> Result<()> {
    let count = extra_files.len();
    if signature == InlineeSignature::Normal && count != 0 {
        return Err(invalid(
            "inlinee line signature 0 does not allow extra files",
        ));
    }
    for field in fields {
        Output::new(out).u32(field)?;
    }
    if signature == InlineeSignature::ExtraFiles {
        Output::new(out).u32(to_u32(
            count,
            "inlinee extra file count does not fit in 32 bits",
        )?)?;
        for offset in extra_files {
            Output::new(out).u32(offset.0)?;
        }
    }
    Ok(())
}

entry_list!(
    /// A list of byte offsets into a `DEBUG_S_FILECHKSMS` subsection.
    ChecksumOffsets,
    /// An iterator over [`ChecksumOffsets`].
    ChecksumOffsetsIter,
    ChecksumOffset,
    OFFSET_SIZE,
    |bytes| ChecksumOffset(u32::from_le_bytes(bytes))
);

/// An iterator over the entries of an [`InlineeLines`] subsection.
#[derive(Clone, Debug)]
pub struct InlineeLinesIter<'a> {
    /// The remaining validated entries.
    bytes: &'a [u8],
    /// The entry layout.
    signature: InlineeSignature,
    /// The number of remaining entries.
    left: usize,
}

impl<'a> Iterator for InlineeLinesIter<'a> {
    type Item = InlineeLine<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        // `InlineeLines::parse` validated the entries, so this cannot fail.
        let (entry, rest) = split_entry(self.bytes, self.signature).ok()?;
        self.bytes = rest;
        self.left = self.left.saturating_sub(1);
        Some(entry)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}

impl ExactSizeIterator for InlineeLinesIter<'_> {}

impl FusedIterator for InlineeLinesIter<'_> {}

/// Split one entry from the start of `bytes`; return it and the rest.
fn split_entry(bytes: &[u8], signature: InlineeSignature) -> Result<(InlineeLine<'_>, &[u8])> {
    let (Some(inlinee), Some(checksum_offset), Some(source_line)) =
        (read_u32(bytes, 0), read_u32(bytes, 4), read_u32(bytes, 8))
    else {
        return Err(truncated("inlinee line entry is truncated"));
    };
    let mut rest = &bytes[ENTRY_HEADER_SIZE..];
    let mut extra = &rest[..0];
    if signature == InlineeSignature::ExtraFiles {
        let count =
            read_u32(rest, 0).ok_or_else(|| truncated("inlinee extra file count is truncated"))?;
        let size = usize::try_from(count)
            .ok()
            .and_then(|count| count.checked_mul(OFFSET_SIZE))
            .ok_or_else(|| too_large("inlinee extra file count is too large"))?;
        let files = &rest[OFFSET_SIZE..];
        if files.len() < size {
            return Err(truncated("inlinee extra file list is truncated"));
        }
        (extra, rest) = files.split_at(size);
    }
    let entry = InlineeLine {
        inlinee: IdIndex(inlinee),
        checksum_offset: ChecksumOffset(checksum_offset),
        source_line,
        extra_files: ChecksumOffsets::new(extra),
    };
    Ok((entry, rest))
}
