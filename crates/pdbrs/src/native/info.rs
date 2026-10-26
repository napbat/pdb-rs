//! PDB Info stream and named stream directory.

use alloc::vec::Vec;
use core::ops::Range;

use super::Identity;
use super::msf::StreamId;
use crate::io::ReadAt;
use crate::{Error, ErrorKind, Result};

/// A named stream entry, with its name preserved as bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NamedStream<'a> {
    /// The name without its terminating NUL.
    pub name: &'a [u8],
    /// The MSF stream index.
    pub stream: StreamId,
}

#[derive(Debug)]
struct Entry {
    name: Range<usize>,
    stream: StreamId,
}

/// A decoded modern PDB Info stream with owned serialized bytes.
#[derive(Debug)]
pub struct Info {
    bytes: Vec<u8>,
    version: u32,
    signature: u32,
    identity: Identity,
    entries: Vec<Entry>,
    features: Range<usize>,
}

impl Info {
    /// Decode a modern Info stream, including the named stream hash table.
    ///
    /// Unknown feature codes remain available. This checks stream references
    /// syntactically; the owning PDB checks that the referenced streams exist.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported version, malformed table or string,
    /// truncated input, arithmetic overflow or failed allocation.
    pub fn parse(bytes: Vec<u8>) -> Result<Self> {
        let input = bytes.as_slice();
        let mut reader = Reader {
            bytes: input,
            position: 0,
        };
        let version = reader.u32()?;
        if version != 20_000_404 {
            return Err(invalid(ErrorKind::UnsupportedVersion, 0));
        }
        let signature = reader.u32()?;
        let age = reader.u32()?;
        let mut guid = [0; 16];
        guid.copy_from_slice(reader.take(16)?);
        let string_size = usize::try_from(reader.u32()?)
            .map_err(|_| invalid(ErrorKind::LimitExceeded, reader.position))?;
        let string_start = reader.position;
        let strings = reader.take(string_size)?;
        let size = reader.u32()?;
        let capacity = reader.u32()?;
        if size > capacity {
            return Err(invalid(ErrorKind::InvalidFormat, reader.position - 8));
        }
        let present = reader.bit_vector(capacity)?;
        let deleted = reader.bit_vector(capacity)?;
        let mut population = 0_u64;
        for (index, word) in present.as_chunks::<4>().0.iter().enumerate() {
            let value = u32::from_le_bytes(*word);
            population += u64::from(value.count_ones());
            if let Some(other) = deleted.get(index * 4..index * 4 + 4) {
                let tombstones = u32::from_le_bytes([other[0], other[1], other[2], other[3]]);
                if value & tombstones != 0 {
                    return Err(invalid(ErrorKind::InvalidFormat, reader.position));
                }
            }
        }
        if population != u64::from(size) {
            return Err(invalid(ErrorKind::InvalidFormat, reader.position));
        }
        let count = usize::try_from(size)
            .map_err(|_| invalid(ErrorKind::LimitExceeded, reader.position))?;
        if count > (input.len() - reader.position) / 8 {
            return Err(invalid(ErrorKind::UnexpectedEof, reader.position));
        }
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(count)
            .map_err(|_| invalid(ErrorKind::LimitExceeded, reader.position))?;
        for _ in 0..count {
            let name_offset = usize::try_from(reader.u32()?)
                .map_err(|_| invalid(ErrorKind::InvalidReference, reader.position))?;
            let stream = StreamId(reader.u32()?);
            let suffix = strings
                .get(name_offset..)
                .ok_or_else(|| invalid(ErrorKind::InvalidReference, reader.position - 8))?;
            let length = suffix
                .iter()
                .position(|byte| *byte == 0)
                .ok_or_else(|| invalid(ErrorKind::InvalidFormat, string_start + name_offset))?;
            let start = string_start + name_offset;
            entries.push(Entry {
                name: start..start + length,
                stream,
            });
        }
        entries.sort_unstable_by(|left, right| {
            input[left.name.clone()].cmp(&input[right.name.clone()])
        });
        if entries
            .windows(2)
            .any(|pair| input[pair[0].name.clone()] == input[pair[1].name.clone()])
        {
            return Err(invalid(ErrorKind::InvalidFormat, string_start));
        }
        let features = reader.position..input.len();
        if features.len() % 4 != 0 {
            return Err(invalid(ErrorKind::InvalidFormat, features.start));
        }
        Ok(Self {
            bytes,
            version,
            signature,
            identity: Identity::Modern { guid, age },
            entries,
            features,
        })
    }

    /// Return the Info stream layout version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Return the original 32-bit signature.
    #[must_use]
    pub const fn signature(&self) -> u32 {
        self.signature
    }

    /// Return the GUID-and-age identity.
    #[must_use]
    pub const fn identity(&self) -> Identity {
        self.identity
    }

    /// Iterate named streams in bytewise name order.
    pub fn named_streams(&self) -> impl ExactSizeIterator<Item = NamedStream<'_>> {
        self.entries.iter().map(|entry| NamedStream {
            name: &self.bytes[entry.name.clone()],
            stream: entry.stream,
        })
    }

    /// Find a named stream without decoding its name as text.
    #[must_use]
    pub fn named_stream(&self, name: &[u8]) -> Option<StreamId> {
        self.entries
            .binary_search_by(|entry| self.bytes[entry.name.clone()].cmp(name))
            .ok()
            .map(|index| self.entries[index].stream)
    }

    /// Return feature codes, including codes unknown to this reader.
    pub fn features(&self) -> impl ExactSizeIterator<Item = u32> + '_ {
        self.bytes[self.features.clone()]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| u32::from_le_bytes(*word))
    }
}

fn invalid(kind: ErrorKind, offset: usize) -> Error {
    Error::new(kind, None)
        .with_context("PDB Info")
        .with_stream(1, offset as u64)
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(count)
            .ok_or_else(|| invalid(ErrorKind::OffsetOverflow, self.position))?;
        let result = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| invalid(ErrorKind::UnexpectedEof, self.position))?;
        self.position = end;
        Ok(result)
    }

    fn u32(&mut self) -> Result<u32> {
        let position = self.position;
        self.position = self
            .position
            .checked_add(4)
            .ok_or_else(|| invalid(ErrorKind::OffsetOverflow, position))?;
        self.bytes
            .read_u32_at(position as u64)
            .map_err(|_| invalid(ErrorKind::UnexpectedEof, position))
    }

    fn bit_vector(&mut self, capacity: u32) -> Result<&'a [u8]> {
        let count = self.u32()?;
        if count > capacity.div_ceil(32) {
            return Err(invalid(ErrorKind::InvalidFormat, self.position - 4));
        }
        let length = usize::try_from(u64::from(count) * 4)
            .map_err(|_| invalid(ErrorKind::LimitExceeded, self.position))?;
        let words = self.take(length)?;
        if count != 0 && count == capacity.div_ceil(32) && !capacity.is_multiple_of(32) {
            let last = words.read_u32_at(u64::from(count - 1) * 4)?;
            if last >> (capacity % 32) != 0 {
                return Err(invalid(ErrorKind::InvalidFormat, self.position - 4));
            }
        }
        Ok(words)
    }
}
