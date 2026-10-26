//! The TPI and IPI streams, their hash streams and their records.

use pdbrs::native::codeview::Record;
use pdbrs::native::codeview::field_list::FieldList;
use pdbrs::native::codeview::id_record::{BuildInfoArgument, IdRecord, IdTable};
use pdbrs::native::codeview::index::{IdIndex, TypeIndex};
use pdbrs::native::codeview::numeric::Integer;
use pdbrs::native::codeview::type_record::TypeLeaf;
use pdbrs::native::names::StringTable;
use pdbrs::native::types::{ForwardRefs, TypeHashes, TypeStream, TypeTable, type_record_hash};

use crate::input::Input;
use crate::ok;

/// The most records that one input decodes, to bound the time of one run.
const MAX_RECORDS: usize = 1 << 16;

/// Decode a TPI stream. The input is a hash stream part and then the type stream.
pub fn types(data: &[u8]) {
    let mut input = Input::new(data);
    let hash = input.part();
    if let Some(stream) = ok(TypeStream::parse(input.rest().to_vec())) {
        walk_types(&stream, Some(hash));
    }
}

/// Decode an IPI stream. The input is a hash stream part, a `/names` part and the ID stream.
pub fn ids(data: &[u8]) {
    let mut input = Input::new(data);
    let hash = input.part();
    let names = ok(StringTable::parse(input.part()));
    let end = TypeIndex(input.u32());
    if let Some(stream) = ok(TypeStream::parse(input.rest().to_vec())) {
        walk_ids(&stream, Some(hash), end, names.as_ref());
    }
}

/// Decode the payload of one leaf record: the first two bytes are the kind.
///
/// # Panics
///
/// Panics if a numeric leaf decodes without bytes.
pub fn leaf(data: &[u8]) {
    let mut input = Input::new(data);
    let kind = input.u16();
    let record = Record::new(0, kind, input.rest());
    decode_leaf(record);
    let _ = IdRecord::decode(record);
    let _ = pdbrs::native::codeview::symbol::Symbol::decode(record);
    if let Some(list) = ok(FieldList::parse(record.data)) {
        for member in list.members().take(MAX_RECORDS) {
            if member.is_err() {
                break;
            }
        }
    }
    let mut rest = record.data;
    while let Some((_, tail)) = ok(Integer::parse_prefix(rest)) {
        assert!(tail.len() < rest.len(), "a numeric leaf consumes bytes");
        rest = tail;
    }
}

/// Decode one type leaf and the lists in it.
fn decode_leaf(record: Record<'_>) {
    let Some(leaf) = ok(TypeLeaf::decode(record)) else {
        return;
    };
    let _ = leaf.tag();
    match &leaf {
        TypeLeaf::FieldList(list) => {
            for member in list.members().take(MAX_RECORDS) {
                if member.is_err() {
                    break;
                }
            }
        }
        TypeLeaf::MethodList(list) => {
            for entry in list.entries().take(MAX_RECORDS) {
                if entry.is_err() {
                    break;
                }
            }
        }
        TypeLeaf::ArgumentList(list) => {
            assert_eq!(list.iter().count(), list.len());
        }
        TypeLeaf::VtableShape(shape) => {
            let _ = shape.slots().count();
        }
        TypeLeaf::VirtualFunctionTable(table) => {
            let _ = table.table_name();
            let _ = table.method_names().take(MAX_RECORDS).count();
        }
        _ => {}
    }
}

/// Decode every record of a TPI stream and use the hash stream, if it decodes.
pub(crate) fn walk_types(stream: &TypeStream, hash: Option<&[u8]>) {
    let hashes = hash.and_then(|bytes| ok(TypeHashes::parse(stream.header(), bytes)));
    if let Some(hashes) = &hashes {
        let _ = hashes.check(stream);
        let _ = (hashes.bucket_count(), hashes.adjusters().len());
        let _ = hashes.hash_values().count();
        let _ = hashes.index_offsets().count();
        for index in [0, 1, stream.header().index_begin, stream.header().index_end] {
            let _ = hashes.locate(stream, index);
            let _ = hashes.hash_value(index);
        }
    }
    let Some(table) = ok(TypeTable::new(stream)) else {
        return;
    };
    let forward = ok(ForwardRefs::new(&table, hashes.as_ref()));
    for record in stream.records().take(MAX_RECORDS) {
        let Some(record) = ok(record) else {
            break;
        };
        let _ = stream.full_record(&record.record);
        if let Some(bytes) = ok(stream.full_record(&record.record)) {
            let _ = type_record_hash(bytes);
        }
        decode_leaf(record.record);
        let index = TypeIndex(record.index);
        let _ = table.record(index);
        if let Some(leaf) = ok(table.leaf(index)) {
            if matches!(leaf, TypeLeaf::FieldList(_))
                && let Some(members) = ok(table.members(index))
            {
                for member in members.take(MAX_RECORDS) {
                    if member.is_err() {
                        break;
                    }
                }
            }
            if let (Some(forward), Some(tag)) = (&forward, leaf.tag())
                && tag.options.is_forward_reference()
            {
                let _ = forward.resolve(index);
            }
        }
        let _ = table.field_list(index);
        let _ = table.id_record(IdIndex(record.index));
    }
    let _ = table.record_at_index(0);
}

/// Decode every record of an IPI stream and resolve the strings.
pub(crate) fn walk_ids(
    stream: &TypeStream,
    hash: Option<&[u8]>,
    tpi_end: TypeIndex,
    names: Option<&StringTable<'_>>,
) {
    if let Some(hashes) = hash.and_then(|bytes| ok(TypeHashes::parse(stream.header(), bytes))) {
        let _ = hashes.check(stream);
    }
    let Some(table) = ok(IdTable::new(stream)) else {
        return;
    };
    let _ = table.check(tpi_end);
    let _ = (table.index_begin(), table.index_end(), table.len());
    let mut text = Vec::new();
    for record in stream.records().take(MAX_RECORDS) {
        let Some(record) = ok(record) else {
            break;
        };
        let id = IdIndex(record.index);
        assert!(table.record(id).is_some(), "a framed ID has a record");
        match ok(table.leaf(id)) {
            Some(IdRecord::String(_) | IdRecord::SubstringList(_)) => {
                text.clear();
                let _ = table.resolve_string(id, &mut text);
            }
            Some(IdRecord::UdtModuleSourceLine(line)) => {
                if let Some(names) = names {
                    let _ = names.get(line.name_offset);
                }
            }
            Some(IdRecord::BuildInfo(info)) => {
                for argument in [
                    BuildInfoArgument::CurrentDirectory,
                    BuildInfoArgument::BuildTool,
                    BuildInfoArgument::SourceFile,
                    BuildInfoArgument::ProgramDatabase,
                    BuildInfoArgument::CommandLine,
                ] {
                    if let Some(id) = info.argument(argument) {
                        text.clear();
                        let _ = table.resolve_string(id, &mut text);
                    }
                }
            }
            _ => {}
        }
    }
}
