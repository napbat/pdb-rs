//! Integer numeric leaves in symbol records, with their stored form.
//!
//! [`Integer`] does not keep the leaf tag. A producer can write a value with
//! a wider leaf than necessary, for example `LF_ULONG` for 1. To encode the
//! record again to the same bytes, the symbol decoders keep the stored leaf
//! bytes next to the value. The encoder writes the stored bytes if they
//! hold the value. Otherwise it writes the shortest leaf for the value.

use super::super::Fields;
use super::super::numeric::Integer;
use super::super::writer::FieldWriter;
use crate::Result;

/// `LF_CHAR`: a signed 8-bit value.
const LF_CHAR: u16 = 0x8000;
/// `LF_SHORT`: a signed 16-bit value.
const LF_SHORT: u16 = 0x8001;
/// `LF_USHORT`: an unsigned 16-bit value.
const LF_USHORT: u16 = 0x8002;
/// `LF_LONG`: a signed 32-bit value.
const LF_LONG: u16 = 0x8003;
/// `LF_ULONG`: an unsigned 32-bit value.
const LF_ULONG: u16 = 0x8004;
/// `LF_QUADWORD`: a signed 64-bit value.
const LF_QUADWORD: u16 = 0x8009;
/// `LF_UQUADWORD`: an unsigned 64-bit value.
const LF_UQUADWORD: u16 = 0x800a;
/// `LF_OCTWORD`: a signed 128-bit value.
const LF_OCTWORD: u16 = 0x8017;
/// `LF_UOCTWORD`: an unsigned 128-bit value.
const LF_UOCTWORD: u16 = 0x8018;

/// Read an integer leaf and return the value and the stored bytes.
pub(super) fn read_integer<'a>(fields: &mut Fields<'a>) -> Result<(Integer, &'a [u8])> {
    let before = fields.rest();
    let value = fields.integer()?;
    let used = before.len() - fields.rest().len();
    Ok((value, &before[..used]))
}

/// Write `value`. Use `stored` if it is a leaf that holds exactly `value`.
pub(super) fn write_integer(
    out: &mut FieldWriter<'_>,
    value: Integer,
    stored: &[u8],
) -> Result<()> {
    if Integer::parse_prefix(stored).is_ok_and(|(parsed, rest)| parsed == value && rest.is_empty())
    {
        return out.bytes(stored);
    }
    match value {
        Integer::Unsigned(value) => {
            if let Ok(small) = u16::try_from(value)
                && small < 0x8000
            {
                out.u16(small)
            } else if let Ok(value) = u16::try_from(value) {
                out.u16(LF_USHORT)?;
                out.u16(value)
            } else if let Ok(value) = u32::try_from(value) {
                out.u16(LF_ULONG)?;
                out.u32(value)
            } else if let Ok(value) = u64::try_from(value) {
                out.u16(LF_UQUADWORD)?;
                out.u64(value)
            } else {
                out.u16(LF_UOCTWORD)?;
                out.bytes(&value.to_le_bytes())
            }
        }
        Integer::Signed(value) => {
            if let Ok(value) = i8::try_from(value) {
                out.u16(LF_CHAR)?;
                out.bytes(&value.to_le_bytes())
            } else if let Ok(value) = i16::try_from(value) {
                out.u16(LF_SHORT)?;
                out.bytes(&value.to_le_bytes())
            } else if let Ok(value) = i32::try_from(value) {
                out.u16(LF_LONG)?;
                out.i32(value)
            } else if let Ok(value) = i64::try_from(value) {
                out.u16(LF_QUADWORD)?;
                out.bytes(&value.to_le_bytes())
            } else {
                out.u16(LF_OCTWORD)?;
                out.bytes(&value.to_le_bytes())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;
    use crate::ErrorKind;

    /// Encode `value` without a stored form and decode it again.
    fn canonical(value: Integer) -> Vec<u8> {
        let mut out = Vec::new();
        write_integer(&mut FieldWriter::new(&mut out), value, &[]).unwrap();
        let (parsed, rest) = Integer::parse_prefix(&out).unwrap();
        assert_eq!(parsed, value);
        assert_eq!(rest, &[] as &[u8]);
        out
    }

    #[test]
    fn canonical_leaves_use_the_shortest_width() {
        assert_eq!(canonical(Integer::Unsigned(0x7fff)), [0xff, 0x7f]);
        assert_eq!(
            canonical(Integer::Unsigned(0x8000)),
            [0x02, 0x80, 0x00, 0x80]
        );
        assert_eq!(canonical(Integer::Unsigned(0x1_0000)).len(), 6);
        assert_eq!(canonical(Integer::Unsigned(1 << 32)).len(), 10);
        assert_eq!(canonical(Integer::Unsigned(u128::MAX)).len(), 18);
        assert_eq!(canonical(Integer::Signed(-1)), [0x00, 0x80, 0xff]);
        assert_eq!(canonical(Integer::Signed(-129)).len(), 4);
        assert_eq!(canonical(Integer::Signed(i128::from(i32::MIN))).len(), 6);
        assert_eq!(canonical(Integer::Signed(i128::from(i64::MIN))).len(), 10);
        assert_eq!(canonical(Integer::Signed(i128::MIN)).len(), 18);
    }

    #[test]
    fn stored_form_is_kept_when_it_holds_the_value() {
        let stored = [0x04, 0x80, 1, 0, 0, 0];
        let mut fields = Fields::new(&stored, ErrorKind::InvalidFormat, "test");
        let (value, bytes) = read_integer(&mut fields).unwrap();
        assert_eq!(value, Integer::Unsigned(1));
        let mut out = Vec::new();
        write_integer(&mut FieldWriter::new(&mut out), value, bytes).unwrap();
        assert_eq!(out, stored);
        out.clear();
        write_integer(&mut FieldWriter::new(&mut out), Integer::Unsigned(2), bytes).unwrap();
        assert_eq!(out, [2, 0]);
    }
}
