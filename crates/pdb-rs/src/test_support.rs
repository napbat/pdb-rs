//! Helpers for unit tests.

use alloc::vec::Vec;

/// Decode a hexadecimal string. The function ignores ASCII whitespace.
///
/// # Panics
///
/// Panics if `text` has an odd digit count or a character that is not a
/// hexadecimal digit.
pub(crate) fn hex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .map(|byte| match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => panic!("not a hexadecimal digit: {byte:#04x}"),
        })
        .collect();
    let (pairs, _) = digits.as_chunks::<2>();
    pairs.iter().map(|&[high, low]| (high << 4) | low).collect()
}
