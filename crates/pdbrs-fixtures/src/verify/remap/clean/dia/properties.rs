//! The section and offset properties of `DIA2Dump -sym` without an RVA.
//!
//! `-sym` prints the exception handler of a procedure and the parent of an `S_SEPCODE` scope
//! as a section property and an offset property ([`SECTION_OFFSETS`]). DIA keeps the source
//! section and offset in the OMAP output, and the clean output has new ones. The check thus
//! replaces each pair with the new RVA ([`normalize_answers`]).

use pdbrs::native::address::SectionOffset;

use super::{normalize, property};

/// The pairs of section and offset properties that `DIA2Dump -sym` prints without an RVA:
/// the section property, the offset property and the name of the address.
const SECTION_OFFSETS: [(&str, &str, &str); 2] = [
    (
        "section number of exception handler",
        "offset of exception handler",
        "exception handler",
    ),
    (
        "associated symbol section",
        "associated symbol offset",
        "associated symbol",
    ),
];

/// Return the value of a hexadecimal property line (`name:<tab> 0x...`).
fn hex_property(line: &str) -> Option<u32> {
    let value = line.split_once(":\t")?.1.trim();
    u32::from_str_radix(value.strip_prefix("0x").unwrap_or(value), 16).ok()
}

/// Replace each pair of [`SECTION_OFFSETS`] properties in `lines` with one line:
/// `<name> RVA (section and offset):<tab> 0x<new RVA>` with the RVA that `rva` gives, or
/// `<name> without an RVA:<tab> <section>:<offset>` if `rva` gives none.
fn section_offsets(lines: &mut Vec<String>, rva: &dyn Fn(SectionOffset) -> Option<u32>) {
    for (section_name, offset_name, name) in SECTION_OFFSETS {
        let find = |lines: &[String], wanted: &str| {
            lines.iter().position(|line| property(line) == Some(wanted))
        };
        let (Some(at), Some(offset_at)) = (find(lines, section_name), find(lines, offset_name))
        else {
            continue;
        };
        let (Some(section), Some(offset)) =
            (hex_property(&lines[at]), hex_property(&lines[offset_at]))
        else {
            continue;
        };
        let address = u16::try_from(section)
            .ok()
            .and_then(|section| rva(SectionOffset { section, offset }));
        lines[at] = match address {
            Some(address) => format!("{name} RVA (section and offset):\t 0x{address:X}"),
            None => format!("{name} without an RVA:\t {section:X}:{offset:X}"),
        };
        lines.remove(offset_at);
    }
}

/// Normalize the lines of each answer, and replace the [`SECTION_OFFSETS`] pairs with the
/// new RVA that `rva` gives.
pub(super) fn normalize_answers(
    answers: &[Option<Vec<String>>],
    rva: &dyn Fn(SectionOffset) -> Option<u32>,
) -> Vec<Option<Vec<String>>> {
    answers
        .iter()
        .map(|answer| {
            answer.as_ref().map(|lines| {
                let mut lines: Vec<String> =
                    lines.iter().filter_map(|line| normalize(line)).collect();
                section_offsets(&mut lines, rva);
                lines
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_offset_pairs_become_rvas() {
        // The OMAP output keeps section 1, which moves from RVA 0x1000 to RVA 0x4000. The
        // clean output has section 2 at RVA 0x4000.
        let omap_rva =
            |address: SectionOffset| (address.section == 1).then_some(0x4000 + address.offset);
        let clean_rva =
            |address: SectionOffset| (address.section == 2).then_some(0x4000 + address.offset);
        let answer = |section: u16, offset: u32| {
            Some(
                [
                    "                          SymTag:\t 0x5".to_owned(),
                    format!("section number of exception handler:\t 0x{section:X}"),
                    format!("     offset of exception handler:\t 0x{offset:X}"),
                    format!("       associated symbol section:\t 0x{section:X}"),
                    format!("        associated symbol offset:\t 0x{offset:X}"),
                ]
                .to_vec(),
            )
        };
        let omap = normalize_answers(&[answer(1, 0x4)], &omap_rva);
        let clean = normalize_answers(&[answer(2, 0x4)], &clean_rva);
        assert_eq!(omap, clean);
        assert_eq!(
            omap[0].as_ref().unwrap()[1..],
            [
                "exception handler RVA (section and offset):\t 0x4004",
                "associated symbol RVA (section and offset):\t 0x4004"
            ]
        );
        // Another clean offset is a difference.
        assert_ne!(omap, normalize_answers(&[answer(2, 0x8)], &clean_rva));
        // An address without an RVA keeps its section and offset.
        let none = normalize_answers(&[answer(0, 0)], &clean_rva);
        assert_eq!(
            none[0].as_ref().unwrap()[1],
            "exception handler without an RVA:\t 0:0"
        );
        assert_eq!(none, normalize_answers(&[answer(0, 0)], &omap_rva));
    }
}
