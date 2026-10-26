//! C13 line information in module streams.
//!
//! The C13 line region of a module stream holds subsections. Each
//! subsection has a kind and data. [`LineSubsection::decode`] decodes the
//! data of these kinds:
//!
//! - [`DEBUG_S_LINES`]: line blocks of one code contribution.
//! - [`DEBUG_S_FILECHKSMS`]: the file checksum table of the module.
//! - [`DEBUG_S_INLINEELINES`]: the source lines of inlined functions.
//!
//! Data of all other kinds stays raw. [`SubsectionKind`] classifies these
//! kinds and does not decode them. [`ModuleLines`] decodes all subsections
//! of one module stream and validates the file checksum references.
//!
//! File names stay byte offsets into the `/names` stream. This module does
//! not read the `/names` stream.
//!
//! The layouts come from these sources:
//!
//! - `docs/reference/microsoft-pdb-rs/docs/codeview/line_data.md`.
//! - `DebugLinesSubsection.cpp`, `DebugChecksumsSubsection.cpp` and
//!   `DebugInlineeLinesSubsection.cpp` of LLVM.
//! - `CV_DebugSLinesHeader_t`, `CV_Line_t`, `CV_Column_t` and
//!   `DEBUG_S_SUBSECTION_TYPE` in `cvinfo.h` of `microsoft/microsoft-pdb`.

use crate::{Error, ErrorKind, Result};

mod blocks;
mod checksums;
mod inlinee;
mod module;

pub use blocks::{
    ALWAYS_STEP_INTO_LINE, ColumnEntries, ColumnEntriesIter, ColumnEntry, LineBlock, LineBlocks,
    LineEntries, LineEntriesIter, LineEntry, LinesHeader, LinesSubsection, NEVER_STEP_INTO_LINE,
};
pub use checksums::{ChecksumKind, FileChecksum, FileChecksums, FileChecksumsIter};
pub use inlinee::{
    ChecksumOffsets, ChecksumOffsetsIter, InlineeLine, InlineeLines, InlineeLinesIter,
    InlineeSignature,
};
pub use module::{ModuleLines, ModuleSubsection};

/// The kind bit that tells a reader to ignore the subsection.
pub const DEBUG_S_IGNORE: u32 = 0x8000_0000;
/// Symbol records (`DEBUG_S_SYMBOLS`).
pub const DEBUG_S_SYMBOLS: u32 = 0xf1;
/// Line blocks (`DEBUG_S_LINES`).
pub const DEBUG_S_LINES: u32 = 0xf2;
/// A string table (`DEBUG_S_STRINGTABLE`).
pub const DEBUG_S_STRINGTABLE: u32 = 0xf3;
/// File checksums (`DEBUG_S_FILECHKSMS`).
pub const DEBUG_S_FILECHKSMS: u32 = 0xf4;
/// Frame data (`DEBUG_S_FRAMEDATA`).
pub const DEBUG_S_FRAMEDATA: u32 = 0xf5;
/// Inlinee source lines (`DEBUG_S_INLINEELINES`).
pub const DEBUG_S_INLINEELINES: u32 = 0xf6;
/// Cross-scope imports (`DEBUG_S_CROSSSCOPEIMPORTS`).
pub const DEBUG_S_CROSSSCOPEIMPORTS: u32 = 0xf7;
/// Cross-scope exports (`DEBUG_S_CROSSSCOPEEXPORTS`).
pub const DEBUG_S_CROSSSCOPEEXPORTS: u32 = 0xf8;
/// IL lines (`DEBUG_S_IL_LINES`).
pub const DEBUG_S_IL_LINES: u32 = 0xf9;
/// A function metadata token map (`DEBUG_S_FUNC_MDTOKEN_MAP`).
pub const DEBUG_S_FUNC_MDTOKEN_MAP: u32 = 0xfa;
/// A type metadata token map (`DEBUG_S_TYPE_MDTOKEN_MAP`).
pub const DEBUG_S_TYPE_MDTOKEN_MAP: u32 = 0xfb;
/// Merged assembly input (`DEBUG_S_MERGED_ASSEMBLYINPUT`).
pub const DEBUG_S_MERGED_ASSEMBLYINPUT: u32 = 0xfc;
/// COFF symbol RVAs (`DEBUG_S_COFF_SYMBOL_RVA`).
pub const DEBUG_S_COFF_SYMBOL_RVA: u32 = 0xfd;
/// An inlinee metadata token map (`DEBUG_S_INLINEE_MDTOKEN_MAP`).
pub const DEBUG_S_INLINEE_MDTOKEN_MAP: u32 = 0xfe;
/// XFG type hashes (`DEBUG_S_XFGHASH_TYPE`).
pub const DEBUG_S_XFGHASH_TYPE: u32 = 0xff;
/// XFG virtual hashes (`DEBUG_S_XFGHASH_VIRTUAL`).
pub const DEBUG_S_XFGHASH_VIRTUAL: u32 = 0x100;

/// The class of a raw C13 subsection kind.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum SubsectionKind {
    /// [`DEBUG_S_SYMBOLS`].
    Symbols,
    /// [`DEBUG_S_LINES`]. This crate decodes it.
    Lines,
    /// [`DEBUG_S_STRINGTABLE`].
    StringTable,
    /// [`DEBUG_S_FILECHKSMS`]. This crate decodes it.
    FileChecksums,
    /// [`DEBUG_S_FRAMEDATA`].
    FrameData,
    /// [`DEBUG_S_INLINEELINES`]. This crate decodes it.
    InlineeLines,
    /// [`DEBUG_S_CROSSSCOPEIMPORTS`].
    CrossScopeImports,
    /// [`DEBUG_S_CROSSSCOPEEXPORTS`].
    CrossScopeExports,
    /// [`DEBUG_S_IL_LINES`].
    IlLines,
    /// [`DEBUG_S_FUNC_MDTOKEN_MAP`].
    FunctionTokenMap,
    /// [`DEBUG_S_TYPE_MDTOKEN_MAP`].
    TypeTokenMap,
    /// [`DEBUG_S_MERGED_ASSEMBLYINPUT`].
    MergedAssemblyInput,
    /// [`DEBUG_S_COFF_SYMBOL_RVA`].
    CoffSymbolRva,
    /// [`DEBUG_S_INLINEE_MDTOKEN_MAP`].
    InlineeTokenMap,
    /// [`DEBUG_S_XFGHASH_TYPE`].
    XfgHashType,
    /// [`DEBUG_S_XFGHASH_VIRTUAL`].
    XfgHashVirtual,
    /// A kind with the [`DEBUG_S_IGNORE`] bit. The value is the raw kind.
    Ignored(u32),
    /// A kind without a known name. The value is the raw kind.
    Unknown(u32),
}

impl SubsectionKind {
    /// Classify a raw subsection kind.
    #[must_use]
    pub const fn from_raw(kind: u32) -> Self {
        match kind {
            DEBUG_S_SYMBOLS => Self::Symbols,
            DEBUG_S_LINES => Self::Lines,
            DEBUG_S_STRINGTABLE => Self::StringTable,
            DEBUG_S_FILECHKSMS => Self::FileChecksums,
            DEBUG_S_FRAMEDATA => Self::FrameData,
            DEBUG_S_INLINEELINES => Self::InlineeLines,
            DEBUG_S_CROSSSCOPEIMPORTS => Self::CrossScopeImports,
            DEBUG_S_CROSSSCOPEEXPORTS => Self::CrossScopeExports,
            DEBUG_S_IL_LINES => Self::IlLines,
            DEBUG_S_FUNC_MDTOKEN_MAP => Self::FunctionTokenMap,
            DEBUG_S_TYPE_MDTOKEN_MAP => Self::TypeTokenMap,
            DEBUG_S_MERGED_ASSEMBLYINPUT => Self::MergedAssemblyInput,
            DEBUG_S_COFF_SYMBOL_RVA => Self::CoffSymbolRva,
            DEBUG_S_INLINEE_MDTOKEN_MAP => Self::InlineeTokenMap,
            DEBUG_S_XFGHASH_TYPE => Self::XfgHashType,
            DEBUG_S_XFGHASH_VIRTUAL => Self::XfgHashVirtual,
            _ if kind & DEBUG_S_IGNORE != 0 => Self::Ignored(kind),
            _ => Self::Unknown(kind),
        }
    }

    /// Return `true` if [`LineSubsection::decode`] decodes this kind.
    #[must_use]
    pub const fn is_decoded(self) -> bool {
        matches!(self, Self::Lines | Self::FileChecksums | Self::InlineeLines)
    }
}

/// The data of one C13 subsection.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LineSubsection<'a> {
    /// A [`DEBUG_S_LINES`] subsection.
    Lines(LinesSubsection<'a>),
    /// A [`DEBUG_S_FILECHKSMS`] subsection.
    FileChecksums(FileChecksums<'a>),
    /// A [`DEBUG_S_INLINEELINES`] subsection.
    InlineeLines(InlineeLines<'a>),
    /// A subsection of a kind that this crate does not decode.
    Unsupported {
        /// The class of the raw kind.
        kind: SubsectionKind,
        /// The raw subsection data.
        data: &'a [u8],
    },
}

impl<'a> LineSubsection<'a> {
    /// Decode the data of a subsection with the raw `kind`.
    ///
    /// `data` excludes the subsection header and the alignment padding
    /// after the data, as in [`Subsection::data`](crate::native::dbi::Subsection::data).
    /// Return [`LineSubsection::Unsupported`] for a kind that
    /// [`SubsectionKind::is_decoded`] rejects.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`LinesSubsection::decode`],
    /// [`FileChecksums::decode`] and [`InlineeLines::decode`]. The errors do
    /// not have a file offset or a stream location.
    pub fn decode(kind: u32, data: &'a [u8]) -> Result<Self> {
        Ok(match SubsectionKind::from_raw(kind) {
            SubsectionKind::Lines => Self::Lines(LinesSubsection::decode(data)?),
            SubsectionKind::FileChecksums => Self::FileChecksums(FileChecksums::decode(data)?),
            SubsectionKind::InlineeLines => Self::InlineeLines(InlineeLines::decode(data)?),
            kind => Self::Unsupported { kind, data },
        })
    }
}

/// Define a borrowed list of fixed-size little-endian entries.
macro_rules! entry_list {
    (
        $(#[$list_meta:meta])*
        $list:ident,
        $(#[$iter_meta:meta])*
        $iter:ident,
        $entry:ty,
        $size:expr,
        $parse:expr
    ) => {
        $(#[$list_meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub struct $list<'a> {
            /// The serialized entries.
            entries: &'a [[u8; $size]],
        }

        impl<'a> $list<'a> {
            /// Wrap serialized entries. Ignore a partial entry at the end.
            pub(super) const fn new(bytes: &'a [u8]) -> Self {
                Self {
                    entries: bytes.as_chunks::<{ $size }>().0,
                }
            }

            /// Return the number of entries.
            #[must_use]
            pub const fn len(&self) -> usize {
                self.entries.len()
            }

            /// Return `true` if the list has no entries.
            #[must_use]
            pub const fn is_empty(&self) -> bool {
                self.entries.is_empty()
            }

            /// Return the entry at `position`, or `None` past the end.
            #[must_use]
            pub fn get(&self, position: usize) -> Option<$entry> {
                self.entries.get(position).copied().map($parse)
            }

            /// Return an iterator over the entries.
            #[must_use]
            pub fn iter(&self) -> $iter<'a> {
                $iter {
                    entries: self.entries.iter(),
                }
            }
        }

        impl<'a> IntoIterator for $list<'a> {
            type Item = $entry;
            type IntoIter = $iter<'a>;

            fn into_iter(self) -> Self::IntoIter {
                self.iter()
            }
        }

        impl<'a> IntoIterator for &$list<'a> {
            type Item = $entry;
            type IntoIter = $iter<'a>;

            fn into_iter(self) -> Self::IntoIter {
                self.iter()
            }
        }

        $(#[$iter_meta])*
        #[derive(Clone, Debug)]
        pub struct $iter<'a> {
            /// The remaining serialized entries.
            entries: core::slice::Iter<'a, [u8; $size]>,
        }

        impl Iterator for $iter<'_> {
            type Item = $entry;

            fn next(&mut self) -> Option<Self::Item> {
                self.entries.next().copied().map($parse)
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                self.entries.size_hint()
            }
        }

        impl DoubleEndedIterator for $iter<'_> {
            fn next_back(&mut self) -> Option<Self::Item> {
                self.entries.next_back().copied().map($parse)
            }
        }

        impl ExactSizeIterator for $iter<'_> {}

        impl core::iter::FusedIterator for $iter<'_> {}
    };
}

use entry_list;

/// Read a little-endian `u32` at `offset`, if the bytes are present.
fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let chunk = bytes.get(offset..)?.first_chunk::<4>()?;
    Some(u32::from_le_bytes(*chunk))
}

/// Return a truncation error with `context`.
fn truncated(context: &'static str) -> Error {
    Error::new(ErrorKind::UnexpectedEof, None).with_context(context)
}

/// Return an invalid-format error with `context`.
fn invalid(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, None).with_context(context)
}

/// Return the error for a count that does not fit in memory.
fn too_large(context: &'static str) -> Error {
    Error::new(ErrorKind::LimitExceeded, None).with_context(context)
}
