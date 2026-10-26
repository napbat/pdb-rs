//! Checked access to the `#Strings`, `#US`, `#Blob` and `#GUID` heaps (ECMA-335 II.24.2.3
//! and II.24.2.4).
//!
//! Each function reads only the requested entry through the positional source. It checks
//! that the entry starts and ends inside its heap before it allocates.

use alloc::vec::Vec;

use super::root::{MetadataRoot, MetadataStream};
use crate::error::PortableLocation;
use crate::io::ReadAt;
use crate::portable::compressed;
use crate::{Error, ErrorKind, Result};

/// The number of bytes that a string read requests at a time.
const STRING_CHUNK: u32 = 64;

/// The range of one heap stream. An absent stream has size 0.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Heap {
    stream: MetadataStream,
    offset: u32,
    size: u32,
}

impl Heap {
    /// Find the heap in the root and check its first entry.
    ///
    /// The `#Strings`, `#US` and `#Blob` heaps start with an empty entry (byte 0). The size
    /// of the `#GUID` heap is a multiple of 16.
    pub(crate) fn read(
        source: &(impl ReadAt + ?Sized),
        root: &MetadataRoot,
        stream: MetadataStream,
    ) -> Result<Self> {
        let (offset, size) = root
            .stream(stream)
            .map_or((0, 0), |header| (header.offset(), header.size()));
        let heap = Self {
            stream,
            offset,
            size,
        };
        if stream == MetadataStream::Guid {
            if size % 16 != 0 {
                return Err(heap.error(
                    ErrorKind::InvalidFormat,
                    0,
                    "#GUID heap size is not a multiple of 16",
                ));
            }
        } else if size > 0 && read_u8(source, u64::from(offset))? != 0 {
            return Err(heap.error(
                ErrorKind::InvalidFormat,
                0,
                "heap does not start with an empty entry",
            ));
        }
        Ok(heap)
    }

    /// Return the heap size in bytes.
    pub(crate) const fn size(&self) -> u32 {
        self.size
    }

    /// Return `true` if `offset` is a valid start of an entry: 0, or inside the heap.
    pub(crate) const fn contains(&self, offset: u32) -> bool {
        offset == 0 || offset < self.size
    }

    /// Return the number of GUIDs in the `#GUID` heap.
    pub(crate) const fn guid_count(&self) -> u32 {
        self.size / 16
    }

    /// Read the bytes of the string at `offset`, without its terminator.
    pub(crate) fn string(
        &self,
        source: &(impl ReadAt + ?Sized),
        offset: u32,
        limit: usize,
    ) -> Result<Vec<u8>> {
        if offset == 0 && self.size == 0 {
            return Ok(Vec::new());
        }
        self.check_start(offset)?;
        let mut text = Vec::new();
        let mut position = offset;
        let mut chunk = [0; STRING_CHUNK as usize];
        loop {
            let take = (self.size - position).min(STRING_CHUNK);
            if take == 0 {
                return Err(self.error(
                    ErrorKind::InvalidFormat,
                    offset,
                    "string has no terminator in the heap",
                ));
            }
            let bytes = &mut chunk[..take as usize];
            source.read_exact_at(self.absolute(position), bytes)?;
            let end = bytes.iter().position(|&byte| byte == 0);
            let part = &bytes[..end.unwrap_or(bytes.len())];
            if text.len() + part.len() > limit {
                return Err(self.error(
                    ErrorKind::LimitExceeded,
                    offset,
                    "string is larger than the limit",
                ));
            }
            text.try_reserve(part.len()).map_err(|_| {
                self.error(ErrorKind::LimitExceeded, offset, "string allocation failed")
            })?;
            text.extend_from_slice(part);
            if end.is_some() {
                return Ok(text);
            }
            position += take;
        }
    }

    /// Read the bytes of the blob at `offset` (`#Blob` or `#US`), without its length prefix.
    pub(crate) fn blob(
        &self,
        source: &(impl ReadAt + ?Sized),
        offset: u32,
        limit: usize,
    ) -> Result<Vec<u8>> {
        if offset == 0 && self.size == 0 {
            return Ok(Vec::new());
        }
        self.check_start(offset)?;
        let available = (self.size - offset).min(4);
        let mut prefix = [0; 4];
        let prefix = &mut prefix[..available as usize];
        source.read_exact_at(self.absolute(offset), prefix)?;
        let (length, rest) = compressed::decode_unsigned(prefix).map_err(|error| {
            let context = if error.kind() == ErrorKind::UnexpectedEof {
                "blob length prefix ends after the heap"
            } else {
                "blob length prefix is not a compressed integer"
            };
            self.error(ErrorKind::InvalidFormat, offset, context)
        })?;
        let consumed = match prefix.len() - rest.len() {
            1 => 1,
            2 => 2,
            _ => 4,
        };
        let start = offset + consumed;
        if length > self.size - start {
            return Err(self.error(ErrorKind::InvalidFormat, offset, "blob ends after the heap"));
        }
        let length = usize::try_from(length).map_err(|_| {
            self.error(
                ErrorKind::LimitExceeded,
                offset,
                "blob is larger than the address space",
            )
        })?;
        if length > limit {
            return Err(self.error(
                ErrorKind::LimitExceeded,
                offset,
                "blob is larger than the limit",
            ));
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| self.error(ErrorKind::LimitExceeded, offset, "blob allocation failed"))?;
        bytes.resize(length, 0);
        source.read_exact_at(self.absolute(start), &mut bytes)?;
        Ok(bytes)
    }

    /// Read the GUID at the 1-based `index`. Return `None` for index 0.
    pub(crate) fn guid(
        &self,
        source: &(impl ReadAt + ?Sized),
        index: u32,
    ) -> Result<Option<[u8; 16]>> {
        if index == 0 {
            return Ok(None);
        }
        if index > self.guid_count() {
            return Err(self.error(
                ErrorKind::InvalidReference,
                index,
                "GUID index is after the #GUID heap",
            ));
        }
        let mut guid = [0; 16];
        source.read_exact_at(self.absolute((index - 1) * 16), &mut guid)?;
        Ok(Some(guid))
    }

    /// Check that an entry starts inside the heap.
    fn check_start(&self, offset: u32) -> Result<()> {
        if offset < self.size {
            Ok(())
        } else {
            Err(self.error(
                ErrorKind::InvalidReference,
                offset,
                "heap offset is after the heap",
            ))
        }
    }

    /// Return the metadata offset of a heap offset.
    fn absolute(&self, offset: u32) -> u64 {
        u64::from(self.offset) + u64::from(offset)
    }

    /// Make an error at a heap offset.
    pub(crate) fn error(&self, kind: ErrorKind, offset: u32, context: &'static str) -> Error {
        Error::new(kind, Some(self.absolute(offset)))
            .with_context(context)
            .with_portable(
                PortableLocation::new()
                    .with_stream(self.stream)
                    .with_heap_offset(offset),
            )
    }
}

/// Read one byte at `offset`.
fn read_u8(source: &(impl ReadAt + ?Sized), offset: u64) -> Result<u8> {
    let mut byte = [0];
    source.read_exact_at(offset, &mut byte)?;
    Ok(byte[0])
}
