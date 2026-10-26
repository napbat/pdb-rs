//! The global reference region of a module stream.
//!
//! The region is an array of `u32` byte offsets into the symbol record
//! stream, one for each global symbol that the module refers to.
//! `Mod1::fCopyGlobalRef` in `PDB/dbi/mod.cpp` of `microsoft/microsoft-pdb`
//! adds the offset of each `S_PROCREF`, `S_LPROCREF`, `S_DATAREF`,
//! `S_TOKENREF` and shared data record that the module makes. A relink uses
//! the list to remove the references of the module from the global symbol
//! index.

use core::iter::FusedIterator;

use super::module_error;
use crate::native::symbols::{RecordOffsets, SymbolOffset};
use crate::{ErrorKind, Result};

/// The size of one reference.
pub(super) const REF_SIZE: usize = 4;

/// The decoded global references of a module stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct GlobalRefs<'a> {
    /// The serialized offsets.
    offsets: &'a [[u8; REF_SIZE]],
    /// The module stream index for errors.
    stream: u32,
    /// The offset of the first reference in the module stream.
    base: usize,
}

impl<'a> GlobalRefs<'a> {
    /// Split the region `bytes` at module stream offset `base` into
    /// references.
    pub(super) fn parse(bytes: &'a [u8], stream: u32, base: usize) -> Result<Self> {
        let (offsets, rest) = bytes.as_chunks::<REF_SIZE>();
        if !rest.is_empty() {
            return Err(module_error(
                stream,
                ErrorKind::InvalidFormat,
                base,
                "module global reference size is not a multiple of 4",
            ));
        }
        Ok(Self {
            offsets,
            stream,
            base,
        })
    }

    /// Return the number of references.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.offsets.len()
    }

    /// Return `true` if the module has no global references.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Return the reference at `position`, or `None` past the end.
    #[must_use]
    pub fn get(&self, position: usize) -> Option<SymbolOffset> {
        self.offsets.get(position).copied().map(parse)
    }

    /// Return an iterator over the references in stream order.
    #[must_use]
    pub fn iter(&self) -> GlobalRefsIter<'a> {
        GlobalRefsIter {
            offsets: self.offsets.iter(),
        }
    }

    /// Make sure that each reference identifies the start of a record in the
    /// symbol record stream.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] for the first reference that
    /// is outside the symbol record stream or inside a record. The error
    /// identifies the module stream and the logical offset of the reference.
    pub fn validate(&self, records: &RecordOffsets) -> Result<()> {
        for (position, offset) in self.iter().enumerate() {
            if records.contains(offset) {
                continue;
            }
            let inside =
                usize::try_from(offset.0).is_ok_and(|start| start < records.stream_length());
            let context = if inside {
                "module global reference is not at the start of a symbol record"
            } else {
                "module global reference is not inside the symbol record stream"
            };
            let location = self.base.saturating_add(position * REF_SIZE);
            return Err(module_error(
                self.stream,
                ErrorKind::InvalidReference,
                location,
                context,
            ));
        }
        Ok(())
    }
}

impl<'a> IntoIterator for GlobalRefs<'a> {
    type Item = SymbolOffset;
    type IntoIter = GlobalRefsIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &GlobalRefs<'a> {
    type Item = SymbolOffset;
    type IntoIter = GlobalRefsIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// An iterator over [`GlobalRefs`].
#[derive(Clone, Debug)]
pub struct GlobalRefsIter<'a> {
    /// The remaining serialized offsets.
    offsets: core::slice::Iter<'a, [u8; REF_SIZE]>,
}

impl Iterator for GlobalRefsIter<'_> {
    type Item = SymbolOffset;

    fn next(&mut self) -> Option<Self::Item> {
        self.offsets.next().copied().map(parse)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.offsets.size_hint()
    }
}

impl DoubleEndedIterator for GlobalRefsIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.offsets.next_back().copied().map(parse)
    }
}

impl ExactSizeIterator for GlobalRefsIter<'_> {}

impl FusedIterator for GlobalRefsIter<'_> {}

/// Decode one serialized reference.
const fn parse(bytes: [u8; REF_SIZE]) -> SymbolOffset {
    SymbolOffset(u32::from_le_bytes(bytes))
}
