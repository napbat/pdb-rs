//! The `.pdata` and `.xdata` streams of the new image for the clean output of `verify-remap`.
//!
//! pdb-rs does not decode the unwind information in `.xdata`, so the caller gives the streams
//! of the new image (`ImageRemap::with_unwind_streams`). The range sets move only code
//! sections, and the `.xdata` section has an identity range. The check thus keeps the
//! `.xdata` stream and rewrites each `.pdata` runtime function through the ranges:
//!
//! - Each part of the function code with a range gets one entry, with the new start of the
//!   part and the same unwind information. A split function thus gets one entry for each
//!   part. The bytes without a range get no entry.
//! - The unwind information address maps through the ranges. An address without a range
//!   stays.
//! - The entries are sorted by the new start.
//!
//! An x64 entry gives the function end. An ARM64 entry gives the function length in the
//! packed unwind data (flag 1 or 2), or else in the first word of its `.xdata` record.

use pdb_rs::native::address::Rva;
use pdb_rs::native::debug_stream::{
    Arm64RuntimeFunction, DebugRecord, RuntimeFunctions, RvaVaBlob, X64RuntimeFunction,
};
use pdb_rs::native::section::Machine;

use super::ranges::RangeSet;

/// One runtime function: the code from `begin` for `length` bytes, with its unwind word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Function {
    /// The RVA of the first byte.
    pub begin: u32,
    /// The length of the code in bytes.
    pub length: u32,
    /// The unwind information address (x64) or the unwind word (ARM64).
    pub unwind: u32,
}

/// Return the runtime functions of the new image: one function for each part of `functions`
/// with a range in `set`, sorted by the new start. `map_unwind` is `true` if the unwind word
/// is an address.
pub(super) fn rewrite(functions: &[Function], set: &RangeSet, map_unwind: bool) -> Vec<Function> {
    let mut out: Vec<Function> = functions
        .iter()
        .flat_map(|function| {
            let unwind = if map_unwind {
                set.map(function.unwind).unwrap_or(function.unwind)
            } else {
                function.unwind
            };
            set.parts(function.begin, function.length)
                .into_iter()
                .map(move |part| Function {
                    begin: part.new,
                    length: part.length,
                    unwind,
                })
        })
        .collect();
    out.sort_unstable();
    out
}

/// Return the function length of an ARM64 entry, or 1 if neither the packed data nor the
/// `.xdata` record gives it.
fn arm64_length(entry: Arm64RuntimeFunction, xdata: &RvaVaBlob) -> u32 {
    let length = if entry.flag() == 0 {
        xdata
            .data_at(Rva(entry.unwind_data))
            .and_then(|data| data.first_chunk::<4>())
            .map_or(0, |word| (u32::from_le_bytes(*word) & 0x3_ffff) * 4)
    } else {
        ((entry.unwind_data >> 2) & 0x7ff) * 4
    };
    length.max(1)
}

/// Return `true` if an ARM64 unwind word is the address of an `.xdata` record.
const fn arm64_unwind_is_address(unwind_data: u32) -> bool {
    unwind_data.trailing_zeros() >= 2
}

/// Return the `.pdata` and `.xdata` streams of the new image of `set`, from the source
/// streams `pdata` and `xdata` of an image for `machine`.
///
/// # Errors
///
/// Returns an error if pdb-rs cannot decode the runtime functions of `machine` or cannot
/// encode the new stream.
pub(super) fn unwind_streams(
    pdata: &RvaVaBlob,
    xdata: &RvaVaBlob,
    machine: Machine,
    set: &RangeSet,
) -> Result<(RvaVaBlob, RvaVaBlob), String> {
    let error = |error: pdb_rs::Error| format!(".pdata: {error}");
    let functions = pdata
        .runtime_functions(machine)
        .map_err(error)?
        .ok_or_else(|| {
            format!(".pdata: pdb-rs does not decode the runtime functions of {machine:?}")
        })?;
    let mut data = Vec::new();
    match functions {
        RuntimeFunctions::X64(entries) => {
            let list: Vec<Function> = entries
                .map(|entry| Function {
                    begin: entry.begin,
                    length: entry.end.saturating_sub(entry.begin),
                    unwind: entry.unwind_info,
                })
                .collect();
            for function in rewrite(&list, set, true) {
                let mut entry = X64RuntimeFunction::default();
                entry.begin = function.begin;
                entry.end = function.begin + function.length;
                entry.unwind_info = function.unwind;
                entry.encode(&mut data).map_err(error)?;
            }
        }
        RuntimeFunctions::Arm64(entries) => {
            let list: Vec<Function> = entries
                .map(|entry| Function {
                    begin: entry.begin,
                    length: arm64_length(entry, xdata),
                    unwind: entry.unwind_data,
                })
                .collect();
            let mut new = Vec::with_capacity(list.len());
            for function in &list {
                let map = arm64_unwind_is_address(function.unwind);
                new.extend(rewrite(std::slice::from_ref(function), set, map));
            }
            new.sort_unstable();
            for function in new {
                let mut entry = Arm64RuntimeFunction::default();
                entry.begin = function.begin;
                entry.unwind_data = function.unwind;
                entry.encode(&mut data).map_err(error)?;
            }
        }
        _ => {
            return Err(format!(
                ".pdata: unknown runtime function layout of {machine:?}"
            ));
        }
    }
    let pdata = RvaVaBlob::new(pdata.header(), pdata.extra_header(), &data).map_err(error)?;
    Ok((pdata, xdata.clone()))
}

#[cfg(test)]
mod tests {
    use super::super::ranges::{Range, SetKind};
    use super::*;

    /// `.text` at 0x1000: 0x1000 to 0x1010 moves to 0x5000, 0x1010 to 0x1020 has no range,
    /// 0x1020 to 0x1030 moves to 0x4000 and 0x1030 to 0x1040 moves to 0x6000. `.xdata` at
    /// 0x3000 keeps its RVAs.
    fn set() -> RangeSet {
        let range = |old, length, new| Range { old, length, new };
        RangeSet {
            kind: SetKind::Split,
            sections: Vec::new(),
            ranges: vec![
                range(0x1000, 0x10, 0x5000),
                range(0x1020, 0x10, 0x4000),
                range(0x1030, 0x10, 0x6000),
                range(0x3000, 0x100, 0x3000),
            ],
        }
    }

    fn function(begin: u32, length: u32, unwind: u32) -> Function {
        Function {
            begin,
            length,
            unwind,
        }
    }

    #[test]
    fn a_split_function_gets_one_entry_for_each_part() {
        let new = rewrite(&[function(0x1020, 0x20, 0x3010)], &set(), true);
        assert_eq!(
            new,
            [
                function(0x4000, 0x10, 0x3010),
                function(0x6000, 0x10, 0x3010)
            ]
        );
    }

    #[test]
    fn entries_are_sorted_by_the_new_start() {
        let new = rewrite(
            &[
                function(0x1000, 0x10, 0x3000),
                function(0x1020, 0x10, 0x3020),
            ],
            &set(),
            true,
        );
        assert_eq!(
            new,
            [
                function(0x4000, 0x10, 0x3020),
                function(0x5000, 0x10, 0x3000)
            ]
        );
    }

    #[test]
    fn code_without_a_range_gets_no_entry() {
        assert_eq!(rewrite(&[function(0x1010, 0x10, 0x3000)], &set(), true), []);
        // A partly removed function keeps the part with a range.
        assert_eq!(
            rewrite(&[function(0x1008, 0x10, 0x3000)], &set(), true),
            [function(0x5008, 0x8, 0x3000)]
        );
    }

    #[test]
    fn packed_unwind_data_does_not_map() {
        let packed = 0x0000_0045;
        assert!(!arm64_unwind_is_address(packed));
        assert_eq!(
            rewrite(&[function(0x1000, 0x10, packed)], &set(), false),
            [function(0x5000, 0x10, packed)]
        );
    }

    #[test]
    fn x64_streams_round_trip_through_the_blob_codec() {
        let mut data = Vec::new();
        for (begin, end, unwind) in [(0x1000, 0x1010, 0x3000), (0x1020, 0x1040, 0x3010)] {
            let mut entry = X64RuntimeFunction::default();
            entry.begin = begin;
            entry.end = end;
            entry.unwind_info = unwind;
            entry.encode(&mut data).unwrap();
        }
        let header = pdb_rs::native::debug_stream::BlobHeader::default();
        let pdata = RvaVaBlob::new(header, &[], &data).unwrap();
        let xdata = RvaVaBlob::new(header, &[], &[0; 16]).unwrap();
        let (new, kept) = unwind_streams(&pdata, &xdata, Machine::AMD64, &set()).unwrap();
        assert_eq!(kept, xdata);
        let Some(RuntimeFunctions::X64(entries)) = new.runtime_functions(Machine::AMD64).unwrap()
        else {
            panic!("no x64 entries");
        };
        let entries: Vec<(u32, u32, u32)> = entries
            .map(|entry| (entry.begin, entry.end, entry.unwind_info))
            .collect();
        assert_eq!(
            entries,
            [
                (0x4000, 0x4010, 0x3010),
                (0x5000, 0x5010, 0x3000),
                (0x6000, 0x6010, 0x3010)
            ]
        );
    }
}
