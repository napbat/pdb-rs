//! Write a byte stream, then inspect it through positional access.

use pdbrs::io::{Cursor, ReadAt, SeekReader, Write, WriteLe};
use pdbrs::{ContainerKind, probe};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut storage = [0; 8];
    let mut output = Cursor::new(storage.as_mut_slice());
    output.write_all(b"BSJB")?;
    output.write_u32_le(0x1234_5678)?;

    let reader = SeekReader::new(Cursor::new(storage.as_slice()))?;
    let kind = probe(&reader)?;
    let value = reader.read_u32_at(4)?;
    assert_eq!(kind, ContainerKind::CliMetadata);
    assert_eq!(value, 0x1234_5678);
    println!("{kind:?}: positional value {value:#010x}; signature identification only");
    Ok(())
}
