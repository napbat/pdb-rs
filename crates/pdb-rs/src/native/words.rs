//! Little-endian word access for fixed-size records.
//!
//! The decoders split a record array into 4-byte words after they check its
//! length. All fields of these records are `u32` values or pairs of `u16`
//! values in one word. Reads therefore cannot fail after the length check.

/// Return word `index` of `record` as a little-endian `u32`.
///
/// Callers use constant indexes that are less than the record width.
pub(crate) fn word(record: &[[u8; 4]], index: usize) -> u32 {
    u32::from_le_bytes(record[index])
}

/// Return word `index` of `record` as two little-endian `u16` values.
///
/// The first value comes from the lower address.
pub(crate) fn halves(record: &[[u8; 4]], index: usize) -> (u16, u16) {
    let [b0, b1, b2, b3] = record[index];
    (u16::from_le_bytes([b0, b1]), u16::from_le_bytes([b2, b3]))
}
