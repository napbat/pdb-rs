//! `DEBUG_S_FILECHKSMS` subsections: the file checksum table of a module.

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::{invalid, pad4, put, put_u32, too_large, truncated};
use crate::{Error, ErrorKind, Result};

/// The size of an entry header: a name offset, a size and a kind.
const ENTRY_HEADER_SIZE: usize = 6;

/// The alignment of each entry in the subsection.
const ENTRY_ALIGNMENT: usize = 4;

/// The algorithm of a file checksum.
///
/// The values and sizes come from `line_data.md` of the Microsoft `pdb-rs`
/// documentation and `FileChecksumKind` in LLVM `CodeView.h`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ChecksumKind {
    /// No checksum (kind 0, 0 bytes).
    None,
    /// MD5 (kind 1, 16 bytes).
    Md5,
    /// SHA-1 (kind 2, 20 bytes).
    Sha1,
    /// SHA-256 (kind 3, 32 bytes).
    Sha256,
}

impl ChecksumKind {
    /// Classify a raw checksum kind. Return `None` for an unknown kind.
    #[must_use]
    pub const fn from_raw(kind: u8) -> Option<Self> {
        match kind {
            0 => Some(Self::None),
            1 => Some(Self::Md5),
            2 => Some(Self::Sha1),
            3 => Some(Self::Sha256),
            _ => None,
        }
    }

    /// Return the raw kind value.
    #[must_use]
    pub const fn raw(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Md5 => 1,
            Self::Sha1 => 2,
            Self::Sha256 => 3,
        }
    }

    /// Return the checksum size in bytes that the kind requires.
    #[must_use]
    pub const fn size(self) -> usize {
        match self {
            Self::None => 0,
            Self::Md5 => 16,
            Self::Sha1 => 20,
            Self::Sha256 => 32,
        }
    }
}

/// One entry of a file checksum table.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct FileChecksum<'a> {
    /// The byte offset of the entry in the subsection data.
    ///
    /// Line blocks and inlinee entries refer to the entry with this value.
    pub offset: u32,
    /// The byte offset of the file name in the `/names` stream.
    pub name_offset: u32,
    /// The checksum algorithm.
    pub kind: ChecksumKind,
    /// The checksum bytes. The length is [`ChecksumKind::size`].
    pub checksum: &'a [u8],
}

/// A decoded `DEBUG_S_FILECHKSMS` subsection.
///
/// [`FileChecksums::decode`] validates all entries and records their
/// offsets. The entry iterator then cannot fail.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileChecksums<'a> {
    /// The validated subsection data.
    data: &'a [u8],
    /// The offset of each entry, in increasing order.
    offsets: Vec<u32>,
}

impl<'a> FileChecksums<'a> {
    /// Decode and validate the data of a `DEBUG_S_FILECHKSMS` subsection.
    ///
    /// These rules apply:
    ///
    /// - The checksum size matches the checksum kind.
    /// - Zero bytes pad each entry to a 4-byte boundary. The padding after
    ///   the last entry can be absent, because the subsection framing can
    ///   exclude it. A part of the padding is not valid.
    /// - The entries fill the data exactly.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] if an entry or its padding is
    ///   truncated.
    /// - [`ErrorKind::UnsupportedVersion`] if a checksum kind is unknown.
    /// - [`ErrorKind::InvalidFormat`] if a checksum size does not match its
    ///   kind, or if a padding byte is not zero.
    /// - [`ErrorKind::OffsetOverflow`] if an entry offset does not fit in
    ///   32 bits.
    /// - [`ErrorKind::LimitExceeded`] if the offset table cannot be
    ///   allocated.
    ///
    /// The errors do not have a file offset or a stream location.
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        let mut offsets = Vec::new();
        let mut position = 0;
        while position < data.len() {
            let offset = u32::try_from(position).map_err(|_| {
                Error::new(ErrorKind::OffsetOverflow, None)
                    .with_context("file checksum offset does not fit in 32 bits")
            })?;
            let (_, end) = parse_entry(data, offset)?;
            let next = if end == data.len() {
                end
            } else {
                end.next_multiple_of(ENTRY_ALIGNMENT)
            };
            let padding = data
                .get(end..next)
                .ok_or_else(|| truncated("file checksum padding is truncated"))?;
            if padding.iter().any(|&byte| byte != 0) {
                return Err(invalid("file checksum padding is not zero"));
            }
            offsets
                .try_reserve(1)
                .map_err(|_| too_large("file checksum table cannot be allocated"))?;
            offsets.push(offset);
            position = next;
        }
        Ok(Self { data, offsets })
    }

    /// Return the number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    /// Return `true` if the table has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Return the entry that starts at `offset`.
    ///
    /// Return `None` if no entry starts at `offset`. An offset inside an
    /// entry does not identify an entry.
    #[must_use]
    pub fn get(&self, offset: u32) -> Option<FileChecksum<'a>> {
        self.offsets.binary_search(&offset).ok()?;
        parse_entry(self.data, offset).ok().map(|(entry, _)| entry)
    }

    /// Return `true` if an entry starts at `offset`.
    #[must_use]
    pub fn contains(&self, offset: u32) -> bool {
        self.offsets.binary_search(&offset).is_ok()
    }

    /// Return an iterator over the entries in subsection order.
    #[must_use]
    pub fn iter(&self) -> FileChecksumsIter<'_, 'a> {
        FileChecksumsIter {
            data: self.data,
            offsets: self.offsets.iter(),
        }
    }

    /// Append the subsection data to `out`.
    ///
    /// Each entry gets the padding that it had in the decoded data, so the
    /// output is equal to that data. The padding after the last entry can
    /// be absent.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let ends = self
            .offsets
            .iter()
            .skip(1)
            .map(|&next| usize::try_from(next).unwrap_or(usize::MAX))
            .chain([self.data.len()]);
        for (entry, next) in self.iter().zip(ends) {
            write_entry(entry.name_offset, entry.kind, entry.checksum, out)?;
            // `decode` checked that the entry and its padding end at `next`.
            let start = usize::try_from(entry.offset).unwrap_or(usize::MAX);
            let end = start.saturating_add(ENTRY_HEADER_SIZE + entry.checksum.len());
            let padding = next.saturating_sub(end).min(ENTRY_ALIGNMENT - 1);
            put(out, &[0; ENTRY_ALIGNMENT - 1][..padding])?;
        }
        Ok(())
    }
}

/// Append one entry of a file checksum table and its padding to `out`.
///
/// The entry starts at a 4-byte boundary of the table. The output offset of
/// the entry, relative to the table start, is the value that line blocks
/// and inlinee entries use to refer to it.
///
/// # Errors
///
/// - [`ErrorKind::InvalidFormat`] if the checksum size is not
///   [`ChecksumKind::size`].
/// - [`ErrorKind::LimitExceeded`] if the output cannot grow.
pub fn encode_file_checksum(
    name_offset: u32,
    kind: ChecksumKind,
    checksum: &[u8],
    out: &mut Vec<u8>,
) -> Result<()> {
    write_entry(name_offset, kind, checksum, out)?;
    pad4(out, ENTRY_HEADER_SIZE + checksum.len())
}

/// Append the header and the checksum of one entry to `out`.
fn write_entry(
    name_offset: u32,
    kind: ChecksumKind,
    checksum: &[u8],
    out: &mut Vec<u8>,
) -> Result<()> {
    if checksum.len() != kind.size() {
        return Err(invalid("file checksum size does not match its kind"));
    }
    put_u32(out, name_offset)?;
    // The sizes of all kinds are at most 32, so the size fits in `u8`.
    put(out, &[checksum.len().to_le_bytes()[0], kind.raw()])?;
    put(out, checksum)
}

impl<'s, 'a> IntoIterator for &'s FileChecksums<'a> {
    type Item = FileChecksum<'a>;
    type IntoIter = FileChecksumsIter<'s, 'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over the entries of a [`FileChecksums`] table.
#[derive(Clone, Debug)]
pub struct FileChecksumsIter<'s, 'a> {
    /// The validated subsection data.
    data: &'a [u8],
    /// The remaining entry offsets.
    offsets: core::slice::Iter<'s, u32>,
}

impl<'a> Iterator for FileChecksumsIter<'_, 'a> {
    type Item = FileChecksum<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        // `FileChecksums::decode` validated each offset, so this cannot fail.
        let &offset = self.offsets.next()?;
        parse_entry(self.data, offset).ok().map(|(entry, _)| entry)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.offsets.size_hint()
    }
}

impl ExactSizeIterator for FileChecksumsIter<'_, '_> {}

impl FusedIterator for FileChecksumsIter<'_, '_> {}

/// Decode the entry at `offset`; return it and the end of its checksum.
fn parse_entry(data: &[u8], offset: u32) -> Result<(FileChecksum<'_>, usize)> {
    let start =
        usize::try_from(offset).map_err(|_| truncated("file checksum entry is truncated"))?;
    let header = data
        .get(start..)
        .and_then(<[u8]>::first_chunk::<ENTRY_HEADER_SIZE>)
        .ok_or_else(|| truncated("file checksum entry header is truncated"))?;
    let [n0, n1, n2, n3, size, kind] = *header;
    let kind = ChecksumKind::from_raw(kind).ok_or_else(|| {
        Error::new(ErrorKind::UnsupportedVersion, None)
            .with_context("file checksum kind is not supported")
    })?;
    let size = usize::from(size);
    if size != kind.size() {
        return Err(invalid("file checksum size does not match its kind"));
    }
    // The header is inside `data`, so `start + 6` does not overflow.
    let checksum_start = start + ENTRY_HEADER_SIZE;
    let end = checksum_start + size;
    let checksum = data
        .get(checksum_start..end)
        .ok_or_else(|| truncated("file checksum bytes are truncated"))?;
    let entry = FileChecksum {
        offset,
        name_offset: u32::from_le_bytes([n0, n1, n2, n3]),
        kind,
        checksum,
    };
    Ok((entry, end))
}
