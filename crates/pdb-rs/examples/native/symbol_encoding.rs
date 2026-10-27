//! Encode each decoded symbol record again and compare the bytes.

use pdb_rs::native::codeview::symbol::{ModuleSymbolWriter, Symbol};

use crate::{Outcome, Report};

/// Compare the records of `writer`, with recomputed scope links, with the
/// symbol records of a module stream.
pub(crate) fn rewrite_module(
    writer: Option<ModuleSymbolWriter>,
    original: &[u8],
    report: &mut Report,
) -> Outcome {
    let Some(writer) = writer else {
        return Ok(());
    };
    report.add("module symbol streams rewritten", 1);
    // The run stops at the first difference, so a finished run prints 0.
    report.add("module symbol streams rewritten with different bytes", 0);
    if writer.finish()? != original {
        return Err("a module symbol stream rewrites to different bytes".into());
    }
    Ok(())
}

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
    if symbol.trailing_zeros() >= 4 {
        // The HLSL compiler fxc writes S_LOCAL records of a fixed length.
        report.add("symbols with four or more padding bytes", 1);
    }
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
