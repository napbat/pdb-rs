//! The byte search for the build root in the streams of a remapped PDB (#46).
//!
//! Each occurrence of the root must be in a field that the decision comment of #46 excludes.
//! The search reads the output with `pdbrs` and puts each occurrence in a class:
//!
//! - inside a string of `S_ENVBLOCK` or `LF_STRING_ID`, after its first byte: a command line
//!   or a piece of a long string (the task gives each string whole to the function);
//! - inside the command of `/LinkInfo`;
//! - after ` at ` in a type record, an ID record name or a symbol name: Clang names an unnamed
//!   type or a lambda after its declaration place;
//! - in a `sourcelink` stream.
//!
//! Each other occurrence is a problem.

use std::collections::BTreeMap;

use pdbrs::native::Pdb;
use pdbrs::native::auxiliary::{LinkInfo, NamedStreamKind};
use pdbrs::native::codeview::Records;
use pdbrs::native::codeview::id_record::LF_STRING_ID;
use pdbrs::native::codeview::symbol::{ModuleTypeReference, S_ENVBLOCK};
use pdbrs::native::msf::StreamId;

use super::super::super::source_paths::BuildRoot;

/// The class of an occurrence inside a command line or a piece of a long string.
const COMMAND_LINE: &str = "inside a command line string (S_ENVBLOCK, LF_STRING_ID)";
/// The class of an occurrence inside the `/LinkInfo` command.
const LINK_COMMAND: &str = "inside the /LinkInfo command";
/// The class of an occurrence in a name after ` at `.
const NAME: &str = "in a type, ID or symbol name after ` at ` (Clang)";
/// The class of an occurrence in a `sourcelink` stream.
const SOURCE_LINK: &str = "in sourcelink JSON";

/// The role of a stream for the search.
#[derive(Clone, Copy, Debug)]
enum Role {
    /// The TPI stream.
    Types,
    /// The IPI stream: records after the header.
    Ids,
    /// A module type stream: records without a header.
    TypeRecords,
    /// The symbol record stream.
    Symbols,
    /// A module stream with a symbol region of this end.
    Module(usize),
    /// The `/LinkInfo` stream.
    LinkInfo,
    /// A `sourcelink` stream.
    SourceLink,
}

/// The result of the search in one PDB.
#[derive(Debug, Default)]
pub(super) struct Search {
    /// The number of occurrences in each excluded class.
    pub classes: BTreeMap<&'static str, usize>,
    /// The occurrences outside the excluded fields.
    pub problems: Vec<String>,
}

/// Return the role of each stream of `pdb` that the search knows.
fn roles(pdb: &Pdb<&[u8]>) -> pdbrs::Result<BTreeMap<u32, Role>> {
    let mut roles = BTreeMap::new();
    roles.insert(2, Role::Types);
    roles.insert(4, Role::Ids);
    for named in pdb.info().named_streams() {
        let role = match NamedStreamKind::classify(&named.name) {
            NamedStreamKind::LinkInfo => Role::LinkInfo,
            NamedStreamKind::SourceLink | NamedStreamKind::NumberedSourceLink { .. } => {
                Role::SourceLink
            }
            _ => continue,
        };
        roles.insert(named.stream.0, role);
    }
    let Some(dbi) = pdb.debug_info()? else {
        return Ok(roles);
    };
    if let Some(id) = dbi.header().symbol_stream {
        roles.insert(id.0, Role::Symbols);
    }
    for module in dbi.modules() {
        let (index, record) = module?;
        let (Some(id), Some(stream)) = (record.stream, pdb.module(index)?) else {
            continue;
        };
        roles.insert(
            id.0,
            Role::Module(stream.symbol_offset() + stream.symbol_data().len()),
        );
        if let Some(reference) = ModuleTypeReference::find(stream.symbol_data())? {
            for id in reference.source().streams() {
                roles.insert(id.0, Role::TypeRecords);
            }
        }
    }
    Ok(roles)
}

/// Return `true` if ` at ` comes right before `at`.
fn after_at(bytes: &[u8], at: usize) -> bool {
    bytes[..at].ends_with(b" at ")
}

/// Return the record of `bytes` (a record sequence that starts at `base`) that holds `at`:
/// its kind and the range of its payload.
fn record_at(bytes: &[u8], base: usize, at: usize) -> Option<(u16, usize, usize)> {
    let records = bytes.get(base..)?;
    for record in Records::new(records).map_while(Result::ok) {
        let start = base + record.offset as usize + 4;
        let end = start + record.data.len();
        if (start..end).contains(&at) {
            return Some((record.kind, start, end));
        }
    }
    None
}

/// Return `true` if `at` is inside a NUL-terminated string of `bytes[start..end]` and not at
/// the start of that string. The strings start at `start`.
fn inside_string(bytes: &[u8], start: usize, end: usize, at: usize) -> bool {
    let mut first = start;
    for (offset, &byte) in bytes[start..end].iter().enumerate() {
        let position = start + offset;
        if position == at {
            return position != first;
        }
        if byte == 0 {
            first = position + 1;
        }
    }
    false
}

/// Return the class of an occurrence at `at` in a record sequence that starts at `base`.
/// `strings` tells where the strings of the string kinds start in the payload.
fn class_in_records(
    bytes: &[u8],
    base: usize,
    at: usize,
    string_kind: u16,
    strings: usize,
) -> Option<&'static str> {
    if after_at(bytes, at) {
        return Some(NAME);
    }
    let (kind, start, end) = record_at(bytes, base, at)?;
    (kind == string_kind && inside_string(bytes, start + strings, end, at)).then_some(COMMAND_LINE)
}

/// Return the class of the occurrence at `at` of a stream with `role`.
fn class(role: Role, bytes: &[u8], stream: StreamId, at: usize) -> Option<&'static str> {
    match role {
        Role::Types | Role::Symbols => after_at(bytes, at).then_some(NAME),
        Role::Ids => {
            let header = u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?) as usize;
            class_in_records(bytes, header, at, LF_STRING_ID.0, 4)
        }
        Role::TypeRecords => class_in_records(bytes, 0, at, LF_STRING_ID.0, 4),
        // The symbol region starts after the 4-byte signature. `S_ENVBLOCK` has a flags
        // byte before its strings.
        Role::Module(end) => {
            if at >= end {
                return None;
            }
            class_in_records(&bytes[..end], 4, at, S_ENVBLOCK.0, 1)
        }
        Role::LinkInfo => {
            let info = LinkInfo::parse(bytes, stream).ok()??;
            let command = 24 + info.current_directory.len() + 1;
            (command..command + info.command.len())
                .contains(&at)
                .then_some(LINK_COMMAND)
        }
        Role::SourceLink => Some(SOURCE_LINK),
    }
}

/// Search the root in each stream of the PDB `bytes`.
pub(super) fn search(root: &BuildRoot, bytes: &[u8]) -> Result<Search, String> {
    let read = |error: pdbrs::Error| format!("pdbrs cannot read the output: {error}");
    let pdb = Pdb::parse(bytes).map_err(read)?;
    let roles = roles(&pdb).map_err(read)?;
    let mut result = Search::default();
    for id in 0..pdb.container().stream_count() {
        let stream = StreamId(id);
        let Ok(data) = pdb.stream_bytes(stream) else {
            continue;
        };
        for at in root.find(&data) {
            let found = roles
                .get(&id)
                .and_then(|&role| class(role, &data, stream, at));
            match found {
                Some(class) => *result.classes.entry(class).or_default() += 1,
                None => result.problems.push(format!(
                    "stream {id} holds the build root at offset {at}: {}",
                    String::from_utf8_lossy(&data[at..data.len().min(at + 60)])
                )),
            }
        }
    }
    Ok(result)
}
