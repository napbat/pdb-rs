//! The library checks of module and shared symbol records.
//!
//! The example reports each broken rule as an inconsistency and continues.

use std::collections::{BTreeMap, HashMap, HashSet};

use pdb_rs::ErrorKind;
use pdb_rs::native::codeview::index::{IdIndex, ModuleIndex, ModuleSymbolOffset, TypeIndex};
use pdb_rs::native::codeview::symbol::validate::{
    ModuleRecord, ValidationContext, ValidationReport, validate_module_symbols,
    validate_shared_symbols,
};
use pdb_rs::native::codeview::{LeafKind, Records, SymbolKind};
use pdb_rs::native::dbi::ModuleStream;
use pdb_rs::native::msf::StreamId;

use crate::{Context, Outcome, Report};

/// The kinds of the module records, for the module reference checks.
#[derive(Default)]
pub(crate) struct ModuleRecords {
    /// The kind of each record by module index and module stream offset.
    kinds: HashMap<(ModuleIndex, ModuleSymbolOffset), SymbolKind>,
    /// The modules with a symbol stream.
    read: HashSet<ModuleIndex>,
}

/// Return the virtual size of each image section.
fn section_lengths(context: &Context<'_>) -> Vec<u32> {
    context
        .addresses
        .as_ref()
        .map_or_else(Vec::new, |addresses| {
            addresses
                .sections()
                .iter()
                .map(|header| header.virtual_size)
                .collect()
        })
}

/// Return the IPI record kind of `id`.
fn id_kind(context: &Context<'_>, id: IdIndex) -> Option<LeafKind> {
    context.ids.record(id).map(|record| LeafKind(record.kind))
}

/// Make the library context from the example context.
///
/// For a module whose indices refer to records outside the PDB, the library does not check
/// the type and ID indices, as the editor plan does.
fn validation_context<'c>(
    context: &'c Context<'_>,
    id_kinds: &'c dyn Fn(IdIndex) -> Option<LeafKind>,
    lengths: &'c [u32],
) -> ValidationContext<'c> {
    let base = if context.indices_checked {
        ValidationContext::new(context.tpi_end, context.ids.index_end()).with_id_kinds(id_kinds)
    } else {
        ValidationContext::new(TypeIndex(u32::MAX), IdIndex(u32::MAX))
    }
    .with_module_count(context.module_count)
    .with_type_sizes(context.type_size);
    if context.addresses.is_some() {
        base.with_section_lengths(lengths)
    } else {
        base
    }
}

/// Add the counts and the violations of a report.
fn add(report: &mut Report, label: &'static str, result: &ValidationReport) {
    report.add(label, result.records);
    let mut rules = BTreeMap::new();
    for violation in &result.violations {
        rules
            .entry(violation.context)
            .or_insert((0_usize, violation))
            .0 += 1;
    }
    for (context, (count, first)) in rules {
        report.inconsistency(format!(
            "{count} x {context}: first in stream {} at offset {:#x}, kind {}",
            first.stream.0,
            first.offset,
            first
                .record_kind
                .map_or_else(|| "unknown".to_owned(), |kind| format!("{kind:#06x}")),
        ));
    }
}

/// Validate one module stream and keep the kinds of its records.
pub(crate) fn module(
    stream: &ModuleStream,
    index: u16,
    context: &Context<'_>,
    records: &mut ModuleRecords,
    report: &mut Report,
) -> Outcome {
    let base = ModuleSymbolOffset(u32::try_from(stream.symbol_offset())?);
    let index = ModuleIndex(u32::from(index));
    records.read.insert(index);
    for record in Records::new(stream.symbol_data()) {
        let Ok(record) = record else { break };
        records.kinds.insert(
            (index, ModuleSymbolOffset(base.0 + record.offset)),
            SymbolKind(record.kind),
        );
    }
    let lengths = section_lengths(context);
    let id_kinds = |id| id_kind(context, id);
    let validation = validation_context(context, &id_kinds, &lengths);
    match validate_module_symbols(stream.symbol_data(), stream.stream(), base, &validation) {
        Ok(result) => add(report, "module symbols validated", &result),
        Err(error) if error.kind() == ErrorKind::UnsupportedVersion => {
            report.add("module symbol streams not validated", 1);
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// Validate the shared symbol record stream.
pub(crate) fn shared(
    bytes: &[u8],
    stream: StreamId,
    context: &Context<'_>,
    records: &ModuleRecords,
    report: &mut Report,
) -> Outcome {
    let lengths = section_lengths(context);
    let id_kinds = |id| id_kind(context, id);
    let module_records = |module, offset| {
        if !records.read.contains(&module) {
            return ModuleRecord::Unknown;
        }
        records
            .kinds
            .get(&(module, offset))
            .map_or(ModuleRecord::Missing, |&kind| ModuleRecord::Kind(kind))
    };
    let validation =
        validation_context(context, &id_kinds, &lengths).with_module_records(&module_records);
    let result = validate_shared_symbols(bytes, stream, &validation)?;
    add(report, "shared symbols validated", &result);
    Ok(())
}
