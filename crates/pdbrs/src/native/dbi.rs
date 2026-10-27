//! The DBI stream (stream 3) and its module list.
//!
//! [`Dbi::parse`] decodes the modern DBI header and checks the ranges of all
//! seven substreams. Each substream stays available as raw bytes through
//! [`Dbi::substream`]. [`Dbi::modules`] decodes the module info substream
//! lazily. [`Dbi::section_contributions`], [`Dbi::section_map`],
//! [`Dbi::source_info`], [`Dbi::type_server_map`], [`Dbi::ec_names`] and
//! [`Dbi::debug_streams`] decode the other substreams on request.
//!
//! Each substream has an encoder next to its decoder. [`Dbi::encode`] writes
//! a decoded stream again with the same bytes. [`DbiBuilder`] holds an owned
//! form of all substreams for a PDB writer.
//!
//! [`ModuleStream`] splits one module stream into its symbol, C11 line, C13
//! line and global reference regions. This module does not decode the C11 or
//! C13 line formats. It only frames the C13 region into [`Subsection`] items.
//!
//! The layouts follow <https://llvm.org/docs/PDB/DbiStream.html>,
//! <https://llvm.org/docs/PDB/ModiStream.html> and Microsoft `PDB/dbi/dbi.h`.

use alloc::vec::Vec;
use core::ops::Range;

use super::msf::StreamId;
use super::names::StringTable;
use super::output::no_memory;
use crate::{Error, ErrorKind, Result};

mod builder;
mod debug_header;
mod header;
mod module;
mod module_info;
mod section_contribution;
mod section_map;
mod sources;
mod type_server;
mod write;
pub use builder::DbiBuilder;
pub use debug_header::{DebugStream, DebugStreams, encode_debug_streams};
pub use header::DbiHeader;
pub use module::{
    GlobalRefs, GlobalRefsIter, ModuleRegionSizes, ModuleStream, ModuleStreamParts, Subsection,
    Subsections,
};
pub use module_info::{Module, Modules};
pub use section_contribution::{
    SECTION_CONTRIBUTION_V2, SECTION_CONTRIBUTION_V60, SectionContribution,
    SectionContributionVersion, SectionContributions, SectionContributionsIter,
};
pub use section_map::{SectionMap, SectionMapEntry, SegmentFlags};
pub use sources::{FileNames, ModuleSources, SourceInfo};
pub use type_server::TypeServer;

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
#[non_exhaustive]
pub enum Substream {
    /// The module info records. [`Dbi::modules`] decodes them.
    ModuleInfo,
    /// The section contribution records. [`Dbi::section_contributions`] decodes them.
    SectionContributions,
    /// The section map. [`Dbi::section_map`] decodes it.
    SectionMap,
    /// The source file info for all modules. [`Dbi::source_info`] decodes it.
    SourceInfo,
    /// The type server map. [`Dbi::type_server_map`] decodes it.
    TypeServerMap,
    /// The EC substream, a string table of names. [`Dbi::ec_names`] decodes it.
    Ec,
    /// The optional debug header. [`Dbi::debug_streams`] decodes it.
    OptionalDebugHeader,
}

impl Substream {
    /// All substreams in disk order.
    pub const ALL: [Self; SUBSTREAM_COUNT] = [
        Self::ModuleInfo,
        Self::SectionContributions,
        Self::SectionMap,
        Self::SourceInfo,
        Self::TypeServerMap,
        Self::Ec,
        Self::OptionalDebugHeader,
    ];
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
        let header = DbiHeader::decode(fixed);
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
            failed: false,
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

    /// Decode the section contribution substream.
    ///
    /// Return `None` if the substream is empty.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`SectionContributions::parse`], located at
    /// the start of the substream in stream 3.
    pub fn section_contributions(&self) -> Result<Option<SectionContributions<'_>>> {
        self.decode(Substream::SectionContributions, SectionContributions::parse)
    }

    /// Decode the section map substream.
    ///
    /// Return `None` if the substream is empty.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`SectionMap::parse`], located at the start of
    /// the substream in stream 3.
    pub fn section_map(&self) -> Result<Option<SectionMap>> {
        self.decode(Substream::SectionMap, SectionMap::parse)
    }

    /// Decode the source file info substream.
    ///
    /// Return `None` if the substream is empty. The module count of the
    /// substream must equal the number of module info records.
    ///
    /// # Errors
    ///
    /// - The errors of [`SourceInfo::parse`] and of [`Dbi::modules`].
    /// - [`ErrorKind::InvalidFormat`] if the module counts differ.
    ///
    /// Source info errors are located at the start of the substream in stream 3.
    pub fn source_info(&self) -> Result<Option<SourceInfo<'_>>> {
        let Some(info) = self.decode(Substream::SourceInfo, SourceInfo::parse)? else {
            return Ok(None);
        };
        let mut modules = 0_usize;
        for module in self.modules() {
            module?;
            modules += 1;
        }
        if modules != info.modules.len() {
            return Err(dbi_error(
                ErrorKind::InvalidFormat,
                self.substream_offset(Substream::SourceInfo),
                "DBI source info module count differs from the module info count",
            ));
        }
        Ok(Some(info))
    }

    /// Decode the type server map substream.
    ///
    /// Linkers that do not use type servers write an empty map. The function
    /// returns an empty vector for it.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`TypeServer::parse_map`], located at the start
    /// of the substream in stream 3.
    pub fn type_server_map(&self) -> Result<Vec<TypeServer>> {
        Ok(self
            .decode(Substream::TypeServerMap, TypeServer::parse_map)?
            .unwrap_or_default())
    }

    /// Decode the EC substream as a string table.
    ///
    /// Return `None` if the substream is empty.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`StringTable::parse`], located at the start of
    /// the substream in stream 3.
    pub fn ec_names(&self) -> Result<Option<StringTable<'_>>> {
        self.decode(Substream::Ec, StringTable::parse)
    }

    /// Decode a non-empty substream and add the substream location to errors.
    fn decode<'a, T>(
        &'a self,
        kind: Substream,
        parse: impl FnOnce(&'a [u8]) -> Result<T>,
    ) -> Result<Option<T>> {
        let bytes = self.substream(kind);
        if bytes.is_empty() {
            return Ok(None);
        }
        parse(bytes).map(Some).map_err(|error| {
            error.with_stream(StreamId(DBI_STREAM), logical(self.substream_offset(kind)))
        })
    }
}

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
        .with_stream(StreamId(DBI_STREAM), logical(offset))
}

/// Convert a logical offset to the error representation.
fn logical(offset: usize) -> u64 {
    u64::try_from(offset).unwrap_or(u64::MAX)
}

/// Convert a serialized 16-bit stream index; the nil index becomes `None`.
fn stream_id(value: u16) -> Option<StreamId> {
    (value != NIL_STREAM).then_some(StreamId(u32::from(value)))
}

/// Convert a stream to its serialized 16-bit index; `None` becomes the nil index.
///
/// # Errors
///
/// Returns [`ErrorKind::LimitExceeded`] if the index is larger than `0xfffe`.
fn stream_index(stream: Option<StreamId>) -> Result<u16> {
    let Some(stream) = stream else {
        return Ok(NIL_STREAM);
    };
    u16::try_from(stream.0)
        .ok()
        .filter(|&index| index != NIL_STREAM)
        .ok_or_else(|| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("stream index does not fit in 16 bits")
        })
}

/// Convert a count to a `u16` field.
///
/// # Errors
///
/// Returns [`ErrorKind::LimitExceeded`] with `context` if the value does not fit.
fn to_u16(value: usize, context: &'static str) -> Result<u16> {
    u16::try_from(value)
        .map_err(|_| Error::new(ErrorKind::LimitExceeded, None).with_context(context))
}

/// Copy bytes into a new vector.
fn copy_bytes(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(bytes.len())
        .map_err(|_| no_memory())?;
    copy.extend_from_slice(bytes);
    Ok(copy)
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
