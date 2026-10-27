//! `DEBUG_S_FUNC_MDTOKEN_MAP` and `DEBUG_S_TYPE_MDTOKEN_MAP` subsections:
//! metadata token maps of C++/CLI code.
//!
//! `cvinfo.h` does not define the layout. The layout comes from
//! `DumpModTokenMap` in `cvdump/dumpsym7.cpp` and
//! `Mod1::fUpdateTiOrIdReferencedByNonSyms` in `PDB/dbi/mod.cpp` of
//! `microsoft/microsoft-pdb`: a `u32` entry count, then the entries of two
//! `u32` values each. The first value is the key. In a type map it is a type
//! index that `link.exe` maps to the PDB. The second value is a metadata
//! token if bit 31 is set, and an offset if it is clear.

use alloc::vec::Vec;

use super::{entry_list, invalid, read_u32, to_u32, truncated};
use crate::Result;
use crate::bytes::into_owned;
use crate::native::codeview::index::TypeIndex;
use crate::native::output::Output;

/// The size of the entry count.
const COUNT_SIZE: usize = 4;

/// The size of one entry.
const ENTRY_SIZE: usize = 8;

/// The bit of the value that marks a metadata token.
const TOKEN_BIT: u32 = 0x8000_0000;

/// The value of a [`TokenMapEntry`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TokenMapValue {
    /// A metadata token (bit 31 set). The value is bits 0 to 30.
    Token(u32),
    /// An offset (bit 31 clear). `cvdump` prints it as `off=`.
    Offset(u32),
}

impl TokenMapValue {
    /// Classify a raw value.
    #[must_use]
    pub const fn from_raw(value: u32) -> Self {
        if value & TOKEN_BIT == 0 {
            Self::Offset(value)
        } else {
            Self::Token(value & !TOKEN_BIT)
        }
    }

    /// Return the raw value, or `None` if the value does not fit in
    /// 31 bits.
    #[must_use]
    pub const fn raw(self) -> Option<u32> {
        match self {
            Self::Token(value) if value & TOKEN_BIT == 0 => Some(value | TOKEN_BIT),
            Self::Offset(value) if value & TOKEN_BIT == 0 => Some(value),
            Self::Token(_) | Self::Offset(_) => None,
        }
    }
}

impl Default for TokenMapValue {
    /// Return `Offset(0)`, the value of the raw value 0.
    fn default() -> Self {
        Self::Offset(0)
    }
}

/// One entry of a [`TokenMap`].
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct TokenMapEntry {
    /// The key. In a `DEBUG_S_TYPE_MDTOKEN_MAP` subsection it is a type
    /// index ([`TokenMapEntry::type_index`]).
    pub key: u32,
    /// The value.
    pub value: TokenMapValue,
}

impl TokenMapEntry {
    /// Return the key as a type index, for an entry of a
    /// `DEBUG_S_TYPE_MDTOKEN_MAP` subsection.
    #[must_use]
    pub const fn type_index(&self) -> TypeIndex {
        TypeIndex(self.key)
    }

    /// Decode a serialized entry.
    const fn parse(bytes: [u8; ENTRY_SIZE]) -> Self {
        let [k0, k1, k2, k3, v0, v1, v2, v3] = bytes;
        Self {
            key: u32::from_le_bytes([k0, k1, k2, k3]),
            value: TokenMapValue::from_raw(u32::from_le_bytes([v0, v1, v2, v3])),
        }
    }

    /// Append the 8-byte entry to `out`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if
    ///   the value does not fit in 31 bits.
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if
    ///   the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        let value = self
            .value
            .raw()
            .ok_or_else(|| invalid("token map value does not fit in 31 bits"))?;
        Output::new(out).u32(self.key)?;
        Output::new(out).u32(value)
    }
}

entry_list!(
    /// The entries of a [`TokenMap`].
    TokenMapEntries,
    /// An iterator over [`TokenMapEntries`].
    TokenMapEntriesIter,
    TokenMapEntry,
    ENTRY_SIZE,
    TokenMapEntry::parse
);

/// A decoded `DEBUG_S_FUNC_MDTOKEN_MAP` or `DEBUG_S_TYPE_MDTOKEN_MAP`
/// subsection.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TokenMap<'a> {
    /// The entries.
    entries: TokenMapEntries<'a>,
}

into_owned!(struct TokenMap { entries; });

impl<'a> TokenMap<'a> {
    /// Parse the data of a token map subsection.
    ///
    /// The count and the entries must fill the data exactly.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`](crate::ErrorKind::UnexpectedEof) if
    ///   the count or the entries are truncated.
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if
    ///   bytes follow the entries.
    ///
    /// The errors do not have a file offset or a stream location.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let count = read_u32(data, 0).ok_or_else(|| truncated("token map count is truncated"))?;
        let entries = &data[COUNT_SIZE..];
        let size = usize::try_from(count)
            .ok()
            .and_then(|count| count.checked_mul(ENTRY_SIZE))
            .filter(|&size| size <= entries.len())
            .ok_or_else(|| truncated("token map entries are truncated"))?;
        if size != entries.len() {
            return Err(invalid("token map has bytes after its entries"));
        }
        Ok(Self {
            entries: TokenMapEntries::new(entries),
        })
    }

    /// Return the entries.
    #[must_use]
    pub const fn entries(&self) -> &TokenMapEntries<'a> {
        &self.entries
    }

    /// Append the subsection data to `out`: the count and the entries.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`TokenMap::encode_parts`].
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        write_map(self.entries.iter(), out)
    }

    /// Append the data of a token map subsection with `entries` to `out`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat) if a
    ///   value does not fit in 31 bits.
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if
    ///   the count does not fit in 32 bits or if the output cannot grow.
    pub fn encode_parts(entries: &[TokenMapEntry], out: &mut Vec<u8>) -> Result<()> {
        write_map(entries.iter().copied(), out)
    }
}

/// Append the count and the entries of `entries` to `out`.
fn write_map(
    entries: impl ExactSizeIterator<Item = TokenMapEntry>,
    out: &mut Vec<u8>,
) -> Result<()> {
    Output::new(out).u32(to_u32(
        entries.len(),
        "token map count does not fit in 32 bits",
    )?)?;
    for entry in entries {
        entry.encode(out)?;
    }
    Ok(())
}
