//! Encode each decoded symbol record again and compare the bytes.

use pdbrs::native::codeview::symbol::Symbol;

use crate::{Outcome, Report};

/// Encode `symbol` and make sure that the bytes equal the original record.
///
/// The run fails at the first record that encodes to different bytes.
pub(crate) fn reencode(symbol: &Symbol<'_>, report: &mut Report) -> Outcome {
    let record = symbol.record();
    let mut original = Vec::with_capacity(record.data.len() + 4);
    original.extend_from_slice(&u16::try_from(record.data.len() + 2)?.to_le_bytes());
    original.extend_from_slice(&record.kind.to_le_bytes());
    original.extend_from_slice(record.data);
    let mut encoded = Vec::new();
    symbol.encode(&mut encoded)?;
    report.add("symbols re-encoded", 1);
    // The run stops at the first difference, so a finished run prints 0.
    report.add("symbols re-encoded with different bytes", 0);
    if encoded != original {
        return Err(format!(
            "symbol {:#06x} at offset {} re-encodes to different bytes",
            record.kind, record.offset
        )
        .into());
    }
    Ok(())
}
