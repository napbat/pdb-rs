//! `DEBUG_S_LINES` subsections: line blocks of one code contribution.

use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::{entry_list, invalid, put_u16, put_u32, read_u32, to_u32, too_large, truncated};
use crate::{Error, ErrorKind, Result};

/// The line number that tells a debugger to always step into the code
/// (`0xfeefee`).
pub const ALWAYS_STEP_INTO_LINE: u32 = 0x00fe_efee;

/// The line number that tells a debugger to never step into the code
/// (`0xf00f00`).
pub const NEVER_STEP_INTO_LINE: u32 = 0x00f0_0f00;

/// The header flag that tells that each block has column entries
/// (`CV_LINES_HAVE_COLUMNS`).
const HAVE_COLUMNS: u16 = 0x1;

/// The size of the subsection header.
const HEADER_SIZE: usize = 12;

/// The size of a block header: a checksum offset, a count and a size.
const BLOCK_HEADER_SIZE: usize = 12;

/// The size of one line entry.
const LINE_SIZE: usize = 8;

/// The size of one column entry.
const COLUMN_SIZE: usize = 4;

/// The mask of the start line in the line flags.
const START_LINE_MASK: u32 = 0x00ff_ffff;

/// The shift of the end line delta in the line flags.
const END_DELTA_SHIFT: u32 = 24;

/// The mask of the end line delta after the shift.
const END_DELTA_MASK: u32 = 0x7f;

/// The statement bit in the line flags.
const STATEMENT_BIT: u32 = 0x8000_0000;

/// The header of a `DEBUG_S_LINES` subsection (`CV_DebugSLinesHeader_t`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct LinesHeader {
    /// The section offset of the code contribution.
    pub code_offset: u32,
    /// The section number of the code contribution.
    pub segment: u16,
    /// The raw flags. Only bit 0 ([`LinesHeader::has_columns`]) is defined.
    pub flags: u16,
    /// The size of the code contribution in bytes.
    pub code_size: u32,
}

impl LinesHeader {
    /// Create a header from its fields.
    #[must_use]
    pub const fn new(code_offset: u32, segment: u16, flags: u16, code_size: u32) -> Self {
        Self {
            code_offset,
            segment,
            flags,
            code_size,
        }
    }

    /// Return `true` if each block has column entries.
    #[must_use]
    pub const fn has_columns(&self) -> bool {
        self.flags & HAVE_COLUMNS != 0
    }

    /// Append the 12-byte header to `out`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the flags have a bit other than
    ///   the column flag.
    /// - [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        if self.flags & !HAVE_COLUMNS != 0 {
            return Err(invalid("line subsection header has unknown flags"));
        }
        put_u32(out, self.code_offset)?;
        put_u16(out, self.segment)?;
        put_u16(out, self.flags)?;
        put_u32(out, self.code_size)
    }
}

/// A decoded `DEBUG_S_LINES` subsection.
///
/// [`LinesSubsection::decode`] validates all blocks. The block iterator
/// then cannot fail.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct LinesSubsection<'a> {
    /// The subsection header.
    header: LinesHeader,
    /// The validated blocks after the header.
    blocks: &'a [u8],
    /// The number of blocks.
    block_count: usize,
}

impl<'a> LinesSubsection<'a> {
    /// Decode and validate the data of a `DEBUG_S_LINES` subsection.
    ///
    /// These rules apply:
    ///
    /// - The header flags contain only the column flag.
    /// - The size of each block is exactly its header, its line entries and,
    ///   with the column flag, its column entries. Thus the bytes of blocks
    ///   cannot overlap and no unaccounted bytes are between blocks.
    /// - The code offset of each line entry is below the code size in the
    ///   header.
    /// - The code offsets do not decrease, also across blocks. Thus the code
    ///   ranges of blocks cannot overlap. The Microsoft `pdb-rs`
    ///   documentation (`line_data.md`) requires sorted blocks.
    /// - The blocks fill the data exactly.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] if the header, a block header or the
    ///   entries of a block are truncated.
    /// - [`ErrorKind::UnsupportedVersion`] if the header has unknown flags.
    /// - [`ErrorKind::InvalidFormat`] if a block size does not match its
    ///   entries, if a line offset is outside the code contribution, or if
    ///   a line offset is below the offset of the entry before it.
    /// - [`ErrorKind::LimitExceeded`] if a block size does not fit in memory.
    ///
    /// The errors do not have a file offset or a stream location.
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        let (header_bytes, blocks) = data
            .split_first_chunk::<HEADER_SIZE>()
            .ok_or_else(|| truncated("line subsection header is truncated"))?;
        let [o0, o1, o2, o3, s0, s1, f0, f1, c0, c1, c2, c3] = *header_bytes;
        let header = LinesHeader {
            code_offset: u32::from_le_bytes([o0, o1, o2, o3]),
            segment: u16::from_le_bytes([s0, s1]),
            flags: u16::from_le_bytes([f0, f1]),
            code_size: u32::from_le_bytes([c0, c1, c2, c3]),
        };
        if header.flags & !HAVE_COLUMNS != 0 {
            return Err(Error::new(ErrorKind::UnsupportedVersion, None)
                .with_context("line subsection header has unknown flags"));
        }
        let mut rest = blocks;
        let mut block_count = 0usize;
        let mut previous = 0;
        while !rest.is_empty() {
            let (block, next) = split_block(rest, header)?;
            for line in block.lines {
                if line.offset >= header.code_size {
                    return Err(invalid("line offset is outside the code contribution"));
                }
                if line.offset < previous {
                    return Err(invalid("line offsets decrease, so line blocks overlap"));
                }
                previous = line.offset;
            }
            rest = next;
            block_count += 1;
        }
        Ok(Self {
            header,
            blocks,
            block_count,
        })
    }

    /// Return the subsection header.
    #[must_use]
    pub const fn header(&self) -> &LinesHeader {
        &self.header
    }

    /// Return the number of blocks.
    #[must_use]
    pub const fn block_count(&self) -> usize {
        self.block_count
    }

    /// Return an iterator over the blocks.
    #[must_use]
    pub const fn blocks(&self) -> LineBlocks<'a> {
        LineBlocks {
            bytes: self.blocks,
            header: self.header,
            left: self.block_count,
        }
    }

    /// Append the subsection data to `out`: the header and all blocks.
    ///
    /// The output is equal to the data that [`LinesSubsection::decode`]
    /// read.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`LinesHeader::encode`] and
    /// [`LineBlock::encode`].
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        self.header.encode(out)?;
        for block in self.blocks() {
            block.encode(out)?;
        }
        Ok(())
    }
}

/// The lines of one source file in a `DEBUG_S_LINES` subsection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct LineBlock<'a> {
    /// The byte offset of the file entry in the `DEBUG_S_FILECHKSMS`
    /// subsection of the same module.
    pub checksum_offset: u32,
    /// The line entries.
    pub lines: LineEntries<'a>,
    /// The column entries, if the header has the column flag.
    ///
    /// The list has one entry for each line entry, in the same order.
    pub columns: Option<ColumnEntries<'a>>,
}

impl LineBlock<'_> {
    /// Append the block to `out`: the block header, the line entries and
    /// the column entries.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`encode_line_block`].
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        write_block(
            self.checksum_offset,
            self.lines.iter(),
            self.columns.map(|columns| columns.iter()),
            out,
        )
    }
}

/// Append one block of a `DEBUG_S_LINES` subsection to `out`.
///
/// Give `columns` only if the subsection header has the column flag
/// ([`LinesHeader::has_columns`]). Give one column entry for each line
/// entry. The function computes the block size.
///
/// # Errors
///
/// - [`ErrorKind::InvalidFormat`] if the number of column entries is not
///   the number of line entries, or if a line entry does not fit its bit
///   fields ([`LineEntry::encode`]).
/// - [`ErrorKind::LimitExceeded`] if the count or the block size does not
///   fit in 32 bits, or if the output cannot grow.
pub fn encode_line_block(
    checksum_offset: u32,
    lines: &[LineEntry],
    columns: Option<&[ColumnEntry]>,
    out: &mut Vec<u8>,
) -> Result<()> {
    write_block(
        checksum_offset,
        lines.iter().copied(),
        columns.map(|columns| columns.iter().copied()),
        out,
    )
}

/// Append one block with the entries of `lines` and `columns` to `out`.
fn write_block(
    checksum_offset: u32,
    lines: impl ExactSizeIterator<Item = LineEntry>,
    columns: Option<impl ExactSizeIterator<Item = ColumnEntry>>,
    out: &mut Vec<u8>,
) -> Result<()> {
    let count = lines.len();
    let entry_size = match &columns {
        Some(columns) if columns.len() != count => {
            return Err(invalid(
                "line block has a different number of column entries and line entries",
            ));
        }
        Some(_) => LINE_SIZE + COLUMN_SIZE,
        None => LINE_SIZE,
    };
    let size = count
        .checked_mul(entry_size)
        .and_then(|size| size.checked_add(BLOCK_HEADER_SIZE))
        .ok_or_else(|| too_large("line block size does not fit in 32 bits"))?;
    put_u32(out, checksum_offset)?;
    put_u32(
        out,
        to_u32(count, "line block count does not fit in 32 bits")?,
    )?;
    put_u32(
        out,
        to_u32(size, "line block size does not fit in 32 bits")?,
    )?;
    for line in lines {
        line.encode(out)?;
    }
    for column in columns.into_iter().flatten() {
        column.encode(out)?;
    }
    Ok(())
}

/// One line entry (`CV_Line_t`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct LineEntry {
    /// The code offset relative to [`LinesHeader::code_offset`].
    pub offset: u32,
    /// The first line number (bits 0 to 23).
    ///
    /// [`ALWAYS_STEP_INTO_LINE`] and [`NEVER_STEP_INTO_LINE`] are special
    /// values.
    pub line_start: u32,
    /// The number of lines after the first line (bits 24 to 30).
    pub end_delta: u8,
    /// `true` if the entry starts a statement (bit 31).
    pub is_statement: bool,
}

impl LineEntry {
    /// Create a line entry from its fields.
    #[must_use]
    pub const fn new(offset: u32, line_start: u32, end_delta: u8, is_statement: bool) -> Self {
        Self {
            offset,
            line_start,
            end_delta,
            is_statement,
        }
    }

    /// Decode a serialized line entry.
    const fn parse(bytes: [u8; LINE_SIZE]) -> Self {
        let [o0, o1, o2, o3, f0, f1, f2, f3] = bytes;
        let flags = u32::from_le_bytes([f0, f1, f2, f3]);
        // The mask keeps 7 bits, so the value fits in `u8`.
        let end_delta = ((flags >> END_DELTA_SHIFT) & END_DELTA_MASK).to_le_bytes()[0];
        Self {
            offset: u32::from_le_bytes([o0, o1, o2, o3]),
            line_start: flags & START_LINE_MASK,
            end_delta,
            is_statement: flags & STATEMENT_BIT != 0,
        }
    }

    /// Append the 8-byte entry to `out`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the start line does not fit in
    ///   24 bits or the end line delta does not fit in 7 bits.
    /// - [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        if self.line_start & !START_LINE_MASK != 0 {
            return Err(invalid("line number does not fit in 24 bits"));
        }
        let end_delta = u32::from(self.end_delta);
        if end_delta & !END_DELTA_MASK != 0 {
            return Err(invalid("line end delta does not fit in 7 bits"));
        }
        let statement = if self.is_statement { STATEMENT_BIT } else { 0 };
        put_u32(out, self.offset)?;
        put_u32(
            out,
            self.line_start | end_delta << END_DELTA_SHIFT | statement,
        )
    }
}

/// One column entry (`CV_Column_t`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ColumnEntry {
    /// The first column.
    pub start: u16,
    /// The column after the end, or 0 if the end is not known.
    pub end: u16,
}

impl ColumnEntry {
    /// Create a column entry from its fields.
    #[must_use]
    pub const fn new(start: u16, end: u16) -> Self {
        Self { start, end }
    }

    /// Decode a serialized column entry.
    const fn parse(bytes: [u8; COLUMN_SIZE]) -> Self {
        let [s0, s1, e0, e1] = bytes;
        Self {
            start: u16::from_le_bytes([s0, s1]),
            end: u16::from_le_bytes([e0, e1]),
        }
    }

    /// Append the 4-byte entry to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        put_u16(out, self.start)?;
        put_u16(out, self.end)
    }
}

entry_list!(
    /// The line entries of a [`LineBlock`].
    LineEntries,
    /// An iterator over [`LineEntries`].
    LineEntriesIter,
    LineEntry,
    LINE_SIZE,
    LineEntry::parse
);

entry_list!(
    /// The column entries of a [`LineBlock`].
    ColumnEntries,
    /// An iterator over [`ColumnEntries`].
    ColumnEntriesIter,
    ColumnEntry,
    COLUMN_SIZE,
    ColumnEntry::parse
);

/// An iterator over the blocks of a [`LinesSubsection`].
#[derive(Clone, Debug)]
pub struct LineBlocks<'a> {
    /// The remaining validated blocks.
    bytes: &'a [u8],
    /// The subsection header.
    header: LinesHeader,
    /// The number of remaining blocks.
    left: usize,
}

impl<'a> Iterator for LineBlocks<'a> {
    type Item = LineBlock<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        // `LinesSubsection::decode` validated the blocks, so this cannot fail.
        let (block, rest) = split_block(self.bytes, self.header).ok()?;
        self.bytes = rest;
        self.left = self.left.saturating_sub(1);
        Some(block)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}

impl ExactSizeIterator for LineBlocks<'_> {}

impl FusedIterator for LineBlocks<'_> {}

/// Split one block from the start of `bytes`; return it and the rest.
fn split_block(bytes: &[u8], header: LinesHeader) -> Result<(LineBlock<'_>, &[u8])> {
    let (Some(checksum_offset), Some(count), Some(size)) =
        (read_u32(bytes, 0), read_u32(bytes, 4), read_u32(bytes, 8))
    else {
        return Err(truncated("line block header is truncated"));
    };
    let count = usize::try_from(count).map_err(|_| too_large("line block count is too large"))?;
    let entry_size = if header.has_columns() {
        LINE_SIZE + COLUMN_SIZE
    } else {
        LINE_SIZE
    };
    let expected = count
        .checked_mul(entry_size)
        .and_then(|size| size.checked_add(BLOCK_HEADER_SIZE))
        .ok_or_else(|| too_large("line block count is too large"))?;
    if usize::try_from(size).ok() != Some(expected) {
        return Err(invalid("line block size does not match its entries"));
    }
    if bytes.len() < expected {
        return Err(truncated("line block entries are truncated"));
    }
    let (block, rest) = bytes.split_at(expected);
    let entries = &block[BLOCK_HEADER_SIZE..];
    // `expected` covers `count` line entries and, with columns, `count`
    // column entries, so the split point is inside `entries`.
    let (lines, columns) = entries.split_at(count * LINE_SIZE);
    let block = LineBlock {
        checksum_offset,
        lines: LineEntries::new(lines),
        columns: header.has_columns().then(|| ColumnEntries::new(columns)),
    };
    Ok((block, rest))
}
