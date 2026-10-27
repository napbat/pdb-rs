//! Tests for type sizes, module type streams and `S_MOD_TYPEREF` sources.

use pdb_rs::ErrorKind;
use pdb_rs::native::codeview::index::{ModuleIndex, TypeIndex};
use pdb_rs::native::codeview::symbol::{
    ModuleTypeReference, ModuleTypeReferenceFlags, ModuleTypeSource,
};
use pdb_rs::native::msf::StreamId;
use pdb_rs::native::types::{ForwardRefs, TypeStream, TypeTable, type_size};

/// Append a type record of `kind` with `payload` and `LF_PAD` padding.
fn record(out: &mut Vec<u8>, kind: u16, payload: &[u8]) {
    let mut body = payload.to_vec();
    while !(body.len() + 2).is_multiple_of(4) {
        let left = u8::try_from(4 - (body.len() + 2) % 4).unwrap();
        body.push(0xf0 | left);
    }
    out.extend_from_slice(&u16::try_from(body.len() + 2).unwrap().to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&body);
}

/// Return the payload of a structure with `size`, `properties` and `name`.
fn structure(size: u16, properties: u16, name: &[u8]) -> Vec<u8> {
    let mut payload = 0_u16.to_le_bytes().to_vec();
    payload.extend_from_slice(&properties.to_le_bytes());
    for _ in 0..3 {
        payload.extend_from_slice(&0_u32.to_le_bytes());
    }
    payload.extend_from_slice(&size.to_le_bytes());
    payload.extend_from_slice(name);
    payload.push(0);
    payload
}

/// Build records 0x1000 to 0x1006:
///
/// - 0x1000: `LF_MODIFIER` of `int`.
/// - 0x1001: `LF_ARRAY` of 12 bytes.
/// - 0x1002: a forward reference to `S`.
/// - 0x1003: the structure `S` with 8 bytes.
/// - 0x1004: `LF_ENUM` with the underlying type `int`.
/// - 0x1005: `LF_PROCEDURE` (no size).
/// - 0x1006: `LF_POINTER` with 8 bytes.
fn records() -> Vec<u8> {
    let mut out = Vec::new();
    let mut modifier = 0x74_u32.to_le_bytes().to_vec();
    modifier.extend_from_slice(&1_u16.to_le_bytes());
    record(&mut out, 0x1001, &modifier);
    let mut array = 0x74_u32.to_le_bytes().to_vec();
    array.extend_from_slice(&0x23_u32.to_le_bytes());
    array.extend_from_slice(&12_u16.to_le_bytes());
    array.push(0);
    record(&mut out, 0x1503, &array);
    record(&mut out, 0x1505, &structure(0, 0x80, b"S"));
    record(&mut out, 0x1505, &structure(8, 0, b"S"));
    let mut enumeration = 0_u16.to_le_bytes().to_vec();
    enumeration.extend_from_slice(&0_u16.to_le_bytes());
    enumeration.extend_from_slice(&0x74_u32.to_le_bytes());
    enumeration.extend_from_slice(&0_u32.to_le_bytes());
    enumeration.extend_from_slice(b"E\0");
    record(&mut out, 0x1507, &enumeration);
    let mut procedure = 0x74_u32.to_le_bytes().to_vec();
    procedure.extend_from_slice(&[0, 0, 0, 0]);
    procedure.extend_from_slice(&0_u32.to_le_bytes());
    record(&mut out, 0x1008, &procedure);
    let mut pointer = 0x74_u32.to_le_bytes().to_vec();
    pointer.extend_from_slice(&(0x0c_u32 | (8 << 13)).to_le_bytes());
    record(&mut out, 0x1002, &pointer);
    out
}

#[test]
fn makes_a_type_stream_from_header_less_records() {
    let bytes = records();
    let stream = TypeStream::from_records(&bytes).unwrap();
    assert_eq!(stream.record_bytes(), &bytes[..]);
    assert_eq!(stream.header().index_begin, 0x1000);
    assert_eq!(stream.header().index_end, 0x1007);
    assert_eq!(stream.header().hash_stream, None);
    let error = TypeStream::from_records(&bytes[..bytes.len() - 1]).unwrap_err();
    assert_ne!(error.kind(), ErrorKind::LimitExceeded);
}

#[test]
fn computes_type_sizes() {
    let stream = TypeStream::from_records(&records()).unwrap();
    let table = TypeTable::new(&stream).unwrap();
    let forward = ForwardRefs::new(&table, None).unwrap();
    let size = |index: u32, forward: Option<&ForwardRefs<'_, '_>>| {
        type_size(&table, forward, TypeIndex(index)).unwrap()
    };
    assert_eq!(size(0x74, None), Some(4));
    assert_eq!(size(0x0603, None), Some(8), "64-bit pointer to void");
    assert_eq!(size(0x0000, None), None, "no type");
    assert_eq!(size(0x1000, None), Some(4));
    assert_eq!(size(0x1001, None), Some(12));
    assert_eq!(
        size(0x1002, None),
        None,
        "a forward reference needs ForwardRefs"
    );
    assert_eq!(size(0x1002, Some(&forward)), Some(8));
    assert_eq!(size(0x1003, None), Some(8));
    assert_eq!(size(0x1004, None), Some(4));
    assert_eq!(size(0x1005, None), None);
    assert_eq!(size(0x1006, None), Some(8));
    assert_eq!(size(0x1007, None), None, "past the stream");
}

/// Return a reference with `flags` and the two words.
fn reference(flags: u32, word0: u16, word1: u16) -> ModuleTypeReference {
    let mut reference = ModuleTypeReference::default();
    reference.flags = ModuleTypeReferenceFlags(flags);
    reference.word0 = word0;
    reference.word1 = word1;
    reference
}

#[test]
fn gives_the_source_of_module_type_records() {
    assert_eq!(reference(0x01, 0, 0).source(), ModuleTypeSource::NoTypes);
    assert_eq!(
        reference(0x08, 0x12, 0).source(),
        ModuleTypeSource::TypeRecords {
            stream: StreamId(0x12)
        }
    );
    assert_eq!(
        reference(0x10, 0x20, 0x21).source(),
        ModuleTypeSource::TypeServer {
            types: StreamId(0x20),
            ids: StreamId(0x21)
        }
    );
    assert_eq!(
        reference(0x20, 3, 0).source(),
        ModuleTypeSource::OtherModule {
            module: ModuleIndex(3)
        }
    );
    // Reserved bits do not change the source.
    assert_eq!(
        reference(0x8000_0020, 3, 0).source(),
        ModuleTypeSource::OtherModule {
            module: ModuleIndex(3)
        }
    );
    for flags in [0, 0x02, 0x04, 0x06, 0x0e, 0x18, 0x21] {
        let value = reference(flags, 1, 2);
        assert_eq!(value.source(), ModuleTypeSource::Other(value));
        assert!(!value.source().has_external_precompiled_types());
    }
}

/// `cl /Z7 /Yc` and `/Yu` with `link /DEBUG:LAZY` (MSVC 14.51) write `0x0c`
/// for the owner of the precompiled types and `0x0a` with the owner module in
/// word1 (`msvc-x64-pch-lazy`).
#[test]
fn gives_the_source_of_precompiled_type_modules() {
    let owner = reference(0x0c, 35, 0).source();
    assert_eq!(
        owner,
        ModuleTypeSource::PrecompiledTypeOwner {
            stream: StreamId(35)
        }
    );
    let user = reference(0x0a, 39, 0).source();
    assert_eq!(
        user,
        ModuleTypeSource::PrecompiledTypeUser {
            stream: StreamId(39),
            owner: ModuleIndex(0)
        }
    );
    for source in [owner, user] {
        assert!(source.has_external_precompiled_types());
        assert_eq!(
            source.streams().collect::<Vec<_>>(),
            [StreamId(source_stream(source))]
        );
    }
    // A new stream number replaces word0; the owner module in word1 stays.
    let mut value = reference(0x0a, 39, 4);
    value.map_streams(|_| Ok(StreamId(9))).expect("mapped");
    assert_eq!(value, reference(0x0a, 9, 4));
}

/// Return the stream of a precompiled-type source.
fn source_stream(source: ModuleTypeSource) -> u32 {
    match source {
        ModuleTypeSource::PrecompiledTypeOwner { stream }
        | ModuleTypeSource::PrecompiledTypeUser { stream, .. } => stream.0,
        _ => panic!("not a precompiled-type source"),
    }
}

#[test]
fn maps_only_the_stream_words_of_module_type_records() {
    let streams = |value: ModuleTypeReference| value.source().streams().collect::<Vec<_>>();
    assert_eq!(streams(reference(0x08, 0x12, 7)), [StreamId(0x12)]);
    assert_eq!(
        streams(reference(0x10, 0x20, 0x21)),
        [StreamId(0x20), StreamId(0x21)]
    );
    for flags in [0x01, 0x20, 0x06] {
        assert_eq!(streams(reference(flags, 1, 2)), []);
        let mut value = reference(flags, 1, 2);
        value
            .map_streams(|_| panic!("the record names no stream"))
            .expect("no stream");
        assert_eq!(value, reference(flags, 1, 2));
    }
    let mut value = reference(0x10, 0x20, 0x21);
    value
        .map_streams(|id| Ok(StreamId(id.0 + 0x100)))
        .expect("mapped");
    assert_eq!(value, reference(0x10, 0x120, 0x121));
    // The type records keep the second word.
    let mut value = reference(0x08, 0x12, 7);
    value.map_streams(|_| Ok(StreamId(5))).expect("mapped");
    assert_eq!(value, reference(0x08, 5, 7));
    let error = reference(0x08, 0x12, 0)
        .map_streams(|_| Ok(StreamId(0x1_0000)))
        .expect_err("the word has 16 bits");
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
}
