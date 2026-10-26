//! Rules of the `StateMachineMethod` and `CustomDebugInformation` tables.

use alloc::collections::BTreeSet;

use super::{Rule, ValidationReport};
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::debug::rows::row_error;
use crate::portable::metadata::{CodedIndex, CustomDebugInformationRow, TableId};
use crate::{ErrorKind, Result};

/// Check the order, the uniqueness and the methods of the state machine rows.
pub(super) fn state_machine_methods<S: ReadAt>(
    pdb: &Pdb<S>,
    report: &mut ValidationReport,
) -> Result<()> {
    let mut previous = None;
    let mut kickoffs = BTreeSet::new();
    let mut items = 0;
    for method in pdb.state_machine_methods() {
        let method = method?;
        let error = |context, column| {
            row_error(
                ErrorKind::InvalidFormat,
                context,
                TableId::STATE_MACHINE_METHOD,
                method.id().get(),
                Some(column),
            )
        };
        let (move_next, kickoff) = (method.move_next_method(), method.kickoff_method());
        if move_next.is_nil() {
            return Err(error("MoveNextMethod is nil", "MoveNextMethod"));
        }
        if kickoff.is_nil() {
            return Err(error("KickoffMethod is nil", "KickoffMethod"));
        }
        if previous.is_some_and(|previous| move_next <= previous) {
            return Err(error(
                "StateMachineMethod table is not sorted by MoveNextMethod or has a duplicate",
                "MoveNextMethod",
            ));
        }
        previous = Some(move_next);
        if !kickoffs.insert(kickoff) {
            return Err(error(
                "two state machines have the same KickoffMethod",
                "KickoffMethod",
            ));
        }
        items += 1;
    }
    report.checked(Rule::StateMachineMethods, items);
    Ok(())
}

/// Check that the custom debug information rows are sorted by parent and have a parent.
pub(super) fn custom_debug_information<S: ReadAt>(
    pdb: &Pdb<S>,
    report: &mut ValidationReport,
) -> Result<()> {
    let mut previous = None;
    let mut items = 0;
    for row in pdb.rows::<CustomDebugInformationRow>() {
        let (id, row) = row?;
        let error = |context| {
            row_error(
                ErrorKind::InvalidFormat,
                context,
                TableId::CUSTOM_DEBUG_INFORMATION,
                id.get(),
                Some("Parent"),
            )
        };
        // The row reader checked the tag, so the token encodes.
        let parent = CodedIndex::HasCustomDebugInformation
            .encode(row.parent)
            .filter(|_| row.parent.row() != 0)
            .ok_or_else(|| error("custom debug information parent is nil"))?;
        if previous.is_some_and(|previous| parent < previous) {
            return Err(error(
                "CustomDebugInformation table is not sorted by Parent",
            ));
        }
        previous = Some(parent);
        items += 1;
    }
    report.checked(Rule::CustomDebugInformationParents, items);
    Ok(())
}
