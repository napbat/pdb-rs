//! The path fields of the named streams: the `srcsrv` source file lines, the
//! `/LinkInfo` current directory and the names of the `/src/files/` streams.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use core::ops::Range;

use super::{NamedBytes, Parts, PathMap, invalid, push};
use crate::native::auxiliary::{LinkInfo, SourceServer};
use crate::native::editor::named::Content;
use crate::native::injected_source::SOURCE_FILE_STREAM_PREFIX;
use crate::native::output::no_memory;
use crate::{Bytes, Error, ErrorKind, Result};

/// Return the byte ranges of the source file paths of a `srcsrv` stream.
fn path_ranges(stream: &NamedBytes) -> Result<Vec<Range<usize>>> {
    SourceServer::parse(&stream.bytes, stream.stream)?.source_file_path_ranges()
}

/// Collect the paths of the `srcsrv` and `/LinkInfo` streams.
pub(super) fn collect(parts: &Parts, values: &mut BTreeSet<Vec<u8>>) -> Result<()> {
    if let Some(server) = &parts.source_server {
        for range in path_ranges(server)? {
            let path = &server.bytes[range];
            if !path.is_empty() && !values.contains(path) {
                values.insert(path.to_vec());
            }
        }
    }
    if let Some(link) = &parts.link_info
        && let Some(info) = LinkInfo::parse(&link.bytes, link.stream)?
        && !info.current_directory.is_empty()
        && !values.contains(info.current_directory.as_bytes())
    {
        values.insert(info.current_directory.to_vec());
    }
    Ok(())
}

/// Give the paths of the named streams their new values, and give each
/// `/src/files/` stream of a changed virtual file name its new name.
pub(super) fn apply(parts: &mut Parts, map: &PathMap) -> Result<()> {
    if let Some(server) = &parts.source_server
        && let Some(bytes) = source_server(server, map)?
        && let Some(entry) = parts.named.entries.get_mut(server.entry)
    {
        entry.content = Content::Bytes(bytes);
    }
    if let Some(link) = &parts.link_info
        && let Some(bytes) = link_info(link, map)?
        && let Some(entry) = parts.named.entries.get_mut(link.entry)
    {
        entry.content = Content::Bytes(bytes);
    }
    rename_files(parts, map)
}

/// Return the `srcsrv` stream with the new source file paths, or `None` if
/// no path changes.
fn source_server(server: &NamedBytes, map: &PathMap) -> Result<Option<Vec<u8>>> {
    let ranges = path_ranges(server)?;
    let mut out = Vec::new();
    let mut expected = Vec::new();
    let mut start = 0;
    let mut changed = false;
    for range in ranges {
        let path = &server.bytes[range.clone()];
        let new = map.get(path).map_or(path, Vec::as_slice);
        changed |= new != path;
        let kept = &server.bytes[start..range.start];
        out.try_reserve(kept.len() + new.len())
            .map_err(|_| no_memory())?;
        out.extend_from_slice(kept);
        out.extend_from_slice(new);
        push(&mut expected, new)?;
        start = range.end;
    }
    if !changed {
        return Ok(None);
    }
    let rest = &server.bytes[start..];
    out.try_reserve(rest.len()).map_err(|_| no_memory())?;
    out.extend_from_slice(rest);
    let check = NamedBytes {
        entry: server.entry,
        stream: server.stream,
        bytes: out,
    };
    let ranges = path_ranges(&check)?;
    let same = ranges.len() == expected.len()
        && ranges
            .iter()
            .zip(&expected)
            .all(|(range, path)| &check.bytes[range.clone()] == *path);
    if !same {
        return Err(invalid(
            "the new srcsrv text does not give each new path as the first field of its line",
        ));
    }
    Ok(Some(check.bytes))
}

/// Return the `/LinkInfo` stream with the new current directory, or `None`
/// if it stays.
fn link_info(link: &NamedBytes, map: &PathMap) -> Result<Option<Vec<u8>>> {
    let Some(mut info) = LinkInfo::parse(&link.bytes, link.stream)? else {
        return Ok(None);
    };
    let Some(new) = map.get(info.current_directory.as_bytes()) else {
        return Ok(None);
    };
    let too_large = || {
        Error::new(ErrorKind::LimitExceeded, None)
            .with_context("/LinkInfo record size overflows u32")
    };
    let old = u32::try_from(info.current_directory.len()).map_err(|_| too_large())?;
    let added = u32::try_from(new.len()).map_err(|_| too_large())?;
    info.size = info
        .size
        .checked_sub(old)
        .and_then(|size| size.checked_add(added))
        .ok_or_else(too_large)?;
    info.current_directory = Bytes::from(new.clone());
    let mut bytes = Vec::new();
    info.encode(&mut bytes)?;
    Ok(Some(bytes))
}

/// Return the name of the content stream of the virtual file name `name`.
fn file_stream(name: &[u8]) -> Result<Vec<u8>> {
    let mut stream = Vec::new();
    stream
        .try_reserve_exact(SOURCE_FILE_STREAM_PREFIX.len() + name.len())
        .map_err(|_| no_memory())?;
    stream.extend_from_slice(SOURCE_FILE_STREAM_PREFIX);
    stream.extend_from_slice(name);
    Ok(stream)
}

/// Give the `/src/files/` stream of each changed virtual file name its new
/// name. The stream keeps its stream ID and its contents.
fn rename_files(parts: &mut Parts, map: &PathMap) -> Result<()> {
    for old in &parts.virtual_names {
        let Some(new) = map.get(old) else {
            continue;
        };
        let old = file_stream(old)?;
        let new = file_stream(new)?;
        let entries = &mut parts.named.entries;
        let taken = entries
            .iter()
            .any(|entry| entry.name.as_bytes() == new && entry.content != Content::Removed);
        if taken {
            return Err(invalid(
                "two injected source files get the same virtual file name",
            ));
        }
        if let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry.name.as_bytes() == old && entry.content != Content::Removed)
        {
            entry.name = Bytes::from(new);
        }
    }
    Ok(())
}
