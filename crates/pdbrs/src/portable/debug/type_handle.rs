//! `TypeDefOrRefOrSpecEncoded` values (ECMA-335 II.23.2.8).
//!
//! The value is a compressed unsigned integer. Its low 2 bits select the table (0 `TypeDef`,
//! 1 `TypeRef`, 2 `TypeSpec`) and the other bits hold the row number. The tables are external
//! tables of the assembly; the validation checks the rows against the `#Pdb` row counts.

use crate::Result;
use crate::portable::custom::blob::{Reader, Writer, invalid, too_large};
use crate::portable::metadata::{MetadataToken, TableId};

/// The tables of the encoding in tag order.
const TABLES: [TableId; 3] = [TableId::TYPE_DEF, TableId::TYPE_REF, TableId::TYPE_SPEC];

/// Read a `TypeDefOrRefOrSpecEncoded` value and return its token.
///
/// Return [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) for tag 3 and for row 0.
pub(super) fn read(reader: &mut Reader<'_>) -> Result<MetadataToken> {
    let value = reader.unsigned()?;
    let table = TABLES
        .get((value & 3) as usize)
        .ok_or_else(|| invalid("type handle has the invalid tag 3"))?;
    let row = value >> 2;
    if row == 0 {
        return Err(invalid("type handle has the nil row"));
    }
    // The row has at most 27 bits, so it fits in a token.
    MetadataToken::new(*table, row).ok_or_else(|| invalid("type handle row is too large"))
}

/// Write a token as a `TypeDefOrRefOrSpecEncoded` value.
///
/// Return [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if the token is not a
/// non-nil `TypeDef`, `TypeRef` or `TypeSpec` token, and
/// [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if the row does not fit.
pub(super) fn write(writer: &mut Writer, token: MetadataToken) -> Result<()> {
    let tag = TABLES
        .iter()
        .position(|table| table.number() == token.table())
        .ok_or_else(|| invalid("type handle is not a TypeDef, TypeRef or TypeSpec token"))?;
    if token.row() == 0 {
        return Err(invalid("type handle has the nil row"));
    }
    let value = token
        .row()
        .checked_mul(4)
        .ok_or_else(|| too_large("type handle row does not fit"))?;
    // `tag` is at most 2.
    writer.unsigned(value | u32::try_from(tag).unwrap_or(0))
}
