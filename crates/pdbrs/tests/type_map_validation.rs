//! Tests for the reference validation of type map streams.

use pdbrs::native::auxiliary::{TypeMapContext, TypeMapRule, TypeMapStream};
use pdbrs::native::codeview::id_record::IdTable;
use pdbrs::native::codeview::index::{IdIndex, TypeIndex};
use pdbrs::native::msf::StreamId;
use pdbrs::native::types::TypeStream;

/// The type map stream in the tests.
const STREAM: StreamId = StreamId(6);

/// Serialize little-endian `u32` values.
fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// Build a version 2 type map stream with `first`, `map`, record `offsets`
/// and function ID `pairs`.
fn object_map(first: u32, map: &[u32], offsets: &[u32], pairs: &[u32]) -> Vec<u8> {
    let mut bytes = words(&[2]);
    bytes.extend_from_slice(&0xffff_u16.to_le_bytes());
    bytes.extend_from_slice(&[0, 0]);
    let count = u32::try_from(map.len()).unwrap();
    bytes.extend(words(&[7, first, count]));
    bytes.extend(words(map));
    bytes.extend(words(offsets));
    bytes.extend(words(pairs));
    bytes
}

/// Return IPI records: 0x1000 is `LF_FUNC_ID` of type 0x1005, 0x1001 is
/// `LF_STRING_ID`.
fn ipi() -> TypeStream {
    let mut records = Vec::new();
    let mut function = words(&[0, 0x1005]);
    function.extend_from_slice(b"f\0\xf2\xf1");
    records.extend_from_slice(&u16::try_from(function.len() + 2).unwrap().to_le_bytes());
    records.extend_from_slice(&0x1601_u16.to_le_bytes());
    records.extend_from_slice(&function);
    let mut string = words(&[0]);
    string.extend_from_slice(b"s\0\xf2\xf1");
    records.extend_from_slice(&u16::try_from(string.len() + 2).unwrap().to_le_bytes());
    records.extend_from_slice(&0x1605_u16.to_le_bytes());
    records.extend_from_slice(&string);
    TypeStream::from_records(&records).unwrap()
}

/// Return the rules, positions and values of the violations of `bytes`.
fn findings(bytes: &[u8], context: &TypeMapContext<'_>) -> Vec<(TypeMapRule, usize, u32)> {
    TypeMapStream::parse(bytes, STREAM)
        .unwrap()
        .validate(context)
        .unwrap()
        .iter()
        .map(|violation| (violation.rule, violation.position, violation.value))
        .collect()
}

#[test]
fn checks_type_server_maps_per_stream() {
    let stream = ipi();
    let ids = IdTable::new(&stream).unwrap();
    let context = TypeMapContext::new(TypeIndex(0x1010), IdIndex(0x1002)).with_ids(&ids);
    // Types 0x100f, 0, 0x1010, 0x0fff; IDs 0x1000, 0x1002; pairs (0x1000,
    // 0x1005), (0x1001, 0x1005), (0x1002, 0x1005), (0x1000, 0x1010).
    let bytes = words(&[
        1, 4, 2, 0x100f, 0, 0x1010, 0x0fff, 0x1000, 0x1002, 0x1000, 0x1005, 0x1001, 0x1005, 0x1002,
        0x1005, 0x1000, 0x1010,
    ]);
    assert_eq!(
        findings(&bytes, &context),
        [
            (TypeMapRule::TypeValue, 2, 0x1010),
            (TypeMapRule::TypeValue, 3, 0x0fff),
            (TypeMapRule::IdValue, 1, 0x1002),
            (TypeMapRule::FunctionMapping, 1, 0x1002),
            (TypeMapRule::FunctionInput, 2, 0x1002),
            (TypeMapRule::FunctionType, 3, 0x1010),
        ]
    );
}

#[test]
fn checks_object_maps_and_ties_function_ids_to_the_ipi_stream() {
    let stream = ipi();
    let ids = IdTable::new(&stream).unwrap();
    let context = TypeMapContext::new(TypeIndex(0x1010), IdIndex(0x1002)).with_ids(&ids);
    // A value up to the larger end is valid; 0 is not mapped.
    let map = [0x1000, 0x100f, 0, 0x1001, 0x1010];
    let offsets = [0, 4, 12, 16, 24, 32];
    let pairs = [0x1000, 0x1005, 0x1002, 0x1005, 0x1003, 0x1006];
    let bytes = object_map(0x1000, &map, &offsets, &pairs);
    assert_eq!(
        findings(&bytes, &context),
        [
            (TypeMapRule::ObjectValue, 4, 0x1010),
            (TypeMapRule::FunctionMapping, 2, 0x1001),
        ]
    );
    // Without the IPI records, the mapping is not checked.
    let ends = TypeMapContext::new(TypeIndex(0x1010), IdIndex(0x1002));
    assert_eq!(
        findings(&bytes, &ends),
        [(TypeMapRule::ObjectValue, 4, 0x1010)]
    );
}

#[test]
fn checks_object_map_offsets_and_first_index() {
    let context = TypeMapContext::new(TypeIndex(0x1010), IdIndex(0x1002));
    let bytes = object_map(0x0fff, &[0, 0, 0], &[4, 8, 10, 10], &[0x0ffe, 0x1005]);
    assert_eq!(
        findings(&bytes, &context),
        [
            (TypeMapRule::FirstIndex, 0, 0x0fff),
            (TypeMapRule::RecordOffset, 0, 4),
            (TypeMapRule::RecordOffset, 2, 10),
            (TypeMapRule::RecordOffset, 3, 10),
            (TypeMapRule::FunctionInput, 0, 0x0ffe),
        ]
    );
    let wide = object_map(0xffff_fffe, &[0, 0, 0], &[0, 4, 8, 12], &[]);
    assert_eq!(
        findings(&wide, &context),
        [(TypeMapRule::FirstIndex, 0, 0xffff_fffe)]
    );
}
