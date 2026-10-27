//! [`PdbEditor::remap_source_paths`].
//!
//! The task has two steps. [`stage`] reads the parts that hold paths into
//! local values (a copy of each part that the model already has), calls the
//! function, checks every new path and changes the local values. Then
//! [`PdbEditor::store_paths`] moves the local values into the model. Only the
//! first step can fail, so an error leaves the model unchanged.
//!
//! - [`load`] reads the parts.
//! - [`offsets`] walks every `/names` offset field of the parts.
//! - [`table`] builds a string table again without the old paths.
//! - [`values`] collects and changes the path fields.
//! - [`named`] changes the `srcsrv`, `/LinkInfo` and `/src/` streams.

mod load;
mod named;
mod offsets;
mod table;
mod values;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use super::named::Content;
use super::types::RecordList;
use super::{
    DebugInfo, DebugStreamList, IdList, ModuleSlot, NamedStreams, Names, PdbEditor, TypeList,
    UnnamedStreams, missing,
};
use crate::io::ReadAt;
use crate::native::Producer;
use crate::native::codeview::index::NameOffset;
use crate::native::codeview::type_record::TypeLeaf;
use crate::native::dbi::DebugStream;
use crate::native::msf::StreamId;
use crate::native::names::StringTableBuilder;
use crate::native::output::no_memory;
use crate::native::types::HashAdjuster;
use crate::{Error, ErrorKind, Result};

/// The new value of each path that the function changed.
type PathMap = BTreeMap<Vec<u8>, Vec<u8>>;

/// A stream with type and ID records that an `S_MOD_TYPEREF` record names.
#[derive(Clone, Debug)]
struct TypeRecordStream {
    /// The stream ID in the model.
    id: StreamId,
    /// The record bytes.
    bytes: Vec<u8>,
    /// `true` if the model holds the stream as a stream without a name; else
    /// it is a source stream.
    in_model: bool,
    /// `true` if the task changed the bytes.
    changed: bool,
}

/// The `/src/headerblock` stream of the parts.
#[derive(Clone, Debug)]
struct HeaderBlock {
    /// The position of the stream in the named stream entries.
    entry: usize,
    /// The stream ID for the error locations.
    stream: StreamId,
    /// The stream bytes.
    bytes: Vec<u8>,
    /// The producer whose insertion rules a new table follows.
    producer: Producer,
    /// `true` if the task changed the bytes.
    changed: bool,
}

/// The new FPO stream of the parts.
#[derive(Clone, Debug)]
struct NewFpo {
    /// The stream ID for the error locations.
    stream: StreamId,
    /// The stream bytes.
    bytes: Vec<u8>,
    /// `true` if the task changed the bytes.
    changed: bool,
}

/// The parts of the model that hold paths or `/names` offsets, as local
/// values.
struct Parts {
    /// The named streams.
    named: NamedStreams,
    /// The modules, all decoded.
    slots: Vec<ModuleSlot>,
    /// The IPI records, if the model has an IPI stream.
    ids: Option<IdList>,
    /// The TPI records, if the task needs to change the TPI hash adjusters.
    types: Option<TypeList>,
    /// The TPI hash adjusters.
    type_adjusters: Vec<HashAdjuster>,
    /// The `/names` stream, if the model has one.
    names: Option<StringTableBuilder>,
    /// The DBI values, if the model has a DBI stream.
    dbi: Option<DebugInfo>,
    /// The new FPO stream, if the model has one.
    fpo: Option<NewFpo>,
    /// The `/src/headerblock` stream, if the model has one with bytes.
    header_block: Option<HeaderBlock>,
    /// The streams that the `S_MOD_TYPEREF` records name.
    module_types: Vec<TypeRecordStream>,
    /// The virtual file names of the injected source files, before the
    /// remap.
    virtual_names: Vec<Vec<u8>>,
    /// The `srcsrv` stream, if the model has one.
    source_server: Option<NamedBytes>,
    /// The `/LinkInfo` stream, if the model has one.
    link_info: Option<NamedBytes>,
}

/// The bytes of a named stream of the parts.
#[derive(Clone, Debug)]
struct NamedBytes {
    /// The position of the stream in the named stream entries.
    entry: usize,
    /// The stream ID for the error locations.
    stream: StreamId,
    /// The stream bytes.
    bytes: Vec<u8>,
}

/// The roles of the `/names` offsets of the parts.
#[derive(Debug, Default)]
struct NameRoles {
    /// The offsets of path fields.
    paths: BTreeSet<u32>,
    /// The offsets of other fields: frame programs and hash adjuster names.
    other: BTreeSet<u32>,
}

/// Push a value with a checked allocation.
fn push<T>(list: &mut Vec<T>, value: T) -> Result<()> {
    list.try_reserve(1).map_err(|_| no_memory())?;
    list.push(value);
    Ok(())
}

/// Return an [`ErrorKind::InvalidFormat`] error with `context`.
fn invalid(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, None).with_context(context)
}

/// Call `remap` once for each path of `values`, in byte order. Check each
/// new path and return the changed paths.
fn new_paths(
    values: &BTreeSet<Vec<u8>>,
    remap: &mut impl FnMut(&[u8]) -> Option<Vec<u8>>,
) -> Result<PathMap> {
    let mut map = PathMap::new();
    for value in values {
        let Some(new) = remap(value) else {
            continue;
        };
        if new.is_empty() {
            return Err(invalid("a new path is empty"));
        }
        if new.contains(&0) {
            return Err(invalid("a new path contains a NUL byte"));
        }
        if new != *value {
            map.insert(value.clone(), new);
        }
    }
    Ok(map)
}

/// Read and change everything that the remap needs. Change nothing in the
/// model. Return `None` if the function changes no path.
fn stage<S: ReadAt>(
    editor: &PdbEditor<'_, S>,
    remap: &mut impl FnMut(&[u8]) -> Option<Vec<u8>>,
) -> Result<Option<(Staged, usize)>> {
    let mut parts = load::load(editor)?;
    let mut roles = NameRoles::default();
    offsets::map_name_offsets(&mut offsets::fields(&mut parts), &mut |role, offset| {
        let set = match role {
            offsets::Role::Path => &mut roles.paths,
            offsets::Role::Other => &mut roles.other,
        };
        set.insert(offset.0);
        Ok(offset)
    })?;
    let values = values::collect(&parts, &roles)?;
    let map = new_paths(&values, remap)?;
    if map.is_empty() {
        return Ok(None);
    }
    if let Some(names) = &parts.names
        && let Some((table, paths, other)) = table::rebuild(names, &map, &roles)?
    {
        load::load_types(editor, &mut parts)?;
        offsets::map_name_offsets(&mut offsets::fields(&mut parts), &mut |role, offset| {
            let moved = match role {
                offsets::Role::Path => paths.get(&offset.0),
                offsets::Role::Other => other.get(&offset.0),
            };
            moved
                .map(|&moved| NameOffset(moved))
                .ok_or_else(|| invalid("a /names offset does not name a string of the table"))
        })?;
        parts.names = Some(table);
    }
    values::apply(&mut parts, &map)?;
    named::apply(&mut parts, &map)?;
    let count = map.len();
    Ok(Some((finish(editor, parts)?, count)))
}

impl<S: ReadAt> PdbEditor<'_, S> {
    /// Give the source paths of the PDB new values with the function `remap`,
    /// and return the number of paths that get a new value.
    ///
    /// The task calls `remap` once for each distinct path, in byte order.
    /// `remap` returns the new path, or `None` to keep the path. The task
    /// compares paths as bytes: two paths that differ only in case are two
    /// paths. Each field with the path gets the new path:
    ///
    /// - the `/names` strings of the file checksums, the cross-scope import
    ///   module names, the `S_FILESTATIC` records, the `LF_UDT_MOD_SRC_LINE`
    ///   records and the injected source files, and the `/names` strings that
    ///   no decoded field names;
    /// - the DBI source file names, the module names and object file names of
    ///   the DBI module records, the EC names and the names and paths of the
    ///   type server map;
    /// - the path of `S_OBJNAME` and each string of `S_ENVBLOCK`;
    /// - the string of each `LF_STRING_ID` record of the IPI stream and of
    ///   the streams that `S_MOD_TYPEREF` records name;
    /// - the names of the `/src/files/` streams of the injected source
    ///   files: `/src/files/` and the new virtual file name;
    /// - the first field of each line of the `source files` section of
    ///   `srcsrv`, and the current directory of `/LinkInfo`.
    ///
    /// The fields with a string give the whole string to `remap`. The task
    /// cannot tell a path from other text in these fields, so `remap` also
    /// gets namespace names, command lines and the pieces of long
    /// `LF_STRING_ID` strings. Return `None` for them. The task does not
    /// search for a path inside a string, and it does not change the names
    /// of types and symbols, the `/LinkInfo` command, `sourcelink` and the
    /// contents of injected files.
    ///
    /// If a changed path is a `/names` string, the task builds `/names` again
    /// without the old path and writes the new offset into each field with a
    /// `/names` offset. The EC name table gets the same rule. A function that
    /// keeps every path changes nothing, so the plan copies every stream.
    ///
    /// The task decodes every module, the IPI records and the other parts
    /// that hold paths. After a change, the plan rejects data without a
    /// decoder that can hold a path or a `/names` offset (see
    /// [`PdbEditor`](PdbEditor#rejected-edits)). The task does not change
    /// symbols or addresses, so
    /// [`IdentityPolicy::Preserve`](crate::native::IdentityPolicy::Preserve)
    /// keeps the match with the image.
    ///
    /// ```no_run
    /// use pdbrs::native::{EditOptions, IdentityPolicy, Pdb};
    ///
    /// # #[cfg(feature = "std")]
    /// # fn main() -> pdbrs::Result<()> {
    /// let pdb = Pdb::open("app.pdb")?;
    /// let mut editor = pdb.edit(IdentityPolicy::Preserve);
    /// // Replace the build directory with `/_/`.
    /// let changed = editor.remap_source_paths(|path| {
    ///     path.strip_prefix(br"C:\build\".as_slice())
    ///         .map(|rest| [b"/_/".as_slice(), rest].concat())
    /// })?;
    /// editor.plan(EditOptions::default())?.write_file("app.remapped.pdb")?;
    /// # let _ = changed;
    /// # Ok(())
    /// # }
    /// # #[cfg(not(feature = "std"))]
    /// # fn main() {}
    /// ```
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if a new path is empty or contains a
    ///   NUL byte, if two injected files get the same virtual file name, or
    ///   if the new `srcsrv` text does not give each new path as the first
    ///   field of its line.
    /// - [`ErrorKind::LimitExceeded`] if a record does not encode with its
    ///   new path: the record is longer than its `u16` length field allows.
    /// - [`ErrorKind::MissingStream`] if the editor has no source and the
    ///   model does not have a part that the task reads.
    /// - The errors of the stream readers and of the decoders of the parts.
    ///
    /// On an error, the model does not change.
    pub fn remap_source_paths<F>(&mut self, mut remap: F) -> Result<usize>
    where
        F: FnMut(&[u8]) -> Option<Vec<u8>>,
    {
        let Some((parts, count)) = stage(self, &mut remap)? else {
            return Ok(0);
        };
        self.store_paths(parts);
        Ok(count)
    }

    /// Store the parts of a path remap in the model.
    fn store_paths(&mut self, staged: Staged) {
        let Staged {
            named,
            slots,
            ids,
            types,
            names,
            dbi,
            debug,
            unnamed,
        } = staged;
        self.named = Some(named);
        self.modules = Some(slots);
        if ids.is_some() {
            self.ids = ids;
        }
        if types.is_some() {
            self.types = types;
        }
        if let Some(names) = names {
            self.names = Some(Names::Table(names));
        }
        if dbi.is_some() {
            self.dbi = dbi;
        }
        if debug.is_some() {
            self.debug = debug;
        }
        if unnamed.is_some() {
            self.unnamed = unnamed;
        }
        self.paths_remapped = true;
    }
}

/// The parts that the model gets from a path remap.
struct Staged {
    /// The named streams.
    named: NamedStreams,
    /// The modules.
    slots: Vec<ModuleSlot>,
    /// The IPI records.
    ids: Option<IdList>,
    /// The TPI records with new hash adjusters.
    types: Option<TypeList>,
    /// The `/names` stream.
    names: Option<StringTableBuilder>,
    /// The DBI values.
    dbi: Option<DebugInfo>,
    /// The optional debug streams with a new FPO stream.
    debug: Option<DebugStreamList>,
    /// The streams without a name with new module type streams.
    unnamed: Option<UnnamedStreams>,
}

/// Make the parts that the model gets from the changed local values.
fn finish<S: ReadAt>(editor: &PdbEditor<'_, S>, parts: Parts) -> Result<Staged> {
    let Parts {
        mut named,
        slots,
        ids,
        mut types,
        type_adjusters,
        names,
        dbi,
        fpo,
        header_block,
        module_types,
        virtual_names: _,
        source_server: _,
        link_info: _,
    } = parts;
    if let Some(types) = &mut types {
        set_adjusters(&mut types.list, &type_adjusters);
    }
    if let Some(block) = header_block.filter(|block| block.changed)
        && let Some(entry) = named.entries.get_mut(block.entry)
    {
        entry.content = Content::Bytes(block.bytes);
    }
    let mut debug = None;
    if let Some(fpo) = fpo.filter(|fpo| fpo.changed) {
        let mut list = if let Some(list) = &editor.debug {
            list.clone()
        } else {
            let dbi = editor
                .cow_source()?
                .debug_info()?
                .ok_or_else(|| missing("PDB has no DBI stream"))?;
            DebugStreamList::from_dbi(dbi)?
        };
        list.set(DebugStream::NewFpo, fpo.bytes)?;
        debug = Some(list);
    }
    let mut unnamed = None;
    for stream in module_types.into_iter().filter(|stream| stream.changed) {
        let list = match &mut unnamed {
            Some(list) => list,
            None => unnamed.insert(match &editor.unnamed {
                Some(list) => list.clone(),
                None => UnnamedStreams::new(editor.cow_source()?.container().stream_count()),
            }),
        };
        store_type_stream(list, stream)?;
    }
    Ok(Staged {
        named,
        slots,
        ids,
        types,
        names,
        dbi,
        debug,
        unnamed,
    })
}

/// Put the new bytes of a module type stream into the streams without a
/// name.
fn store_type_stream(unnamed: &mut UnnamedStreams, stream: TypeRecordStream) -> Result<()> {
    let list = if stream.in_model {
        &mut unnamed.entries
    } else {
        &mut unnamed.replaced
    };
    match list.iter_mut().find(|(id, _)| *id == stream.id) {
        Some((_, bytes)) => *bytes = stream.bytes,
        None => push(list, (stream.id, stream.bytes))?,
    }
    Ok(())
}

/// Give the TPI records the hash adjusters `adjusters`. The list notes a
/// change of the adjusters.
fn set_adjusters(list: &mut RecordList<TypeLeaf<'static>>, adjusters: &[HashAdjuster]) {
    if list.adjusters != adjusters {
        list.adjusters.clear();
        list.adjusters.extend_from_slice(adjusters);
        list.adjusters_changed = true;
    }
}
