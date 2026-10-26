//! Round trips of the native record encoders and stream writers.
//!
//! Each round trip decodes a value, encodes it and decodes the output. The encoder must
//! accept each decoded value, and the second value must equal the first. Where the
//! documentation of an encoder says that a decoded value gives the input bytes again, the
//! round trip also compares the bytes.

pub mod streams;
pub mod symbols;

use pdbrs::native::codeview::field_list::{LLVM_FIELD_LIST_LIMIT, Member, encode_field_list};
use pdbrs::native::codeview::id_record::IdRecord;
use pdbrs::native::codeview::index::TypeIndex;
use pdbrs::native::codeview::numeric::Numeric;
use pdbrs::native::codeview::type_record::TypeLeaf;
use pdbrs::native::codeview::{Record, Records};
use pdbrs::native::types::{TypeHashes, TypeStream, TypeStreamOptions, encode_type_stream};

use crate::input::Input;
use crate::ok;

/// The most records that one input encodes, to bound the time of one run.
const MAX_RECORDS: usize = 1 << 14;

/// Return the only record in `bytes`.
fn single_record(bytes: &[u8]) -> Record<'_> {
    let mut records = Records::new(bytes);
    let record = records
        .next()
        .expect("an encoded record frames")
        .expect("an encoded record frames");
    assert!(records.next().is_none(), "an encoder writes one record");
    record
}

/// Return the members of a field list, or `None` if a member does not decode.
fn members<'a>(
    list: &pdbrs::native::codeview::field_list::FieldList<'a>,
) -> Option<Vec<Member<'a>>> {
    list.members().collect::<pdbrs::Result<_>>().ok()
}

/// Encode a type leaf and check that the output decodes to the same leaf.
///
/// A field list and a method list keep their raw bytes, with the padding of the input. The
/// encoder writes the standard `LF_PAD` bytes, so the round trip compares the decoded members
/// and entries of these lists. Return the encoded record.
fn type_leaf(leaf: &TypeLeaf<'_>) -> Vec<u8> {
    let mut out = Vec::new();
    leaf.encode(&mut out).expect("a decoded type leaf encodes");
    let again = TypeLeaf::decode(single_record(&out)).expect("an encoded type leaf decodes");
    match (&again, leaf) {
        (TypeLeaf::FieldList(left), TypeLeaf::FieldList(right)) => {
            assert_eq!(members(left), members(right));
            assert_eq!(left.continuation(), right.continuation());
            field_list(right.members());
        }
        (TypeLeaf::MethodList(left), TypeLeaf::MethodList(right)) => {
            let entries = |list: &pdbrs::native::codeview::type_record::MethodList<'_>| {
                list.entries().collect::<pdbrs::Result<Vec<_>>>().ok()
            };
            assert_eq!(entries(left), entries(right));
        }
        _ => assert_eq!(&again, leaf),
    }
    out
}

/// Split field-list members into records and check that the records give the members again.
fn field_list<'a>(members: impl Iterator<Item = pdbrs::Result<Member<'a>>>) {
    let Some(members) = members
        .take(MAX_RECORDS)
        .filter(|member| !matches!(member, Ok(Member::Continuation(_))))
        .collect::<pdbrs::Result<Vec<_>>>()
        .ok()
    else {
        return;
    };
    let mut out = Vec::new();
    if encode_field_list(&members, TypeIndex(0x1000), LLVM_FIELD_LIST_LIMIT, &mut out).is_err() {
        return;
    }
    // The writer puts the continuation records first; the chain starts at the last record.
    let records: Vec<Record<'_>> = Records::new(&out)
        .collect::<pdbrs::Result<_>>()
        .expect("encoded field lists frame");
    let mut again = Vec::new();
    for record in records.iter().rev() {
        let TypeLeaf::FieldList(list) =
            TypeLeaf::decode(*record).expect("an encoded field list decodes")
        else {
            panic!("the field-list writer wrote another leaf kind");
        };
        for member in list.members() {
            let member = member.expect("an encoded member decodes");
            if !matches!(member, Member::Continuation(_)) {
                again.push(member);
            }
        }
    }
    assert_eq!(again, members);
}

/// Encode an ID record and check that the output decodes to the same record.
fn id_record(record: &IdRecord<'_>) -> Vec<u8> {
    let mut out = Vec::new();
    record
        .encode(&mut out)
        .expect("a decoded ID record encodes");
    let again = IdRecord::decode(single_record(&out)).expect("an encoded ID record decodes");
    assert_eq!(&again, record);
    out
}

/// Round-trip one record payload: the first two bytes are the kind.
///
/// # Panics
///
/// Panics if an encoder rejects a decoded value or gives other values or bytes.
pub fn leaf(data: &[u8]) {
    let mut input = Input::new(data);
    let kind = input.u16();
    let record = Record::new(0, kind, input.rest());
    if let Some(leaf) = ok(TypeLeaf::decode(record)) {
        type_leaf(&leaf);
    }
    if let Some(id) = ok(IdRecord::decode(record)) {
        id_record(&id);
    }
    if let Some(symbol) = ok(pdbrs::native::codeview::symbol::Symbol::decode(record)) {
        symbols::symbol(&symbol);
    }
    if let Some((numeric, rest)) = ok(Numeric::parse_prefix(record.data)) {
        let mut out = Vec::new();
        numeric
            .encode(&mut out)
            .expect("a decoded numeric leaf encodes");
        let used = record.data.len() - rest.len();
        assert_eq!(
            out,
            &record.data[..used],
            "a numeric leaf gives its bytes again"
        );
        assert_eq!(out.len(), numeric.encoded_len());
    }
}

/// Encode each record of a type stream and rebuild the stream.
///
/// `encode` returns the encoded record, or `None` if the record does not decode.
fn rebuild(
    stream: &TypeStream,
    hash: &[u8],
    mut encode: impl FnMut(Record<'_>) -> Option<(Vec<u8>, bool)>,
) {
    let mut records = Vec::new();
    for record in stream.records().take(MAX_RECORDS) {
        let Some(record) = ok(record) else {
            return;
        };
        // A record offset is a location, not a value: decode the record at offset 0.
        let Some((encoded, exact)) = encode(Record::new(0, record.record.kind, record.record.data))
        else {
            return;
        };
        // The encoders give the input record again for a decoded record of a padded stream
        // (a record length that is a multiple of 4), except the padding between field-list
        // members (see `type_leaf`).
        let full = stream
            .full_record(&record.record)
            .expect("a framed record has bytes");
        if exact && full.len().is_multiple_of(4) {
            assert_eq!(
                encoded, full,
                "record {:#x} re-encodes to other bytes",
                record.index
            );
        }
        records.extend_from_slice(&encoded);
    }
    let hashes = ok(TypeHashes::parse(stream.header(), hash));
    let adjusters = hashes.as_ref().map_or_default(TypeHashes::adjusters);
    let options = TypeStreamOptions::from_header(stream.header(), adjusters);
    let Some(written) = ok(encode_type_stream(&records, &options)) else {
        return;
    };
    let again = TypeStream::parse(written.stream).expect("a written type stream parses");
    assert_eq!(
        again.record_bytes(),
        records,
        "a written stream keeps the records"
    );
    if let Some(hash_stream) = &written.hash_stream {
        let hashes =
            TypeHashes::parse(again.header(), hash_stream).expect("a written hash stream parses");
        hashes
            .check(&again)
            .expect("a written hash stream matches its records");
    }
}

/// Encode each record of a TPI stream and rebuild the stream and its hash stream.
///
/// The input is a hash stream part and then the type stream, as for the `native_types` target.
pub fn types(data: &[u8]) {
    let mut input = Input::new(data);
    let hash = input.part();
    let Some(stream) = ok(TypeStream::parse(input.rest().to_vec())) else {
        return;
    };
    rebuild(&stream, hash, |record| {
        ok(TypeLeaf::decode(record))
            .map(|leaf| (type_leaf(&leaf), !matches!(leaf, TypeLeaf::FieldList(_))))
    });
}

/// Encode each record of an IPI stream and rebuild the stream and its hash stream.
///
/// The input has the layout of the `native_ids` target.
pub fn ids(data: &[u8]) {
    let mut input = Input::new(data);
    let hash = input.part();
    let _names = input.part();
    let _end = input.u32();
    let Some(stream) = ok(TypeStream::parse(input.rest().to_vec())) else {
        return;
    };
    rebuild(&stream, hash, |record| {
        ok(IdRecord::decode(record)).map(|id| (id_record(&id), true))
    });
}
