//! The DBI stream (stream 3) and its module list.
//!
//! [`Dbi::parse`] decodes the modern DBI header and checks the ranges of all
//! seven substreams. Each substream stays available as raw bytes through
//! [`Dbi::substream`], also when no typed decoder exists for it yet.
//! [`Dbi::modules`] decodes the module info substream lazily.
//!
//! [`ModuleStream`] splits one module stream into its symbol, C11 line, C13
//! line and global reference regions. This module does not decode the C11 or
//! C13 line formats. It only frames the C13 region into [`Subsection`] items.
//!
//! The layouts follow <https://llvm.org/docs/PDB/DbiStream.html> and
//! <https://llvm.org/docs/PDB/ModiStream.html>.

use alloc::vec::Vec;
use core::iter::FusedIterator;
use core::ops::Range;

use super::msf::StreamId;
use crate::{Error, ErrorKind, Result};

mod module;
pub use module::{ModuleStream, Subsection, Subsections};

/// The fixed MSF stream index of the DBI stream.
const DBI_STREAM: u32 = 3;
/// The size of the DBI header and of the fixed module info prefix.
const FIXED_SIZE: usize = 64;
/// The `VersionSignature` value of the modern DBI header.
const MODERN_SIGNATURE: i32 = -1;
/// The `V70` DBI version, the only version that this decoder accepts.
const VERSION_V70: u32 = 19_990_903;
/// The serialized value of an absent stream index.
const NIL_STREAM: u16 = 0xffff;

/// The header offsets of the substream sizes, in disk order.
///
/// The EC substream comes before the optional debug header on disk, but its
/// size field comes after the size field of the optional debug header.
const SIZE_FIELDS: [(usize, &str); SUBSTREAM_COUNT] = [
    (24, "DBI module info size is negative"),
    (28, "DBI section contribution size is negative"),
    (32, "DBI section map size is negative"),
    (36, "DBI source info size is negative"),
    (40, "DBI type server map size is negative"),
    (52, "DBI EC substream size is negative"),
    (48, "DBI optional debug header size is negative"),
];
/// The number of DBI substreams.
const SUBSTREAM_COUNT: usize = 7;

/// A substream of the DBI stream, in disk order.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Substream {
    /// The module info records. [`Dbi::modules`] decodes them.
    ModuleInfo,
    /// The section contribution records.
    SectionContributions,
    /// The section map.
    SectionMap,
    /// The source file info for all modules.
    SourceInfo,
    /// The type server map.
    TypeServerMap,
    /// The EC substream, a string table of names.
    Ec,
    /// The optional debug header. [`Dbi::debug_streams`] decodes it.
    OptionalDebugHeader,
}

/// A slot in the optional debug header.
///
/// Each slot holds the index of a stream with one kind of debug data.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DebugStream {
    /// Old frame pointer omission data.
    Fpo,
    /// Exception data.
    Exception,
    /// Fixup data.
    Fixup,
    /// The address map from the image to the source image.
    OmapToSource,
    /// The address map from the source image to the image.
    OmapFromSource,
    /// The section headers of the image.
    SectionHeader,
    /// The token to RID map.
    TokenRidMap,
    /// The `.xdata` section contents.
    Xdata,
    /// The `.pdata` section contents.
    Pdata,
    /// New frame pointer omission data.
    NewFpo,
    /// The section headers of the image before address remapping.
    OriginalSectionHeader,
}

impl DebugStream {
    /// Return the slot index of this kind in the optional debug header.
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// The decoded fields of a modern DBI header.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct DbiHeader {
    /// The PDB revision counter.
    pub age: u32,
    /// The global symbol stream, if present.
    pub global_stream: Option<StreamId>,
    /// The bit field with the toolchain version.
    pub build_number: u16,
    /// The public symbol stream, if present.
    pub public_stream: Option<StreamId>,
    /// The version of the PDB writer library.
    pub pdb_dll_version: u16,
    /// The symbol record stream, if present.
    pub symbol_stream: Option<StreamId>,
    /// The rebuild number of the PDB writer library.
    pub pdb_dll_rebuild: u16,
    /// The index of the MFC type server in the type server map.
    pub mfc_type_server_index: u32,
    /// The build flags, for example incremental linking.
    pub flags: u16,
    /// The `IMAGE_FILE_MACHINE` value.
    pub machine: u16,
}

/// A decoded modern DBI stream with owned serialized bytes.
#[derive(Clone, Debug)]
pub struct Dbi {
    bytes: Vec<u8>,
    header: DbiHeader,
    /// The substream ranges in `bytes`, in [`Substream`] order.
    substreams: [Range<usize>; SUBSTREAM_COUNT],
}

impl Dbi {
    /// Decode the DBI header and check the substream ranges.
    ///
    /// The substreams must fill the stream after the header exactly. This
    /// function does not decode module records; [`Dbi::modules`] does that.
    /// It does not check that referenced streams exist.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnexpectedEof`] if the header or a substream is truncated.
    /// - [`ErrorKind::UnsupportedVersion`] if the header is not a modern `V70` header.
    /// - [`ErrorKind::InvalidFormat`] for a negative substream size, bytes after
    ///   the last substream or an optional debug header with an odd size.
    /// - [`ErrorKind::OffsetOverflow`] if a substream end cannot be represented.
    ///
    /// Errors identify stream 3 and a logical offset in it.
    pub fn parse(bytes: Vec<u8>) -> Result<Self> {
        let fixed = bytes.first_chunk::<FIXED_SIZE>().ok_or_else(|| {
            dbi_error(
                ErrorKind::UnexpectedEof,
                bytes.len(),
                "DBI header is truncated",
            )
        })?;
        if i32::from_le_bytes(field(fixed, 0)) != MODERN_SIGNATURE {
            return Err(dbi_error(
                ErrorKind::UnsupportedVersion,
                0,
                "DBI header does not have the modern signature",
            ));
        }
        if u32_at(fixed, 4) != VERSION_V70 {
            return Err(dbi_error(
                ErrorKind::UnsupportedVersion,
                4,
                "DBI version is not V70",
            ));
        }
        let header = DbiHeader {
            age: u32_at(fixed, 8),
            global_stream: stream_id(u16_at(fixed, 12)),
            build_number: u16_at(fixed, 14),
            public_stream: stream_id(u16_at(fixed, 16)),
            pdb_dll_version: u16_at(fixed, 18),
            symbol_stream: stream_id(u16_at(fixed, 20)),
            pdb_dll_rebuild: u16_at(fixed, 22),
            mfc_type_server_index: u32_at(fixed, 44),
            flags: u16_at(fixed, 56),
            machine: u16_at(fixed, 58),
        };
        let substreams = substream_ranges(fixed, bytes.len())?;
        let debug = &substreams[Substream::OptionalDebugHeader as usize];
        if !debug.len().is_multiple_of(2) {
            return Err(dbi_error(
                ErrorKind::InvalidFormat,
                48,
                "DBI optional debug header size is odd",
            ));
        }
        Ok(Self {
            bytes,
            header,
            substreams,
        })
    }

    /// Return the decoded header fields.
    #[must_use]
    pub const fn header(&self) -> &DbiHeader {
        &self.header
    }

    /// Return the raw bytes of a substream.
    #[must_use]
    pub fn substream(&self, kind: Substream) -> &[u8] {
        &self.bytes[self.substreams[kind as usize].clone()]
    }

    /// Return the logical offset of a substream in the DBI stream.
    #[must_use]
    pub fn substream_offset(&self, kind: Substream) -> usize {
        self.substreams[kind as usize].start
    }

    /// Return an iterator over the module info records.
    #[must_use]
    pub fn modules(&self) -> Modules<'_> {
        let range = &self.substreams[Substream::ModuleInfo as usize];
        Modules {
            bytes: &self.bytes[range.clone()],
            base: range.start,
            position: 0,
            index: 0,
        }
    }

    /// Return an iterator over all slots of the optional debug header.
    ///
    /// The iterator also returns slots after the slots that [`DebugStream`] names.
    #[must_use]
    pub fn debug_streams(&self) -> DebugStreams<'_> {
        DebugStreams {
            slots: self
                .substream(Substream::OptionalDebugHeader)
                .as_chunks::<2>()
                .0
                .iter(),
        }
    }

    /// Return the stream in one slot of the optional debug header.
    ///
    /// Return `None` if the slot holds the nil index or if the header has no such slot.
    #[must_use]
    pub fn debug_stream(&self, kind: DebugStream) -> Option<StreamId> {
        self.debug_streams().nth(kind.index()).flatten()
    }
}

/// An iterator over the slots of the optional debug header.
///
/// Each item is `None` for a slot that holds the nil stream index.
#[derive(Clone, Debug)]
pub struct DebugStreams<'a> {
    slots: core::slice::Iter<'a, [u8; 2]>,
}

impl Iterator for DebugStreams<'_> {
    type Item = Option<StreamId>;

    fn next(&mut self) -> Option<Self::Item> {
        let slot = self.slots.next()?;
        Some(stream_id(u16::from_le_bytes(*slot)))
    }

    fn nth(&mut self, count: usize) -> Option<Self::Item> {
        let slot = self.slots.nth(count)?;
        Some(stream_id(u16::from_le_bytes(*slot)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.slots.size_hint()
    }
}

impl ExactSizeIterator for DebugStreams<'_> {}

impl FusedIterator for DebugStreams<'_> {}

/// One module info record from the DBI stream.
///
/// Names stay raw bytes. They are often paths in an unspecified code page.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Module<'a> {
    /// The zero-based position of the record in the module info substream.
    pub index: u32,
    /// The logical offset of the record in the DBI stream.
    pub offset: u32,
    /// The module flags.
    pub flags: u16,
    /// The module stream, or `None` if the module has no debug data.
    pub stream: Option<StreamId>,
    /// The size of the symbol region in the module stream, with its signature.
    pub symbol_bytes: u32,
    /// The size of the C11 line region in the module stream.
    pub c11_bytes: u32,
    /// The size of the C13 line region in the module stream.
    pub c13_bytes: u32,
    /// The number of source files that contributed to the module.
    pub source_file_count: u16,
    /// The module name without its terminating NUL.
    pub name: &'a [u8],
    /// The object file name without its terminating NUL.
    pub object_file: &'a [u8],
}

/// An iterator over the module info records of a DBI stream.
///
/// The iterator returns one error for a malformed record and then stops.
#[derive(Clone, Debug)]
pub struct Modules<'a> {
    /// The module info substream.
    bytes: &'a [u8],
    /// The logical offset of the substream in the DBI stream.
    base: usize,
    /// The offset of the next record in `bytes`.
    position: usize,
    /// The index of the next record.
    index: u32,
}

impl<'a> Modules<'a> {
    /// Decode the record at the current position.
    fn read(&self) -> Result<(Module<'a>, usize)> {
        let start = self.position;
        let location = |offset: usize| self.base.saturating_add(offset);
        let rest = &self.bytes[start..];
        let fixed = rest.first_chunk::<FIXED_SIZE>().ok_or_else(|| {
            dbi_error(
                ErrorKind::UnexpectedEof,
                location(self.bytes.len()),
                "DBI module info prefix is truncated",
            )
        })?;
        let (name, after_name) = c_string(rest, FIXED_SIZE).ok_or_else(|| {
            dbi_error(
                ErrorKind::UnexpectedEof,
                location(self.bytes.len()),
                "DBI module name has no terminating NUL",
            )
        })?;
        let (object_file, after_object) = c_string(rest, after_name).ok_or_else(|| {
            dbi_error(
                ErrorKind::UnexpectedEof,
                location(self.bytes.len()),
                "DBI object file name has no terminating NUL",
            )
        })?;
        // The record starts at a 4-byte boundary, so the record length aligns the end.
        let padded = align4(after_object)
            .filter(|&end| end <= rest.len())
            .ok_or_else(|| {
                dbi_error(
                    ErrorKind::UnexpectedEof,
                    location(self.bytes.len()),
                    "DBI module info padding is truncated",
                )
            })?;
        let offset = u32::try_from(location(start)).map_err(|_| {
            dbi_error(
                ErrorKind::OffsetOverflow,
                location(start),
                "DBI module offset does not fit in 32 bits",
            )
        })?;
        let module = Module {
            index: self.index,
            offset,
            flags: u16_at(fixed, 32),
            stream: stream_id(u16_at(fixed, 34)),
            symbol_bytes: u32_at(fixed, 36),
            c11_bytes: u32_at(fixed, 40),
            c13_bytes: u32_at(fixed, 44),
            source_file_count: u16_at(fixed, 48),
            name,
            object_file,
        };
        Ok((module, start + padded))
    }
}

impl<'a> Iterator for Modules<'a> {
    type Item = Result<Module<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.position >= self.bytes.len() {
            return None;
        }
        match self.read() {
            Ok((module, next)) => {
                self.position = next;
                // Each record has at least 64 bytes, so the count fits when offsets fit.
                self.index = self.index.saturating_add(1);
                Some(Ok(module))
            }
            Err(error) => {
                self.position = self.bytes.len();
                Some(Err(error))
            }
        }
    }
}

impl FusedIterator for Modules<'_> {}

/// Compute the substream ranges from the header and the stream length.
fn substream_ranges(
    fixed: &[u8; FIXED_SIZE],
    length: usize,
) -> Result<[Range<usize>; SUBSTREAM_COUNT]> {
    const EMPTY: Range<usize> = 0..0;
    let mut ranges = [EMPTY; SUBSTREAM_COUNT];
    let mut start = FIXED_SIZE;
    for (range, &(offset, context)) in ranges.iter_mut().zip(SIZE_FIELDS.iter()) {
        let size = i32::from_le_bytes(field(fixed, offset));
        let size = u32::try_from(size)
            .map_err(|_| dbi_error(ErrorKind::InvalidFormat, offset, context))?;
        let size = usize::try_from(size).map_err(|_| {
            dbi_error(
                ErrorKind::LimitExceeded,
                offset,
                "DBI substream size does not fit in memory",
            )
        })?;
        let end = start.checked_add(size).ok_or_else(|| {
            dbi_error(
                ErrorKind::OffsetOverflow,
                start,
                "DBI substream end overflows",
            )
        })?;
        if end > length {
            return Err(dbi_error(
                ErrorKind::UnexpectedEof,
                length,
                "DBI substream extends past the end of the stream",
            ));
        }
        *range = start..end;
        start = end;
    }
    if start != length {
        return Err(dbi_error(
            ErrorKind::InvalidFormat,
            start,
            "DBI stream has bytes after the last substream",
        ));
    }
    Ok(ranges)
}

/// Create an error at a logical offset in the DBI stream.
fn dbi_error(kind: ErrorKind, offset: usize, context: &'static str) -> Error {
    Error::new(kind, None)
        .with_context(context)
        .with_stream(DBI_STREAM, logical(offset))
}

/// Convert a logical offset to the error representation.
fn logical(offset: usize) -> u64 {
    u64::try_from(offset).unwrap_or(u64::MAX)
}

/// Convert a serialized 16-bit stream index; the nil index becomes `None`.
fn stream_id(value: u16) -> Option<StreamId> {
    (value != NIL_STREAM).then_some(StreamId(u32::from(value)))
}

/// Copy `N` bytes at a fixed offset of a fixed-size prefix.
///
/// Callers use constant offsets that lie inside the prefix.
fn field<const N: usize>(fixed: &[u8; FIXED_SIZE], offset: usize) -> [u8; N] {
    let mut value = [0; N];
    value.copy_from_slice(&fixed[offset..offset + N]);
    value
}

/// Read a little-endian `u16` at a fixed offset of a fixed-size prefix.
fn u16_at(fixed: &[u8; FIXED_SIZE], offset: usize) -> u16 {
    u16::from_le_bytes(field(fixed, offset))
}

/// Read a little-endian `u32` at a fixed offset of a fixed-size prefix.
fn u32_at(fixed: &[u8; FIXED_SIZE], offset: usize) -> u32 {
    u32::from_le_bytes(field(fixed, offset))
}

/// Read a little-endian `u32` at `offset`, if the bytes are available.
fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let value = bytes.get(offset..)?.first_chunk::<4>()?;
    Some(u32::from_le_bytes(*value))
}

/// Split a NUL-terminated string at `start`; return it and the offset after the NUL.
fn c_string(bytes: &[u8], start: usize) -> Option<(&[u8], usize)> {
    let rest = bytes.get(start..)?;
    let length = rest.iter().position(|&byte| byte == 0)?;
    Some((&rest[..length], start + length + 1))
}

/// Round `offset` up to a multiple of 4.
fn align4(offset: usize) -> Option<usize> {
    offset.checked_add(3).map(|value| value & !3)
}
