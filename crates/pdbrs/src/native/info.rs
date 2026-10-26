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
//!
//! [`Info::encode`] writes a decoded stream again with the same bytes. [`InfoBuilder`] makes
//! or changes a stream with the insertion rules of Microsoft `NMTNI` and LLVM
//! `NamedStreamMap`, and [`IdentityPolicy`](super::IdentityPolicy) sets the matching identity.

mod builder;

pub use builder::{InfoBuilder, PeMatch};

use alloc::vec::Vec;
use core::ops::Range;

use super::Identity;
use super::hash_table::{Cursor, HashTable};
use super::msf::StreamId;
use super::names::HashVersion;
use crate::{Error, ErrorKind, Result};

/// The Info stream version that this reader decodes (`PdbImplVC70`).
pub const VERSION_VC70: u32 = 20_000_404;

/// The stream ID of the Info stream.
const INFO_STREAM: StreamId = StreamId(1);

/// A feature code after the named-stream map (`PdbRaw_FeatureSig` in LLVM `RawConstants.h`).
///
/// The value is open: a stream can have codes that this crate does not know.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Feature(pub u32);

impl Feature {
    /// `20091201`: the writer is Visual C++ 11.0 or later.
    pub const VC110: Self = Self(20_091_201);
    /// `20140508`: the PDB has an IPI stream (Visual C++ 14.0 or later).
    pub const VC140: Self = Self(20_140_508);
    /// `NOTM` (LLVM `NoTypeMerge`): the writer did not merge the type records.
    pub const NO_TYPE_MERGE: Self = Self(0x4d54_4f4e);
    /// `MINI`: a FASTLINK PDB (`/DEBUG:FASTLINK`). The symbol indexes use the larger
    /// bucket count, and the type records stay in the object files.
    pub const MINIMAL_DEBUG_INFO: Self = Self(0x494e_494d);
}

/// A named stream entry, with its name preserved as bytes.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct NamedStream<'a> {
    /// The name without its terminating NUL.
    pub name: &'a [u8],
    /// The byte offset of the name in the string buffer: the hash table key.
    ///
    /// Writers add names to the end of the buffer, so the offsets give the order of the
    /// insertions.
    pub name_offset: u32,
    /// The MSF stream index.
    pub stream: StreamId,
}

#[derive(Debug)]
struct Entry {
    name: Range<usize>,
    offset: u32,
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
    /// The string buffer of the named-stream map.
    strings: Range<usize>,
    /// The serialized hash table of the named-stream map.
    table: Range<usize>,
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
        let table_start = cursor.position();
        let table = HashTable::<u32>::read(&mut cursor)?;
        let table_end = cursor.position();
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(table.len())
            .map_err(|_| invalid(ErrorKind::LimitExceeded, cursor.position()))?;
        for entry in table.entries() {
            let length = name_length(strings, entry.key, string_start)?;
            let start = string_start + to_usize(entry.key);
            entries.push(Entry {
                name: start..start + length,
                offset: entry.key,
                stream: StreamId(entry.value),
            });
        }
        table.check_placement(|entry| {
            let start = to_usize(entry.key);
            let end = start + name_length(strings, entry.key, string_start)?;
            Ok(name_hash(&strings[start..end]))
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
            strings: string_start..table_start,
            table: table_start..table_end,
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
            name_offset: entry.offset,
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

    /// Return the feature codes in stream order, including codes unknown to this reader.
    pub fn features(&self) -> impl ExactSizeIterator<Item = Feature> + '_ {
        self.bytes[self.features.clone()]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| Feature(u32::from_le_bytes(*word)))
    }

    /// Return `true` if the stream has the feature code of `feature`.
    #[must_use]
    pub fn has_feature(&self, feature: Feature) -> bool {
        self.features().any(|code| code == feature)
    }

    /// Return the serialized bytes of the stream.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Append the serialized stream to `out`.
    ///
    /// The encoder writes each part from its decoded value: the header, the string buffer,
    /// the hash table with its bucket layout, `niMac` and the feature codes. A decoded
    /// stream thus gives the same bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        InfoBuilder::from_info(self)?.encode(out)
    }

    /// Return the string buffer of the named-stream map.
    fn string_buffer(&self) -> &[u8] {
        &self.bytes[self.strings.clone()]
    }

    /// Decode the hash table of the named-stream map again.
    fn table(&self) -> Result<HashTable<'_, u32>> {
        HashTable::<u32>::parse(&self.bytes, self.table.start, INFO_STREAM).map(|(table, _)| table)
    }
}

/// Return the named-stream map hash of a name: [`HashVersion::V1`] truncated to 16 bits.
fn name_hash(name: &[u8]) -> u32 {
    HashVersion::V1.hash(name) & 0xffff
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
        .with_stream(INFO_STREAM, u64::try_from(offset).unwrap_or(u64::MAX))
}
