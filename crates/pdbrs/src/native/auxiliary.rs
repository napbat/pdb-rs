//! Auxiliary named streams: `/LinkInfo`, `/TMCache`, `/UDTSRCLINEUNDONE` and `sourcelink`.
//!
//! The PDB Info stream maps these names to stream indices. [`NamedStreamKind::classify`] maps a
//! named-stream name to the decoder for its contents. Each decoder takes the stream bytes and
//! the stream ID. Errors have the stream ID and a logical offset, and no physical offset.

mod link_info;
mod source_link;
mod tm_cache;
mod udt_source_line_undone;

pub use link_info::{LINK_INFO_VERSION_TWO, LinkInfo};
pub use source_link::SourceLink;
pub use tm_cache::TmCache;
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

/// The decoder that a named stream needs, from its exact name bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
        match name {
            b"/names" => Self::Names,
            LINK_INFO_STREAM_NAME => Self::LinkInfo,
            TM_CACHE_STREAM_NAME => Self::TmCache,
            UDT_SOURCE_LINE_UNDONE_STREAM_NAME => Self::UdtSourceLineUndone,
            HEADER_BLOCK_STREAM_NAME => Self::SourceHeaderBlock,
            SOURCE_LINK_STREAM_NAME => Self::SourceLink,
            _ => Self::Other,
        }
    }
}
