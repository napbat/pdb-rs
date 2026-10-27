//! The reference checks of a plan.

use crate::native::IndexSpace;
use crate::native::codeview::id_record::IdRecord;
use crate::native::codeview::index::IdIndex;
use crate::native::codeview::symbol::validate::{Rule, ValidationReport};
use crate::native::codeview::type_record::TypeLeaf;
use crate::native::types::{ReferenceLimits, TypeTable, validate_type_references};
use crate::{Error, ErrorKind, Result};

/// Return `true` if the plan rejects a violation of `rule`.
///
/// The plan rejects broken framing and references that do not resolve. The
/// other rules describe producer conventions; the plan keeps such records.
const fn rejected(rule: Rule) -> bool {
    matches!(
        rule,
        Rule::Walk
            | Rule::TypeIndexRange
            | Rule::IdIndexRange
            | Rule::ModuleIndex
            | Rule::ModuleRecordMissing
    )
}

/// Return the first rejected violation of `report` as an error.
pub(super) fn check(report: &ValidationReport) -> Result<()> {
    match report
        .violations
        .iter()
        .find(|violation| rejected(violation.rule))
    {
        Some(violation) => Err(violation.error()),
        None => Ok(()),
    }
}

/// Return the error for a forward reference in `space`.
fn forward(space: IndexSpace) -> Error {
    Error::new(ErrorKind::UnsupportedEdit, None)
        .with_context(
            "a record refers to itself or to a later record of its index space; \
             the editor does not write forward references",
        )
        .with_index_space(space)
}

/// Check the type references of a TPI stream and reject forward references.
///
/// A record that refers to itself makes a cycle, and the reference check
/// rejects a cycle with [`ErrorKind::InvalidFormat`]. Each cycle has a
/// forward reference, so the plan then looks for one and gives the error of
/// [`forward`].
pub(super) fn check_types(table: &TypeTable<'_>) -> Result<()> {
    let summary = match validate_type_references(table, ReferenceLimits::default()) {
        Ok(summary) => summary,
        Err(error) if error.kind() == ErrorKind::InvalidFormat && has_forward(table)? => {
            return Err(forward(IndexSpace::TypeIndex));
        }
        Err(error) => return Err(error),
    };
    if summary.forward_references != 0 {
        return Err(forward(IndexSpace::TypeIndex));
    }
    Ok(())
}

/// Return `true` if a record of `table` refers to itself or to a later
/// record.
fn has_forward(table: &TypeTable<'_>) -> Result<bool> {
    for record in table.stream().records() {
        let record = record?;
        let leaf = TypeLeaf::decode(record.record)?;
        let mut found = false;
        leaf.for_each_reference(|reference| {
            found |= !reference.index.is_primitive() && reference.index.0 >= record.index;
            Ok(())
        })?;
        if found {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Reject an ID record that refers to itself or to a later ID record.
pub(super) fn check_id_order<'r>(
    records: impl Iterator<Item = (IdIndex, &'r IdRecord<'static>)>,
) -> Result<()> {
    for (index, record) in records {
        let later = |id: IdIndex| id != IdIndex::NONE && id.0 >= index.0;
        let forward_reference = match record {
            IdRecord::Function(function) => function.scope.is_some_and(later),
            IdRecord::BuildInfo(info) => info.arguments.iter().any(later),
            IdRecord::SubstringList(list) => list.iter().any(later),
            IdRecord::String(string) => string.substrings.is_some_and(later),
            IdRecord::UdtSourceLine(line) => later(line.source_file),
            _ => false,
        };
        if forward_reference {
            return Err(forward(IndexSpace::IdIndex));
        }
    }
    Ok(())
}
