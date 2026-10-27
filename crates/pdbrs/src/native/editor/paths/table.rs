//! Build a string table again without the old paths (decision 2 of #46).
//!
//! The new table gets the names of the source table in offset order, with
//! the insertion rules of `NMT` ([`StringTableBuilder::insert`]). A name that
//! the remap changes gets its new value at its place; a new value that is
//! already present is not added again. An unchanged name stays, also if no
//! field names it. A changed name that a field of another role (a frame
//! program or a hash adjuster name) also names stays as well.

use alloc::collections::{BTreeMap, BTreeSet};

use super::{NameRoles, PathMap, invalid};
use crate::Result;
use crate::native::codeview::index::NameOffset;
use crate::native::names::StringTableBuilder;

/// The new offset of each old offset.
pub(super) type Moves = BTreeMap<u32, u32>;

/// Return `true` if the function got the name at `offset`: a path field
/// names it, or no field of another role names it.
pub(super) fn applies(roles: &NameRoles, offset: u32) -> bool {
    roles.paths.contains(&offset) || !roles.other.contains(&offset)
}

/// Add `name` to `table` and return its offset. The empty name at offset 0
/// stays at offset 0: each table starts with that NUL byte.
fn insert(table: &mut StringTableBuilder, offset: u32, name: &[u8]) -> Result<u32> {
    if offset == 0 && name.is_empty() {
        return Ok(0);
    }
    Ok(table.insert(name)?.0)
}

/// Build `source` again if `map` changes a name that the function got.
/// Return the new table and the new offsets for the fields of each role:
/// the path fields and the other fields. Return `None` if no such name
/// changes.
///
/// `roles` gives the offsets that the fields hold.
pub(super) fn rebuild(
    source: &StringTableBuilder,
    map: &PathMap,
    roles: &NameRoles,
) -> Result<Option<(StringTableBuilder, Moves, Moves)>> {
    let mut offsets: BTreeSet<u32> = source.name_offsets().map(|offset| offset.0).collect();
    offsets.extend(roles.paths.iter().copied());
    offsets.extend(roles.other.iter().copied());
    let name = |offset: u32| {
        source
            .get(NameOffset(offset))
            .ok_or_else(|| invalid("a /names offset does not name a string of the table"))
    };
    let mut changed = false;
    for &offset in &offsets {
        if applies(roles, offset) && map.contains_key(name(offset)?) {
            changed = true;
            break;
        }
    }
    if !changed {
        return Ok(None);
    }
    let mut table = StringTableBuilder::new(source.hash_version())?;
    let mut paths = Moves::new();
    let mut other = Moves::new();
    for offset in offsets {
        let old = name(offset)?;
        if let Some(new) = map.get(old).filter(|_| applies(roles, offset)) {
            paths.insert(offset, insert(&mut table, offset, new)?);
            if roles.other.contains(&offset) {
                other.insert(offset, insert(&mut table, offset, old)?);
            }
        } else {
            let moved = insert(&mut table, offset, old)?;
            paths.insert(offset, moved);
            other.insert(offset, moved);
        }
    }
    Ok(Some((table, paths, other)))
}
