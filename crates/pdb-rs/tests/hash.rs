//! PDB hash functions checked against values from other producers.

use pdb_rs::native::hash::{hash_buffer_v8, hash_string_v1};

/// MSVC `link.exe` placed these named streams in a 14-slot hash table.
///
/// Each name occupies its home slot `(hash & 0xffff) % 14`, so the slot
/// checks the full hash path, including odd-length remainders.
#[test]
fn string_v1_matches_msvc_named_stream_slots() {
    for (name, slot) in [
        (&b"/names"[..], 5),
        (b"/LinkInfo", 7),
        (b"/TMCache", 7),
        (b"/UDTSRCLINEUNDONE", 4),
        (b"/src/headerblock", 9),
    ] {
        assert_eq!((hash_string_v1(name) & 0xffff) % 14, slot, "{name:?}");
    }
}

#[test]
fn string_v1_folds_ascii_case_but_keeps_byte_order() {
    // The mask sets bit 5 in every byte lane before mixing.
    assert_eq!(hash_string_v1(b"ABCD"), hash_string_v1(b"abcd"));
    assert_ne!(hash_string_v1(b"ab"), hash_string_v1(b"ba"));
}

#[test]
fn buffer_v8_is_crc32_without_initial_or_final_inversion() {
    // zlib computes the reference: crc32(data, 0xffffffff) ^ 0xffffffff.
    assert_eq!(hash_buffer_v8(b"123456789"), 0x2dfd_2d88);
    assert_eq!(hash_buffer_v8(b""), 0);
}
