//! The TPI and IPI streams of a modern native PDB.
//!
//! Both streams start with the same 56-byte header. A sequence of `CodeView`
//! records follows the header. The first record has the index
//! [`TypeHeader::index_begin`], and each next record has the next index.
//! Indices below [`FIRST_RECORD_INDEX`] identify primitive types. They do not
//! have records.
//!
//! [`TypeStream`] validates the header and frames the records on demand.
//! [`TypeTable`] finds records by index, and [`ForwardRefs`] resolves
//! forward references to their definitions. [`TypeHashes`] decodes the
//! hash stream and checks it against the records. [`TypeOffsets`] holds only
//! the header and the index offset pairs. Use
//! [`crate::native::codeview::type_record::TypeLeaf`] to decode a TPI record.

use alloc::vec::Vec;
use core::iter::FusedIterator;

use crate::native::Producer;
use crate::native::codeview::index::FIRST_RECORD_INDEX;
use crate::native::codeview::{Fields, Record, Records};
use crate::native::msf::StreamId;
use crate::{Error, ErrorKind, Result};

mod forward;
mod hash;
mod lookup;
mod offsets;
mod size;
mod validate;
mod write;

pub use forward::ForwardRefs;
pub use hash::{HashAdjuster, HashValues, IndexOffset, IndexOffsets, TypeHashes, type_record_hash};
pub use lookup::{FieldMembers, TypeTable};
pub use offsets::TypeOffsets;
pub use size::type_size;
pub use validate::{ReferenceLimits, ReferenceSummary, validate_type_references};
pub use write::{
    DEFAULT_HASH_BUCKETS, TypeStreamBytes, TypeStreamOptions, encode_hash_adjusters,
    encode_type_stream,
};

/// The TPI and IPI version that this crate supports (`PdbTpiV80`).
pub const TPI_V80: u32 = 20_040_203;

/// The size of the version 8.0 stream header in bytes.
pub const HEADER_SIZE: usize = 56;

/// The hash key size that version 8.0 requires.
const HASH_KEY_SIZE: u32 = 4;

/// The size of the length field and the kind field of a record.
const RECORD_PREFIX_SIZE: usize = 4;

/// The smallest valid hash bucket count.
const MIN_HASH_BUCKETS: u32 = 0x1000;

/// The largest valid hash bucket count.
const MAX_HASH_BUCKETS: u32 = 0x4_0000;

/// The serialized value of an absent stream index.
const NIL_STREAM: u16 = 0xffff;

/// The size of one index offset pair in the hash stream.
const INDEX_OFFSET_SIZE: u32 = 8;

/// The smallest size of a record: the length field and the kind field.
const MIN_RECORD_SIZE: u32 = 4;

/// A range of bytes in the hash stream.
///
/// The values are the serialized values. Microsoft writes the offset 0 and
/// the length `0xffff_ffff` in a header that has no hash stream.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct HashBuffer {
    /// The signed byte offset in the hash stream.
    pub offset: i32,
    /// The length in bytes.
    pub length: u32,
}

/// The header of a TPI or IPI stream.
///
/// [`TypeStream::parse`] validates these values:
///
/// - `version` is [`TPI_V80`] and `header_size` is [`HEADER_SIZE`].
/// - `index_begin` is at least [`FIRST_RECORD_INDEX`] and at most `index_end`.
/// - The stream contains exactly `record_bytes` bytes after the header.
/// - The hash key size and the hash bucket count are valid.
/// - If `hash_stream` is present, each non-empty hash buffer has a
///   non-negative offset and an end that fits in 32 bits.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct TypeHeader {
    /// The stream version.
    pub version: u32,
    /// The size of the header in bytes.
    pub header_size: u32,
    /// The index of the first record.
    pub index_begin: u32,
    /// The index after the last record.
    pub index_end: u32,
    /// The number of record bytes after the header.
    pub record_bytes: u32,
    /// The stream that contains the hash data, if present.
    pub hash_stream: Option<StreamId>,
    /// The stream that contains auxiliary hash data, if present.
    pub hash_aux_stream: Option<StreamId>,
    /// The size of one hash key in bytes.
    pub hash_key_size: u32,
    /// The number of hash buckets.
    pub hash_bucket_count: u32,
    /// The hash value of each record.
    pub hash_values: HashBuffer,
    /// The index offset pairs that help to find records.
    pub index_offsets: HashBuffer,
    /// The hash adjustment table.
    pub hash_adjustments: HashBuffer,
}

impl TypeHeader {
    /// Return the number of records that the header declares.
    #[must_use]
    pub const fn record_count(&self) -> u32 {
        // `parse` makes sure that `index_begin <= index_end`.
        self.index_end - self.index_begin
    }

    /// Return the producer whose hash stream layout the header shows.
    ///
    /// A header with a hash stream and index offsets that start where the
    /// empty hash adjuster buffer starts shows [`Producer::Llvm`]. A header
    /// without a hash stream and with a zero-length hash value buffer shows
    /// [`Producer::Llvm`] too. All other headers show
    /// [`Producer::Microsoft`]. The two layouts give the same bytes when they
    /// cannot be told apart. [`TypeStreamOptions::with_producer`] describes
    /// the two layouts.
    #[must_use]
    pub const fn producer(&self) -> Producer {
        let adjusters = self.hash_adjustments;
        let offsets = self.index_offsets;
        let llvm = match self.hash_stream {
            Some(_) => {
                adjusters.length == 0 && offsets.length != 0 && adjusters.offset == offsets.offset
            }
            None => self.hash_values.length == 0,
        };
        if llvm {
            Producer::Llvm
        } else {
            Producer::Microsoft
        }
    }
}

/// A parsed TPI or IPI stream that owns its bytes.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TypeStream {
    /// The full stream, including the header.
    bytes: Vec<u8>,
    /// The validated header.
    header: TypeHeader,
}

impl TypeStream {
    /// Validate the header of a full TPI or IPI stream and keep the bytes.
    ///
    /// The caller loads the stream and applies a size limit. This function
    /// does not frame the records. [`TypeStream::records`] reports framing
    /// errors and record count errors.
    ///
    /// # Errors
    ///
    /// The errors do not have a file offset. All header fields start at
    /// fixed stream offsets.
    ///
    /// - [`ErrorKind::UnexpectedEof`] if the stream is shorter than the
    ///   header or the declared record bytes.
    /// - [`ErrorKind::UnsupportedVersion`] if the version is not [`TPI_V80`].
    /// - [`ErrorKind::InvalidFormat`] if a header value is not valid, if the
    ///   declared count needs more bytes than the record bytes, or if bytes
    ///   follow the record bytes.
    /// - [`ErrorKind::InvalidReference`] if a hash stream is present and a
    ///   non-empty hash buffer has a negative offset or an end past 32 bits.
    pub fn parse(bytes: Vec<u8>) -> Result<Self> {
        let header = parse_header(&bytes)?;
        Ok(Self { bytes, header })
    }

    /// Make a type stream from record bytes that have no stream header.
    ///
    /// The first record gets the index [`FIRST_RECORD_INDEX`]. The stream
    /// gets a version 8.0 header without a hash stream, as
    /// [`encode_type_stream`] writes it with [`Producer::Llvm`].
    /// [`TypeStream::record_bytes`] gives `records` again.
    ///
    /// Use this function for the streams that an `S_MOD_TYPEREF` record
    /// names in a PDB without type merging
    /// ([`ModuleTypeSource`](crate::native::codeview::symbol::ModuleTypeSource)).
    ///
    /// # Errors
    ///
    /// Returns the errors of [`encode_type_stream`]: for example
    /// [`ErrorKind::InvalidFormat`] if `records` is not a sequence of
    /// complete records.
    pub fn from_records(records: &[u8]) -> Result<Self> {
        let options = TypeStreamOptions::default()
            .with_index_begin(FIRST_RECORD_INDEX)
            .with_hash_bucket_count(DEFAULT_HASH_BUCKETS)
            .with_producer(Producer::Llvm);
        Self::parse(encode_type_stream(records, &options)?.stream)
    }

    /// Return the validated header.
    #[must_use]
    pub const fn header(&self) -> &TypeHeader {
        &self.header
    }

    /// Return the record bytes after the header.
    #[must_use]
    pub fn record_bytes(&self) -> &[u8] {
        // `parse` makes sure that the stream ends at the end of the records.
        &self.bytes[HEADER_SIZE..]
    }

    /// Return the full stream: the header and the record bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return the full stream bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Return an iterator over the records and their indices.
    #[must_use]
    pub fn records(&self) -> TypeRecords<'_> {
        TypeRecords {
            records: Records::new(self.record_bytes()),
            next_index: self.header.index_begin,
            index_end: self.header.index_end,
            done: false,
        }
    }

    /// Return the complete bytes of a record of this stream.
    ///
    /// The bytes are the length field, the kind field and the payload with
    /// its padding. Type hashes cover these bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] if `record` does not borrow
    /// its payload from this stream at [`Record::offset`].
    pub fn full_record(&self, record: &Record<'_>) -> Result<&[u8]> {
        let bytes = self.record_bytes();
        let full = usize::try_from(record.offset).ok().and_then(|start| {
            let end = start
                .checked_add(RECORD_PREFIX_SIZE)?
                .checked_add(record.data.len())?;
            bytes.get(start..end)
        });
        match full {
            Some(full)
                if core::ptr::eq(full[RECORD_PREFIX_SIZE..].as_ptr(), record.data.as_ptr()) =>
            {
                Ok(full)
            }
            _ => Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("record is not a record of this type stream")),
        }
    }

    /// Frame the record at `offset` in the record bytes and give it `index`.
    ///
    /// The caller makes sure that `index` is the index of that record.
    pub(crate) fn record_at(&self, offset: u32, index: u32) -> Result<TypeRecord<'_>> {
        let rest = usize::try_from(offset)
            .ok()
            .and_then(|start| self.record_bytes().get(start..))
            .filter(|rest| !rest.is_empty())
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidReference, None)
                    .with_context("type record offset is past the record bytes")
            })?;
        let record = Records::new(rest).next().ok_or_else(|| {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("type record offset is past the record bytes")
        })??;
        Ok(TypeRecord {
            index,
            record: Record { offset, ..record },
        })
    }
}

/// A type record or an ID record with its index.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct TypeRecord<'a> {
    /// The index of the record.
    pub index: u32,
    /// The framed record.
    ///
    /// [`Record::offset`] is relative to the first byte after the header.
    pub record: Record<'a>,
}

/// An iterator over the records of a [`TypeStream`].
///
/// The iterator returns an error if the number of records is not the count
/// in the header. It returns one error and then stops.
#[derive(Clone, Debug)]
pub struct TypeRecords<'a> {
    /// The record framing.
    records: Records<'a>,
    /// The index of the next record.
    next_index: u32,
    /// The index after the last declared record.
    index_end: u32,
    /// `true` after the end or after an error.
    done: bool,
}

impl TypeRecords<'_> {
    /// Return the stream offset of the next record.
    ///
    /// The offset includes the header. After an error, the offset identifies
    /// the record that caused the error, or the end of the records.
    #[must_use]
    pub const fn position(&self) -> usize {
        HEADER_SIZE + self.records.position()
    }
}

impl<'a> Iterator for TypeRecords<'a> {
    type Item = Result<TypeRecord<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let Some(result) = self.records.next() else {
            self.done = true;
            if self.next_index == self.index_end {
                return None;
            }
            return Some(Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context(
                    "type stream has fewer records than its header declares",
                )));
        };
        let record = match result {
            Ok(record) => record,
            Err(error) => {
                self.done = true;
                return Some(Err(error));
            }
        };
        if self.next_index == self.index_end {
            self.done = true;
            return Some(Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context(
                    "type stream has more records than its header declares",
                )));
        }
        let index = self.next_index;
        // `index < index_end`, so the increment cannot overflow.
        self.next_index += 1;
        Some(Ok(TypeRecord { index, record }))
    }
}

impl FusedIterator for TypeRecords<'_> {}

/// Return an invalid-format error with `context`.
fn invalid(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, None).with_context(context)
}

/// Convert a serialized stream index.
fn stream_id(value: u16) -> Option<StreamId> {
    (value != NIL_STREAM).then_some(StreamId(u32::from(value)))
}

/// Read a hash buffer location.
fn hash_buffer(fields: &mut Fields<'_>) -> Result<HashBuffer> {
    let offset = fields.i32()?;
    let length = fields.u32()?;
    Ok(HashBuffer { offset, length })
}

/// Read and validate the header at the start of `bytes`.
fn parse_header(bytes: &[u8]) -> Result<TypeHeader> {
    parse_header_prefix(bytes, bytes.len())
}

/// Read and validate the header at the start of `bytes`, the first bytes of a type stream of
/// `stream_len` bytes.
///
/// `bytes` needs only the header. The checks are the checks of [`TypeStream::parse`].
pub(crate) fn parse_header_prefix(bytes: &[u8], stream_len: usize) -> Result<TypeHeader> {
    let mut fields = Fields::new(
        bytes,
        ErrorKind::UnexpectedEof,
        "type stream is shorter than its header",
    );
    let version = fields.u32()?;
    if version != TPI_V80 {
        return Err(Error::new(ErrorKind::UnsupportedVersion, None)
            .with_context("type stream version is not 8.0"));
    }
    let header_size = fields.u32()?;
    if usize::try_from(header_size).ok() != Some(HEADER_SIZE) {
        return Err(invalid("type stream header size is not 56"));
    }
    let header = TypeHeader {
        version,
        header_size,
        index_begin: fields.u32()?,
        index_end: fields.u32()?,
        record_bytes: fields.u32()?,
        hash_stream: stream_id(fields.u16()?),
        hash_aux_stream: stream_id(fields.u16()?),
        hash_key_size: fields.u32()?,
        hash_bucket_count: fields.u32()?,
        hash_values: hash_buffer(&mut fields)?,
        index_offsets: hash_buffer(&mut fields)?,
        hash_adjustments: hash_buffer(&mut fields)?,
    };
    validate_indices(&header)?;
    validate_length(&header, stream_len.saturating_sub(HEADER_SIZE))?;
    validate_hash(&header)?;
    Ok(header)
}

/// Validate the index range against the record bytes.
fn validate_indices(header: &TypeHeader) -> Result<()> {
    if header.index_begin < FIRST_RECORD_INDEX {
        return Err(invalid("first type index is a primitive type index"));
    }
    if header.index_end < header.index_begin {
        return Err(invalid("type index end is before the type index begin"));
    }
    // Each record needs at least 4 bytes.
    let fits = u64::from(header.record_count()) * u64::from(MIN_RECORD_SIZE)
        <= u64::from(header.record_bytes);
    if !fits {
        return Err(invalid(
            "type record count needs more bytes than the record bytes",
        ));
    }
    Ok(())
}

/// Validate the declared record bytes against the `available` bytes after the header.
fn validate_length(header: &TypeHeader, available: usize) -> Result<()> {
    match usize::try_from(header.record_bytes) {
        Ok(length) if length == available => Ok(()),
        Ok(length) if length < available => {
            Err(invalid("type stream has bytes after its record bytes"))
        }
        _ => Err(Error::new(ErrorKind::UnexpectedEof, None)
            .with_context("type stream is shorter than its record bytes")),
    }
}

/// Validate the hash parameters and, with a hash stream, the hash buffers.
fn validate_hash(header: &TypeHeader) -> Result<()> {
    if header.hash_key_size != HASH_KEY_SIZE {
        return Err(invalid("type stream hash key size is not 4"));
    }
    if !(MIN_HASH_BUCKETS..=MAX_HASH_BUCKETS).contains(&header.hash_bucket_count) {
        return Err(invalid("type stream hash bucket count is out of range"));
    }
    // Without a hash stream, the buffers do not refer to data.
    if header.hash_stream.is_none() {
        return Ok(());
    }
    for buffer in [
        header.hash_values,
        header.index_offsets,
        header.hash_adjustments,
    ] {
        validate_buffer(buffer)?;
    }
    let values = header.hash_values.length;
    if !values.is_multiple_of(HASH_KEY_SIZE)
        || (values != 0 && values / HASH_KEY_SIZE != header.record_count())
    {
        return Err(invalid("type hash value count is not the record count"));
    }
    if !header
        .index_offsets
        .length
        .is_multiple_of(INDEX_OFFSET_SIZE)
    {
        return Err(invalid("type index offset buffer has a partial entry"));
    }
    Ok(())
}

/// Make sure that a non-empty buffer has a valid 32-bit range.
fn validate_buffer(buffer: HashBuffer) -> Result<()> {
    if buffer.length == 0 {
        return Ok(());
    }
    let valid = u32::try_from(buffer.offset)
        .ok()
        .and_then(|offset| offset.checked_add(buffer.length))
        .is_some();
    if valid {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::InvalidReference, None)
            .with_context("type hash buffer range is not valid"))
    }
}
