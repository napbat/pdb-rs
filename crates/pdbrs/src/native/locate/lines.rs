//! The line entry of a `DEBUG_S_LINES` subsection that holds an offset.

use crate::native::lines::{ColumnEntry, LineEntry, LinesSubsection};

/// The line entry that holds a code offset.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct LineMatch {
    /// The byte offset of the file entry in the `DEBUG_S_FILECHKSMS`
    /// subsection of the module.
    pub checksum_offset: u32,
    /// The line entry.
    pub entry: LineEntry,
    /// The column entry of the line entry, if the subsection has columns.
    pub column: Option<ColumnEntry>,
    /// The number of bytes from the entry offset to the next larger entry
    /// offset, or to the end of the code contribution after the last entry.
    pub length: u32,
}

/// Find the line entry that holds `offset`.
///
/// `offset` is relative to the start of the code contribution
/// ([`LinesHeader::code_offset`](crate::native::lines::LinesHeader::code_offset)).
/// The entry that holds it has the largest entry offset at or below
/// `offset`. Entries can share an offset: for example, MSVC writes the line
/// of the function name and the line of the opening brace at the start of
/// a function. All entries but the last one of such a group have an empty
/// range. The first entry of the group holds the shared offset itself, and
/// the last entry holds the other offsets of its range. DIA
/// (`findLinesByRVA`) and `llvm-symbolizer` give the same entries. The
/// range ends at the next larger entry offset, or at the code size after
/// the last entry.
///
/// Return `None` if `offset` is at or after the code size, or before the
/// first entry.
#[must_use]
pub fn find_line(lines: &LinesSubsection<'_>, offset: u32) -> Option<LineMatch> {
    let code_size = lines.header().code_size;
    if offset >= code_size {
        return None;
    }
    let mut found: Option<LineMatch> = None;
    let mut end = code_size;
    'blocks: for block in lines.blocks() {
        for (position, entry) in block.lines.iter().enumerate() {
            if entry.offset > offset {
                // `LinesSubsection::decode` checked that offsets do not
                // decrease, so this entry ends the found range.
                end = entry.offset;
                break 'blocks;
            }
            if entry.offset == offset
                && let Some(first) = &mut found
                && first.entry.offset == offset
            {
                // A later entry has the same offset, so the range of the
                // first entry is empty.
                first.length = 0;
                return found;
            }
            found = Some(LineMatch {
                checksum_offset: block.checksum_offset,
                entry,
                column: block.columns.and_then(|columns| columns.get(position)),
                length: 0,
            });
        }
    }
    found.map(|line| LineMatch {
        length: end - line.entry.offset,
        ..line
    })
}
