//! The PDB Info stream, its named stream map and the `/names` string table.

use pdbrs::native::auxiliary::NamedStreamKind;
use pdbrs::native::codeview::index::NameOffset;
use pdbrs::native::hash_table::HashTable;
use pdbrs::native::info::Info;
use pdbrs::native::msf::StreamId;
use pdbrs::native::names::StringTable;
use pdbrs::native::symbols::BucketCount;

use crate::ok;

/// Decode an Info stream and look up each named stream again.
///
/// # Panics
///
/// Panics if a lookup does not find a decoded name.
pub fn info(data: &[u8]) {
    let Some(info) = ok(Info::parse(data.to_vec())) else {
        return;
    };
    let _ = (info.version(), info.signature(), info.identity().age());
    let _ = (info.named_stream_capacity(), info.next_name_index());
    for named in info.named_streams() {
        let _ = NamedStreamKind::classify(named.name);
        assert!(
            info.named_stream(named.name).is_some(),
            "a decoded name has a stream"
        );
    }
    let buckets = BucketCount::from_features(info.features());
    let _ = buckets.bucket_of(b"main");
}

/// Decode a PDB hash table of `u32` values at the start of `data`.
///
/// # Panics
///
/// Panics if the table ends after the input.
pub fn hash_table(data: &[u8]) {
    if let Some((table, end)) = ok(HashTable::<u32>::parse(data, 0, StreamId(1))) {
        assert!(end <= data.len());
        let _ = (table.capacity(), table.deleted_count());
        for (bucket, _) in table.entries().iter().enumerate() {
            let _ = table.is_deleted(u32::try_from(bucket).unwrap_or(u32::MAX));
        }
        let _ = table.check_placement(|entry| Ok(entry.key));
    }
}

/// Decode a `/names` string table and look up each name.
pub fn names(data: &[u8]) {
    if let Some(table) = ok(StringTable::parse(data)) {
        walk_names(&table);
    }
}

/// Look up each name of a string table by offset and by name.
pub(crate) fn walk_names(table: &StringTable<'_>) {
    let _ = (table.hash_version(), table.name_count(), table.strings());
    let verified = table.validate_buckets().is_ok();
    for offset in table.bucket_offsets() {
        let _ = table.get(NameOffset(offset));
    }
    for entry in table.entries() {
        let name = ok(table.get(entry.offset));
        if let Some(name) = name
            && verified
            && !name.is_empty()
        {
            assert!(
                ok(table.find(name)).flatten().is_some(),
                "a verified table finds each name"
            );
        }
    }
}
