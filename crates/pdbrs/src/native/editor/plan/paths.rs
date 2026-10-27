//! The plan check after a path remap.
//!
//! [`PdbEditor::remap_source_paths`] changes the paths and the `/names`
//! offsets of the fields that this crate decodes. Data without a decoder can
//! hold a path or a `/names` offset that the task did not change. The plan
//! rejects such data with [`ErrorKind::UnsupportedEdit`] and the stream of
//! the data:
//!
//! - a module whose symbols do not form a scope tree, a symbol record of an
//!   unknown kind (in a module stream or in the symbol record stream), an
//!   `S_MOD_TYPEREF` record with flags of unknown meaning;
//! - C11 line data, a C13 subsection of an unknown kind, an ignored C13
//!   subsection, a `DEBUG_S_SYMBOLS` or a `DEBUG_S_STRINGTABLE` subsection;
//! - an IPI record of an unknown kind;
//! - a source named stream of an unknown kind, and in the copy-on-write
//!   layout a present stream that no other stream names and that does not
//!   decode as a type map stream (the scan of [`super::opaque`]).
//!
//! [`PdbEditor::remap_source_paths`]: crate::native::PdbEditor::remap_source_paths
//! [`ErrorKind::UnsupportedEdit`]: crate::ErrorKind::UnsupportedEdit

use super::cow::{IPI_STREAM, Work};
use super::opaque::{self, Messages};
use crate::io::ReadAt;
use crate::native::auxiliary::NamedStreamKind;
use crate::native::codeview::Records;
use crate::native::codeview::id_record::IdRecord;
use crate::native::codeview::symbol::{ModuleTypeSource, Symbol, SymbolData};
use crate::native::dbi::ModuleStream;
use crate::native::editor::module::{ModuleContents, Symbols, symbol_base};
use crate::native::editor::named::Content;
use crate::native::editor::record::SymbolRecord;
use crate::native::editor::{ModuleSlot, PdbEditor, tree};
use crate::native::lines::LineSubsection;
use crate::{Error, ErrorKind, Result};

/// The error contexts of the stream scan after a path remap.
const STREAMS: Messages = Messages {
    named: "the path remap changes paths and /names offsets, and a named stream of an unknown kind can hold them",
    unclaimed: "the path remap changes paths and /names offsets, and a stream that no other stream names can hold them",
};

/// The context for a symbol record of an unknown kind.
const UNKNOWN_SYMBOL: &str = "the path remap changes paths and /names offsets, and a symbol record of an unknown kind can hold them";

/// Return the rejection error with `context`.
fn opaque(context: &'static str) -> Error {
    Error::new(ErrorKind::UnsupportedEdit, None).with_context(context)
}

/// Check the decoded parts of the model after a path remap.
pub(super) fn check_model<S: ReadAt>(editor: &PdbEditor<'_, S>) -> Result<()> {
    for slot in editor.modules.iter().flatten() {
        match slot {
            ModuleSlot::Decoded(module) => check_module(module)?,
            ModuleSlot::Source => {
                return Err(opaque(
                    "the path remap changes paths and /names offsets, and a module that the editor did not decode can hold them",
                ));
            }
        }
    }
    if let Some(ids) = &editor.ids {
        for entry in &ids.list.entries {
            if matches!(entry.value, IdRecord::Unsupported(_)) {
                return Err(opaque(
                    "the path remap changes paths and /names offsets, and an IPI record of an unknown kind can hold them",
                )
                .with_stream(IPI_STREAM, u64::from(entry.source.unwrap_or(0))));
            }
        }
    }
    for entry in editor.named.iter().flat_map(|named| &named.entries) {
        if let Some(stream) = entry.source
            && entry.content != Content::Removed
            && NamedStreamKind::classify(&entry.name) == NamedStreamKind::Other
        {
            return Err(opaque(STREAMS.named).with_stream(stream, 0));
        }
    }
    if let Some(shared) = &editor.symbols {
        let stream = match editor.source {
            Some(pdb) => pdb.debug_info()?.and_then(|dbi| dbi.header().symbol_stream),
            None => None,
        };
        for record in shared.records.iter().flatten() {
            if matches!(record.data(), SymbolData::Unsupported(_)) {
                let error = opaque(UNKNOWN_SYMBOL);
                return Err(match stream {
                    Some(stream) => error.with_stream(stream, origin_offset(record, 0)),
                    None => error,
                });
            }
        }
    }
    Ok(())
}

/// Return the stream offset of the source record of `record`, or 0.
fn origin_offset(record: &SymbolRecord, base: u32) -> u64 {
    record.origin().map_or(0, |symbol| {
        u64::from(symbol.record().offset) + u64::from(base)
    })
}

/// Check one decoded module.
fn check_module(module: &ModuleContents) -> Result<()> {
    let source = module.source().and_then(|source| source.stream.as_ref());
    let base = source.map_or(0, symbol_base);
    let locate = |error: Error, offset: u64| match source.map(ModuleStream::stream) {
        Some(stream) => error.with_stream(stream, offset),
        None => error,
    };
    match module.symbol_region() {
        Symbols::Opaque => {
            return Err(locate(
                opaque(
                    "the path remap changes paths and /names offsets, and the symbols of a module that do not form a scope tree can hold them",
                ),
                0,
            ));
        }
        Symbols::Tree(nodes) => tree::walk(nodes, &mut |record| match record.data() {
            SymbolData::Unsupported(_) => {
                Err(locate(opaque(UNKNOWN_SYMBOL), origin_offset(record, base)))
            }
            SymbolData::ModuleTypeReference(reference)
                if matches!(reference.source(), ModuleTypeSource::Other(_)) =>
            {
                Err(locate(
                    opaque(
                        "the path remap changes /names offsets, and the module type streams of an S_MOD_TYPEREF record with flags of unknown meaning can hold them",
                    ),
                    origin_offset(record, base),
                ))
            }
            _ => Ok(()),
        })?,
        Symbols::Absent => {}
    }
    if source.is_some_and(|stream| !stream.c11_data().is_empty()) {
        return Err(locate(
            opaque(
                "the path remap changes paths and /names offsets, and C11 line data can hold them",
            ),
            0,
        ));
    }
    for subsection in module.lines() {
        if matches!(
            subsection,
            LineSubsection::Unsupported { .. }
                | LineSubsection::Ignored { .. }
                | LineSubsection::Symbols(_)
                | LineSubsection::StringTable(_)
        ) {
            return Err(locate(
                opaque(
                    "the path remap changes paths and /names offsets, and a C13 subsection without a decoded path field can hold them",
                ),
                0,
            ));
        }
    }
    Ok(())
}

/// Check the source streams of a copy-on-write plan after a path remap:
/// the streams without a known role, the named streams of an unknown kind and
/// the records of the symbol record stream if the model did not decode them.
pub(super) fn check_source<S: ReadAt>(work: &mut Work<'_, S>) -> Result<()> {
    let pdb = work.pdb;
    let decoded_symbols = work.editor.symbols.is_some();
    let context = work.context()?;
    opaque::check_streams(pdb, context, &STREAMS)?;
    if decoded_symbols {
        return Ok(());
    }
    let Some(id) = context.dbi.header().symbol_stream else {
        return Ok(());
    };
    let bytes = pdb.symbol_records()?.unwrap_or_default();
    for record in Records::new(bytes) {
        let record = record?;
        if matches!(Symbol::decode(record)?.data(), SymbolData::Unsupported(_)) {
            return Err(opaque(UNKNOWN_SYMBOL).with_stream(id, u64::from(record.offset)));
        }
    }
    Ok(())
}
