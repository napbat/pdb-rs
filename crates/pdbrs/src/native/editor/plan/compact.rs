//! The compaction of the TPI and IPI index spaces after a removal.
//!
//! A removal leaves a tombstone in the model. The plan gives each record that
//! is not removed the next output index, in model order ([`map`]). It writes
//! the TPI and IPI records with the output indices ([`tables`]). Then it
//! reads each other output stream that a decoder of this crate decodes, and
//! writes the output index into each reference ([`outputs`]):
//!
//! - [`records`]: the fields of the TPI and IPI records;
//! - [`symbols`]: the symbol records of the symbol record stream and of the
//!   module streams;
//! - [`lines`]: the C13 subsections of the module streams;
//! - [`streams`]: the type map streams and `/UDTSRCLINEUNDONE`.
//!
//! An index field keeps its width. Thus no record and no stream changes its
//! size, and no offset moves. [`bytes`] writes the changed bytes of a record
//! into its source bytes.
//!
//! Other changes of the plan can reuse the walk of [`outputs`] and the patch
//! function of the plan for other fixed-width fields.

mod bytes;
mod lines;
pub(super) mod map;
mod outputs;
mod records;
mod streams;
mod symbols;
pub(super) mod tables;

pub(super) use outputs::rewrite;

use super::super::PdbEditor;
use super::super::types::{ListRecord, RecordList};
use crate::Result;
use map::{IndexMaps, SpaceMap};

/// Return the map of `list`, or `None` if it has no removed record.
fn space_map<T: ListRecord>(list: &RecordList<T>) -> Result<Option<SpaceMap>> {
    if !list.has_removals() {
        return Ok(None);
    }
    SpaceMap::new(list.entries.iter().map(|entry| entry.removed)).map(Some)
}

/// Return the maps of the model. A space without a removed record does not
/// compact.
pub(super) fn maps<S>(editor: &PdbEditor<'_, S>) -> Result<IndexMaps> {
    let types = match &editor.types {
        Some(types) => space_map(&types.list)?,
        None => None,
    };
    let ids = match &editor.ids {
        Some(ids) => space_map(&ids.list)?,
        None => None,
    };
    Ok(IndexMaps::new(types, ids))
}
