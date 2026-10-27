//! Symbol records, the global symbol index (GSI) and the public symbol index (PSI).

use pdb_rs::native::codeview::Records;
use pdb_rs::native::codeview::index::ModuleSymbolOffset;
use pdb_rs::native::codeview::symbol::{
    AsyncMethodInfo, AttributeValue, CustomDebugInfo, CustomDebugInfoData, ModuleSymbols, Symbol,
    SymbolAttribute, SymbolData,
};
use pdb_rs::native::msf::StreamId;
use pdb_rs::native::symbols::{
    BucketCount, GlobalSymbolIndex, HashTable, PublicSymbolIndex, RecordOffsets,
};

use crate::input::Input;
use crate::ok;

/// The most records that one walk visits, to bound the time of one run.
const MAX_RECORDS: usize = 1 << 16;

/// The stream number that the targets give to symbol data.
const STREAM: StreamId = StreamId(7);

/// Decode a symbol record stream as shared records and as module records.
pub fn symbols(data: &[u8]) {
    let _ = walk_records(data, STREAM);
    for scoped in ModuleSymbols::new(data, STREAM, ModuleSymbolOffset(4), 64).take(MAX_RECORDS) {
        let Some(scoped) = ok(scoped) else {
            break;
        };
        let _ = (scoped.depth, scoped.offset);
        walk_symbol(&scoped.symbol);
    }
}

/// Decode a GSI and a PSI. The input is a selector byte, a GSI part, a PSI part and the
/// symbol records.
pub fn indexes(data: &[u8]) {
    let mut input = Input::new(data);
    let buckets = if input.u8() & 1 == 0 {
        BucketCount::Standard
    } else {
        BucketCount::Minimal
    };
    let gsi = input.part().to_vec();
    let psi = input.part().to_vec();
    let records = input.rest();
    let offsets = walk_records(records, STREAM);
    if let Some(globals) = ok(GlobalSymbolIndex::parse(gsi, StreamId(8), buckets)) {
        walk_globals(&globals, records, offsets.as_ref());
    }
    if let Some(publics) = ok(PublicSymbolIndex::parse(psi, StreamId(9), buckets)) {
        walk_publics(&publics, records, offsets.as_ref());
    }
}

/// Frame and decode each shared symbol record, and return the record offsets.
pub(crate) fn walk_records(records: &[u8], stream: StreamId) -> Option<RecordOffsets> {
    for record in Records::new(records).take(MAX_RECORDS) {
        let Some(record) = ok(record) else {
            break;
        };
        if let Some(symbol) = ok(Symbol::decode(record)) {
            walk_symbol(&symbol);
        }
    }
    let offsets = ok(RecordOffsets::frame(records, stream))?;
    assert_eq!(offsets.iter().count(), offsets.len());
    for offset in offsets.iter().take(MAX_RECORDS) {
        assert!(offsets.contains(offset));
        assert!(
            ok(offset.record(records)).is_some(),
            "a framed offset has a record"
        );
    }
    Some(offsets)
}

/// Call the accessors of a decoded symbol and decode its nested data.
pub(crate) fn walk_symbol(symbol: &Symbol<'_>) {
    let _ = (symbol.name(), symbol.address(), symbol.links());
    let _ = (symbol.type_index(), symbol.id_index());
    if let Some(reference) = symbol.procedure_reference() {
        let _ = reference.target;
    }
    if let Some(index) = symbol.type_index() {
        let _ = index.primitive();
    }
    match symbol.data() {
        SymbolData::Oem(oem) => {
            if let Ok(Some(attribute)) = oem.attribute() {
                walk_attribute(&attribute);
            }
        }
        SymbolData::InlineSite(site) => {
            let _ = site.annotations.iter().take(MAX_RECORDS).count();
        }
        SymbolData::Inlinees(list) => {
            assert_eq!(list.iter().count(), list.len());
        }
        SymbolData::Callees(list) => {
            let functions = list.functions();
            assert_eq!(functions.iter().count(), functions.len());
            let _ = list.invocations().count();
        }
        SymbolData::EnvironmentBlock(block) => {
            let _ = block.strings.iter().take(MAX_RECORDS).count();
        }
        _ => {}
    }
}

/// Decode a symbol attribute and the custom debug information in it.
fn walk_attribute(attribute: &SymbolAttribute<'_>) {
    let _ = attribute.name_units().count();
    match ok(attribute.decode()) {
        Some(AttributeValue::CustomDebugInfo(info)) => walk_custom_debug_info(&info),
        Some(AttributeValue::AsyncMethodInfo(info)) => {
            let _ = info.steps.iter().count();
        }
        _ => {}
    }
}

/// Decode a native symbol attribute or its value.
///
/// The first input byte selects the format of the rest: bit 0 and bit 1 clear give an `MD2`
/// value, bit 0 set gives an `asyncMethodInfo` value, and bit 1 set gives a whole attribute
/// (the UTF-16 name and the value).
pub fn attribute_value(data: &[u8]) {
    let Some((&selector, value)) = data.split_first() else {
        return;
    };
    if selector & 2 != 0 {
        if let Some(attribute) = ok(SymbolAttribute::parse(value)) {
            walk_attribute(&attribute);
        }
    } else if selector & 1 == 0 {
        if let Some(info) = ok(CustomDebugInfo::parse(value)) {
            walk_custom_debug_info(&info);
        }
    } else if let Some(info) = ok(AsyncMethodInfo::parse(value)) {
        let _ = info.steps.iter().count();
    }
}

/// Decode each record of a native custom debug information value.
fn walk_custom_debug_info(info: &CustomDebugInfo<'_>) {
    assert_eq!(info.records().count(), info.len());
    for record in info.records() {
        let Some(data) = ok(record.decode()) else {
            continue;
        };
        match data {
            CustomDebugInfoData::UsingGroups(groups) => {
                let _ = groups.iter().count();
            }
            CustomDebugInfoData::StateMachineHoistedLocalScopes(scopes) => {
                let _ = scopes.iter().count();
            }
            CustomDebugInfoData::DynamicLocals(locals) => {
                let _ = locals.iter().count();
            }
            CustomDebugInfoData::TupleElementNames(names) => {
                let _ = names.iter().take(MAX_RECORDS).count();
            }
            CustomDebugInfoData::EditAndContinueLocalSlotMap(map) => {
                let _ = map.iter().take(MAX_RECORDS).count();
            }
            CustomDebugInfoData::EditAndContinueLambdaMap(map) => {
                let _ = map.closures().take(MAX_RECORDS).count();
                let _ = map.lambdas().take(MAX_RECORDS).count();
            }
            CustomDebugInfoData::EditAndContinueStateMachineStateMap(map) => {
                let _ = map.iter().take(MAX_RECORDS).count();
            }
            _ => {}
        }
    }
}

/// Check a symbol hash table against the records and look up each indexed name.
fn walk_table(table: &HashTable<'_>, records: &[u8], offsets: Option<&RecordOffsets>) {
    let _ = (
        table.header(),
        table.bucket_count(),
        table.nonempty_bucket_count(),
    );
    assert_eq!(table.records().count(), table.len());
    let _ = table.buckets().take(MAX_RECORDS).count();
    let checked = offsets.is_some_and(|offsets| table.check_records(offsets).is_ok());
    for entry in table.records().take(MAX_RECORDS) {
        let Some(record) = ok(entry.symbol.record(records)) else {
            assert!(!checked, "a checked index names a record");
            continue;
        };
        if let Some(symbol) = ok(Symbol::decode(record))
            && let Some(name) = symbol.name()
        {
            let _ = table.lookup(name).count();
        }
    }
}

/// Decode a GSI against the symbol records.
pub(crate) fn walk_globals(
    globals: &GlobalSymbolIndex,
    records: &[u8],
    offsets: Option<&RecordOffsets>,
) {
    walk_table(&globals.hash_table(), records, offsets);
}

/// Decode a PSI against the symbol records.
pub(crate) fn walk_publics(
    publics: &PublicSymbolIndex,
    records: &[u8],
    offsets: Option<&RecordOffsets>,
) {
    walk_table(&publics.hash_table(), records, offsets);
    let _ = publics.header();
    assert_eq!(publics.address_map().count(), publics.address_map().len());
    assert_eq!(publics.thunk_map().count(), publics.thunk_map().len());
    assert_eq!(
        publics.section_starts().count(),
        publics.section_starts().len()
    );
    if let Some(offsets) = offsets {
        let _ = publics.check_records(offsets);
    }
    let _ = publics.check_address_order(|offset| {
        let symbol = Symbol::decode(offset.record(records)?)?;
        let address = symbol.address().ok_or_else(|| {
            pdb_rs::Error::new(pdb_rs::ErrorKind::InvalidReference, None)
                .with_context("public symbol has no address")
        })?;
        Ok((address.section, address.offset))
    });
}
