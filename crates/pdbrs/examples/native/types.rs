//! The TPI and IPI streams, their hash streams and their records.

use pdbrs::io::FileReader;
use pdbrs::native::Pdb;
use pdbrs::native::codeview::id_record::{IdRecord, IdTable};
use pdbrs::native::codeview::index::{IdIndex, TypeIndex};
use pdbrs::native::codeview::type_record::TypeLeaf;
use pdbrs::native::names::StringTable;
use pdbrs::native::types::{ForwardRefs, HEADER_SIZE, TypeHashes, TypeStream, TypeTable};

use crate::{Outcome, Report};

/// Load the hash stream of a type stream, if the header names one.
pub(crate) fn hash_stream_bytes(
    pdb: &Pdb<FileReader>,
    types: &TypeStream,
    report: &mut Report,
) -> Outcome<Option<Vec<u8>>> {
    let Some(id) = types.header().hash_stream else {
        return Ok(None);
    };
    report.claim(id);
    if let Some(aux) = types.header().hash_aux_stream {
        report.claim(aux);
        report.undecoded("auxiliary type hash stream".to_owned());
    }
    Ok(Some(pdb.stream_bytes(id)?))
}

/// Decode every TPI record, its field lists and its forward references.
pub(crate) fn types(tpi: &TypeStream, hash_bytes: Option<&[u8]>, report: &mut Report) -> Outcome {
    let parsed = hash_bytes
        .map(|bytes| TypeHashes::parse(tpi.header(), bytes))
        .transpose();
    let hashes = report.supported(parsed, "TPI hash stream")?.flatten();
    if let Some(hashes) = &hashes {
        report.supported(hashes.verify(tpi), "TPI hash values")?;
    }
    let table = TypeTable::new(tpi)?;
    let forward = report.supported(
        ForwardRefs::new(&table, hashes.as_ref()),
        "TPI forward-reference index",
    )?;
    let base = u64::try_from(HEADER_SIZE)?;
    let (mut resolved, mut unresolved) = (0, 0);
    for record in tpi.records() {
        let record = record?;
        report.add("TPI records", 1);
        let location =
            |error: pdbrs::Error| error.with_stream(2, base + u64::from(record.record.offset));
        let decoded = TypeLeaf::decode(record.record).map_err(location);
        let what = format!("TPI leaf {:#06x}", record.record.kind);
        let Some(leaf) = report.supported(decoded, &what)? else {
            continue;
        };
        match &leaf {
            TypeLeaf::Unsupported(raw) => {
                report.undecoded(format!("TPI leaf {:#06x}", raw.kind));
            }
            TypeLeaf::FieldList(_) => {
                for member in table.members(TypeIndex(record.index)).map_err(location)? {
                    if report
                        .supported(member.map_err(location), "field-list member")?
                        .is_none()
                    {
                        break;
                    }
                    report.add("field-list members", 1);
                }
            }
            TypeLeaf::MethodList(list) => {
                for entry in list.entries() {
                    entry.map_err(location)?;
                }
            }
            _ => {}
        }
        let forward_reference = leaf
            .tag()
            .is_some_and(|tag| tag.options.is_forward_reference());
        if let (true, Some(forward)) = (forward_reference, &forward) {
            match forward.resolve(TypeIndex(record.index))? {
                Some(_) => resolved += 1,
                None => unresolved += 1,
            }
        }
    }
    report.add("forward references resolved", resolved);
    report.add("forward references without definition", unresolved);
    Ok(())
}

/// Decode every IPI record and validate its references.
pub(crate) fn identifiers<'a>(
    ipi: &'a TypeStream,
    hash_bytes: Option<&[u8]>,
    tpi: &TypeStream,
    names: &StringTable<'_>,
    report: &mut Report,
) -> Outcome<IdTable<'a>> {
    if let Some(bytes) = hash_bytes
        && let Some(hashes) =
            report.supported(TypeHashes::parse(ipi.header(), bytes), "IPI hash stream")?
    {
        report.supported(hashes.verify(ipi), "IPI hash values")?;
    }
    let ids = IdTable::new(ipi)?;
    report.supported(
        ids.validate(TypeIndex(tpi.header().index_end)),
        "IPI references",
    )?;
    let mut text = Vec::new();
    for record in ipi.records() {
        let id = IdIndex(record?.index);
        report.add("IPI records", 1);
        let Some(decoded) = report.supported(ids.decode(id), "IPI record")? else {
            continue;
        };
        match decoded {
            IdRecord::Unsupported(raw) => {
                report.undecoded(format!("IPI leaf {:#06x}", raw.kind));
            }
            IdRecord::String(_) => {
                text.clear();
                ids.resolve_string(id, &mut text)?;
            }
            IdRecord::UdtModuleSourceLine(line) => {
                names.get(line.name_offset)?;
            }
            _ => {}
        }
    }
    Ok(ids)
}
