//! Zero-copy framing of `CodeView` records.
//!
//! A `CodeView` record starts with a little-endian `u16` length and a
//! little-endian `u16` kind. The length counts the kind and the payload. It
//! does not count the length field. Symbol records and type records use the
//! same framing.
//!
//! [`Records`] frames a byte sequence into [`Record`] values. It keeps
//! records of all kinds, also kinds that this crate cannot decode. It keeps
//! the padding at the end of a record as part of [`Record::data`].
//!
//! [`Record::symbol`] decodes a symbol record into a typed
//! [`symbol::Symbol`]. It does not demangle names.

use core::iter::FusedIterator;

use crate::{Error, ErrorKind, Result};

pub mod field_list;
pub mod id_record;
pub mod index;
pub mod numeric;
pub mod type_record;
<<<<<<< HEAD
#[expect(
    dead_code,
    reason = "the type record encoders of #29 use the rest of the shared writer"
)]
=======
>>>>>>> main
pub(crate) mod writer;

pub mod symbol;

use index::{IdIndex, TypeIndex};
use numeric::{Integer, Numeric};

/// The size of the length field and the kind field at the start of a record.
const PREFIX_LEN: usize = 4;

/// The smallest valid value of the length field. It covers the kind field.
const MIN_RECORD_LEN: u16 = 2;

/// One framed `CodeView` record.
///
/// The record borrows its payload from the sequence that [`Records`] frames.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Record<'a> {
    /// The byte offset of the length field in the framed sequence.
    ///
    /// This offset is relative to the slice given to [`Records::new`]. It is
    /// not a file offset.
    pub offset: u32,
    /// The record kind, for example [`symbol::S_PUB32`] or a type leaf kind.
    pub kind: u16,
    /// The payload after the kind field.
    ///
    /// The payload includes the padding at the end of the record.
    pub data: &'a [u8],
}

impl<'a> Record<'a> {
    /// Decode the record as a symbol.
    ///
    /// A kind that this crate does not decode gives
    /// [`symbol::SymbolData::Unsupported`] with the raw payload. Do not use
    /// this function on type records: type leaf kinds and symbol kinds use
    /// different value spaces.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`] if the payload of a decoded kind
    /// is too short for its fields, has a name without a NUL terminator, has
    /// padding that is not zero, or breaks a rule of its kind. Returns
    /// [`ErrorKind::UnsupportedVersion`] for a constant value that is not an
    /// integer. The error does not have a file offset.
    pub fn symbol(&self) -> Result<symbol::Symbol<'a>> {
        symbol::Symbol::decode(*self)
    }
}

/// An iterator that frames a byte sequence into `CodeView` records.
///
/// The iterator does not copy record bytes. It stops after the last complete
/// record at the end of the sequence. It returns one error for a malformed
/// record and then stops.
#[derive(Clone, Debug)]
pub struct Records<'a> {
    /// The full sequence.
    bytes: &'a [u8],
    /// The offset of the next record in `bytes`.
    position: usize,
    /// `true` after the iterator returned an error.
    failed: bool,
}

impl<'a> Records<'a> {
    /// Create an iterator over the records in `bytes`.
    ///
    /// The first record starts at offset 0. Remove a stream signature before
    /// you call this function.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            position: 0,
            failed: false,
        }
    }

    /// Return the offset of the next record in the sequence.
    ///
    /// After an error, the offset identifies the start of the record that
    /// caused the error. Use this offset to add a location to the error.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

    /// Frame the record at the current position and move past it.
    fn read(&mut self) -> Result<Record<'a>> {
        let offset = u32::try_from(self.position).map_err(|_| {
            Error::new(ErrorKind::OffsetOverflow, None)
                .with_context("CodeView record offset does not fit in 32 bits")
        })?;
        let rest = &self.bytes[self.position..];
        let Some((&[l0, l1, k0, k1], _)) = rest.split_first_chunk::<PREFIX_LEN>() else {
            return Err(Error::new(ErrorKind::UnexpectedEof, None)
                .with_context("CodeView record prefix is truncated"));
        };
        let length = u16::from_le_bytes([l0, l1]);
        if length < MIN_RECORD_LEN {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("CodeView record length does not cover the kind"));
        }
        // The length field does not count itself.
        let total = usize::from(length) + 2;
        let Some(record) = rest.get(PREFIX_LEN..total) else {
            return Err(Error::new(ErrorKind::UnexpectedEof, None)
                .with_context("CodeView record extends past the end of the sequence"));
        };
        // `total` bytes are inside `rest`, so the sum is at most `bytes.len()`.
        self.position += total;
        Ok(Record {
            offset,
            kind: u16::from_le_bytes([k0, k1]),
            data: record,
        })
    }
}

impl<'a> Iterator for Records<'a> {
    type Item = Result<Record<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.position >= self.bytes.len() {
            return None;
        }
        let result = self.read();
        self.failed = result.is_err();
        Some(result)
    }
}

impl FusedIterator for Records<'_> {}

/// A reader for little-endian fixed fields in a bounded byte slice.
///
/// Each read moves past the bytes that it consumes. A read past the end
/// returns an error with the configured kind and context, and no file offset.
pub(crate) struct Fields<'a> {
    /// The bytes that are not read yet.
    bytes: &'a [u8],
    /// The error kind for a read past the end.
    kind: ErrorKind,
    /// The error context for a read past the end.
    context: &'static str,
}

impl<'a> Fields<'a> {
    /// Create a reader over `bytes`.
    pub(crate) const fn new(bytes: &'a [u8], kind: ErrorKind, context: &'static str) -> Self {
        Self {
            bytes,
            kind,
            context,
        }
    }

    /// Return the error for a read past the end.
    fn short(&self) -> Error {
        Error::new(self.kind, None).with_context(self.context)
    }

    /// Read a fixed number of bytes.
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let bytes = self.bytes;
        let (head, rest) = bytes.split_first_chunk::<N>().ok_or_else(|| self.short())?;
        self.bytes = rest;
        Ok(*head)
    }

    /// Read a `u8`.
    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(u8::from_le_bytes(self.array()?))
    }

    /// Read a little-endian `u16`.
    pub(crate) fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    /// Read a little-endian `u32`.
    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    /// Read a little-endian `i32`.
    pub(crate) fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    /// Read a little-endian `u64`.
    pub(crate) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    /// Read a 32-bit type index.
    pub(crate) fn type_index(&mut self) -> Result<TypeIndex> {
        Ok(TypeIndex(self.u32()?))
    }

    /// Read a 32-bit ID index.
    pub(crate) fn id_index(&mut self) -> Result<IdIndex> {
        Ok(IdIndex(self.u32()?))
    }
    /// Read `count` bytes.
    pub(crate) fn bytes(&mut self, count: usize) -> Result<&'a [u8]> {
        if count > self.bytes.len() {
            return Err(self.short());
        }
        let (head, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Ok(head)
    }

    /// Read a NUL-terminated name and consume its terminator.
    ///
    /// The name excludes the terminator. It is not decoded as text.
    pub(crate) fn name(&mut self) -> Result<&'a [u8]> {
        let end = self
            .bytes
            .iter()
            .position(|&byte| byte == 0)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("CodeView name has no NUL terminator")
            })?;
        let (name, rest) = self.bytes.split_at(end);
        self.bytes = &rest[1..];
        Ok(name)
    }

    /// Read an integer numeric leaf.
    ///
    /// A truncated leaf returns the configured error. A non-integer leaf
    /// returns [`ErrorKind::UnsupportedVersion`].
    pub(crate) fn integer(&mut self) -> Result<Integer> {
        let (value, rest) = Integer::parse_prefix(self.bytes).map_err(|error| {
            if error.kind() == ErrorKind::UnexpectedEof {
                self.short()
            } else {
                error
            }
        })?;
        self.bytes = rest;
        Ok(value)
    }

    /// Read a numeric leaf of any kind.
    ///
    /// A truncated leaf returns the configured error. An unknown leaf tag
    /// returns [`ErrorKind::UnsupportedVersion`].
    pub(crate) fn numeric(&mut self) -> Result<Numeric<'a>> {
        let (value, rest) = Numeric::parse_prefix(self.bytes).map_err(|error| {
            if error.kind() == ErrorKind::UnexpectedEof {
                self.short()
            } else {
                error
            }
        })?;
        self.bytes = rest;
        Ok(value)
    }

    /// Return `true` if all bytes are read.
    pub(crate) const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
    /// Make sure that the remaining bytes are type-record padding.
    ///
    /// Type records pad with `LF_PAD` bytes. Each byte is `0xf0` plus the
    /// number of padding bytes from that byte to the end, for example
    /// `f3 f2 f1`. At most three padding bytes are valid.
    pub(crate) fn finish_type_padding(&self) -> Result<()> {
        let count = self.bytes.len();
        let valid = count < 4
            && self
                .bytes
                .iter()
                .zip((1..=count).rev())
                .all(|(&byte, left)| usize::from(byte) == 0xf0 + left);
        if valid {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("CodeView type record has invalid padding"))
        }
    }

    /// Make sure that the remaining bytes are fewer than four zero bytes.
    ///
    /// Symbol records in PDB streams pad with zero bytes.
    pub(crate) fn finish_zero_padding(&self) -> Result<()> {
        if self.bytes.len() < 4 && self.bytes.iter().all(|&byte| byte == 0) {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("CodeView symbol record has invalid padding"))
        }
    }
    /// Return the bytes that are not read yet.
    pub(crate) const fn rest(&self) -> &'a [u8] {
        self.bytes
    }
}
