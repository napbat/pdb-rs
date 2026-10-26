//! Tests for TPI and IPI hash streams and record hashes.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::type_record::{ClassOptions, LF_POINTER, LF_STRUCTURE};
use pdbrs::native::hash::{hash_buffer_v8, hash_string_v1};
use pdbrs::native::msf::StreamId;
use pdbrs::native::types::{
    HEADER_SIZE, HashAdjuster, IndexOffset, TPI_V80, TypeHashes, TypeStream, type_record_hash,
};

const BUCKETS: u32 = 0x1000;
const HASH_STREAM: u16 = 7;

/// Frame `payload` as a record of `kind` and pad it to 4 bytes.
fn record(kind: u16, payload: &[u8]) -> Vec<u8> {
    let mut body = kind.to_le_bytes().to_vec();
    body.extend_from_slice(payload);
    let count = (4 - (body.len() + 2) % 4) % 4;
    for left in (1..=count).rev() {
        body.push(0xf0 + u8::try_from(left).unwrap());
    }
    let mut bytes = u16::try_from(body.len()).unwrap().to_le_bytes().to_vec();
    bytes.extend_from_slice(&body);
    bytes
}

/// An `LF_STRUCTURE` record with no members.
fn structure(options: ClassOptions, name: &[u8], unique_name: Option<&[u8]>) -> Vec<u8> {
    let options = options.0 | unique_name.map_or(0, |_| ClassOptions::HAS_UNIQUE_NAME.0);
    let mut payload = Vec::new();
    payload.extend_from_slice(&0_u16.to_le_bytes());
    payload.extend_from_slice(&options.to_le_bytes());
    payload.extend_from_slice(&[0; 12]);
    payload.extend_from_slice(&0_u16.to_le_bytes());
    payload.extend_from_slice(name);
    payload.push(0);
    if let Some(unique_name) = unique_name {
        payload.extend_from_slice(unique_name);
        payload.push(0);
    }
    record(LF_STRUCTURE.0, &payload)
}

fn pointer() -> Vec<u8> {
    let mut payload = 0x0074_u32.to_le_bytes().to_vec();
    payload.extend_from_slice(&(0x0c_u32 | (8 << 13)).to_le_bytes());
    record(LF_POINTER.0, &payload)
}

/// Serialize a type stream with a hash stream reference and three buffers.
fn type_stream(records: &[Vec<u8>], buffers: [(i32, u32); 3]) -> TypeStream {
    let body = records.concat();
    let count = u32::try_from(records.len()).unwrap();
    let mut bytes = Vec::new();
    for value in [
        TPI_V80,
        56,
        0x1000,
        0x1000 + count,
        u32::try_from(body.len()).unwrap(),
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&HASH_STREAM.to_le_bytes());
    bytes.extend_from_slice(&0xffff_u16.to_le_bytes());
    bytes.extend_from_slice(&4_u32.to_le_bytes());
    bytes.extend_from_slice(&BUCKETS.to_le_bytes());
    for (offset, length) in buffers {
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(&length.to_le_bytes());
    }
    assert_eq!(bytes.len(), HEADER_SIZE);
    bytes.extend_from_slice(&body);
    TypeStream::parse(bytes).unwrap()
}

/// A serialized hash adjuster table with one entry in bucket 0.
fn adjusters(name_offset: u32, index: u32) -> Vec<u8> {
    words(&[1, 1, 1, 1, 0, name_offset, index])
}

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// A valid fixture: three records, their hashes, two pairs and one adjuster.
struct Fixture {
    records: Vec<Vec<u8>>,
    values: Vec<u32>,
    pairs: Vec<(u32, u32)>,
    adjusters: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let records = vec![
            structure(ClassOptions(0), b"S", None),
            pointer(),
            structure(ClassOptions::FORWARD_REFERENCE, b"S", None),
        ];
        let values = records
            .iter()
            .map(|record| type_record_hash(record).unwrap() % BUCKETS)
            .collect();
        let second = u32::try_from(records[0].len() + records[1].len()).unwrap();
        Self {
            records,
            values,
            pairs: vec![(0x1000, 0), (0x1002, second)],
            adjusters: adjusters(5, 0x1001),
        }
    }

    fn buffers(&self) -> ([(i32, u32); 3], Vec<u8>) {
        let values = words(&self.values);
        let pairs: Vec<u8> = self
            .pairs
            .iter()
            .flat_map(|&(index, offset)| words(&[index, offset]))
            .collect();
        let length = |bytes: &[u8]| u32::try_from(bytes.len()).unwrap();
        let buffers = [
            (0, length(&values)),
            (i32::try_from(values.len()).unwrap(), length(&pairs)),
            (
                i32::try_from(values.len() + pairs.len()).unwrap(),
                length(&self.adjusters),
            ),
        ];
        (buffers, [values, pairs, self.adjusters.clone()].concat())
    }

    fn stream(&self) -> TypeStream {
        type_stream(&self.records, self.buffers().0)
    }

    fn hash_bytes(&self) -> Vec<u8> {
        self.buffers().1
    }
}

fn parse_error(stream: &TypeStream, bytes: &[u8]) -> ErrorKind {
    TypeHashes::parse(stream.header(), bytes)
        .unwrap_err()
        .kind()
}

#[test]
fn hashes_udts_by_name_or_unique_name() {
    let plain = structure(ClassOptions(0), b"S", None);
    assert_eq!(type_record_hash(&plain).unwrap(), hash_string_v1(b"S"));
    // A unique name does not change the hash of a type that is not scoped.
    let unique = structure(ClassOptions(0), b"S", Some(b".?AUS@@"));
    assert_eq!(type_record_hash(&unique).unwrap(), hash_string_v1(b"S"));
    let scoped = structure(ClassOptions::SCOPED, b"S", Some(b".?AUS@?1??f@@"));
    assert_eq!(
        type_record_hash(&scoped).unwrap(),
        hash_string_v1(b".?AUS@?1??f@@")
    );
}

#[test]
fn hashes_forward_references_and_anonymous_udts_by_bytes() {
    for record in [
        structure(ClassOptions::FORWARD_REFERENCE, b"S", None),
        structure(ClassOptions(0), b"<unnamed-tag>", Some(b"u")),
        structure(ClassOptions(0), b"a::__unnamed", Some(b"u")),
        structure(ClassOptions::SCOPED, b"S", None),
    ] {
        assert_eq!(type_record_hash(&record).unwrap(), hash_buffer_v8(&record));
    }
    // An anonymous name without a unique name hashes the name.
    let record = structure(ClassOptions(0), b"<unnamed-tag>", None);
    assert_eq!(
        type_record_hash(&record).unwrap(),
        hash_string_v1(b"<unnamed-tag>")
    );
    // The suffix must follow a scope separator.
    let record = structure(ClassOptions(0), b"a__unnamed", Some(b"u"));
    assert_eq!(
        type_record_hash(&record).unwrap(),
        hash_string_v1(b"a__unnamed")
    );
}

#[test]
fn hashes_source_line_records_by_udt_index() {
    for (kind, payload) in [
        (0x1606_u16, &[0x34, 0x12, 0, 0, 1, 0, 0, 0, 9, 0, 0, 0][..]),
        (0x1607, &[0x34, 0x12, 0, 0, 1, 0, 0, 0, 9, 0, 0, 0, 2, 0]),
    ] {
        let line = record(kind, payload);
        assert_eq!(
            type_record_hash(&line).unwrap(),
            hash_string_v1(&[0x34, 0x12, 0, 0])
        );
        // The payload is too short even with its padding bytes.
        let short = record(kind, &payload[..payload.len() - 4]);
        assert_eq!(
            type_record_hash(&short).unwrap_err().kind(),
            ErrorKind::InvalidFormat
        );
    }
}

#[test]
fn hashes_other_records_by_bytes() {
    let record = pointer();
    assert_eq!(type_record_hash(&record).unwrap(), hash_buffer_v8(&record));
}

#[test]
fn rejects_hash_input_that_is_not_one_record() {
    let mut bytes = pointer();
    bytes.push(0);
    assert_eq!(
        type_record_hash(&bytes).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        type_record_hash(&[]).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
    assert_eq!(
        type_record_hash(&pointer()[..6]).unwrap_err().kind(),
        ErrorKind::UnexpectedEof
    );
    // A wide class without its name is malformed.
    let class2 = record(0x1608, &[0; 20]);
    assert_eq!(
        type_record_hash(&class2).unwrap_err().kind(),
        ErrorKind::InvalidFormat
    );
}

#[test]
fn hashes_wide_udt_records_by_name() {
    // Options, field list, derivation list, shape, member count 1, size 8, "W".
    let mut payload = vec![0; 16];
    payload.extend_from_slice(&[1, 0, 8, 0, b'W', 0]);
    let class2 = record(0x1608, &payload);
    assert_eq!(type_record_hash(&class2).unwrap(), hash_string_v1(b"W"));
    let mut union2 = vec![0; 8];
    union2.extend_from_slice(&[1, 0, 8, 0, b'U', 0]);
    assert_eq!(
        type_record_hash(&record(0x160a, &union2)).unwrap(),
        hash_string_v1(b"U")
    );
}

#[test]
fn decodes_and_verifies_hash_buffers() {
    let fixture = Fixture::new();
    let stream = fixture.stream();
    let bytes = fixture.hash_bytes();
    let hashes = TypeHashes::parse(stream.header(), &bytes).unwrap();
    assert_eq!(hashes.bucket_count(), BUCKETS);
    assert_eq!(hashes.hash_values().collect::<Vec<_>>(), fixture.values);
    assert_eq!(hashes.hash_value(0x1001), Some(fixture.values[1]));
    assert_eq!(hashes.hash_value(0x1003), None);
    assert_eq!(
        hashes.index_offsets().collect::<Vec<_>>(),
        [
            IndexOffset {
                index: 0x1000,
                offset: 0
            },
            IndexOffset {
                index: 0x1002,
                offset: fixture.pairs[1].1
            },
        ]
    );
    assert_eq!(
        hashes.adjusters(),
        [HashAdjuster {
            name_offset: 5,
            index: 0x1001
        }]
    );
    hashes.verify(&stream).unwrap();
    for record in stream.records() {
        let record = record.unwrap();
        assert_eq!(hashes.locate(&stream, record.index).unwrap(), record);
    }
    assert_eq!(
        hashes.locate(&stream, 0x1003).unwrap_err().kind(),
        ErrorKind::InvalidReference
    );
}

#[test]
fn accepts_empty_buffers() {
    let fixture = Fixture::new();
    let stream = type_stream(&fixture.records, [(0, 0), (0, 0), (0, 0)]);
    let hashes = TypeHashes::parse(stream.header(), &[]).unwrap();
    assert_eq!(hashes.hash_values().len(), 0);
    assert_eq!(hashes.adjusters(), []);
    hashes.verify(&stream).unwrap();
    // Without pairs, `locate` frames from the first record.
    let last = stream.records().last().unwrap().unwrap();
    assert_eq!(hashes.locate(&stream, 0x1002).unwrap(), last);
}

#[test]
fn verify_reports_mismatched_values_and_pairs() {
    let mut fixture = Fixture::new();
    fixture.values[1] = (fixture.values[1] + 1) % BUCKETS;
    let stream = fixture.stream();
    let bytes = fixture.hash_bytes();
    let error = TypeHashes::parse(stream.header(), &bytes)
        .unwrap()
        .verify(&stream)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(
        error.stream_location(),
        Some((StreamId(u32::from(HASH_STREAM)), 4))
    );

    let mut fixture = Fixture::new();
    fixture.pairs[1].1 += 4;
    let stream = fixture.stream();
    let bytes = fixture.hash_bytes();
    let error = TypeHashes::parse(stream.header(), &bytes)
        .unwrap()
        .verify(&stream)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    assert_eq!(
        error.stream_location(),
        Some((StreamId(u32::from(HASH_STREAM)), 12 + 8))
    );

    let mut fixture = Fixture::new();
    fixture.adjusters = adjusters(5, 0x1003);
    let stream = fixture.stream();
    let bytes = fixture.hash_bytes();
    let hashes = TypeHashes::parse(stream.header(), &bytes).unwrap();
    assert_eq!(
        hashes.verify(&stream).unwrap_err().kind(),
        ErrorKind::InvalidReference
    );
}

#[test]
fn verify_rejects_another_type_stream() {
    let fixture = Fixture::new();
    let bytes = fixture.hash_bytes();
    let stream = fixture.stream();
    let hashes = TypeHashes::parse(stream.header(), &bytes).unwrap();
    let other = type_stream(&fixture.records[..2], [(0, 8), (8, 0), (8, 0)]);
    assert_eq!(
        hashes.verify(&other).unwrap_err().kind(),
        ErrorKind::InvalidReference
    );
}

#[test]
fn rejects_buffers_outside_the_stream_and_gaps() {
    let fixture = Fixture::new();
    let stream = fixture.stream();
    let bytes = fixture.hash_bytes();
    assert_eq!(
        parse_error(&stream, &bytes[..bytes.len() - 1]),
        ErrorKind::InvalidReference
    );
    let mut longer = bytes.clone();
    longer.push(0);
    assert_eq!(parse_error(&stream, &longer), ErrorKind::InvalidFormat);

    let (mut buffers, bytes) = fixture.buffers();
    buffers[2].0 += 4;
    buffers[2].1 -= 4;
    let stream = type_stream(&fixture.records, buffers);
    assert_eq!(parse_error(&stream, &bytes), ErrorKind::InvalidFormat);
}

#[test]
fn rejects_hash_stream_without_reference() {
    let fixture = Fixture::new();
    let mut bytes = Vec::new();
    for value in [TPI_V80, 56, 0x1000, 0x1000, 0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&[0xff; 4]);
    bytes.extend_from_slice(&4_u32.to_le_bytes());
    bytes.extend_from_slice(&BUCKETS.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]);
    let stream = TypeStream::parse(bytes).unwrap();
    assert_eq!(
        parse_error(&stream, &fixture.hash_bytes()),
        ErrorKind::MissingStream
    );
}

#[test]
fn rejects_invalid_values_and_pairs() {
    let mut fixture = Fixture::new();
    fixture.values[0] = BUCKETS;
    assert_eq!(
        parse_error(&fixture.stream(), &fixture.hash_bytes()),
        ErrorKind::InvalidFormat
    );

    for (pairs, expected) in [
        (vec![(0x1002, 40), (0x1000, 0)], ErrorKind::InvalidFormat),
        (vec![(0x1000, 0), (0x1000, 4)], ErrorKind::InvalidFormat),
        (vec![(0x1000, 4), (0x1001, 4)], ErrorKind::InvalidFormat),
        (vec![(0x0fff, 0), (0x1001, 4)], ErrorKind::InvalidReference),
        (vec![(0x1000, 0), (0x1003, 4)], ErrorKind::InvalidReference),
        (
            vec![(0x1000, 0), (0x1001, 1000)],
            ErrorKind::InvalidReference,
        ),
    ] {
        let mut fixture = Fixture::new();
        fixture.pairs = pairs;
        assert_eq!(
            parse_error(&fixture.stream(), &fixture.hash_bytes()),
            expected,
            "{:?}",
            fixture.pairs
        );
    }
}

#[test]
fn rejects_malformed_adjuster_tables() {
    for table in [
        // A capacity of zero.
        words(&[0, 0, 0, 0]),
        // The size is above the maximum load of the capacity.
        words(&[3, 2, 1, 3, 0, 1, 1, 2, 2, 3, 3]),
        // The present bits do not match the size.
        words(&[2, 4, 1, 1, 0, 5, 0x1001]),
        // A bucket is present and deleted.
        words(&[1, 4, 1, 1, 1, 1, 5, 0x1001]),
        // A present bucket is past the capacity.
        words(&[1, 4, 1, 0x10, 0, 5, 0x1001]),
        // The entries are truncated.
        words(&[1, 4, 1, 1, 0, 5]),
        // The word count of a bit vector is too large.
        words(&[1, 4, 9, 1]),
        // Bytes follow the entries.
        words(&[1, 4, 1, 1, 0, 5, 0x1001, 0]),
    ] {
        let mut fixture = Fixture::new();
        fixture.adjusters = table;
        let kind = parse_error(&fixture.stream(), &fixture.hash_bytes());
        assert!(
            matches!(kind, ErrorKind::InvalidFormat | ErrorKind::UnexpectedEof),
            "{:?}: {kind:?}",
            fixture.adjusters
        );
    }
}
