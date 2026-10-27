//! The `SymbolLocator` comparison of the clean output with the OMAP output (decision 12 of
//! #43).
//!
//! At each sampled new RVA, `SymbolLocator::locate` must give the same answer for both
//! outputs: the procedure name, the RVA and length of the innermost block, the inline call
//! chain (the names and the call site lines) and the line (file and line number). The OMAP
//! output translates the addresses through OMAP; the clean output has the new addresses in
//! its records.

use std::borrow::Cow;

use pdbrs::native::Pdb;
use pdbrs::native::address::Rva;
use pdbrs::native::locate::{Location, SourceLine, SymbolLocator};

use super::{Sample, Tally};

/// A line: the file and the line number.
type Line = (String, u32);

/// The compared parts of a location.
#[derive(Debug, Default, PartialEq, Eq)]
struct Summary {
    /// The procedure name.
    procedure: Option<String>,
    /// The RVA and the length of the innermost block.
    block: Option<(u32, u32)>,
    /// The name and the call site line of each inline frame, innermost first.
    inlines: Vec<(String, Option<Line>)>,
    /// The line.
    line: Option<Line>,
}

/// Return bytes as text.
fn text(bytes: &[u8]) -> String {
    match String::from_utf8_lossy(bytes) {
        Cow::Borrowed(text) => text.to_owned(),
        Cow::Owned(text) => text,
    }
}

/// Return the file and the line number of a line.
fn line(line: &SourceLine<'_>) -> Line {
    (text(line.file), line.line)
}

/// Return the compared parts of a location.
fn summary(location: &Location<'_>) -> Summary {
    Summary {
        procedure: location.procedure.map(|procedure| text(procedure.name)),
        block: location.block.map(|block| (block.start.0, block.length)),
        inlines: location
            .inline_frames
            .iter()
            .map(|frame| {
                (
                    text(frame.qualified_name),
                    frame.call_site.as_ref().map(line),
                )
            })
            .collect(),
        line: location.line.as_ref().map(line),
    }
}

/// Return the answer of `locator` at `rva`.
fn answer(locator: &SymbolLocator, rva: u32) -> Result<Option<Summary>, String> {
    locator
        .locate(Rva(rva))
        .map(|location| location.as_ref().map(summary))
        .map_err(|error| error.to_string())
}

/// Compare `SymbolLocator::locate` on the OMAP output `omap` and the clean output `clean`
/// at the new RVA of each sample.
///
/// # Errors
///
/// Returns an error if pdbrs cannot read an output or make its locator.
pub(super) fn compare(
    omap: &[u8],
    clean: &[u8],
    samples: &[Sample],
    tally: &mut Tally,
) -> Result<(), String> {
    let locator = |bytes: &[u8], name: &str| {
        Pdb::parse(bytes)
            .and_then(|pdb| SymbolLocator::new(&pdb))
            .map_err(|error| format!("SymbolLocator on the {name} output: {error}"))
    };
    let omap = locator(omap, "OMAP")?;
    let clean = locator(clean, "clean")?;
    for sample in samples {
        let expected = answer(&omap, sample.new);
        let actual = answer(&clean, sample.new);
        tally.compared += 1;
        if expected != actual {
            tally.push(|| {
                format!(
                    "SymbolLocator::locate({:#x}) (old RVA {:#x}) gives {actual:?} on the clean output, {expected:?} on the OMAP output",
                    sample.new, sample.old
                )
            });
        }
    }
    Ok(())
}
