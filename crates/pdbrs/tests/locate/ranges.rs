//! Inline annotation ranges and line entry lookup.

use pdbrs::ErrorKind;
use pdbrs::native::codeview::symbol::BinaryAnnotations;
use pdbrs::native::lines::LinesSubsection;
use pdbrs::native::locate::{InlineRange, InlineRanges, find_line};

use crate::{annotation, lines_subsection, signed};

/// Run `bytes` from file 0 and line 100 and return the ranges.
fn ranges(bytes: &[u8]) -> pdbrs::Result<Vec<InlineRange>> {
    let annotations = BinaryAnnotations::new(bytes).unwrap();
    InlineRanges::new(&annotations, 0, 100).collect()
}

#[test]
fn code_length_ends_a_range_and_moves_the_offset_past_it() {
    let bytes = [
        annotation(6, &[signed(1)]),
        annotation(3, &[0x10]),
        annotation(4, &[8]),
        annotation(5, &[0x18]),
        annotation(6, &[signed(2)]),
        annotation(3, &[8]),
        annotation(4, &[4]),
    ]
    .concat();
    let ranges = ranges(&bytes).unwrap();
    assert_eq!(ranges.len(), 2);
    let [first, second] = [ranges[0], ranges[1]];
    assert_eq!(
        (
            first.chunk,
            first.code_offset,
            first.code_length,
            first.file,
            first.line
        ),
        (0, 0x10, Some(8), 0, 101)
    );
    // The second range starts after a gap: 0x18 plus 8.
    assert_eq!(
        (
            second.chunk,
            second.code_offset,
            second.code_length,
            second.file,
            second.line
        ),
        (0, 0x20, Some(4), 0x18, 103)
    );
}

#[test]
fn a_range_without_length_ends_at_the_next_range() {
    // ChangeCodeOffsetAndLineOffset: code delta 4 in bits 0 to 3, line +1.
    let combined = 4 | (signed(1) << 4);
    let bytes = [
        annotation(3, &[0x10]),
        annotation(11, &[combined]),
        annotation(4, &[2]),
    ]
    .concat();
    let ranges = ranges(&bytes).unwrap();
    assert_eq!(
        ranges
            .iter()
            .map(|range| (range.code_offset, range.code_length, range.line))
            .collect::<Vec<_>>(),
        [(0x10, Some(4), 100), (0x14, Some(2), 101)]
    );
}

#[test]
fn code_length_and_offset_give_an_explicit_length() {
    let bytes = [annotation(12, &[3, 0x20]), annotation(12, &[5, 0x10])].concat();
    let ranges = ranges(&bytes).unwrap();
    assert_eq!(
        ranges
            .iter()
            .map(|range| (range.code_offset, range.code_length))
            .collect::<Vec<_>>(),
        [(0x20, Some(3)), (0x30, Some(5))]
    );
}

#[test]
fn the_last_range_without_length_ends_with_its_chunk() {
    let ranges = ranges(&annotation(3, &[0x10])).unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(
        (
            ranges[0].chunk,
            ranges[0].code_offset,
            ranges[0].code_length
        ),
        (0, 0x10, None)
    );
}

#[test]
fn code_offset_base_selects_a_separated_chunk() {
    let bytes = [
        // A range of chunk 0 without a length.
        annotation(3, &[0x10]),
        // Chunk 1 ends that range and sets the code offset to 0.
        annotation(2, &[1]),
        annotation(6, &[signed(1)]),
        annotation(1, &[4]),
        annotation(3, &[0]),
        annotation(4, &[2]),
        // Chunk 2 also starts at code offset 0.
        annotation(2, &[2]),
        annotation(12, &[3, 1]),
    ]
    .concat();
    let ranges = ranges(&bytes).unwrap();
    assert_eq!(
        ranges
            .iter()
            .map(|range| (
                range.chunk,
                range.code_offset,
                range.code_length,
                range.line
            ))
            .collect::<Vec<_>>(),
        [
            (0, 0x10, None, 100),
            (1, 4, Some(2), 101),
            (2, 1, Some(3), 101)
        ]
    );
}

#[test]
fn columns_and_range_kinds_apply_to_later_ranges() {
    let bytes = [
        annotation(9, &[0]),
        annotation(3, &[0]),
        annotation(9, &[5]),
        annotation(13, &[9]),
        annotation(10, &[signed(-2)]),
        annotation(8, &[0]),
        annotation(7, &[3]),
        annotation(3, &[2]),
        annotation(4, &[1]),
    ]
    .concat();
    let ranges = ranges(&bytes).unwrap();
    assert_eq!(ranges[0].column_start, None);
    assert!(ranges[0].is_statement);
    assert_eq!(ranges[0].line_count, 1);
    assert_eq!(ranges[1].column_start, Some(5));
    assert_eq!(ranges[1].column_end, Some(7));
    assert!(!ranges[1].is_statement);
    assert_eq!(ranges[1].line_count, 3);
}

#[test]
fn invalid_programs_give_errors() {
    let kind = |bytes: &[u8]| ranges(bytes).unwrap_err().kind();
    // A line below 0.
    let negative = [annotation(6, &[signed(-101)]), annotation(3, &[0])].concat();
    assert_eq!(kind(&negative), ErrorKind::InvalidFormat);
    // A range that starts before the open range.
    let backwards = [
        annotation(3, &[0x10]),
        annotation(1, &[0]),
        annotation(3, &[0]),
    ]
    .concat();
    assert_eq!(kind(&backwards), ErrorKind::InvalidFormat);
    // A code offset past 32 bits.
    let mut overflow = annotation(1, &[0x1fff_ffff]);
    for _ in 0..8 {
        overflow.extend_from_slice(&annotation(3, &[0x1fff_ffff]));
    }
    assert_eq!(kind(&overflow), ErrorKind::OffsetOverflow);
    // A range kind other than 0 or 1.
    assert_eq!(kind(&annotation(8, &[2])), ErrorKind::InvalidFormat);
}

#[test]
fn errors_stop_the_iterator() {
    let bytes = [annotation(8, &[2]), annotation(3, &[0])].concat();
    let annotations = BinaryAnnotations::new(&bytes).unwrap();
    let mut iterator = InlineRanges::new(&annotations, 0, 1);
    assert!(iterator.next().unwrap().is_err());
    assert!(iterator.next().is_none());
}

/// A subsection of 0x40 bytes with two blocks; the first block shares offset 0.
fn two_blocks() -> Vec<u8> {
    lines_subsection(
        0x10,
        0x40,
        &[
            (0, &[(0x4, 10), (0x4, 11), (0xc, 12)]),
            (8, &[(0x20, 20), (0x30, 30)]),
        ],
    )
}

#[test]
fn find_line_boundaries() {
    let bytes = two_blocks();
    let lines = LinesSubsection::decode(&bytes).unwrap();
    let line =
        |offset| find_line(&lines, offset).map(|found| (found.entry.line_start, found.length));
    // Before the first entry.
    assert_eq!(line(0x3), None);
    // The first entry of the shared offset holds the offset itself.
    assert_eq!(line(0x4), Some((10, 0)));
    // The last entry of the group holds the rest of the range.
    assert_eq!(line(0x5), Some((11, 8)));
    assert_eq!(line(0xb), Some((11, 8)));
    assert_eq!(line(0xc), Some((12, 0x14)));
    // The last byte of the first block and the first byte of the next block.
    assert_eq!(line(0x1f), Some((12, 0x14)));
    let found = find_line(&lines, 0x20).unwrap();
    assert_eq!((found.entry.line_start, found.checksum_offset), (20, 8));
    // The last byte of the contribution and the byte after it.
    assert_eq!(line(0x3f), Some((30, 0x10)));
    assert_eq!(line(0x40), None);
    assert_eq!(line(u32::MAX), None);
}
