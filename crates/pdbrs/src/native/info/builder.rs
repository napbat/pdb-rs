//! An owned PDB Info stream that changes and encodes.
//!
//! The named-stream map follows Microsoft `NMTNI` (`PDB/include/nmtni.h`) and LLVM
//! `NamedStreamMap.cpp`:
//!
//! 1. A new map has an empty string buffer and a hash table with 1 bucket (`Map(1)`,
//!    `OffsetIndexMap(1)`). Both producers write the same bytes for the same insertions.
//! 2. A new name goes to the end of the string buffer. The hash table key is its offset. If
//!    the name is already in the map, `NMTNI::addNiForSz` removes the new copy again.
//! 3. A removal (`NMTNI::deleteNiSzo`) marks the bucket as deleted. The name stays in the
//!    string buffer.
//!
//! The streams of the 29 native fixtures of #32 have the same bytes as a map that gets its
//! names in string buffer order with these rules.

use alloc::vec::Vec;

use super::{Feature, Info, VERSION_VC70, name_hash};
use crate::native::hash_table::{HashTableKeys, OwnedHashTable};
use crate::native::msf::StreamId;
use crate::native::output::{Output, no_memory, to_u32};
use crate::native::{Identity, IdentityPolicy, Producer};
use crate::{Error, ErrorKind, Result};

/// The values that the `RSDS` debug entry of the matching image must hold.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct PeMatch {
    /// The GUID bytes in their serialized order.
    pub guid: [u8; 16],
    /// The age: the DBI age of the PDB.
    pub age: u32,
}

/// The hash function and the name equality of the named-stream map.
struct NameKeys<'s> {
    /// The string buffer.
    strings: &'s [u8],
}

impl NameKeys<'_> {
    /// Return the name at `offset`.
    fn name(&self, offset: u32) -> Result<&[u8]> {
        let rest = usize::try_from(offset)
            .ok()
            .and_then(|start| self.strings.get(start..))
            .unwrap_or_default();
        let length = rest.iter().position(|&byte| byte == 0).ok_or_else(|| {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("named-stream key does not refer to a NUL-terminated name")
        })?;
        Ok(&rest[..length])
    }
}

impl HashTableKeys for NameKeys<'_> {
    fn hash(&self, key: u32) -> Result<u32> {
        Ok(name_hash(self.name(key)?))
    }

    fn same(&self, left: u32, right: u32) -> Result<bool> {
        Ok(left == right || self.name(left)? == self.name(right)?)
    }
}

/// An owned PDB Info stream.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct InfoBuilder {
    /// The stream version.
    version: u32,
    /// The 32-bit signature.
    signature: u32,
    /// The GUID bytes.
    guid: [u8; 16],
    /// The Info age.
    age: u32,
    /// The string buffer of the named-stream map.
    strings: Vec<u8>,
    /// The hash table from name offsets to stream numbers.
    table: OwnedHashTable<u32>,
    /// `niMac` of `NMTNI`.
    next_name_index: u32,
    /// The feature codes.
    features: Vec<Feature>,
}

impl InfoBuilder {
    /// Create a stream with an empty named-stream map and no feature codes.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the allocation fails.
    pub fn new(producer: Producer, signature: u32, guid: [u8; 16], age: u32) -> Result<Self> {
        Ok(Self {
            version: VERSION_VC70,
            signature,
            guid,
            age,
            strings: Vec::new(),
            table: OwnedHashTable::with_capacity(producer, 1)?,
            next_name_index: 0,
            features: Vec::new(),
        })
    }

    /// Copy a decoded stream. Later changes follow the rules of `producer`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the allocation fails.
    pub fn from_info(info: &Info) -> Result<Self> {
        Self::from_info_with(info, Producer::Microsoft)
    }

    /// Copy a decoded stream with the change rules of `producer`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the allocation fails.
    pub fn from_info_with(info: &Info, producer: Producer) -> Result<Self> {
        let Identity::Modern { guid, age } = info.identity() else {
            return Err(Error::new(ErrorKind::UnsupportedVersion, None)
                .with_context("PDB Info stream has no GUID identity"));
        };
        let source = info.string_buffer();
        let mut strings = Vec::new();
        strings
            .try_reserve_exact(source.len())
            .map_err(|_| no_memory())?;
        strings.extend_from_slice(source);
        let mut features = Vec::new();
        features
            .try_reserve_exact(info.features().len())
            .map_err(|_| no_memory())?;
        features.extend(info.features());
        Ok(Self {
            version: info.version(),
            signature: info.signature(),
            guid,
            age,
            strings,
            table: OwnedHashTable::from_table(&info.table()?, producer)?,
            next_name_index: info.next_name_index(),
            features,
        })
    }

    /// Return the GUID-and-age identity of the Info stream.
    #[must_use]
    pub const fn identity(&self) -> Identity {
        Identity::Modern {
            guid: self.guid,
            age: self.age,
        }
    }

    /// Return the 32-bit signature.
    #[must_use]
    pub const fn signature(&self) -> u32 {
        self.signature
    }

    /// Return the string buffer of the named-stream map, with names that a removal kept.
    #[must_use]
    pub fn string_buffer(&self) -> &[u8] {
        &self.strings
    }

    /// Return the bucket count of the named-stream map.
    #[must_use]
    pub const fn named_stream_capacity(&self) -> u32 {
        self.table.capacity()
    }

    /// Return the named streams in bucket order.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if a key does not refer to a name.
    pub fn named_streams(&self) -> Result<Vec<(Vec<u8>, StreamId)>> {
        let keys = self.keys();
        let mut streams = Vec::new();
        streams
            .try_reserve_exact(self.table.len())
            .map_err(|_| no_memory())?;
        for entry in self.table.entries() {
            let name = keys.name(entry.key)?;
            let mut owned = Vec::new();
            owned
                .try_reserve_exact(name.len())
                .map_err(|_| no_memory())?;
            owned.extend_from_slice(name);
            streams.push((owned, StreamId(*entry.value)));
        }
        Ok(streams)
    }

    /// Find the stream of a named stream.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if a key does not refer to a name.
    pub fn named_stream(&self, name: &[u8]) -> Result<Option<StreamId>> {
        let keys = self.keys();
        for entry in self.table.entries() {
            if keys.name(entry.key)? == name {
                return Ok(Some(StreamId(*entry.value)));
            }
        }
        Ok(None)
    }

    /// Add a named stream, or give an existing name another stream.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the name is empty or contains a NUL byte.
    /// - [`ErrorKind::LimitExceeded`] if the string buffer does not fit in `u32` or an
    ///   allocation fails.
    pub fn set_named_stream(&mut self, name: &[u8], stream: StreamId) -> Result<()> {
        let key = self.push_name(name)?;
        let keys = NameKeys {
            strings: &self.strings,
        };
        let result = self.table.insert(key, stream.0, &keys);
        if !matches!(result, Ok(None)) {
            // The name was there already, or the insertion failed: remove the new copy.
            self.retract(key);
        }
        result.map(|_| ())
    }

    /// Remove a named stream and return its stream. The name stays in the string buffer.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`InfoBuilder::set_named_stream`] for the name.
    pub fn remove_named_stream(&mut self, name: &[u8]) -> Result<Option<StreamId>> {
        let key = self.push_name(name)?;
        let keys = NameKeys {
            strings: &self.strings,
        };
        let removed = self.table.remove(key, &keys);
        self.retract(key);
        Ok(removed?.map(StreamId))
    }

    /// Return `niMac`, the word after the named-stream hash table.
    #[must_use]
    pub const fn next_name_index(&self) -> u32 {
        self.next_name_index
    }

    /// Set `niMac`. Microsoft `link.exe` and LLVM write 0.
    pub const fn set_next_name_index(&mut self, value: u32) {
        self.next_name_index = value;
    }

    /// Return the feature codes in stream order.
    pub fn features(&self) -> impl ExactSizeIterator<Item = Feature> + '_ {
        self.features.iter().copied()
    }

    /// Replace the feature codes.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the allocation fails.
    pub fn set_features(&mut self, features: &[Feature]) -> Result<()> {
        let mut codes = Vec::new();
        codes
            .try_reserve_exact(features.len())
            .map_err(|_| no_memory())?;
        codes.extend_from_slice(features);
        self.features = codes;
        Ok(())
    }

    /// Apply an identity policy to this stream and to the DBI header age `dbi_age`.
    ///
    /// Return the values for the `RSDS` debug entry of the matching image. Write the
    /// returned age to the image if the policy changed it.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] for [`IdentityPolicy::Preserve`] if the DBI age is
    ///   larger than the Info age.
    /// - [`ErrorKind::LimitExceeded`] for [`IdentityPolicy::IncrementAge`] if the age is
    ///   `u32::MAX`.
    pub fn apply_identity(&mut self, policy: IdentityPolicy, dbi_age: &mut u32) -> Result<PeMatch> {
        match policy {
            IdentityPolicy::Preserve => {
                if *dbi_age > self.age {
                    return Err(Error::new(ErrorKind::InvalidFormat, None)
                        .with_context("DBI age is larger than the PDB Info age"));
                }
            }
            IdentityPolicy::IncrementAge => {
                self.age = self.age.checked_add(1).ok_or_else(|| {
                    Error::new(ErrorKind::LimitExceeded, None).with_context("PDB age overflows")
                })?;
                *dbi_age = self.age;
            }
            IdentityPolicy::Replace {
                signature,
                guid,
                age,
            } => {
                self.signature = signature;
                self.guid = guid;
                self.age = age;
                *dbi_age = age;
            }
        }
        Ok(PeMatch {
            guid: self.guid,
            age: *dbi_age,
        })
    }

    /// Append the serialized stream to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if a size does not fit in `u32` or the output
    /// cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let mut output = Output::new(out);
        output.u32(self.version)?;
        output.u32(self.signature)?;
        output.u32(self.age)?;
        output.bytes(&self.guid)?;
        output.u32(to_u32(self.strings.len(), "named-stream string buffer")?)?;
        output.bytes(&self.strings)?;
        self.table.encode(out)?;
        let mut output = Output::new(out);
        output.u32(self.next_name_index)?;
        for feature in &self.features {
            output.u32(feature.0)?;
        }
        Ok(())
    }

    /// Return the hash function and the name equality of the map.
    fn keys(&self) -> NameKeys<'_> {
        NameKeys {
            strings: &self.strings,
        }
    }

    /// Append `name` and its NUL to the string buffer and return its offset.
    fn push_name(&mut self, name: &[u8]) -> Result<u32> {
        if name.is_empty() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("named-stream name is empty"));
        }
        let key = to_u32(self.strings.len(), "named-stream string buffer")?;
        to_u32(
            self.strings
                .len()
                .saturating_add(name.len())
                .saturating_add(1),
            "named-stream string buffer",
        )?;
        Output::new(&mut self.strings).c_string(name)?;
        Ok(key)
    }

    /// Remove the name at `key` and all bytes after it from the string buffer.
    fn retract(&mut self, key: u32) {
        if let Ok(start) = usize::try_from(key) {
            self.strings.truncate(start);
        }
    }
}
