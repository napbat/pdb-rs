//! The optional debug streams that the DBI optional debug header names.
//!
//! Each [`DebugStream`] slot of the DBI optional debug header names one stream.
//! The layouts come from `langapi/include/pdb.h` and `PDB/dbi/dbi.h` of
//! `microsoft/microsoft-pdb` (`DBGTYPE`, `Dbg1` and its subclasses):
//!
//! | Slot | Kind | Layout | Type |
//! | --- | --- | --- | --- |
//! | 0 | [`DebugStream::Fpo`] | 16-byte `FPO_DATA` | [`FpoStream`] |
//! | 1 | [`DebugStream::Exception`] | 12-byte `IMAGE_FUNCTION_ENTRY` | [`FunctionEntryStream`] |
//! | 2 | [`DebugStream::Fixup`] | 12-byte `XFIXUP_DATA` | [`FixupStream`] |
//! | 3, 4 | [`DebugStream::OmapToSource`], [`DebugStream::OmapFromSource`] | 8-byte `OMAP_DATA` | [`OmapStream`] |
//! | 5, 10 | [`DebugStream::SectionHeader`], [`DebugStream::OriginalSectionHeader`] | 40-byte `IMAGE_SECTION_HEADER` | [`SectionHeaderStream`] |
//! | 6 | [`DebugStream::TokenRidMap`] | 4-byte `ULONG` | [`TokenRidMap`] |
//! | 7, 8 | [`DebugStream::Xdata`], [`DebugStream::Pdata`] | `DbgRvaVaBlob` header and data | [`RvaVaBlob`] |
//! | 9 | [`DebugStream::NewFpo`] | 32-byte `FRAMEDATA` | [`FrameDataStream`] |
//!
//! The streams have no header, except the blobs of slots 7 and 8. A
//! [`RecordStream`] holds an array of fixed-size records. Its decoder rejects
//! a length that is not a multiple of the record size. Its encoder encodes each
//! decoded record again, so that the output shows the record codec.
//!
//! [`DebugStream`]: super::dbi::DebugStream
//! [`DebugStream::Fpo`]: super::dbi::DebugStream::Fpo
//! [`DebugStream::Exception`]: super::dbi::DebugStream::Exception
//! [`DebugStream::Fixup`]: super::dbi::DebugStream::Fixup
//! [`DebugStream::OmapToSource`]: super::dbi::DebugStream::OmapToSource
//! [`DebugStream::OmapFromSource`]: super::dbi::DebugStream::OmapFromSource
//! [`DebugStream::SectionHeader`]: super::dbi::DebugStream::SectionHeader
//! [`DebugStream::OriginalSectionHeader`]: super::dbi::DebugStream::OriginalSectionHeader
//! [`DebugStream::TokenRidMap`]: super::dbi::DebugStream::TokenRidMap
//! [`DebugStream::Xdata`]: super::dbi::DebugStream::Xdata
//! [`DebugStream::Pdata`]: super::dbi::DebugStream::Pdata
//! [`DebugStream::NewFpo`]: super::dbi::DebugStream::NewFpo
//! [`FpoStream`]: super::fpo::FpoStream
//! [`FrameDataStream`]: super::fpo::FrameDataStream
//! [`SectionHeaderStream`]: super::section::SectionHeaderStream

pub mod blob;
pub mod fixup;
pub mod function;
pub mod omap;

use alloc::vec::Vec;
use core::iter::FusedIterator;
use core::marker::PhantomData;

use super::msf::StreamId;
use super::output::Output;
use crate::{Error, ErrorKind, Result};

pub use blob::{Arm64RuntimeFunction, BlobHeader, RuntimeFunctions, RvaVaBlob, X64RuntimeFunction};
pub use fixup::{Fixup, FixupStream};
pub use function::{FunctionEntry, FunctionEntryStream};
pub use omap::{OmapRecord, OmapStream};

/// The crate-private codec of a [`DebugRecord`].
mod sealed {
    /// Decode a record and name it in errors.
    pub trait Codec: Sized {
        /// The error context for a truncated record.
        const TRUNCATED: &'static str;

        /// Decode one record from its 4-byte words.
        ///
        /// Callers give exactly `SIZE / 4` words.
        fn decode(record: &[[u8; 4]]) -> Self;
    }
}

pub(crate) use sealed::Codec;

/// A fixed-size record of an optional debug stream.
///
/// The trait is sealed. The crate implements it for the record types of the
/// optional debug streams.
pub trait DebugRecord: Copy + Codec {
    /// The size of one serialized record in bytes. The size is a multiple of 4.
    const SIZE: usize;

    /// Append the serialized record to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    fn encode(&self, out: &mut Vec<u8>) -> Result<()>;
}

/// An entry of the token to RID map (slot 6): one `ULONG`.
///
/// `DbgTokenRidMap` in `dbi.h` stores an array of `ULONG` values. The PDB
/// sources do not give the meaning of the index. This decoder keeps the values.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Rid(pub u32);

impl Codec for Rid {
    const TRUNCATED: &'static str = "token RID map entry is truncated";

    fn decode(record: &[[u8; 4]]) -> Self {
        Self(u32::from_le_bytes(record[0]))
    }
}

impl DebugRecord for Rid {
    const SIZE: usize = 4;

    fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        Output::new(out).u32(self.0)
    }
}

/// A decoded token to RID map stream (slot 6).
pub type TokenRidMap = RecordStream<Rid>;

/// A decoded array of fixed-size records with owned serialized bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordStream<R> {
    /// The stream bytes; the length is a multiple of `R::SIZE`.
    bytes: Vec<u8>,
    /// The record type.
    record: PhantomData<fn() -> R>,
}

impl<R: DebugRecord> RecordStream<R> {
    /// Decode the record stream `stream`.
    ///
    /// An empty stream has no records.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::UnexpectedEof`] if the length is not a multiple of
    /// the record size. The error identifies `stream` and the offset of the
    /// truncated record.
    pub fn parse(bytes: Vec<u8>, stream: StreamId) -> Result<Self> {
        check_length(&bytes, R::SIZE, stream, R::TRUNCATED)?;
        Ok(Self {
            bytes,
            record: PhantomData,
        })
    }

    /// Encode `records` into a new stream.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot be allocated.
    pub fn from_records(records: &[R]) -> Result<Self> {
        let mut bytes = Vec::new();
        Output::new(&mut bytes).reserve(records.len().saturating_mul(R::SIZE))?;
        for record in records {
            record.encode(&mut bytes)?;
        }
        Ok(Self {
            bytes,
            record: PhantomData,
        })
    }

    /// Return the number of records.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len() / R::SIZE
    }

    /// Return `true` if the stream has no records.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Return the record at `index`, if the stream has it.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<R> {
        let start = index.checked_mul(R::SIZE)?;
        let end = start.checked_add(R::SIZE)?;
        self.bytes
            .get(start..end)
            .map(|record| R::decode(record.as_chunks::<4>().0))
    }

    /// Return an iterator over the records in stream order.
    #[must_use]
    pub fn iter(&self) -> RecordIter<'_, R> {
        RecordIter::new(&self.bytes)
    }

    /// Return the serialized stream bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Append the stream to `out`.
    ///
    /// The encoder encodes each decoded record again. The output is equal to
    /// the decoded bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        Output::new(out).reserve(self.bytes.len())?;
        for record in self {
            record.encode(out)?;
        }
        Ok(())
    }

    /// Return the encoded stream as a new vector.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`RecordStream::encode`].
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.encode(&mut out)?;
        Ok(out)
    }
}

impl<'a, R: DebugRecord> IntoIterator for &'a RecordStream<R> {
    type Item = R;
    type IntoIter = RecordIter<'a, R>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over the records of a record array.
#[derive(Clone, Debug)]
pub struct RecordIter<'a, R> {
    /// The remaining records of `R::SIZE` bytes each.
    records: core::slice::ChunksExact<'a, u8>,
    /// The record type.
    record: PhantomData<fn() -> R>,
}

impl<'a, R: DebugRecord> RecordIter<'a, R> {
    /// Iterate over the complete records of `bytes`.
    ///
    /// Callers check the length first. The iterator ignores a partial record at the end.
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self {
            records: bytes.chunks_exact(R::SIZE),
            record: PhantomData,
        }
    }
}

impl<R: DebugRecord> Iterator for RecordIter<'_, R> {
    type Item = R;

    fn next(&mut self) -> Option<Self::Item> {
        self.records
            .next()
            .map(|record| R::decode(record.as_chunks::<4>().0))
    }

    fn nth(&mut self, count: usize) -> Option<Self::Item> {
        self.records
            .nth(count)
            .map(|record| R::decode(record.as_chunks::<4>().0))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.records.size_hint()
    }
}

impl<R: DebugRecord> DoubleEndedIterator for RecordIter<'_, R> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.records
            .next_back()
            .map(|record| R::decode(record.as_chunks::<4>().0))
    }
}

impl<R: DebugRecord> ExactSizeIterator for RecordIter<'_, R> {}

impl<R: DebugRecord> FusedIterator for RecordIter<'_, R> {}

/// Check that `bytes` is an array of `size`-byte records.
///
/// # Errors
///
/// Returns [`ErrorKind::UnexpectedEof`] with `context`, `stream` and the
/// offset of the truncated record.
pub(crate) fn check_length(
    bytes: &[u8],
    size: usize,
    stream: StreamId,
    context: &'static str,
) -> Result<()> {
    let tail = bytes.len() % size;
    if tail == 0 {
        return Ok(());
    }
    let offset = u64::try_from(bytes.len() - tail).unwrap_or(u64::MAX);
    Err(Error::new(ErrorKind::UnexpectedEof, None)
        .with_context(context)
        .with_stream(stream.0, offset))
}
