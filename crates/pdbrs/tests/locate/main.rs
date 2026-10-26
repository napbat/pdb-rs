//! Tests for the address and name queries of `native::locate` (#30).

mod pdb;
mod ranges;

/// Compress an unsigned annotation value (`CVCompressData` in `cvinfo.h`).
fn compress(value: u32) -> Vec<u8> {
    match value {
        0..0x80 => vec![u8::try_from(value).unwrap()],
        0x80..0x4000 => (u16::try_from(value).unwrap() | 0x8000)
            .to_be_bytes()
            .to_vec(),
        _ => (value | 0xc000_0000).to_be_bytes().to_vec(),
    }
}

/// Encode a signed annotation operand: bit 0 is the sign.
fn signed(value: i32) -> u32 {
    (value.unsigned_abs() << 1) | u32::from(value < 0)
}

/// Encode one binary annotation with its operands.
fn annotation(opcode: u32, operands: &[u32]) -> Vec<u8> {
    let mut bytes = compress(opcode);
    for &operand in operands {
        bytes.extend_from_slice(&compress(operand));
    }
    bytes
}

/// Serialize little-endian `u32` values.
fn u32s(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// The statement bit of a line entry.
const STATEMENT: u32 = 0x8000_0000;

/// Serialize the data of a `DEBUG_S_LINES` subsection in section 1.
///
/// Each block has a file checksum offset and `(offset, line)` entries.
fn lines_subsection(code_offset: u32, code_size: u32, blocks: &[(u32, &[(u32, u32)])]) -> Vec<u8> {
    let mut bytes = code_offset.to_le_bytes().to_vec();
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&code_size.to_le_bytes());
    for &(checksum, lines) in blocks {
        let count = u32::try_from(lines.len()).unwrap();
        bytes.extend_from_slice(&u32s(&[checksum, count, 12 + 8 * count]));
        for &(offset, line) in lines {
            bytes.extend_from_slice(&u32s(&[offset, line | STATEMENT]));
        }
    }
    bytes
}
