//! Tests for TPI and IPI stream parsing.

use pdbrs::ErrorKind;
use pdbrs::native::msf::StreamId;
use pdbrs::native::types::{FIRST_TYPE_INDEX, HEADER_SIZE, HashBuffer, TPI_V80, TypeStream};

/// The serialized header values that a test can change.
#[derive(Clone, Copy)]
struct TypeFixture {
    version: u32,
    header_size: u32,
    index_begin: u32,
    index_end: u32,
    record_bytes: u32,
    hash_stream: u16,
    hash_key_size: u32,
    hash_bucket_count: u32,
    hash_values: (i32, u32),
    index_offsets: (i32, u32),
    hash_adjustments: (i32, u32),
}

impl TypeFixture {
    /// A header for `count` records in `record_bytes` bytes, with no hash stream.
    fn new(count: u32, record_bytes: usize) -> Self {
        Self {
            version: TPI_V80,
            header_size: 56,
            index_begin: FIRST_TYPE_INDEX,
            index_end: FIRST_TYPE_INDEX + count,
            record_bytes: u32::try_from(record_bytes).unwrap(),
            hash_stream: 0xffff,
            hash_key_size: 4,
            hash_bucket_count: 0x3_ffff,
            // Microsoft writes these values when no hash stream exists.
            hash_values: (0, u32::MAX),
            index_offsets: (0, u32::MAX),
            hash_adjustments: (0, u32::MAX),
        }
    }

    fn serialize(self, records: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in [
            self.version,
            self.header_size,
            self.index_begin,
            self.index_end,
            self.record_bytes,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&self.hash_stream.to_le_bytes());
        bytes.extend_from_slice(&0xffff_u16.to_le_bytes());
        bytes.extend_from_slice(&self.hash_key_size.to_le_bytes());
        bytes.extend_from_slice(&self.hash_bucket_count.to_le_bytes());
        for (offset, length) in [self.hash_values, self.index_offsets, self.hash_adjustments] {
            bytes.extend_from_slice(&offset.to_le_bytes());
            bytes.extend_from_slice(&length.to_le_bytes());
        }
        assert_eq!(bytes.len(), HEADER_SIZE);
        bytes.extend_from_slice(records);
        bytes
    }
}

/// Two type records: an `LF_ARGLIST` with padding and an unknown kind.
fn two_records() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x06, 0x00, 0x01, 0x12, 0x00, 0x00, 0x00, 0x00]);
    // A record with a length that is not a multiple of 4 is valid.
    bytes.extend_from_slice(&[0x04, 0x00, 0xff, 0x7f, 0xaa, 0xbb]);
    bytes
}

fn parse_error(header: TypeFixture, records: &[u8]) -> ErrorKind {
    TypeStream::parse(header.serialize(records))
        .unwrap_err()
        .kind()
}

fn iteration_error(header: TypeFixture, records: &[u8]) -> ErrorKind {
    let stream = TypeStream::parse(header.serialize(records)).unwrap();
    let results: Vec<_> = stream.records().collect();
    let (last, valid) = results.split_last().unwrap();
    assert!(valid.iter().all(Result::is_ok));
    last.as_ref().unwrap_err().kind()
}

#[test]
fn iterates_records_with_their_indices() {
    let records = two_records();
    let stream = TypeStream::parse(TypeFixture::new(2, records.len()).serialize(&records)).unwrap();

    let header = stream.header();
    assert_eq!(header.record_count(), 2);
    assert_eq!(header.hash_stream, None);
    assert_eq!(header.hash_aux_stream, None);
    assert_eq!(
        header.hash_values,
        HashBuffer {
            offset: 0,
            length: u32::MAX
        }
    );
    assert_eq!(stream.record_bytes(), records.as_slice());

    let parsed: Vec<_> = stream.records().map(Result::unwrap).collect();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].index, 0x1000);
    assert_eq!(
        (parsed[0].record.offset, parsed[0].record.kind),
        (0, 0x1201)
    );
    assert_eq!(parsed[1].index, 0x1001);
    assert_eq!(
        (parsed[1].record.offset, parsed[1].record.kind),
        (8, 0x7fff)
    );
    assert_eq!(parsed[1].record.data, &[0xaa, 0xbb]);
}

#[test]
fn accepts_empty_stream() {
    let stream = TypeStream::parse(TypeFixture::new(0, 0).serialize(&[])).unwrap();
    assert!(stream.records().next().is_none());
}

#[test]
fn rejects_unsupported_version() {
    let mut header = TypeFixture::new(0, 0);
    header.version = 19_990_903;
    assert_eq!(parse_error(header, &[]), ErrorKind::UnsupportedVersion);
}

#[test]
fn rejects_wrong_header_size() {
    let mut header = TypeFixture::new(0, 0);
    header.header_size = 60;
    assert_eq!(parse_error(header, &[0; 4]), ErrorKind::InvalidFormat);
}

#[test]
fn rejects_truncated_header() {
    let bytes = TypeFixture::new(0, 0).serialize(&[]);
    for length in [0, 3, 8, HEADER_SIZE - 1] {
        let error = TypeStream::parse(bytes[..length].to_vec()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    }
}

#[test]
fn rejects_invalid_index_ranges() {
    let records = two_records();
    let mut primitive = TypeFixture::new(2, records.len());
    primitive.index_begin = FIRST_TYPE_INDEX - 1;
    primitive.index_end = FIRST_TYPE_INDEX + 1;
    assert_eq!(parse_error(primitive, &records), ErrorKind::InvalidFormat);

    let mut reversed = TypeFixture::new(0, records.len());
    reversed.index_end = FIRST_TYPE_INDEX - 1;
    assert_eq!(parse_error(reversed, &records), ErrorKind::InvalidFormat);

    // Four records need at least 16 bytes.
    let too_many = TypeFixture::new(4, records.len());
    assert_eq!(parse_error(too_many, &records), ErrorKind::InvalidFormat);
}

#[test]
fn rejects_record_bytes_past_the_end() {
    let records = two_records();
    let header = TypeFixture::new(2, records.len() + 1);
    assert_eq!(parse_error(header, &records), ErrorKind::UnexpectedEof);
}

#[test]
fn rejects_bytes_after_the_records() {
    let records = two_records();
    let header = TypeFixture::new(2, records.len() - 4);
    assert_eq!(parse_error(header, &records), ErrorKind::InvalidFormat);
}

#[test]
fn reports_framing_errors_in_record_bytes() {
    let records = two_records();
    let header = TypeFixture::new(3, records.len() + 2);
    let mut padded = records.clone();
    padded.extend_from_slice(&[0x02, 0x00]);
    // The last two bytes are a truncated record prefix.
    assert_eq!(iteration_error(header, &padded), ErrorKind::UnexpectedEof);

    let mut header = TypeFixture::new(2, records.len());
    header.index_end = FIRST_TYPE_INDEX + 3;
    let mut longer = records;
    longer.extend_from_slice(&[0; 4]);
    header.record_bytes = u32::try_from(longer.len()).unwrap();
    // A zero record length is not valid.
    assert_eq!(iteration_error(header, &longer), ErrorKind::InvalidFormat);
}

#[test]
fn reports_missing_records_at_the_end() {
    let records = two_records();
    let header = TypeFixture::new(3, records.len() + 4);
    let mut longer = records.clone();
    longer.truncate(8);
    longer.extend_from_slice(&[0x02, 0x00, 0x01, 0x00]);
    longer.extend_from_slice(&records[8..]);
    let stream = TypeStream::parse(header.serialize(&longer)).unwrap();
    assert_eq!(stream.records().map(Result::unwrap).count(), 3);

    let header = TypeFixture::new(3, records.len());
    let stream = TypeStream::parse(header.serialize(&records)).unwrap();
    let mut iterator = stream.records();
    assert!(iterator.next().unwrap().is_ok());
    assert!(iterator.next().unwrap().is_ok());
    let error = iterator.next().unwrap().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(iterator.position(), HEADER_SIZE + records.len());
    assert!(iterator.next().is_none());
}

#[test]
fn reports_more_records_than_declared() {
    let records = two_records();
    let header = TypeFixture::new(1, records.len());
    assert_eq!(iteration_error(header, &records), ErrorKind::InvalidFormat);
}

#[test]
fn rejects_invalid_hash_parameters() {
    let mut key = TypeFixture::new(0, 0);
    key.hash_key_size = 2;
    assert_eq!(parse_error(key, &[]), ErrorKind::InvalidFormat);

    for count in [0, 0xfff, 0x4_0001] {
        let mut buckets = TypeFixture::new(0, 0);
        buckets.hash_bucket_count = count;
        assert_eq!(parse_error(buckets, &[]), ErrorKind::InvalidFormat);
    }
}

#[test]
fn validates_hash_buffers_only_with_a_hash_stream() {
    let records = two_records();
    let mut header = TypeFixture::new(2, records.len());
    header.hash_stream = 7;
    header.hash_values = (0, 8);
    header.index_offsets = (8, 8);
    // A negative offset is valid for an empty buffer.
    header.hash_adjustments = (-1, 0);
    let stream = TypeStream::parse(header.serialize(&records)).unwrap();
    assert_eq!(stream.header().hash_stream, Some(StreamId(7)));

    let mut negative = header;
    negative.hash_adjustments = (-1, 4);
    assert_eq!(parse_error(negative, &records), ErrorKind::InvalidReference);

    let mut overflow = header;
    overflow.index_offsets = (8, u32::MAX);
    assert_eq!(parse_error(overflow, &records), ErrorKind::InvalidReference);

    let mut count = header;
    count.hash_values = (0, 4);
    assert_eq!(parse_error(count, &records), ErrorKind::InvalidFormat);

    let mut partial = header;
    partial.index_offsets = (8, 12);
    assert_eq!(parse_error(partial, &records), ErrorKind::InvalidFormat);
}
