//! Auxiliary named streams: `/LinkInfo`, `/TMCache`, `/UDTSRCLINEUNDONE`, `sourcelink`,
//! `sourcelink$<n>` and `srcsrv`.
//!
//! The PDB Info stream maps these names to stream indices. [`NamedStreamKind::classify`] maps a
//! named-stream name to the decoder for its contents. Each decoder takes the stream bytes and
//! the stream ID. Errors have the stream ID and a logical offset, and no physical offset.
//! [`TmCache`] names unnamed type map streams; [`TypeMapStream`] decodes them.
//!
//! Each decoded value has an `encode` function that gives the same stream bytes again.

mod link_info;
mod source_link;
mod source_server;
mod tm_cache;
mod type_map;
mod udt_source_line_undone;

pub use link_info::{LINK_INFO_VERSION_ONE, LINK_INFO_VERSION_TWO, LinkInfo};
pub use source_link::SourceLink;
pub use source_server::{
    SourceServer, SourceServerFile, SourceServerSection, SourceServerVariable,
};
pub use tm_cache::{TM_CACHE_VERSION, TmCache, TmCacheBuilder, TmCacheEntry};
pub use type_map::{TypeMapContext, TypeMapKind, TypeMapRule, TypeMapStream, TypeMapViolation};
pub use udt_source_line_undone::UdtSourceLineUndone;

use super::injected_source::{HEADER_BLOCK_STREAM_NAME, SOURCE_FILE_STREAM_PREFIX};

/// The name of the linker information stream.
pub const LINK_INFO_STREAM_NAME: &[u8] = b"/LinkInfo";

/// The name of the type map cache stream.
pub const TM_CACHE_STREAM_NAME: &[u8] = b"/TMCache";

/// The name of the stream with UDT type indices that have no source line record.
pub const UDT_SOURCE_LINE_UNDONE_STREAM_NAME: &[u8] = b"/UDTSRCLINEUNDONE";

/// The name of the Source Link stream of .NET Windows PDBs.
pub const SOURCE_LINK_STREAM_NAME: &[u8] = b"sourcelink";

/// The prefix of the numbered Source Link streams of `link /SOURCELINK`: `sourcelink$1`,
/// `sourcelink$2` and so on.
pub const NUMBERED_SOURCE_LINK_PREFIX: &[u8] = b"sourcelink$";

/// The name of the source server stream that source indexing tools add.
pub const SOURCE_SERVER_STREAM_NAME: &[u8] = b"srcsrv";

/// The decoder that a named stream needs, from its exact name bytes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum NamedStreamKind<'a> {
    /// The `/names` string table.
    Names,
    /// The `/LinkInfo` stream: [`LinkInfo`].
    LinkInfo,
    /// The `/TMCache` stream: [`TmCache`].
    TmCache,
    /// The `/UDTSRCLINEUNDONE` stream: [`UdtSourceLineUndone`].
    UdtSourceLineUndone,
    /// The `/src/headerblock` stream: [`SourceHeaderBlock`](super::injected_source::SourceHeaderBlock).
    SourceHeaderBlock,
    /// A `/src/files/` stream that holds the contents of an injected source file.
    SourceFile {
        /// The virtual file name after the prefix, as bytes.
        virtual_file_name: &'a [u8],
    },
    /// The `sourcelink` stream: [`SourceLink`].
    SourceLink,
    /// A `sourcelink$<n>` stream of `link /SOURCELINK`: [`SourceLink::parse_numbered`].
    ///
    /// `link` writes one stream for each `/SOURCELINK` file, numbered from 1, and then one
    /// empty stream with the next number.
    NumberedSourceLink {
        /// The number after the `$`.
        number: u32,
    },
    /// The `srcsrv` stream: [`SourceServer`].
    SourceServer,
    /// A name that this reader does not recognize.
    Other,
}

impl<'a> NamedStreamKind<'a> {
    /// Classify a named stream by its exact name bytes.
    ///
    /// The comparison is case-sensitive. The writers store these names with the case shown.
    #[must_use]
    pub fn classify(name: &'a [u8]) -> Self {
        if let Some(virtual_file_name) = name.strip_prefix(SOURCE_FILE_STREAM_PREFIX) {
            return Self::SourceFile { virtual_file_name };
        }
        if let Some(number) = name
            .strip_prefix(NUMBERED_SOURCE_LINK_PREFIX)
            .and_then(stream_number)
        {
            return Self::NumberedSourceLink { number };
        }
        match name {
            b"/names" => Self::Names,
            LINK_INFO_STREAM_NAME => Self::LinkInfo,
            TM_CACHE_STREAM_NAME => Self::TmCache,
            UDT_SOURCE_LINE_UNDONE_STREAM_NAME => Self::UdtSourceLineUndone,
            HEADER_BLOCK_STREAM_NAME => Self::SourceHeaderBlock,
            SOURCE_LINK_STREAM_NAME => Self::SourceLink,
            SOURCE_SERVER_STREAM_NAME => Self::SourceServer,
            _ => Self::Other,
        }
    }
}

/// Decode the number of a `sourcelink$<n>` name: decimal digits, from 1, without leading
/// zeros, and not larger than `u32::MAX`.
fn stream_number(digits: &[u8]) -> Option<u32> {
    if digits
        .first()
        .is_none_or(|&first| !(b'1'..=b'9').contains(&first))
    {
        return None;
    }
    digits.iter().try_fold(0_u32, |number, &digit| {
        if !digit.is_ascii_digit() {
            return None;
        }
        number.checked_mul(10)?.checked_add(u32::from(digit - b'0'))
    })
}
