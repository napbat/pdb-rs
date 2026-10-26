//! The C13 subsections that the symbol walk does not resolve, the global
//! references and the re-encoding of every module stream.

use pdbrs::io::FileReader;
use pdbrs::native::Pdb;
use pdbrs::native::dbi::{Dbi, ModuleRegionSizes};
use pdbrs::native::lines::{LineSubsection, ModuleLines};
use pdbrs::native::symbols::RecordOffsets;

use crate::dbi::translate;
use crate::{Context, Outcome, Report};

/// Check one C13 subsection of a kind that `module_lines` does not resolve.
///
/// File references of IL lines resolve through `checksum_name`.
pub(crate) fn other_subsection(
    data: &LineSubsection<'_>,
    checksum_name: &dyn Fn(u32) -> Outcome,
    context: &Context<'_>,
    report: &mut Report,
) -> Outcome {
    match data {
        LineSubsection::Symbols(symbols) => {
            for record in symbols.records() {
                let symbol = record?.symbol()?;
                if matches!(
                    symbol.data(),
                    pdbrs::native::codeview::symbol::SymbolData::Unsupported(_)
                ) {
                    report.undecoded(format!("C13 symbol {:#06x}", symbol.kind()));
                }
            }
            report.add("C13 symbol records", symbols.len());
        }
        LineSubsection::IlLines(lines) => {
            let header = lines.header();
            translate(
                context.addresses.as_ref(),
                header.segment,
                header.code_offset,
                report,
            )?;
            for block in lines.blocks() {
                checksum_name(block.checksum_offset)?;
                report.add("IL line entries", block.lines.len());
            }
        }
        LineSubsection::StringTable(table) => report.add("C13 strings", table.iter().count()),
        LineSubsection::FrameData(frames) => {
            report.add("C13 frame data records", frames.records().len());
        }
        LineSubsection::CrossScopeImports(imports) => {
            for import in imports {
                context.names.get(import.module_name_offset)?;
                report.add("cross-scope imported IDs", import.ids.len());
            }
        }
        LineSubsection::CrossScopeExports(exports) => {
            report.add("cross-scope exports", exports.len());
        }
        LineSubsection::FunctionTokenMap(map) | LineSubsection::TypeTokenMap(map) => {
            report.add("metadata token map entries", map.entries().len());
        }
        LineSubsection::MergedAssemblyInput(inputs) => {
            report.add("merged assembly inputs", inputs.len());
        }
        LineSubsection::XfgTypeHashes(hashes) => report.add("XFG type hashes", hashes.len()),
        LineSubsection::XfgVirtualHashes(hashes) => report.add("XFG virtual hashes", hashes.len()),
        LineSubsection::Ignored { .. } => report.add("ignored C13 subsections", 1),
        _ => report.undecoded(format!("C13 subsection {:#x}", data.kind())),
    }
    Ok(())
}

/// Validate the global references of each module stream, and encode each
/// C13 subsection and each module stream again.
///
/// The run fails if an encoded subsection, C13 region or module stream is
/// not equal to the source bytes, or if the region sizes are not equal to
/// the sizes of the module record.
pub(crate) fn module_streams(pdb: &Pdb<FileReader>, dbi: &Dbi, report: &mut Report) -> Outcome {
    let records = match dbi.header().symbol_stream {
        Some(id) => Some(RecordOffsets::frame(&pdb.stream_bytes(id)?, id)?),
        None => None,
    };
    let mut subsections = (0usize, 0usize);
    let mut regions = (0usize, 0usize);
    let mut streams = (0usize, 0usize);
    for module in dbi.modules() {
        let module = module?;
        let Some(stream) = pdb.module_stream(&module)? else {
            continue;
        };
        let refs = stream.global_refs()?;
        if !refs.is_empty() {
            refs.validate(
                records
                    .as_ref()
                    .ok_or("global references without symbol records")?,
            )?;
        }
        report.add("module global references", refs.len());
        if let Some(lines) = report.supported(ModuleLines::parse(&stream), "C13 line data")? {
            for (framed, decoded) in stream.c13_subsections().zip(lines.subsections()) {
                let mut data = Vec::new();
                decoded.data.encode(&mut data)?;
                subsections.0 += 1;
                subsections.1 += usize::from(data != framed?.data);
            }
            let mut c13 = Vec::new();
            lines.encode(&mut c13)?;
            regions.0 += 1;
            regions.1 += usize::from(c13 != stream.c13_data());
        }
        let encoded = stream.encode()?;
        let expected = ModuleRegionSizes {
            symbol_bytes: module.symbol_bytes,
            c11_bytes: module.c11_bytes,
            c13_bytes: module.c13_bytes,
        };
        let source = pdb.stream_bytes(stream.stream())?;
        streams.0 += 1;
        streams.1 += usize::from(encoded.bytes != source || encoded.sizes != expected);
    }
    println!(
        "C13 subsections re-encoded: {}, different: {}",
        subsections.0, subsections.1
    );
    println!(
        "C13 regions re-encoded: {}, different: {}",
        regions.0, regions.1
    );
    println!(
        "module streams re-encoded: {}, different: {}",
        streams.0, streams.1
    );
    if subsections.1 == 0 && regions.1 == 0 && streams.1 == 0 {
        Ok(())
    } else {
        Err("re-encoded C13 subsections, C13 regions or module streams differ".into())
    }
}
