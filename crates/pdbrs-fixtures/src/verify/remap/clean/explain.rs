//! The explanation of a clean plan that fails.
//!
//! A plan with `RemovedCode::Drop` fails with `UnsupportedEdit` for a record without a split
//! form (for example `S_BLOCK32` or an FPO record) whose code the ranges split, and for a
//! record whose code is only partly removed. The error names the record (`remap_address`) and
//! its source location (`stream_location`). The check accepts the failure only if a range
//! boundary cuts the code of that record: the set maps a part of the code, and not the whole
//! code as one part.
//!
//! The check finds the code of the record:
//!
//! - For a symbol record: it decodes the record at the source location of the error.
//! - For an FPO, new FPO or exception data record: it finds the record that starts at the
//!   address.
//! - Else, or if it cannot decode the record: it uses each procedure of the source that holds
//!   the address.

use pdbrs::native::codeview::symbol::{Symbol, SymbolData};
use pdbrs::native::codeview::{Records, SymbolKind};
use pdbrs::native::msf::StreamId;
use pdbrs::native::{Pdb, RemapAddress, RemapItem};
use pdbrs::{Error, ErrorKind};

use super::super::Source;
use super::super::ranges::RangeSet;

/// The symbol kinds without a split form.
const NO_SPLIT_SYMBOLS: [&str; 10] = [
    "S_BLOCK32",
    "S_WITH32",
    "S_THUNK32",
    "S_TRAMPOLINE",
    "S_CALLSITEINFO",
    "S_HEAPALLOCSITE",
    "S_HYBRIDRANGE",
    "S_SEPCODE",
    "S_INLINESITE",
    "S_INLINESITE2",
];

/// Return the name of a record kind: the `cvinfo.h` name of a symbol kind, else the name of
/// the item.
pub(super) fn item_name(item: RemapItem) -> String {
    match item {
        RemapItem::Symbol(kind) => format!("{kind:?}"),
        other => format!("{other:?}"),
    }
}

/// Return `true` for a record kind without a split form.
fn no_split(item: RemapItem) -> bool {
    match item {
        RemapItem::Symbol(kind) => kind
            .name()
            .is_some_and(|name| NO_SPLIT_SYMBOLS.contains(&name)),
        RemapItem::ModuleFrameData
        | RemapItem::Fpo
        | RemapItem::NewFpo
        | RemapItem::ExceptionData => true,
        _ => false,
    }
}

/// Return the code extent (image RVA and length) of the symbol record of `kind` at the
/// source location `location`, if the check can decode it.
fn symbol_extent(
    source: &Source,
    location: (StreamId, u64),
    kind: SymbolKind,
) -> Option<(u32, u32)> {
    let pdb = Pdb::parse(source.bytes.as_slice()).ok()?;
    let bytes = pdb.stream_bytes(location.0).ok()?;
    let rest = bytes.get(usize::try_from(location.1).ok()?..)?;
    let record = Records::new(rest).next()?.ok()?;
    if record.kind != kind.0 {
        return None;
    }
    let symbol = Symbol::decode(record).ok()?;
    let (address, length) = match symbol.data() {
        SymbolData::Procedure(procedure) => (procedure.address, procedure.code_length),
        SymbolData::Block(block) => (block.address, block.code_length),
        SymbolData::With(with) => (with.address, with.code_length),
        SymbolData::Thunk(thunk) => (thunk.address, u32::from(thunk.length)),
        SymbolData::Trampoline(trampoline) => (trampoline.thunk, u32::from(trampoline.thunk_size)),
        SymbolData::HeapAllocationSite(site) => (site.address, u32::from(site.instruction_length)),
        SymbolData::SeparatedCode(code) => (code.address, code.code_length),
        SymbolData::HybridRange(range) => (range.address, range.length),
        _ => return None,
    };
    Some((source.facts.section_rva(address)?, length))
}

/// Return the code extent of the record that `error` names, if the check can find it.
fn extent(source: &Source, error: &Error, address: RemapAddress, old: u32) -> Option<(u32, u32)> {
    let starting = |list: &[(u32, u32)]| list.iter().find(|extent| extent.0 == old).copied();
    match address.item {
        RemapItem::Symbol(kind) => symbol_extent(source, error.stream_location()?, kind),
        RemapItem::Fpo | RemapItem::NewFpo => starting(&source.facts.frames),
        RemapItem::ExceptionData => starting(&source.facts.exceptions),
        _ => None,
    }
}

/// Explain the failure `error` of a plan with `RemovedCode::Drop`. Return the class (`no split
/// form: <kind>` or `partly removed: <kind>`), or the problem if the set does not cut the
/// record.
pub(super) fn classify(source: &Source, set: &RangeSet, error: &Error) -> Result<String, String> {
    if error.kind() != ErrorKind::UnsupportedEdit {
        return Err(format!("the clean plan fails: {error}"));
    }
    let Some(address) = error.remap_address() else {
        return Err(format!(
            "the clean plan fails without a record address: {error}"
        ));
    };
    let name = item_name(address.item);
    let Some(old) = source.facts.image_rva(address.address.0) else {
        return Err(format!(
            "the clean plan fails at an address without an image RVA: {error}"
        ));
    };
    let cut =
        match extent(source, error, address, old) {
            Some((start, length)) => set.cuts(start, length),
            None => source.facts.procedures.iter().any(|&(start, length)| {
                old.wrapping_sub(start) < length && set.cuts(start, length)
            }),
        };
    if !cut {
        return Err(format!(
            "the clean plan fails for {name} at {old:#x}, but no range boundary cuts the record: {error}"
        ));
    }
    Ok(if no_split(address.item) {
        format!("no split form: {name}")
    } else {
        format!("partly removed: {name}")
    })
}

/// Check the failure `error` of a plan with `RemovedCode::Fail` for a set with removed code:
/// `UnsupportedEdit` with an address without a range.
pub(super) fn removed_code(source: &Source, set: &RangeSet, error: &Error) -> Result<(), String> {
    if error.kind() != ErrorKind::UnsupportedEdit {
        return Err(format!(
            "RemovedCode::Fail fails with {:?}, not UnsupportedEdit: {error}",
            error.kind()
        ));
    }
    let Some(address) = error.remap_address() else {
        return Err(format!(
            "RemovedCode::Fail fails without a record address: {error}"
        ));
    };
    match source.facts.image_rva(address.address.0) {
        Some(old) if set.map(old).is_some() => Err(format!(
            "RemovedCode::Fail names {} at {old:#x}, which has a range: {error}",
            item_name(address.item)
        )),
        _ => Ok(()),
    }
}
