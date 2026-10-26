//! The DBI substreams, the optional debug streams and address translation.

use pdbrs::ErrorKind;
use pdbrs::io::FileReader;
use pdbrs::native::Pdb;
use pdbrs::native::address::{AddressMap, Rva, SectionOffset};
use pdbrs::native::dbi::{Dbi, DbiBuilder, DebugStream, OwnedModule, Substream};
use pdbrs::native::msf::StreamId;
use pdbrs::native::section::SectionHeaders;

use crate::{Context, Outcome, Report};

/// Encode the DBI stream, each substream, each module record and the builder model again.
///
/// Each encoder must give the bytes of the stream.
pub(crate) fn reencode_dbi(pdb: &Pdb<FileReader>, dbi: &Dbi, report: &mut Report) -> Outcome {
    let original = pdb.stream_bytes(StreamId(3))?;
    let mut encoded = Vec::new();
    dbi.encode(&mut encoded)?;
    report.reencoded("DBI streams", &original, &encoded);
    let mut rebuilt = Vec::new();
    DbiBuilder::from_dbi(dbi)?.encode(&mut rebuilt)?;
    report.reencoded("DBI streams from the builder model", &original, &rebuilt);
    for kind in Substream::ALL {
        let mut encoded = Vec::new();
        dbi.encode_substream(kind, &mut encoded)?;
        report.reencoded("DBI substreams", dbi.substream(kind), &encoded);
    }
    for module in dbi.modules() {
        let module = module?;
        let start = usize::try_from(module.offset)? - dbi.substream_offset(Substream::ModuleInfo);
        let mut encoded = Vec::new();
        OwnedModule::from_module(&module)?.encode(&mut encoded)?;
        let end = start + encoded.len();
        let stored = dbi
            .substream(Substream::ModuleInfo)
            .get(start..end)
            .ok_or("module record is outside the module info substream")?;
        report.reencoded("DBI module records", stored, &encoded);
    }
    Ok(())
}

/// Load the section-header stream and claim every optional debug stream.
///
/// [`frame_data`] decodes the FPO and new-FPO streams.
pub(crate) fn section_header_bytes(
    pdb: &Pdb<FileReader>,
    dbi: &Dbi,
    report: &mut Report,
) -> Outcome<Option<Vec<u8>>> {
    let mut headers = None;
    for (slot, id) in dbi.debug_streams().enumerate() {
        let Some(id) = id else { continue };
        report.claim(id);
        if slot == DebugStream::SectionHeader.index() {
            headers = Some(pdb.stream_bytes(id)?);
        } else if slot == DebugStream::OriginalSectionHeader.index() {
            SectionHeaders::parse(&pdb.stream_bytes(id)?)?;
        } else if slot != DebugStream::Fpo.index() && slot != DebugStream::NewFpo.index() {
            report.undecoded(format!("optional debug stream slot {slot}"));
        }
    }
    Ok(headers)
}

/// Decode the FPO and new-FPO streams, and resolve each frame program in `/names`.
pub(crate) fn frame_data(
    pdb: &Pdb<FileReader>,
    dbi: &Dbi,
    context: &Context<'_>,
    report: &mut Report,
) -> Outcome {
    let mut starts = Vec::new();
    if let Some(fpo) = pdb.fpo(dbi)? {
        starts.extend(fpo.iter().map(|record| record.start));
        report.add("FPO records", fpo.len());
    }
    if let Some(frames) = pdb.frame_data(dbi)? {
        for frame in &frames {
            context.names.get(frame.program)?;
            starts.push(frame.start);
        }
        report.add("frame data records", frames.len());
    }
    if let Some(addresses) = &context.addresses {
        for start in starts {
            addresses.section_offset(Rva(start))?;
        }
    }
    Ok(())
}

/// Decode the DBI substreams and return the module count.
pub(crate) fn debug_info_tables(
    dbi: &Dbi,
    addresses: Option<&AddressMap<'_>>,
    report: &mut Report,
) -> Outcome<usize> {
    let mut module_count = 0;
    for module in dbi.modules() {
        module?;
        module_count += 1;
    }
    report.add("modules", module_count);
    if let Some(contributions) = dbi.section_contributions()? {
        for contribution in &contributions {
            if usize::from(contribution.module) >= module_count {
                return Err("section contribution names a missing module".into());
            }
            let offset = u32::try_from(contribution.offset)?;
            translate(addresses, contribution.section, offset, report)?;
            report.add("section contributions", 1);
        }
    }
    if let Some(map) = dbi.section_map()? {
        report.add("section map entries", map.iter().count());
    }
    if let Some(sources) = dbi.source_info()? {
        for module in sources.modules() {
            for name in module.file_names() {
                name?;
                report.add("module source files", 1);
            }
        }
    }
    if let Some(ec) = dbi.ec_names()?
        && report
            .supported(ec.verify_buckets(), "EC name hashing")?
            .is_some()
    {
        report.add("EC names", ec.entries().count());
    }
    report.add("type servers", dbi.type_server_map()?.len());
    Ok(module_count)
}

/// Convert a section-relative address to an RVA and count the result.
pub(crate) fn translate(
    addresses: Option<&AddressMap<'_>>,
    section: u16,
    offset: u32,
    report: &mut Report,
) -> Outcome {
    let Some(map) = addresses else {
        report.add("addresses without section headers", 1);
        return Ok(());
    };
    match map.rva(SectionOffset { section, offset }) {
        Ok(_) => report.add("addresses translated to RVAs", 1),
        // Section 0 and the absolute segment after the last section have no RVA.
        Err(_) if section == 0 || usize::from(section) > map.sections().len() => {
            report.add("addresses without an RVA", 1);
        }
        // OMAP translation is not implemented.
        Err(error) if error.kind() == ErrorKind::UnsupportedVersion => {
            report.add("addresses not translated (OMAP)", 1);
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
