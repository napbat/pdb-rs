//! The index maps of a compaction: the output index of each model index.

use alloc::vec::Vec;
use core::cell::Cell;

use crate::native::IndexSpace;
use crate::native::codeview::index::{FIRST_RECORD_INDEX, IdIndex, TypeIndex};
use crate::native::output::no_memory;
use crate::{Error, ErrorKind, Result};

/// The output index of a model index of a TPI or IPI record.
///
/// A rewriter calls these functions for each index field of a record. An
/// index below `0x1000` (a primitive type, or "no type" and "no ID") stays.
pub(in crate::native::editor::plan) trait IndexRemap {
    /// Return the output index of the type index `index`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] with the space
    /// [`IndexSpace::TypeIndex`] and without a stream location if the record
    /// is removed, or if the index is at or after the end of the records.
    fn type_index(&self, index: TypeIndex) -> Result<TypeIndex>;

    /// Return the output index of the ID index `index`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidReference`] with the space
    /// [`IndexSpace::IdIndex`] and without a stream location if the record is
    /// removed, or if the index is at or after the end of the records.
    fn id_index(&self, index: IdIndex) -> Result<IdIndex>;
}

/// The value of a removed record in [`SpaceMap::output`].
const REMOVED: u32 = u32::MAX;

/// The output indices of one index space.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::native::editor::plan) struct SpaceMap {
    /// The output index of each model record, from `0x1000`, or [`REMOVED`].
    output: Vec<u32>,
    /// The index after the last output record.
    output_end: u32,
}

/// The result of a lookup in a [`SpaceMap`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::native::editor::plan) enum Lookup {
    /// The output index.
    Index(u32),
    /// The record is removed.
    Removed,
    /// The index is at or after the end of the model records.
    Outside,
}

impl SpaceMap {
    /// Make the map of a space whose records start at `0x1000`. `removed`
    /// gives `true` for each removed record, in index order.
    pub(in crate::native::editor::plan) fn new(
        removed: impl ExactSizeIterator<Item = bool>,
    ) -> Result<Self> {
        let mut output = Vec::new();
        output
            .try_reserve_exact(removed.len())
            .map_err(|_| no_memory())?;
        let mut next = FIRST_RECORD_INDEX;
        for removed in removed {
            if removed {
                output.push(REMOVED);
            } else {
                output.push(next);
                next = next.checked_add(1).ok_or_else(|| {
                    Error::new(ErrorKind::LimitExceeded, None)
                        .with_context("record index does not fit in 32 bits")
                })?;
            }
        }
        Ok(Self {
            output,
            output_end: next,
        })
    }

    /// Return the output index of the model index `index`.
    pub(in crate::native::editor::plan) fn lookup(&self, index: u32) -> Lookup {
        let Some(position) = index.checked_sub(FIRST_RECORD_INDEX) else {
            return Lookup::Index(index);
        };
        match self.output.get(position as usize) {
            None => Lookup::Outside,
            Some(&REMOVED) => Lookup::Removed,
            Some(&output) => Lookup::Index(output),
        }
    }

    /// Return `true` if the model record with `index` is removed.
    pub(in crate::native::editor::plan) fn is_removed(&self, index: u32) -> bool {
        self.lookup(index) == Lookup::Removed
    }

    /// Return the index after the last output record.
    pub(in crate::native::editor::plan) const fn output_end(&self) -> u32 {
        self.output_end
    }
}

/// The maps of a compaction, with the record of the spaces whose values
/// changed since the last [`IndexMaps::take_changes`].
#[derive(Debug, Default)]
pub(in crate::native::editor::plan) struct IndexMaps {
    /// The map of the TPI space, or `None` if the space does not compact.
    pub(in crate::native::editor::plan) types: Option<SpaceMap>,
    /// The map of the IPI space, or `None` if the space does not compact.
    pub(in crate::native::editor::plan) ids: Option<SpaceMap>,
    /// `true` if a type index got a new value.
    types_changed: Cell<bool>,
    /// `true` if an ID index got a new value.
    ids_changed: Cell<bool>,
}

impl IndexMaps {
    /// Make the maps of a compaction.
    pub(in crate::native::editor::plan) const fn new(
        types: Option<SpaceMap>,
        ids: Option<SpaceMap>,
    ) -> Self {
        Self {
            types,
            ids,
            types_changed: Cell::new(false),
            ids_changed: Cell::new(false),
        }
    }

    /// Return `true` if no space compacts.
    pub(in crate::native::editor::plan) const fn is_identity(&self) -> bool {
        self.types.is_none() && self.ids.is_none()
    }

    /// Return the spaces whose values changed since the last call, as
    /// `(types, ids)`, and clear the record.
    pub(in crate::native::editor::plan) fn take_changes(&self) -> (bool, bool) {
        (self.types_changed.take(), self.ids_changed.take())
    }

    /// Return the space of the first change of [`IndexMaps::take_changes`]:
    /// the type space if type indices changed, else the ID space.
    pub(in crate::native::editor::plan) fn changed_space(&self) -> Option<IndexSpace> {
        match self.take_changes() {
            (true, _) => Some(IndexSpace::TypeIndex),
            (false, true) => Some(IndexSpace::IdIndex),
            (false, false) => None,
        }
    }
}

/// Map one index through `map`.
fn remap(
    map: Option<&SpaceMap>,
    index: u32,
    space: IndexSpace,
    removed: &'static str,
    outside: &'static str,
) -> Result<u32> {
    let Some(map) = map else {
        return Ok(index);
    };
    let context = match map.lookup(index) {
        Lookup::Index(output) => return Ok(output),
        Lookup::Removed => removed,
        Lookup::Outside => outside,
    };
    Err(Error::new(ErrorKind::InvalidReference, None)
        .with_context(context)
        .with_index_space(space))
}

/// Return the output index of the type index `index` through `map`, or
/// `index` if the TPI space does not compact.
///
/// # Errors
///
/// See [`IndexRemap::type_index`].
pub(in crate::native::editor::plan) fn resolve_type(
    map: Option<&SpaceMap>,
    index: TypeIndex,
) -> Result<TypeIndex> {
    remap(
        map,
        index.0,
        IndexSpace::TypeIndex,
        "a reference names a removed type record",
        "a type index is at or after the end of the type records",
    )
    .map(TypeIndex)
}

/// Return the output index of the ID index `index` through `map`, or
/// `index` if the IPI space does not compact.
///
/// # Errors
///
/// See [`IndexRemap::id_index`].
pub(in crate::native::editor::plan) fn resolve_id(
    map: Option<&SpaceMap>,
    index: IdIndex,
) -> Result<IdIndex> {
    remap(
        map,
        index.0,
        IndexSpace::IdIndex,
        "a reference names a removed ID record",
        "an ID index is at or after the end of the ID records",
    )
    .map(IdIndex)
}

impl IndexRemap for IndexMaps {
    fn type_index(&self, index: TypeIndex) -> Result<TypeIndex> {
        let output = resolve_type(self.types.as_ref(), index)?;
        if output != index {
            self.types_changed.set(true);
        }
        Ok(output)
    }

    fn id_index(&self, index: IdIndex) -> Result<IdIndex> {
        let output = resolve_id(self.ids.as_ref(), index)?;
        if output != index {
            self.ids_changed.set(true);
        }
        Ok(output)
    }
}
