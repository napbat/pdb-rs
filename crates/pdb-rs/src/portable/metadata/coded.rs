//! Coded indexes of the Portable PDB debug tables (ECMA-335 II.24.2.6).
//!
//! A coded index stores a table tag in its low bits and a row number in the other bits. It
//! uses 2 bytes if the largest table of its list has fewer than `2^(16 - tag bits)` rows,
//! else 4 bytes.

use super::ids::{MetadataToken, TableId};

/// A coded index kind that a debug table column uses.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum CodedIndex {
    /// The `Parent` column of `CustomDebugInformation`: 5 tag bits and 27 tables.
    HasCustomDebugInformation,
}

/// The tables of `HasCustomDebugInformation` in tag order, from the Portable PDB
/// specification.
const HAS_CUSTOM_DEBUG_INFORMATION: [TableId; 27] = [
    TableId::METHOD_DEF,
    TableId::FIELD,
    TableId::TYPE_REF,
    TableId::TYPE_DEF,
    TableId::PARAM,
    TableId::INTERFACE_IMPL,
    TableId::MEMBER_REF,
    TableId::MODULE,
    TableId::DECL_SECURITY,
    TableId::PROPERTY,
    TableId::EVENT,
    TableId::STAND_ALONE_SIG,
    TableId::MODULE_REF,
    TableId::TYPE_SPEC,
    TableId::ASSEMBLY,
    TableId::ASSEMBLY_REF,
    TableId::FILE,
    TableId::EXPORTED_TYPE,
    TableId::MANIFEST_RESOURCE,
    TableId::GENERIC_PARAM,
    TableId::GENERIC_PARAM_CONSTRAINT,
    TableId::METHOD_SPEC,
    TableId::DOCUMENT,
    TableId::LOCAL_SCOPE,
    TableId::LOCAL_VARIABLE,
    TableId::LOCAL_CONSTANT,
    TableId::IMPORT_SCOPE,
];

impl CodedIndex {
    /// Return the tables of the coded index in tag order.
    #[must_use]
    pub const fn tables(self) -> &'static [TableId] {
        match self {
            Self::HasCustomDebugInformation => &HAS_CUSTOM_DEBUG_INFORMATION,
        }
    }

    /// Return the number of tag bits.
    #[must_use]
    pub const fn tag_bits(self) -> u32 {
        match self {
            Self::HasCustomDebugInformation => 5,
        }
    }

    /// Return the width in bytes (2 or 4) for the given row counts, indexed by table number.
    #[must_use]
    pub fn width(self, row_counts: &[u32; 64]) -> u8 {
        let limit = 1_u32 << (16 - self.tag_bits());
        let largest = self
            .tables()
            .iter()
            .map(|table| row_counts[table.index()])
            .max()
            .unwrap_or(0);
        if largest < limit { 2 } else { 4 }
    }

    /// Decode a serialized value into a metadata token.
    ///
    /// Return `None` if the tag is not in the list or the row number does not fit in a
    /// token. The caller checks the row number against the row count.
    #[must_use]
    pub fn decode(self, value: u32) -> Option<MetadataToken> {
        let tag = value & ((1 << self.tag_bits()) - 1);
        let table = *self.tables().get(usize::try_from(tag).ok()?)?;
        MetadataToken::new(table, value >> self.tag_bits())
    }

    /// Encode a metadata token. Return `None` if the table is not in the list or the value
    /// does not fit in 32 bits.
    #[must_use]
    pub fn encode(self, token: MetadataToken) -> Option<u32> {
        let tag = self
            .tables()
            .iter()
            .position(|&table| Some(table) == token.table())?;
        let row = token.row().checked_mul(1 << self.tag_bits())?;
        Some(row | u32::try_from(tag).ok()?)
    }
}
