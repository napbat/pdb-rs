//! The lookup scenarios: types by index and by name, symbols by name and by address, and lines.
//!
//! Each scenario loads the PDB and builds the lookup structures before the timed runs. The
//! scenarios `type-table` and `symbol-locator` measure the build of these structures.

use std::collections::BTreeSet;
use std::path::Path;

use pdb_rs::native::Pdb;
use pdb_rs::native::address::Rva;
use pdb_rs::native::codeview::index::TypeIndex;
use pdb_rs::native::codeview::type_record::TypeLeaf;
use pdb_rs::native::locate::SymbolLocator;
use pdb_rs::native::types::{ForwardRefs, TypeHashes, TypeStream, TypeTable};

use crate::Outcome;
use crate::measure::Bench;

/// The most procedures that the address samples use. Each procedure gives two addresses.
const MAX_ADDRESS_PROCEDURES: usize = 2000;

/// The most names that the name samples use.
const MAX_NAMES: usize = 1000;

/// Return the TPI stream of `pdb` and the bytes of its hash stream.
fn tpi(pdb: &Pdb<&[u8]>) -> Outcome<(TypeStream, Option<Vec<u8>>)> {
    let tpi = pdb.types()?.ok_or("missing TPI stream")?;
    let hashes = match tpi.header().hash_stream {
        Some(id) => Some(pdb.stream_bytes(id)?),
        None => None,
    };
    Ok((tpi, hashes))
}

/// Measure the build of the type lookup structures: the TPI stream, its hash stream, the
/// [`TypeTable`] and the [`ForwardRefs`] index.
pub(crate) fn type_table(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    bench.measure(|| {
        let (tpi, hash_bytes) = tpi(&pdb)?;
        let hashes = hash_bytes
            .as_deref()
            .map(|bytes| TypeHashes::parse(tpi.header(), bytes))
            .transpose()?;
        let table = TypeTable::new(&tpi)?;
        std::hint::black_box(ForwardRefs::new(&table, hashes.as_ref())?);
        Ok(u64::try_from(table.len())?)
    })
}

/// Decode the record of each TPI index through [`TypeTable::leaf`], in a fixed pseudo-random
/// order.
pub(crate) fn type_by_index(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    let (tpi, _) = tpi(&pdb)?;
    let table = TypeTable::new(&tpi)?;
    let header = tpi.header();
    let mut indices: Vec<u32> = (header.index_begin..header.index_end).collect();
    indices.sort_by_key(|&index| (index.wrapping_mul(0x9e37_79b1), index));
    bench.measure(|| {
        for &index in &indices {
            std::hint::black_box(table.leaf(TypeIndex(index))?);
        }
        Ok(u64::try_from(indices.len())?)
    })
}

/// Resolve each UDT forward reference to its definition by name ([`ForwardRefs::resolve`]).
///
/// The count is the number of forward references that have a definition.
pub(crate) fn type_by_name(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    let (tpi, hash_bytes) = tpi(&pdb)?;
    let hashes = hash_bytes
        .as_deref()
        .map(|bytes| TypeHashes::parse(tpi.header(), bytes))
        .transpose()?;
    let table = TypeTable::new(&tpi)?;
    let forward = ForwardRefs::new(&table, hashes.as_ref())?;
    let mut references = Vec::new();
    for record in tpi.records() {
        let record = record?;
        if let Some(tag) = TypeLeaf::decode(record.record)?.tag()
            && tag.options.is_forward_reference()
        {
            references.push(TypeIndex(record.index));
        }
    }
    bench.measure(|| {
        let mut found = 0;
        for &index in &references {
            found += u64::from(forward.resolve(index)?.is_some());
        }
        Ok(found)
    })
}

/// Measure [`SymbolLocator::new`]. The count is the number of procedures.
pub(crate) fn symbol_locator(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    let procedures = SymbolLocator::new(&pdb)?.procedures().count();
    bench.measure(|| {
        std::hint::black_box(SymbolLocator::new(&pdb)?);
        Ok(u64::try_from(procedures)?)
    })
}

/// Return the start and the middle address of up to [`MAX_ADDRESS_PROCEDURES`] procedures,
/// spread evenly over all procedures.
fn sample_addresses(locator: &SymbolLocator) -> Outcome<Vec<Rva>> {
    let mut addresses = Vec::new();
    for procedure in procedures(locator, MAX_ADDRESS_PROCEDURES)? {
        if let (Some(start), true) = (procedure.0, procedure.1 > 0) {
            addresses.push(start);
            addresses.push(Rva(start.0 + procedure.1 / 2));
        }
    }
    Ok(addresses)
}

/// Return the start and the length of up to `limit` procedures, spread evenly over all
/// procedures.
fn procedures(locator: &SymbolLocator, limit: usize) -> Outcome<Vec<(Option<Rva>, u32)>> {
    let all: Vec<_> = locator
        .procedures()
        .map(|procedure| procedure.map(|procedure| (procedure.start, procedure.length)))
        .collect::<Result<_, _>>()?;
    let stride = all.len().div_ceil(limit.max(1)).max(1);
    Ok(all.into_iter().step_by(stride).collect())
}

/// Return up to [`MAX_NAMES`] names: the names of procedures and of the public symbols at
/// their starts, as the `locate` example selects them.
pub(crate) fn sample_names(locator: &SymbolLocator) -> Outcome<Vec<Vec<u8>>> {
    let mut names = BTreeSet::new();
    let all: Vec<_> = locator.procedures().collect::<Result<_, _>>()?;
    let stride = all.len().div_ceil(MAX_NAMES.div_ceil(2)).max(1);
    for procedure in all.iter().step_by(stride) {
        names.insert(procedure.name.to_vec());
        if let Some(start) = procedure.start
            && let Some(public) = locator.public_at(start)?
            && public.displacement == 0
        {
            names.insert(public.public.name.to_vec());
        }
    }
    Ok(names
        .into_iter()
        .filter(|name| !name.is_empty())
        .take(MAX_NAMES)
        .collect())
}

/// Find each sample name with [`SymbolLocator::find_name`]. The count is the number of matches.
pub(crate) fn symbol_by_name(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    let locator = SymbolLocator::new(&pdb)?;
    let names = sample_names(&locator)?;
    bench.measure(|| {
        let mut found = 0;
        for name in &names {
            found += u64::try_from(locator.find_name(name)?.len())?;
        }
        Ok(found)
    })
}

/// Find the public symbol at each sample address with [`SymbolLocator::public_at`]. The count
/// is the number of addresses with a public symbol.
pub(crate) fn symbol_by_address(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    let locator = SymbolLocator::new(&pdb)?;
    let addresses = sample_addresses(&locator)?;
    bench.measure(|| {
        let mut found = 0;
        for &rva in &addresses {
            found += u64::from(locator.public_at(rva)?.is_some());
        }
        Ok(found)
    })
}

/// Find the module, the procedure, the inline frames and the line of each sample address with
/// [`SymbolLocator::locate`]. The count is the number of addresses with a line.
pub(crate) fn line_lookup(path: &Path, bench: &mut Bench) -> Outcome {
    let bytes = std::fs::read(path)?;
    let pdb = Pdb::parse(&bytes)?;
    let locator = SymbolLocator::new(&pdb)?;
    let addresses = sample_addresses(&locator)?;
    bench.measure(|| {
        let mut found = 0;
        for &rva in &addresses {
            let location = locator.locate(rva)?;
            found += u64::from(location.is_some_and(|location| location.line.is_some()));
        }
        Ok(found)
    })
}
