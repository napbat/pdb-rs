//! PDB Info stream and named stream directory.
//!
//! The Info stream (stream 1) has this layout (LLVM `PdbStream.md`, LLVM
//! `NamedStreamMap.cpp`, Microsoft `PDB/include/nmtni.h`):
//!
//! 1. The version, the signature, the age and the GUID.
//! 2. The named-stream string buffer: a byte count and NUL-terminated names.
//! 3. A [`HashTable`] from a name offset in the string buffer to a stream index.
//! 4. `niMac`, the next name index of the `NMTNI` name table.
//! 5. Feature codes until the end of the stream.
//!
//! The hash of a name is `hash_string_v1(name)` truncated to 16 bits (`NMTNI::hash`).
//! [`Info::parse`] checks that a linear-probing lookup finds each name.

use alloc::vec::Vec;
use core::ops::Range;

use super::Identity;
use super::hash::hash_string_v1;
use super::hash_table::{Cursor, HashTable};
use super::msf::StreamId;
use crate::{Error, ErrorKind, Result};

/// The Info stream version that this reader decodes (`PdbImplVC70`).
const VERSION_VC70: u32 = 20_000_404;

/// The stream ID of the Info stream.
const INFO_STREAM: StreamId = StreamId(1);

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
    capacity: u32,
    next_name_index: u32,
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
    /// a name outside its probe sequence, truncated input, arithmetic overflow or
    /// failed allocation. Errors have stream 1 and a logical offset.
    pub fn parse(bytes: Vec<u8>) -> Result<Self> {
        let input = bytes.as_slice();
        let mut cursor = Cursor::new(input, INFO_STREAM, "PDB Info");
        let version = cursor.u32()?;
        if version != VERSION_VC70 {
            return Err(invalid(ErrorKind::UnsupportedVersion, 0));
        }
        let signature = cursor.u32()?;
        let age = cursor.u32()?;
        let guid = cursor.array::<16>()?;
        let string_size = usize::try_from(cursor.u32()?)
            .map_err(|_| invalid(ErrorKind::LimitExceeded, cursor.position()))?;
        let string_start = cursor.position();
        let strings = cursor.take(string_size)?;
        let table = HashTable::<u32>::read(&mut cursor)?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(table.len())
            .map_err(|_| invalid(ErrorKind::LimitExceeded, cursor.position()))?;
        for entry in table.entries() {
            let length = name_length(strings, entry.key, string_start)?;
            let start = string_start + to_usize(entry.key);
            entries.push(Entry {
                name: start..start + length,
                stream: StreamId(entry.value),
            });
        }
        table.check_placement(|entry| {
            let start = to_usize(entry.key);
            let end = start + name_length(strings, entry.key, string_start)?;
            Ok(hash_string_v1(&strings[start..end]) & 0xffff)
        })?;
        entries.sort_unstable_by(|left, right| {
            input[left.name.clone()].cmp(&input[right.name.clone()])
        });
        if entries
            .windows(2)
            .any(|pair| input[pair[0].name.clone()] == input[pair[1].name.clone()])
        {
            return Err(invalid(ErrorKind::InvalidFormat, string_start));
        }
        let capacity = table.capacity();
        let next_name_index = cursor.u32()?;
        let features = cursor.position()..input.len();
        if features.len() % 4 != 0 {
            return Err(invalid(ErrorKind::InvalidFormat, features.start));
        }
        Ok(Self {
            bytes,
            version,
            signature,
            identity: Identity::Modern { guid, age },
            entries,
            capacity,
            next_name_index,
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

    /// Return the bucket count of the named-stream hash table.
    #[must_use]
    pub const fn named_stream_capacity(&self) -> u32 {
        self.capacity
    }

    /// Return `niMac`, the next name index of the named-stream name table.
    ///
    /// Microsoft `NMTNI` stores this value after the hash table. LLVM writes zero.
    #[must_use]
    pub const fn next_name_index(&self) -> u32 {
        self.next_name_index
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

/// Return the length of the NUL-terminated name at `offset` in the string buffer.
fn name_length(strings: &[u8], offset: u32, string_start: usize) -> Result<usize> {
    let start = to_usize(offset);
    let suffix = strings
        .get(start..)
        .ok_or_else(|| invalid(ErrorKind::InvalidReference, string_start))?;
    suffix
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| invalid(ErrorKind::InvalidFormat, string_start + start))
}

/// Convert a name offset to `usize`. An offset that does not fit is out of range.
fn to_usize(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

fn invalid(kind: ErrorKind, offset: usize) -> Error {
    Error::new(kind, None)
        .with_context("PDB Info")
        .with_stream(INFO_STREAM.0, u64::try_from(offset).unwrap_or(u64::MAX))
}
