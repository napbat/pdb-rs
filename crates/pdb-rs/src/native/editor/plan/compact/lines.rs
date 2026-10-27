//! The index fields of a module stream: the symbol region and the C13
//! subsections.
//!
//! The space of each C13 index comes from
//! `Mod1::fUpdateTiOrIdReferencedByNonSyms` in `PDB/dbi/mod.cpp` of
//! `microsoft/microsoft-pdb`. `link.exe` maps the inlinee IDs, the type keys
//! of `DEBUG_S_TYPE_MDTOKEN_MAP` and the global IDs of
//! `DEBUG_S_CROSSSCOPEEXPORTS` into the PDB. It keeps the other values. The
//! C11 line region has no indices.

use alloc::vec::Vec;

use super::map::IndexRemap;
use super::records::{Rewrite, unsupported, write_difference};
use super::symbols::symbol_records;
use crate::native::dbi::{Module, ModuleStream};
use crate::native::lines::{LineSubsection, SubsectionKind, TokenMap};
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::{Error, ErrorKind, Result};

/// The size of a C13 subsection header: a kind and a length.
const SUBSECTION_HEADER_SIZE: usize = 8;

/// The bit of an inlinee that marks an ID of another module
/// (`CrossScopeId::IsCrossScopeId` in `cvinfo.h`).
const CROSS_SCOPE_BIT: u32 = 0x8000_0000;

/// The bit of the local ID of an export that marks an IPI index
/// (`DecoratedItemId::IsFuncId` in `cvinfo.h`).
const FUNCTION_ID_BIT: u32 = 0x8000_0000;

/// The change of one C13 subsection.
enum Edit {
    /// No index changes.
    Keep,
    /// The subsection holds symbol records. Rewrite them in place.
    Symbols,
    /// The subsection data before and after the change.
    Bytes(Vec<u8>, Vec<u8>),
}

/// Convert a stream position to a stream offset.
fn stream_offset(position: usize) -> u64 {
    u64::try_from(position).unwrap_or(u64::MAX)
}

/// Write the output indices into the module stream `bytes` of `module`.
///
/// `stream` is the module stream. The function rewrites the symbol records
/// of the symbol region and the index fields of the C13 subsections in
/// place. The C11 line region and the global references stay. Return the
/// stream bytes and `true` if a byte changed.
///
/// # Errors
///
/// - The errors of `ModuleStream::parse` and of the subsection framing.
/// - The errors of [`symbol_records`] for the symbol region and for a
///   `DEBUG_S_SYMBOLS` subsection, with the location of the record.
/// - The errors of the map, of the subsection decoders and
///   [`ErrorKind::UnsupportedEdit`] for a subsection of a kind without a
///   decoder, with the location of the subsection header in `stream`.
pub(in crate::native::editor::plan) fn module_stream(
    bytes: Vec<u8>,
    module: &Module<'_>,
    stream: StreamId,
    map: &impl IndexRemap,
) -> Result<(Vec<u8>, bool)> {
    let parsed = ModuleStream::parse(bytes, module)?;
    let symbols = parsed.symbol_offset()..parsed.symbol_offset() + parsed.symbol_data().len();
    let c13 = symbols.end + parsed.c11_data().len();
    let mut parts = Vec::new();
    {
        let mut subsections = parsed.c13_subsections();
        while let Some(next) = subsections.next_parts() {
            let (offset, kind, data) = next?;
            let header = usize::try_from(offset)
                .ok()
                .and_then(|offset| c13.checked_add(offset))
                .ok_or_else(|| {
                    Error::new(ErrorKind::OffsetOverflow, None)
                        .with_context("C13 subsection offset does not fit in memory")
                        .with_stream(stream, stream_offset(c13))
                })?;
            parts.try_reserve(1).map_err(|_| no_memory())?;
            parts.push((header, kind, data.len()));
        }
    }
    let mut bytes = parsed.into_bytes();
    let base = stream_offset(symbols.start);
    let mut changed = symbol_records(&mut bytes[symbols], stream, base, map)?;
    for (header, kind, length) in parts {
        let start = header + SUBSECTION_HEADER_SIZE;
        changed |= subsection(kind, &mut bytes[start..start + length], stream, header, map)?;
    }
    Ok((bytes, changed))
}

/// Write the output indices into the data `data` of a subsection of `kind`.
///
/// `header` is the stream offset of the subsection header. Return `true` if
/// a byte changed.
fn subsection(
    kind: SubsectionKind,
    data: &mut [u8],
    stream: StreamId,
    header: usize,
    map: &impl IndexRemap,
) -> Result<bool> {
    let offset = stream_offset(header);
    let located = |error: Error| error.with_stream(stream, offset);
    match edit(kind, data, map).map_err(located)? {
        Edit::Keep => Ok(false),
        Edit::Symbols => {
            let base = stream_offset(header + SUBSECTION_HEADER_SIZE);
            symbol_records(data, stream, base, map)
        }
        Edit::Bytes(before, after) => write_difference(data, &before, &after).map_err(located),
    }
}

/// Return the change of the data `data` of a subsection of `kind`.
fn edit<M: IndexRemap>(kind: SubsectionKind, data: &[u8], map: &M) -> Result<Edit> {
    let mut rewrite = Rewrite::new(map);
    let decoded = LineSubsection::decode(kind, data)?;
    let mut after = Vec::new();
    match &decoded {
        LineSubsection::Symbols(_) => return Ok(Edit::Symbols),
        LineSubsection::InlineeLines(lines) => {
            let signature = lines.signature();
            signature.encode(&mut after)?;
            for mut entry in lines {
                // An ID of another module stays (`CrossScopeId`).
                if entry.inlinee.0 & CROSS_SCOPE_BIT == 0 {
                    rewrite.id_index(&mut entry.inlinee)?;
                }
                entry.encode(signature, &mut after)?;
            }
        }
        LineSubsection::TypeTokenMap(token_map) => {
            let entries = token_map.entries();
            let mut output = Vec::new();
            output
                .try_reserve_exact(entries.len())
                .map_err(|_| no_memory())?;
            for mut entry in entries {
                rewrite.type_value(&mut entry.key)?;
                output.push(entry);
            }
            TokenMap::encode_parts(&output, &mut after)?;
        }
        // The XFG type indices are in the type scope of the object file, and
        // `link.exe` does not copy these subsections to a PDB (see
        // `native::lines::xfg_hash`). In a PDB, the space of their indices is
        // not known, so they block the compaction.
        LineSubsection::XfgTypeHashes(_) | LineSubsection::XfgVirtualHashes(_) => {
            return Err(unsupported(
                "an XFG hash subsection holds type indices of an unknown type scope",
            ));
        }
        LineSubsection::CrossScopeExports(exports) => {
            // The local ID is an index of the object file and stays. Bit 31
            // of the local ID selects the space of the global ID.
            for mut entry in exports {
                if entry.local_id & FUNCTION_ID_BIT == 0 {
                    rewrite.type_value(&mut entry.global_id)?;
                } else {
                    rewrite.id_value(&mut entry.global_id)?;
                }
                entry.encode(&mut after)?;
            }
        }
        LineSubsection::Unsupported { .. } => {
            return Err(unsupported(
                "a C13 subsection of a kind without a decoder can refer to a changed index",
            ));
        }
        // An imported ID of `DEBUG_S_CROSSSCOPEIMPORTS` is a local ID of the
        // exporting module, not an index of the PDB: `link.exe` does not map
        // it, and the cross-scope validation compares it with the local IDs
        // of the exports. `link.exe` does not map the keys of
        // `DEBUG_S_FUNC_MDTOKEN_MAP` (`fUpdateTiOrIdReferencedByNonSyms` maps
        // only the type token map), so a key is not an index of this PDB.
        // The other kinds have no indices.
        LineSubsection::CrossScopeImports(_)
        | LineSubsection::FunctionTokenMap(_)
        | LineSubsection::Ignored { .. }
        | LineSubsection::Lines(_)
        | LineSubsection::IlLines(_)
        | LineSubsection::StringTable(_)
        | LineSubsection::FileChecksums(_)
        | LineSubsection::FrameData(_)
        | LineSubsection::MergedAssemblyInput(_)
        | LineSubsection::CoffSymbolRvas(_) => return Ok(Edit::Keep),
    }
    if !rewrite.changed() {
        return Ok(Edit::Keep);
    }
    let mut before = Vec::new();
    decoded.encode(&mut before)?;
    Ok(Edit::Bytes(before, after))
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::module_stream;
    use crate::native::IndexSpace;
    use crate::native::codeview::Records;
    use crate::native::codeview::index::{ChecksumOffset, IdIndex, TypeIndex};
    use crate::native::codeview::symbol::{S_UDT, Symbol, SymbolData, UserDefinedType};
    use crate::native::dbi::{Module, ModuleStream};
    use crate::native::editor::plan::compact::map::IndexMaps;
    use crate::native::editor::plan::compact::records::tests::{removal, words};
    use crate::native::lines::{
        InlineeLine, InlineeLines, InlineeSignature, LineSubsection, SubsectionKind,
    };
    use crate::native::msf::StreamId;
    use crate::{Bytes, ErrorKind};

    /// The module stream of the tests.
    const STREAM: StreamId = StreamId(9);

    /// Return the data of an inlinee line subsection with `inlinees`.
    fn inlinee_data(inlinees: &[u32]) -> Vec<u8> {
        let mut data = Vec::new();
        InlineeSignature::Normal.encode(&mut data).unwrap();
        for (line, &inlinee) in (10..).zip(inlinees) {
            InlineeLine::encode_parts(
                InlineeSignature::Normal,
                IdIndex(inlinee),
                ChecksumOffset(0),
                line,
                &[],
                &mut data,
            )
            .unwrap();
        }
        data
    }

    /// Return a module stream with an `S_UDT` record and the subsection
    /// `subsection`, and its module record.
    fn stream(subsection: &LineSubsection<'_>) -> (Vec<u8>, Module<'static>) {
        let mut bytes = words(&[4]);
        SymbolData::UserDefinedType(UserDefinedType {
            type_index: TypeIndex(0x1002),
            name: Bytes::new(b"point"),
        })
        .encode(S_UDT, &mut bytes)
        .unwrap();
        let symbol_bytes = bytes.len();
        subsection.encode_framed(&mut bytes).unwrap();
        let c13_bytes = bytes.len() - symbol_bytes;
        bytes.extend_from_slice(&words(&[0]));
        let module = Module {
            stream: Some(STREAM),
            symbol_bytes: u32::try_from(symbol_bytes).unwrap(),
            c13_bytes: u32::try_from(c13_bytes).unwrap(),
            ..Module::default()
        };
        (bytes, module)
    }

    #[test]
    fn symbols_and_inlinee_lines_get_the_output_indices() {
        let data = inlinee_data(&[0x1003, 0x8000_0001]);
        let lines = LineSubsection::InlineeLines(InlineeLines::parse(&data).unwrap());
        let (bytes, module) = stream(&lines);
        let length = bytes.len();
        let (bytes, changed) = module_stream(bytes, &module, STREAM, &removal()).unwrap();
        assert!(changed);
        assert_eq!(bytes.len(), length);
        let parsed = ModuleStream::parse(bytes, &module).unwrap();
        let record = Records::new(parsed.symbol_data()).next().unwrap().unwrap();
        let symbol = Symbol::decode(record).unwrap();
        assert_eq!(symbol.type_index(), Some(TypeIndex(0x1001)));
        let subsection = parsed.c13_subsections().next().unwrap().unwrap();
        let LineSubsection::InlineeLines(lines) =
            LineSubsection::decode(subsection.kind, &subsection.data).unwrap()
        else {
            panic!("not inlinee lines");
        };
        let inlinees: Vec<_> = lines.iter().map(|line| line.inlinee).collect();
        assert_eq!(inlinees, [IdIndex(0x1002), IdIndex(0x8000_0001)]);
    }

    #[test]
    fn removed_inlinee_has_the_subsection_location() {
        let data = inlinee_data(&[0x1000]);
        let lines = LineSubsection::InlineeLines(InlineeLines::parse(&data).unwrap());
        let (bytes, module) = stream(&lines);
        let header = u64::from(module.symbol_bytes);
        let error = module_stream(bytes, &module, STREAM, &removal()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidReference);
        assert_eq!(error.index_space(), Some(IndexSpace::IdIndex));
        assert_eq!(error.stream_location(), Some((STREAM, header)));
    }

    #[test]
    fn identity_map_changes_nothing() {
        let data = inlinee_data(&[0x1003]);
        let lines = LineSubsection::InlineeLines(InlineeLines::parse(&data).unwrap());
        let (bytes, module) = stream(&lines);
        let source = bytes.clone();
        let (bytes, changed) =
            module_stream(bytes, &module, STREAM, &IndexMaps::default()).unwrap();
        assert!(!changed);
        assert_eq!(bytes, source);
    }

    #[test]
    fn subsection_without_a_decoder_is_unsupported() {
        let unknown = LineSubsection::decode(SubsectionKind(0x7f), &[1, 0, 0, 0]).unwrap();
        let (bytes, module) = stream(&unknown);
        let error = module_stream(bytes, &module, STREAM, &removal()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::UnsupportedEdit);
    }
}
