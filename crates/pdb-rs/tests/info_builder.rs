//! Encoding, named-stream changes, feature codes and the identity policy of the PDB Info stream.

use pdb_rs::ErrorKind;
use pdb_rs::native::info::{Feature, Info, InfoBuilder};
use pdb_rs::native::msf::StreamId;
use pdb_rs::native::{Identity, IdentityPolicy, Producer};

const GUID: [u8; 16] = [0x42; 16];

fn word(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

/// An Info stream with a deleted bucket, a `niMac` word and two feature codes.
fn decoded() -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in [20_000_404, 123, 7] {
        word(&mut bytes, value);
    }
    bytes.extend_from_slice(&GUID);
    let strings = b"/names\0/opaque\xff\0";
    word(&mut bytes, u32::try_from(strings.len()).unwrap());
    bytes.extend_from_slice(strings);
    // `/names` hashes to bucket 1; `/opaque\xff` hashes to the deleted bucket 3 and is in 4.
    for value in [
        2,
        8,
        1,
        0b1_0010,
        1,
        0b1000,
        0,
        8,
        7,
        9,
        3,
        20_140_508,
        0xdead_beef,
    ] {
        word(&mut bytes, value);
    }
    bytes
}

fn encode(builder: &InfoBuilder) -> Vec<u8> {
    let mut bytes = Vec::new();
    builder.encode(&mut bytes).unwrap();
    bytes
}

#[test]
fn a_decoded_stream_encodes_to_the_same_bytes() {
    let bytes = decoded();
    let info = Info::parse(bytes.clone()).unwrap();
    let mut out = Vec::new();
    info.encode(&mut out).unwrap();
    assert_eq!(out, bytes);
    for producer in [Producer::Microsoft, Producer::Llvm] {
        let builder = InfoBuilder::from_info_with(&info, producer).unwrap();
        assert_eq!(encode(&builder), bytes);
    }
}

#[test]
fn feature_codes_are_typed_and_unknown_codes_stay() {
    let info = Info::parse(decoded()).unwrap();
    assert_eq!(
        info.features().collect::<Vec<_>>(),
        [Feature::VC140, Feature(0xdead_beef)]
    );
    assert!(info.has_feature(Feature::VC140));
    assert!(!info.has_feature(Feature::MINIMAL_DEBUG_INFO));
    assert_eq!(Feature::VC110.0, 20_091_201);
    assert_eq!(Feature::VC140.0, 20_140_508);
    assert_eq!(Feature::MINIMAL_DEBUG_INFO.0.to_le_bytes(), *b"MINI");
    assert_eq!(Feature::NO_TYPE_MERGE.0.to_le_bytes(), *b"NOTM");
    let mut builder = InfoBuilder::from_info(&info).unwrap();
    assert_eq!(
        builder.features().collect::<Vec<_>>(),
        [Feature::VC140, Feature(0xdead_beef)]
    );
    builder
        .set_features(&[Feature::VC140, Feature::MINIMAL_DEBUG_INFO])
        .unwrap();
    let rebuilt = Info::parse(encode(&builder)).unwrap();
    assert_eq!(
        rebuilt
            .features()
            .map(|feature| feature.0)
            .collect::<Vec<_>>(),
        [20_140_508, 0x494e_494d]
    );
}

#[test]
fn new_maps_grow_like_the_producers() {
    for producer in [Producer::Microsoft, Producer::Llvm] {
        let mut builder = InfoBuilder::new(producer, 9, GUID, 1).unwrap();
        assert_eq!(builder.named_stream_capacity(), 1);
        let mut capacities = Vec::new();
        for (index, name) in ["/LinkInfo", "/TMCache", "/names", "/src/headerblock", "/x"]
            .iter()
            .enumerate()
        {
            let stream = StreamId(u32::try_from(index).unwrap() + 5);
            builder.set_named_stream(name.as_bytes(), stream).unwrap();
            capacities.push(builder.named_stream_capacity());
        }
        assert_eq!(capacities, [2, 4, 6, 6, 10]);
        let info = Info::parse(encode(&builder)).unwrap();
        assert_eq!(info.named_stream(b"/names"), Some(StreamId(7)));
        assert_eq!(info.named_streams().len(), 5);
        assert_eq!(info.next_name_index(), 0);
        assert_eq!(builder.named_stream(b"/x").unwrap(), Some(StreamId(9)));
    }
}

#[test]
fn existing_names_and_removals_keep_the_string_buffer_rules() {
    let mut builder = InfoBuilder::new(Producer::Microsoft, 9, GUID, 1).unwrap();
    builder.set_named_stream(b"/names", StreamId(7)).unwrap();
    builder.set_named_stream(b"/LinkInfo", StreamId(5)).unwrap();
    let strings = builder.string_buffer().to_vec();
    // An existing name gets the new stream; the string buffer does not change.
    builder.set_named_stream(b"/names", StreamId(8)).unwrap();
    assert_eq!(builder.string_buffer(), strings);
    assert_eq!(builder.named_stream(b"/names").unwrap(), Some(StreamId(8)));
    // A removal keeps the name in the buffer and leaves a deleted bucket.
    assert_eq!(
        builder.remove_named_stream(b"/names").unwrap(),
        Some(StreamId(8))
    );
    assert_eq!(builder.remove_named_stream(b"/names").unwrap(), None);
    assert_eq!(builder.string_buffer(), strings);
    let info = Info::parse(encode(&builder)).unwrap();
    assert_eq!(info.named_stream(b"/names"), None);
    assert_eq!(info.named_stream(b"/LinkInfo"), Some(StreamId(5)));
    // A name that goes in again is a new copy at the end of the buffer.
    builder.set_named_stream(b"/names", StreamId(10)).unwrap();
    assert_eq!(builder.string_buffer().len(), strings.len() + 7);
    let names = builder.named_streams().unwrap();
    assert_eq!(names.len(), 2);
    for name in [&b""[..], b"a\0b"] {
        let error = builder.set_named_stream(name, StreamId(1)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    }
    assert_eq!(builder.string_buffer().len(), strings.len() + 7);
}

#[test]
fn identity_policies_keep_the_ages_consistent() {
    let mut builder = InfoBuilder::new(Producer::Microsoft, 9, GUID, 2).unwrap();
    // `pdbstr -w` writes Info age 2 and keeps DBI age 1; the image matches age 1.
    let mut dbi_age = 1;
    let matched = builder
        .apply_identity(IdentityPolicy::Preserve, &mut dbi_age)
        .unwrap();
    assert_eq!(matched, Identity::Modern { guid: GUID, age: 1 });
    assert_eq!(dbi_age, 1);
    let matched = builder
        .apply_identity(IdentityPolicy::IncrementAge, &mut dbi_age)
        .unwrap();
    assert_eq!((matched.age(), dbi_age), (3, 3));
    assert_eq!(builder.identity(), Identity::Modern { guid: GUID, age: 3 });
    let guid = [7; 16];
    let matched = builder
        .apply_identity(
            IdentityPolicy::Replace {
                signature: 77,
                guid,
                age: 1,
            },
            &mut dbi_age,
        )
        .unwrap();
    assert_eq!(matched, Identity::Modern { guid, age: 1 });
    assert_eq!(dbi_age, 1);
    assert_eq!(builder.signature(), 77);
    let info = Info::parse(encode(&builder)).unwrap();
    assert_eq!(info.identity(), Identity::Modern { guid, age: 1 });

    let mut larger = 2;
    let error = builder
        .apply_identity(IdentityPolicy::Preserve, &mut larger)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
    let mut last = InfoBuilder::new(Producer::Llvm, 0, GUID, u32::MAX).unwrap();
    let error = last
        .apply_identity(IdentityPolicy::IncrementAge, &mut larger)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
    assert_eq!(larger, 2);
}
