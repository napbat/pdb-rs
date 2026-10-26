//! The TPI and IPI streams, their hash streams and their records.
//!
//! The example also writes each record again, rebuilds each stream and its
//! hash stream, and compares the bytes with the input. It fails if a byte
//! differs.

use pdbrs::io::ReadAt;
use pdbrs::native::Pdb;
use pdbrs::native::codeview::field_list::{LLVM_FIELD_LIST_LIMIT, Member, encode_field_list};
use pdbrs::native::codeview::id_record::{IdRecord, IdTable};
use pdbrs::native::codeview::index::{IdIndex, TypeIndex};
use pdbrs::native::codeview::type_record::TypeLeaf;
use pdbrs::native::names::StringTable;
use pdbrs::native::types::{
    ForwardRefs, HEADER_SIZE, ReferenceLimits, TypeHashes, TypeStream, TypeStreamOptions,
    TypeTable, validate_type_references, write_type_stream,
};

use crate::{Outcome, Report};

/// The record size limit of the Microsoft compiler field-list split.
///
/// The value is the smallest limit that gives the `LF_INDEX` chains of the
/// MSVC fixtures. Microsoft does not document it.
const MSVC_FIELD_LIST_LIMIT: usize = 0xfb00;

/// Load the hash stream of a type stream, if the header names one.
pub(crate) fn hash_stream_bytes(
    pdb: &Pdb<impl ReadAt>,
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

/// Rebuild a type stream and its hash stream from the re-encoded `records`
/// and compare them with the input. Return `true` if all bytes are equal.
fn rebuild(
    stream: &TypeStream,
    records: &[u8],
    hashes: Option<&TypeHashes<'_>>,
    hash_bytes: Option<&[u8]>,
    name: &str,
) -> Outcome<bool> {
    let adjusters = hashes.map(TypeHashes::adjusters).unwrap_or_default();
    let options = TypeStreamOptions::from_header(stream.header(), adjusters);
    let written = write_type_stream(records, &options)?;
    let same_stream = written.stream == stream.bytes();
    let same_hash = written.hash_stream.as_deref() == hash_bytes;
    if !same_stream || !same_hash {
        println!(
            "{name} stream rebuilt with differences: stream {}, hash stream {}",
            if same_stream { "same" } else { "different" },
            if same_hash { "same" } else { "different" },
        );
    }
    Ok(same_stream && same_hash)
}

/// Re-split each field-list chain and compare the record sizes.
///
/// A chain starts at a record that no `LF_INDEX` member refers to. The
/// function returns the number of chains, and the number that neither the
/// LLVM limit nor the Microsoft limit gives again.
fn resplit_chains(table: &TypeTable<'_>, heads: &[TypeIndex]) -> Outcome<(usize, usize)> {
    let mut different = 0;
    for &head in heads {
        let mut members: Vec<Member<'_>> = Vec::new();
        let mut sizes = Vec::new();
        let mut next = Some(head);
        while let Some(index) = next {
            let record = table.record(index)?.record;
            sizes.push(record.data.len() + 4);
            let list = table.field_list(index)?;
            next = list.continuation();
            for member in list.members() {
                let member = member?;
                if !matches!(member, Member::Continuation(_)) {
                    members.push(member);
                }
            }
        }
        sizes.reverse();
        let matches = [LLVM_FIELD_LIST_LIMIT, MSVC_FIELD_LIST_LIMIT]
            .into_iter()
            .map(|limit| -> Outcome<bool> {
                let mut out = Vec::new();
                encode_field_list(&members, TypeIndex(0x1000), limit, &mut out)?;
                let produced: Vec<usize> = pdbrs::native::codeview::Records::new(&out)
                    .map(|record| record.map(|record| record.data.len() + 4))
                    .collect::<Result<_, _>>()?;
                Ok(produced == sizes)
            })
            .collect::<Outcome<Vec<bool>>>()?;
        if !matches.contains(&true) {
            different += 1;
        }
    }
    Ok((heads.len(), different))
}

/// Decode every TPI record, its field lists and its forward references.
///
/// Encode each record again, rebuild the stream and its hash stream, and
/// check the references.
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
    let (mut encoded, mut different) = (Vec::new(), 0_usize);
    let mut continued = std::collections::HashSet::new();
    let mut lists = Vec::new();
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
        let padding = leaf.producer_padding();
        if !padding.is_empty() {
            report.inconsistency(format!(
                "TPI record {:#x} ends with MASM padding {padding:02x?} instead of the LF_PAD run",
                record.index
            ));
        }
        let start = encoded.len();
        leaf.encode(&mut encoded).map_err(location)?;
        if encoded[start..] != *tpi.full_record(&record.record)? {
            different += 1;
            println!("TPI record {:#x} re-encodes differently", record.index);
        }
        match &leaf {
            TypeLeaf::Unsupported(raw) => {
                report.undecoded(format!("TPI leaf {:#06x}", raw.kind));
            }
            TypeLeaf::FieldList(list) => {
                lists.push(TypeIndex(record.index));
                if let Some(next) = list.continuation() {
                    continued.insert(next);
                }
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
    let references = validate_type_references(&table, ReferenceLimits::default())?;
    report.add(
        "TPI type references",
        usize::try_from(references.references)?,
    );
    report.add(
        "TPI forward type references",
        usize::try_from(references.forward_references)?,
    );
    report.add(
        "TPI reference depth",
        usize::try_from(references.max_depth)?,
    );
    let heads: Vec<TypeIndex> = lists
        .into_iter()
        .filter(|index| !continued.contains(index))
        .filter(|&index| {
            table
                .field_list(index)
                .is_ok_and(|list| list.continuation().is_some())
        })
        .collect();
    let (chains, chains_different) = resplit_chains(&table, &heads)?;
    println!("field-list chains re-split: {chains}, different: {chains_different}");
    println!(
        "types re-encoded: {}, different: {different}",
        tpi.header().record_count()
    );
    let same = rebuild(tpi, &encoded, hashes.as_ref(), hash_bytes, "TPI")?;
    println!("TPI stream rebuilt: 1, different: {}", usize::from(!same));
    if different != 0 || chains_different != 0 || !same {
        return Err("TPI records or streams do not re-encode to the same bytes".into());
    }
    Ok(())
}

/// Decode every IPI record and validate its references.
///
/// Encode each record again, and rebuild the stream and its hash stream.
pub(crate) fn identifiers<'a>(
    ipi: &'a TypeStream,
    hash_bytes: Option<&[u8]>,
    tpi: &TypeStream,
    names: &StringTable<'_>,
    report: &mut Report,
) -> Outcome<IdTable<'a>> {
    let mut hashes = None;
    if let Some(bytes) = hash_bytes
        && let Some(parsed) =
            report.supported(TypeHashes::parse(ipi.header(), bytes), "IPI hash stream")?
    {
        report.supported(parsed.verify(ipi), "IPI hash values")?;
        hashes = Some(parsed);
    }
    let ids = IdTable::new(ipi)?;
    report.supported(
        ids.validate(TypeIndex(tpi.header().index_end)),
        "IPI references",
    )?;
    let mut text = Vec::new();
    let (mut encoded, mut different) = (Vec::new(), 0_usize);
    for record in ipi.records() {
        let record = record?;
        let id = IdIndex(record.index);
        report.add("IPI records", 1);
        let Some(decoded) = report.supported(ids.decode(id), "IPI record")? else {
            continue;
        };
        let start = encoded.len();
        decoded.encode(&mut encoded)?;
        if encoded[start..] != *ipi.full_record(&record.record)? {
            different += 1;
            println!("IPI record {:#x} re-encodes differently", record.index);
        }
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
    println!(
        "ids re-encoded: {}, different: {different}",
        ipi.header().record_count()
    );
    let same = rebuild(ipi, &encoded, hashes.as_ref(), hash_bytes, "IPI")?;
    println!("IPI stream rebuilt: 1, different: {}", usize::from(!same));
    if different != 0 || !same {
        return Err("IPI records or streams do not re-encode to the same bytes".into());
    }
    Ok(ids)
}
