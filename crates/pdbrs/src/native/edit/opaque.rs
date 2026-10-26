//! The scan for opaque data that can refer to changed offsets or indices.
//!
//! An edit that moves symbol records or removes type records changes values
//! that other data can hold. The plan rewrites the references that a decoder
//! of this crate knows. It cannot rewrite a reference in data without a
//! decoder, so it rejects the edit if such data exists:
//!
//! - A present stream that no other stream names. Its role is not known.
//! - A named stream of a kind that this crate does not decode.
//! - For moved symbol records also: a symbol record of a kind that this crate
//!   does not decode, in the symbol record stream or in a module stream, a C13
//!   subsection of a kind that this crate does not decode and C11 line data.

use alloc::vec::Vec;

use super::plan::Context;
use crate::io::ReadAt;
use crate::native::Pdb;
use crate::native::auxiliary::{NamedStreamKind, TmCache};
use crate::native::codeview::Records;
use crate::native::codeview::symbol::SymbolData;
use crate::native::lines::{LineSubsection, ModuleLines};
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::{Error, ErrorKind, Result};

/// The data that the scan examines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Scope {
    /// The streams without a known role and the named streams without a
    /// decoder.
    Streams,
    /// The streams, and also the symbol records and the line data.
    Records,
}

/// Return the rejection error with `context`.
fn opaque(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidReference, None).with_context(context)
}

/// Return an error if the PDB has opaque data in `scope`.
pub(super) fn check<S: ReadAt>(pdb: &Pdb<S>, context: &Context, scope: Scope) -> Result<()> {
    check_streams(pdb, context)?;
    if scope == Scope::Records {
        check_records(pdb, context)?;
    }
    Ok(())
}

/// Return an error for a stream without a known role or a named stream
/// without a decoder.
fn check_streams<S: ReadAt>(pdb: &Pdb<S>, context: &Context) -> Result<()> {
    let count = pdb.msf().stream_count();
    let mut claimed = Vec::new();
    claimed
        .try_reserve_exact(count as usize)
        .map_err(|_| no_memory())?;
    claimed.resize(count as usize, false);
    let mut claim = |id: Option<StreamId>| {
        if let Some(slot) = id.and_then(|id| claimed.get_mut(id.0 as usize)) {
            *slot = true;
        }
    };
    for id in 0..5 {
        claim(Some(StreamId(id)));
    }
    let tpi = context.tpi.header();
    claim(tpi.hash_stream);
    claim(tpi.hash_aux_stream);
    if let Some(ipi) = pdb.ids()? {
        claim(ipi.header().hash_stream);
        claim(ipi.header().hash_aux_stream);
    }
    let dbi = &context.dbi;
    let header = dbi.header();
    claim(header.global_stream);
    claim(header.public_stream);
    claim(header.symbol_stream);
    for module in dbi.modules() {
        claim(module?.stream);
    }
    for id in dbi.debug_streams() {
        claim(id);
    }
    for named in pdb.info().named_streams() {
        claim(Some(named.stream));
        match NamedStreamKind::classify(named.name) {
            NamedStreamKind::Other => {
                return Err(opaque(
                    "the edit changes offsets or indices, and a named stream of an unknown kind can refer to them",
                ));
            }
            NamedStreamKind::TmCache => {
                let bytes = pdb.stream_bytes(named.stream)?;
                for map in TmCache::parse(&bytes, named.stream)?.streams() {
                    claim(map);
                }
            }
            _ => {}
        }
    }
    for (id, claimed) in (0..count).zip(claimed) {
        if !claimed && pdb.msf().stream(StreamId(id))?.is_some() {
            return Err(opaque(
                "the edit changes offsets or indices, and a stream that no other stream names can refer to them",
            )
            .with_stream(id, 0));
        }
    }
    Ok(())
}

/// Return an error for a symbol record of an unknown kind, a C13 subsection
/// of an unknown kind or C11 line data.
fn check_records<S: ReadAt>(pdb: &Pdb<S>, context: &Context) -> Result<()> {
    let unknown_symbol = || {
        opaque(
            "the edit moves symbol records, and a symbol record of an unknown kind can refer to them",
        )
    };
    if let Some(id) = context.dbi.header().symbol_stream {
        let bytes = pdb.stream_bytes(id)?;
        for record in Records::new(&bytes) {
            let record = record?;
            if matches!(record.symbol()?.data(), SymbolData::Unsupported(_)) {
                return Err(unknown_symbol().with_stream(id.0, u64::from(record.offset)));
            }
        }
    }
    for module in context.dbi.modules() {
        let module = module?;
        let Some(stream) = pdb.module_stream(&module)? else {
            continue;
        };
        let id = stream.stream().0;
        for record in stream.symbols() {
            let record = record?;
            if matches!(record.symbol()?.data(), SymbolData::Unsupported(_)) {
                return Err(unknown_symbol().with_stream(id, u64::from(record.offset)));
            }
        }
        if !stream.c11_data().is_empty() {
            return Err(opaque(
                "the edit moves symbol records, and C11 line data can refer to them",
            )
            .with_stream(id, 0));
        }
        let lines = ModuleLines::parse(&stream)?;
        for subsection in lines.subsections() {
            if matches!(subsection.data, LineSubsection::Unsupported { .. }) {
                return Err(opaque(
                    "the edit moves symbol records, and a C13 subsection of an unknown kind can refer to them",
                )
                .with_stream(id, u64::from(subsection.offset)));
            }
        }
    }
    Ok(())
}
