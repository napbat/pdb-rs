//! Reference validation of TPI records: invalid references, roles, forward
//! references, cycles and the depth and work limits.

use pdbrs::ErrorKind;
use pdbrs::native::types::{
    DEFAULT_HASH_BUCKETS, HashLayout, ReferenceLimits, TypeStream, TypeStreamOptions, TypeTable,
    validate_type_references, write_type_stream,
};

/// Frame a payload as a padded record of `kind`.
fn record(kind: u16, payload: &[u8]) -> Vec<u8> {
    let mut body = kind.to_le_bytes().to_vec();
    body.extend_from_slice(payload);
    let count = (4 - (body.len() + 2) % 4) % 4;
    for left in (1..=count).rev() {
        body.push(0xf0 + u8::try_from(left).unwrap());
    }
    let mut full = u16::try_from(body.len()).unwrap().to_le_bytes().to_vec();
    full.extend_from_slice(&body);
    full
}

/// An `LF_MODIFIER` record of `index`.
fn modifier(index: u32) -> Vec<u8> {
    record(0x1001, &[&index.to_le_bytes()[..], &[1, 0]].concat())
}

/// An `LF_PROCEDURE` record with an argument list.
fn procedure(argument_list: u32) -> Vec<u8> {
    record(
        0x1008,
        &[
            &3_u32.to_le_bytes()[..],
            &[0, 0, 0, 0],
            &argument_list.to_le_bytes(),
        ]
        .concat(),
    )
}

/// An `LF_ARGLIST` record with no arguments.
fn argument_list() -> Vec<u8> {
    record(0x1201, &0_u32.to_le_bytes())
}

/// A stream with `records`.
fn stream(records: &[Vec<u8>]) -> TypeStream {
    let options = TypeStreamOptions {
        index_begin: 0x1000,
        hash_bucket_count: DEFAULT_HASH_BUCKETS,
        hash_stream: None,
        hash_aux_stream: None,
        layout: HashLayout::Microsoft,
        adjusters: &[],
    };
    TypeStream::parse(
        write_type_stream(&records.concat(), &options)
            .unwrap()
            .stream,
    )
    .unwrap()
}

fn validate(
    records: &[Vec<u8>],
    limits: ReferenceLimits,
) -> pdbrs::Result<pdbrs::native::types::ReferenceSummary> {
    let stream = stream(records);
    let table = TypeTable::new(&stream).unwrap();
    validate_type_references(&table, limits)
}

#[test]
fn counts_valid_references_and_the_depth() {
    let summary = validate(
        &[
            argument_list(),
            procedure(0x1000),
            modifier(0x1001),
            modifier(0x1002),
        ],
        ReferenceLimits::default(),
    )
    .unwrap();
    assert_eq!(summary.references, 4);
    assert_eq!(summary.forward_references, 0);
    assert_eq!(summary.primitive_references, 1);
    assert_eq!(summary.max_depth, 4);
}

#[test]
fn rejects_invalid_references() {
    for records in [
        // An unknown primitive kind.
        vec![modifier(0x0009)],
        // A record index after the last record.
        vec![modifier(0x1005)],
        // A procedure whose argument list is not an `LF_ARGLIST` record.
        vec![modifier(0x74), procedure(0x1000)],
        // A procedure whose argument list is a primitive type.
        vec![procedure(0x74)],
        // A procedure without an argument list.
        vec![procedure(0)],
    ] {
        let error = validate(&records, ReferenceLimits::default()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidReference, "{records:?}");
        assert!(error.stream_location().is_some());
    }
}

#[test]
fn accepts_forward_references_without_a_cycle() {
    let summary = validate(
        &[modifier(0x1001), modifier(0x74)],
        ReferenceLimits::default(),
    )
    .unwrap();
    assert_eq!(summary.forward_references, 1);
    assert_eq!(summary.max_depth, 2);
}

#[test]
fn detects_cycles() {
    for records in [
        vec![modifier(0x1000)],
        vec![modifier(0x1001), modifier(0x1000)],
        vec![
            modifier(0x74),
            modifier(0x1002),
            modifier(0x1003),
            modifier(0x1001),
        ],
    ] {
        let error = validate(&records, ReferenceLimits::default()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat, "{records:?}");
    }
}

#[test]
fn enforces_the_depth_limit() {
    let chain: Vec<Vec<u8>> = (0..10)
        .map(|position| {
            if position == 0 {
                modifier(0x74)
            } else {
                modifier(0x1000 + position - 1)
            }
        })
        .collect();
    let limits = ReferenceLimits {
        max_depth: 10,
        max_work: 1000,
    };
    assert_eq!(validate(&chain, limits).unwrap().max_depth, 10);
    let limits = ReferenceLimits {
        max_depth: 9,
        max_work: 1000,
    };
    assert_eq!(
        validate(&chain, limits).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    // The same limit applies to a chain of forward references.
    let forward: Vec<Vec<u8>> = (0..10)
        .map(|position| {
            if position == 9 {
                modifier(0x74)
            } else {
                modifier(0x1000 + position + 1)
            }
        })
        .collect();
    assert_eq!(
        validate(&forward, limits).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
    let limits = ReferenceLimits {
        max_depth: 10,
        max_work: 1000,
    };
    assert_eq!(validate(&forward, limits).unwrap().max_depth, 10);
}

#[test]
fn enforces_the_work_limit() {
    let records = vec![modifier(0x74), modifier(0x1000), modifier(0x1001)];
    // Three references to check and two record references to visit.
    let limits = ReferenceLimits {
        max_depth: 100,
        max_work: 5,
    };
    assert!(validate(&records, limits).is_ok());
    let limits = ReferenceLimits {
        max_depth: 100,
        max_work: 4,
    };
    assert_eq!(
        validate(&records, limits).unwrap_err().kind(),
        ErrorKind::LimitExceeded
    );
}
