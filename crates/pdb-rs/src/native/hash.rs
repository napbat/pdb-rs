//! Hash functions used by native PDB tables.
//!
//! The functions match the Microsoft PDB implementation. Several PDB tables
//! store only `hash % bucket_count`. Callers apply the reduction that their
//! table uses. The functions read the input as raw bytes. They do not
//! decode text or allocate memory.

/// Return the version 1 string hash (`Hasher::lhashPbCb`).
///
/// The named-stream map, version 1 string tables, global and public symbol
/// indexes and type hashes use this function.
#[must_use]
pub fn hash_string_v1(bytes: &[u8]) -> u32 {
    let (words, rest) = bytes.as_chunks::<4>();
    let mut result = words
        .iter()
        .fold(0_u32, |value, word| value ^ u32::from_le_bytes(*word));
    let mut rest = rest;
    if let Some((pair, tail)) = rest.split_first_chunk::<2>() {
        result ^= u32::from(u16::from_le_bytes(*pair));
        rest = tail;
    }
    if let Some(&byte) = rest.first() {
        result ^= u32::from(byte);
    }
    result |= 0x2020_2020;
    result ^= result >> 11;
    result ^ (result >> 16)
}

/// Return the version 2 string hash (`HasherV2::lhashPbCb` in Microsoft `misc.h`).
///
/// Version 2 string tables use this function. The result is the value before the reduction
/// modulo `0xffff_ffff` that `NMT` applies; LLVM `hashStringV2` returns the same value.
#[must_use]
pub fn hash_string_v2(bytes: &[u8]) -> u32 {
    let step = |hash: u32, item: u32| {
        let hash = hash.wrapping_add(item);
        let hash = hash.wrapping_add(hash << 10);
        hash ^ (hash >> 6)
    };
    let (words, rest) = bytes.as_chunks::<4>();
    let hash = words.iter().fold(0xb170_a1bf, |hash, word| {
        step(hash, u32::from_le_bytes(*word))
    });
    let hash = rest
        .iter()
        .fold(hash, |hash, &byte| step(hash, u32::from(byte)));
    hash.wrapping_mul(1_664_525).wrapping_add(1_013_904_223)
}

/// Return the buffer hash `SigForPbCb`, a CRC-32 with an initial and final value of zero.
///
/// Type hashes use this function for most records. Injected-source entries
/// store the same CRC of their content.
#[must_use]
pub fn hash_buffer_v8(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0_u32, |crc, &byte| {
        let mut value = crc ^ u32::from(byte);
        for _ in 0..8 {
            let mask = (value & 1).wrapping_neg();
            value = (value >> 1) ^ (0xedb8_8320 & mask);
        }
        value
    })
}
