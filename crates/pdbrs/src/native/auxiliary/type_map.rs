//! The type map streams that a `/TMCache` stream names.
//!
//! Each stream caches one type map of an incremental link. The layout comes from
//! `DBI1::DumpTMCache` in `mspdbcore.dll` of MSVC 14.51. All
//! values are little-endian `u32` values unless noted.
//!
//! Version 1 maps a type server (a `/Zi` or `/ZI` compiler PDB):
//!
//! 1. The version (1), the type count and the ID count.
//! 2. The type map: one PDB type index for each type of the type server, from index `0x1000`.
//! 3. The ID map: one PDB index for each ID of the type server, from index `0x1000`.
//! 4. Function ID pairs to the end of the stream.
//!
//! Version 2 maps the `.debug$T` records of one object file:
//!
//! 1. The version (2), a `u16` stream of the precompiled header type map (`0xffff`: none), 2
//!    padding bytes, a signature, the first index and the index count.
//! 2. The map: one PDB index for each index of the object file, from the first index.
//! 3. The byte offsets of the object file records: one more than the index count.
//! 4. Function ID pairs to the end of the stream.
//!
//! A map value of 0 marks an index that the linker did not map. A function ID pair holds a
//! function ID of the input and the PDB type index of its function type.
//!
//! [`TypeMapStream::encode`] gives the bytes of a decoded stream again. It keeps the 2 padding
//! bytes of version 2. [`TypeMapStream::validate`] checks the references to the TPI and IPI
//! streams.

mod validate;

use alloc::vec::Vec;

use crate::native::msf::StreamId;
use crate::native::output::{Output, to_u32};
use crate::{Error, ErrorKind, Result};

pub use validate::{TypeMapContext, TypeMapRule, TypeMapViolation};

/// The first type index and the first ID index of a type server.
const TYPE_SERVER_FIRST_INDEX: u32 = 0x1000;

/// The kind of a type map stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum TypeMapKind {
    /// Version 1: the type map of a type server, with separate type and ID maps.
    TypeServer,
    /// Version 2: the type map of an object file.
    Object {
        /// The type map stream of the precompiled header that the object file uses.
        precompiled: Option<StreamId>,
        /// The signature field.
        signature: u32,
    },
}

/// A decoded type map stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TypeMapStream<'a> {
    /// The stream kind and its header fields.
    kind: TypeMapKind,
    /// The 2 bytes after the precompiled header stream of version 2, kept raw.
    padding: [u8; 2],
    /// The first input index of the maps.
    first_index: u32,
    /// The type map (version 1) or the map (version 2).
    types: &'a [[u8; 4]],
    /// The ID map; empty for version 2.
    ids: &'a [[u8; 4]],
    /// The record offsets; empty for version 1.
    offsets: &'a [[u8; 4]],
    /// The function ID pairs.
    functions: &'a [[u8; 8]],
}

impl<'a> TypeMapStream<'a> {
    /// Decode the type map stream `stream`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] if the header or a map is truncated.
    /// - [`ErrorKind::UnsupportedVersion`] for a version other than 1 or 2.
    /// - [`ErrorKind::InvalidFormat`] if the function ID pairs do not fill the rest of the stream.
    ///
    /// Errors identify `stream` and a logical offset in it.
    pub fn parse(bytes: &'a [u8], stream: StreamId) -> Result<Self> {
        let error = |kind, offset: usize, context| {
            Error::new(kind, None)
                .with_context(context)
                .with_stream(stream, u64::try_from(offset).unwrap_or(u64::MAX))
        };
        let words = bytes.as_chunks::<4>().0;
        let truncated = |offset| {
            error(
                ErrorKind::UnexpectedEof,
                offset,
                "type map stream is truncated",
            )
        };
        let Some(&version) = words.first() else {
            return Err(truncated(bytes.len()));
        };
        let value = |index: usize| words.get(index).map(|word| u32::from_le_bytes(*word));
        let (kind, padding, first_index, header, counts) = match u32::from_le_bytes(version) {
            1 => {
                let (Some(types), Some(ids)) = (value(1), value(2)) else {
                    return Err(truncated(bytes.len()));
                };
                (
                    TypeMapKind::TypeServer,
                    [0; 2],
                    TYPE_SERVER_FIRST_INDEX,
                    3,
                    [types, ids, 0],
                )
            }
            2 => {
                let (Some(pch), Some(signature), Some(first), Some(count)) =
                    (words.get(1), value(2), value(3), value(4))
                else {
                    return Err(truncated(bytes.len()));
                };
                let precompiled = match u16::from_le_bytes([pch[0], pch[1]]) {
                    u16::MAX => None,
                    number => Some(StreamId(u32::from(number))),
                };
                let offsets = count.checked_add(1).ok_or_else(|| truncated(16))?;
                let kind = TypeMapKind::Object {
                    precompiled,
                    signature,
                };
                (kind, [pch[2], pch[3]], first, 5, [count, 0, offsets])
            }
            _ => {
                return Err(error(
                    ErrorKind::UnsupportedVersion,
                    0,
                    "type map stream version is not 1 or 2",
                ));
            }
        };
        let mut rest = &words[header..];
        let mut parts = [&[][..]; 3];
        for (part, count) in parts.iter_mut().zip(counts) {
            let count = usize::try_from(count).unwrap_or(usize::MAX);
            let Some((taken, tail)) = rest.split_at_checked(count) else {
                return Err(truncated(bytes.len()));
            };
            *part = taken;
            rest = tail;
        }
        let consumed = (words.len() - rest.len()) * 4;
        let (functions, tail) = bytes[consumed..].as_chunks::<8>();
        if !tail.is_empty() {
            return Err(error(
                ErrorKind::InvalidFormat,
                bytes.len() - tail.len(),
                "type map function ID pairs do not fill the stream",
            ));
        }
        let [types, ids, offsets] = parts;
        Ok(Self {
            kind,
            padding,
            first_index,
            types,
            ids,
            offsets,
            functions,
        })
    }

    /// Return the stream kind and its header fields.
    #[must_use]
    pub const fn kind(&self) -> TypeMapKind {
        self.kind
    }

    /// Return the first input index of the maps.
    ///
    /// A type server map starts at `0x1000`. An object file map stores the value.
    #[must_use]
    pub const fn first_index(&self) -> u32 {
        self.first_index
    }

    /// Return the PDB index of each input type, from [`TypeMapStream::first_index`] up.
    ///
    /// For an object file, the map covers all record indices of the `.debug$T` section.
    #[must_use]
    pub fn types(&self) -> TypeMapWords<'a> {
        TypeMapWords(self.types.iter())
    }

    /// Return the PDB ID index of each type server ID, from `0x1000` up.
    ///
    /// The iterator is empty for an object file map.
    #[must_use]
    pub fn ids(&self) -> TypeMapWords<'a> {
        TypeMapWords(self.ids.iter())
    }

    /// Return the byte offsets of the object file records.
    ///
    /// The iterator has one item more than [`TypeMapStream::types`] for an object file map.
    /// It is empty for a type server map.
    #[must_use]
    pub fn record_offsets(&self) -> TypeMapWords<'a> {
        TypeMapWords(self.offsets.iter())
    }

    /// Return the function ID pairs: an input function ID and a PDB type index.
    #[must_use]
    pub fn function_types(&self) -> FunctionTypes<'a> {
        FunctionTypes(self.functions.iter())
    }

    /// Decode a function ID pair.
    fn function_type(pair: [u8; 8]) -> (u32, u32) {
        {
            let [a0, a1, a2, a3, b0, b1, b2, b3] = pair;
            (
                u32::from_le_bytes([a0, a1, a2, a3]),
                u32::from_le_bytes([b0, b1, b2, b3]),
            )
        }
    }

    /// Append the stream bytes to `out`.
    ///
    /// A decoded stream gives the same bytes again.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if a count or the precompiled header stream number
    /// does not fit its field, or if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let count = |len: usize| to_u32(len, "type map count");
        let mut output = Output::new(out);
        match self.kind {
            TypeMapKind::TypeServer => {
                output.u32(1)?;
                output.u32(count(self.types.len())?)?;
                output.u32(count(self.ids.len())?)?;
            }
            TypeMapKind::Object {
                precompiled,
                signature,
            } => {
                let stream = match precompiled {
                    None => u16::MAX,
                    Some(id) => u16::try_from(id.0)
                        .ok()
                        .filter(|&number| number != u16::MAX)
                        .ok_or_else(|| {
                            Error::new(ErrorKind::LimitExceeded, None).with_context(
                                "type map precompiled header stream does not fit in u16",
                            )
                        })?,
                };
                output.u32(2)?;
                output.u16(stream)?;
                output.bytes(&self.padding)?;
                output.u32(signature)?;
                output.u32(self.first_index)?;
                output.u32(count(self.types.len())?)?;
            }
        }
        for part in [self.types, self.ids, self.offsets] {
            output.bytes(part.as_flattened())?;
        }
        output.bytes(self.functions.as_flattened())
    }
}

/// An iterator over the 32-bit values of one table of a [`TypeMapStream`].
#[derive(Clone, Debug)]
pub struct TypeMapWords<'a>(core::slice::Iter<'a, [u8; 4]>);

impl Iterator for TypeMapWords<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        self.0.next().map(|x| u32::from_le_bytes(*x))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl ExactSizeIterator for TypeMapWords<'_> {}

impl core::iter::FusedIterator for TypeMapWords<'_> {}

/// An iterator over the function ID pairs of a [`TypeMapStream`].
#[derive(Clone, Debug)]
pub struct FunctionTypes<'a>(core::slice::Iter<'a, [u8; 8]>);

impl Iterator for FunctionTypes<'_> {
    type Item = (u32, u32);

    fn next(&mut self) -> Option<(u32, u32)> {
        self.0.next().map(|x| TypeMapStream::function_type(*x))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl ExactSizeIterator for FunctionTypes<'_> {}

impl core::iter::FusedIterator for FunctionTypes<'_> {}
