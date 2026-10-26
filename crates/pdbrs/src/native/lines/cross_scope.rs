//! `DEBUG_S_CROSSSCOPEIMPORTS` and `DEBUG_S_CROSSSCOPEEXPORTS` subsections.
//!
//! The layouts are `CrossScopeReferences` and `LocalIdAndGlobalIdPair` in
//! `cvinfo.h`, and LLVM `DebugCrossImpSubsection` and
//! `DebugCrossExSubsection`. The item IDs are `CV_ItemId` values: type or
//! ID indexes in the scope of another module or of the PDB. They stay raw
//! `u32` values, because their index space depends on the producer
//! (`Mod1::fUpdateTiOrIdReferencedByNonSyms` in `PDB/dbi/mod.cpp`).
//! [`validate_cross_scope`] checks the references between modules.

mod validate;

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::{entry_list, invalid, read_u32, to_u32, too_large, truncated};
use crate::Result;
use crate::native::output::Output;

pub use validate::{
    CrossScopeContext, CrossScopeModules, CrossScopeRule, CrossScopeViolation, validate_cross_scope,
};

/// The size of one item ID.
const ID_SIZE: usize = 4;

/// The size of an import entry header: a module name offset and a count.
const IMPORT_HEADER_SIZE: usize = 8;

/// The size of one export entry.
const EXPORT_SIZE: usize = 8;

entry_list!(
    /// The item IDs of one [`CrossScopeImport`].
    CrossScopeIds,
    /// An iterator over [`CrossScopeIds`].
    CrossScopeIdsIter,
    u32,
    ID_SIZE,
    u32::from_le_bytes
);

/// The item IDs that a module uses from one other module
/// (`CrossScopeReferences`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CrossScopeImport<'a> {
    /// The byte offset of the name of the other module: the object file
    /// path or the PDB path of its compilation (`PdbIdScope`).
    ///
    /// In a PDB the offset is into the `/names` stream. In an object file it
    /// is into the `DEBUG_S_STRINGTABLE` subsection.
    pub module_name_offset: u32,
    /// The item IDs in the scope of the other module.
    pub ids: CrossScopeIds<'a>,
}

/// A decoded `DEBUG_S_CROSSSCOPEIMPORTS` subsection.
///
/// [`CrossScopeImports::decode`] validates all entries. The entry iterator
/// then cannot fail.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CrossScopeImports<'a> {
    /// The validated entries.
    data: &'a [u8],
    /// The number of entries.
    count: usize,
}

impl<'a> CrossScopeImports<'a> {
    /// Decode the data of a `DEBUG_S_CROSSSCOPEIMPORTS` subsection.
    ///
    /// The entries must fill the data exactly.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`](crate::ErrorKind::UnexpectedEof) if
    ///   an entry header or its item IDs are truncated.
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if
    ///   an item ID count does not fit in memory.
    ///
    /// The errors do not have a file offset or a stream location.
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        let mut rest = data;
        let mut count = 0usize;
        while !rest.is_empty() {
            rest = split_import(rest)?.1;
            count += 1;
        }
        Ok(Self { data, count })
    }

    /// Return the number of entries.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Return `true` if the subsection has no entries.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Return an iterator over the entries.
    #[must_use]
    pub const fn iter(&self) -> CrossScopeImportsIter<'a> {
        CrossScopeImportsIter {
            bytes: self.data,
            left: self.count,
        }
    }

    /// Append the subsection data to `out`.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`CrossScopeImport::encode`].
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        for entry in self {
            entry.encode(out)?;
        }
        Ok(())
    }
}

impl<'a> IntoIterator for CrossScopeImports<'a> {
    type Item = CrossScopeImport<'a>;
    type IntoIter = CrossScopeImportsIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &CrossScopeImports<'a> {
    type Item = CrossScopeImport<'a>;
    type IntoIter = CrossScopeImportsIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl CrossScopeImport<'_> {
    /// Append the entry to `out`.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`CrossScopeImport::encode_parts`].
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        write_import(self.module_name_offset, self.ids.iter(), out)
    }

    /// Append one entry of a `DEBUG_S_CROSSSCOPEIMPORTS` subsection to
    /// `out`: the module name offset, the count and the item IDs.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded)
    /// if the count does not fit in 32 bits or if the output cannot grow.
    pub fn encode_parts(module_name_offset: u32, ids: &[u32], out: &mut Vec<u8>) -> Result<()> {
        write_import(module_name_offset, ids.iter().copied(), out)
    }
}

/// Append an import entry with the item IDs of `ids` to `out`.
fn write_import(
    module_name_offset: u32,
    ids: impl ExactSizeIterator<Item = u32>,
    out: &mut Vec<u8>,
) -> Result<()> {
    Output::new(out).u32(module_name_offset)?;
    Output::new(out).u32(to_u32(
        ids.len(),
        "cross-scope import count does not fit in 32 bits",
    )?)?;
    for id in ids {
        Output::new(out).u32(id)?;
    }
    Ok(())
}

/// An iterator over the entries of a [`CrossScopeImports`] subsection.
#[derive(Clone, Debug)]
pub struct CrossScopeImportsIter<'a> {
    /// The remaining validated entries.
    bytes: &'a [u8],
    /// The number of remaining entries.
    left: usize,
}

impl<'a> Iterator for CrossScopeImportsIter<'a> {
    type Item = CrossScopeImport<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        // `CrossScopeImports::decode` validated the entries, so this cannot fail.
        let (entry, rest) = split_import(self.bytes).ok()?;
        self.bytes = rest;
        self.left = self.left.saturating_sub(1);
        Some(entry)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}

impl ExactSizeIterator for CrossScopeImportsIter<'_> {}

impl FusedIterator for CrossScopeImportsIter<'_> {}

/// Split one import entry from the start of `bytes`; return it and the rest.
fn split_import(bytes: &[u8]) -> Result<(CrossScopeImport<'_>, &[u8])> {
    let (Some(module_name_offset), Some(count)) = (read_u32(bytes, 0), read_u32(bytes, 4)) else {
        return Err(truncated("cross-scope import header is truncated"));
    };
    let size = usize::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(ID_SIZE))
        .ok_or_else(|| too_large("cross-scope import count is too large"))?;
    let ids = &bytes[IMPORT_HEADER_SIZE..];
    if ids.len() < size {
        return Err(truncated("cross-scope import IDs are truncated"));
    }
    let (ids, rest) = ids.split_at(size);
    let entry = CrossScopeImport {
        module_name_offset,
        ids: CrossScopeIds::new(ids),
    };
    Ok((entry, rest))
}

/// One exported item ID (`LocalIdAndGlobalIdPair`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CrossScopeExport {
    /// The item ID in the scope of the module.
    pub local_id: u32,
    /// The item ID in the scope of the PDB, if the scopes are different.
    pub global_id: u32,
}

impl CrossScopeExport {
    /// Create an entry from its fields.
    #[must_use]
    pub const fn new(local_id: u32, global_id: u32) -> Self {
        Self {
            local_id,
            global_id,
        }
    }

    /// Decode a serialized entry.
    const fn parse(bytes: [u8; EXPORT_SIZE]) -> Self {
        let [l0, l1, l2, l3, g0, g1, g2, g3] = bytes;
        Self {
            local_id: u32::from_le_bytes([l0, l1, l2, l3]),
            global_id: u32::from_le_bytes([g0, g1, g2, g3]),
        }
    }

    /// Append the 8-byte entry to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded)
    /// if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        Output::new(out).u32(self.local_id)?;
        Output::new(out).u32(self.global_id)
    }
}

entry_list!(
    /// A decoded `DEBUG_S_CROSSSCOPEEXPORTS` subsection.
    CrossScopeExports,
    /// An iterator over [`CrossScopeExports`].
    CrossScopeExportsIter,
    CrossScopeExport,
    EXPORT_SIZE,
    CrossScopeExport::parse
);

impl<'a> CrossScopeExports<'a> {
    /// Decode the data of a `DEBUG_S_CROSSSCOPEEXPORTS` subsection.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidFormat`](crate::ErrorKind::InvalidFormat)
    /// if the size is not a multiple of 8. The error does not have a file
    /// offset or a stream location.
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        if !data.len().is_multiple_of(EXPORT_SIZE) {
            return Err(invalid(
                "cross-scope export subsection size is not a multiple of 8",
            ));
        }
        Ok(Self::new(data))
    }

    /// Append the subsection data to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded)
    /// if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        for entry in self {
            entry.encode(out)?;
        }
        Ok(())
    }
}
