//! Edit and Continue method information: the local slot map, the lambda and
//! closure map and the state machine state map.
//!
//! The Portable PDB custom debug information and the native Roslyn `MD2`
//! records of kinds 6, 7 and 9 use the same blob formats. The layouts follow
//! the "Edit and Continue Local Slot Map" and "Edit and Continue Lambda and
//! Closure Map" sections of the Portable PDB specification and these Roslyn
//! files (commit `e3c31f15b79e226656e3b082a921d4d60127b1fa`):
//!
//! - `src/Compilers/Core/Portable/Emit/EditAndContinueMethodDebugInformation.cs`
//!   (`UncompressSlotMap`, `SerializeLocalSlots`, `UncompressLambdaMap`,
//!   `SerializeLambdaMap`, `UncompressStateMachineStates` and
//!   `SerializeStateMachineStates`).
//! - `src/Compilers/Core/Portable/CodeGen/LambdaDebugInfo.cs` (the closure
//!   ordinals).
//! - `src/Compilers/Core/Portable/SynthesizedLocalKind.cs` (the slot kinds).
//!
//! A syntax offset is the distance of a syntax node from the start of the
//! method body. It can be negative. Each map stores its syntax offsets as
//! compressed unsigned integers plus a negative baseline.

mod lambdas;
mod slots;
mod states;

pub use lambdas::{EncLambdaAndClosureMap, Lambda, LambdaClosure};
pub use slots::{EncLocalSlotMap, LocalSlot, MAX_LOCAL_SLOT_KIND};
pub use states::{EncStateMachineStateMap, StateMachineState};

use super::blob::{Reader, Writer};
use crate::Result;

/// Read a stored baseline: a compressed unsigned integer that holds the
/// negated baseline.
fn read_baseline(reader: &mut Reader<'_>) -> Result<i32> {
    // A compressed integer has at most 29 bits, so the negation fits.
    Ok(-reader.unsigned()?.cast_signed())
}

/// Read a syntax offset relative to `baseline`.
fn read_syntax_offset(reader: &mut Reader<'_>, baseline: i32) -> Result<i32> {
    // Both values have at most 29 bits, so the sum fits.
    Ok(reader.unsigned()?.cast_signed() + baseline)
}

/// Write the negated `baseline`.
fn write_baseline(writer: &mut Writer, baseline: i32) -> Result<()> {
    writer.unsigned_wide(-i64::from(baseline))
}

/// Write `syntax_offset` relative to `baseline`.
fn write_syntax_offset(writer: &mut Writer, syntax_offset: i32, baseline: i32) -> Result<()> {
    writer.unsigned_wide(i64::from(syntax_offset) - i64::from(baseline))
}
