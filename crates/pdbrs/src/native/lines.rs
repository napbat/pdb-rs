//! C13 line information in module streams.
//!
//! The C13 line region of a module stream holds subsections. Each
//! subsection has a kind and data. [`LineSubsection::decode`] decodes the
//! data of these kinds:
//!
//! - [`DEBUG_S_SYMBOLS`]: symbol records ([`SymbolsSubsection`]).
//! - [`DEBUG_S_LINES`]: line blocks of one code contribution.
//! - [`DEBUG_S_STRINGTABLE`]: the file names of an object file
//!   ([`StringTableSubsection`]).
//! - [`DEBUG_S_FILECHKSMS`]: the file checksum table of the module.
//! - [`DEBUG_S_FRAMEDATA`]: x86 frame data ([`FrameDataSubsection`]).
//! - [`DEBUG_S_INLINEELINES`]: the source lines of inlined functions.
//! - [`DEBUG_S_CROSSSCOPEIMPORTS`] and [`DEBUG_S_CROSSSCOPEEXPORTS`]: the
//!   item IDs that the module uses from other modules and gives to them.
//! - [`DEBUG_S_IL_LINES`]: line blocks of IL code, in the layout of
//!   [`DEBUG_S_LINES`].
//! - [`DEBUG_S_FUNC_MDTOKEN_MAP`] and [`DEBUG_S_TYPE_MDTOKEN_MAP`]: metadata
//!   token maps ([`TokenMap`]).
//! - [`DEBUG_S_MERGED_ASSEMBLYINPUT`]: the inputs of a merged assembly.
//! - [`DEBUG_S_COFF_SYMBOL_RVA`]: the RVAs of the COFF symbols of the object
//!   file, in FASTLINK PDBs.
//! - [`DEBUG_S_XFGHASH_TYPE`] and [`DEBUG_S_XFGHASH_VIRTUAL`]: the XFG
//!   hashes of an object file ([`XfgTypeHashes`], [`XfgVirtualHashes`]).
//!
//! A kind with the [`DEBUG_S_IGNORE`] bit gives [`LineSubsection::Ignored`]:
//! `cvinfo.h` tells readers to ignore the contents. The data of
//! [`DEBUG_S_INLINEE_MDTOKEN_MAP`] and unknown kinds stays raw, because no
//! available source or producer defines their layout (#31). [`ModuleLines`]
//! decodes all subsections of one module stream and validates the file
//! checksum references.
//!
//! Each decoded value has an `encode` function that appends the subsection
//! data again. [`LineSubsection::encode_framed`] adds the subsection header
//! and the alignment padding. Encoders for new data take typed parts, for
//! example [`LinesHeader::encode`] and [`encode_line_block`].
//!
//! File names stay byte offsets into the `/names` stream. This module does
//! not read the `/names` stream.
//!
//! The layouts come from these sources:
//!
//! - `docs/reference/microsoft-pdb-rs/docs/codeview/line_data.md`.
//! - `DebugLinesSubsection.cpp`, `DebugChecksumsSubsection.cpp`,
//!   `DebugInlineeLinesSubsection.cpp`, `DebugSymbolRVASubsection.h`,
//!   `DebugSymbolsSubsection.cpp`, `DebugStringTableSubsection.cpp`,
//!   `DebugFrameDataSubsection.cpp`, `DebugCrossImpSubsection.cpp` and
//!   `DebugCrossExSubsection.cpp` of LLVM.
//! - `CV_DebugSLinesHeader_t`, `CV_Line_t`, `CV_Column_t`, `tagFRAMEDATA`,
//!   `CrossScopeReferences`, `LocalIdAndGlobalIdPair` and
//!   `DEBUG_S_SUBSECTION_TYPE` in `cvinfo.h` of `microsoft/microsoft-pdb`.
//! - `Mod1::fUpdateTiOrIdReferencedByNonSyms` in `PDB/dbi/mod.cpp` and
//!   `DumpModTokenMap`, `DumpModMergedAssemblyInput` and `DumpModILLines`
//!   in `cvdump/dumpsym7.cpp` of `microsoft/microsoft-pdb`.

use alloc::vec::Vec;

use crate::native::output::Output;
use crate::{Error, ErrorKind, Result};

mod assembly_input;
mod blocks;
mod checksums;
mod coff_symbol_rva;
mod cross_scope;
mod frame_data;
mod inlinee;
mod module;
mod string_table;
mod symbol_records;
mod token_map;
mod xfg_hash;

pub use assembly_input::{AssemblyInput, MergedAssemblyInput, MergedAssemblyInputIter};
pub use blocks::{
    ALWAYS_STEP_INTO_LINE, ColumnEntries, ColumnEntriesIter, ColumnEntry, LineBlock, LineBlocks,
    LineEntries, LineEntriesIter, LineEntry, LinesHeader, LinesSubsection, NEVER_STEP_INTO_LINE,
    encode_line_block,
};
pub use checksums::{
    ChecksumKind, FileChecksum, FileChecksums, FileChecksumsIter, encode_file_checksum,
};
pub use coff_symbol_rva::{CoffSymbolRvas, CoffSymbolRvasIter, encode_coff_symbol_rvas};
pub use cross_scope::{
    CrossScopeContext, CrossScopeExport, CrossScopeExports, CrossScopeExportsIter, CrossScopeIds,
    CrossScopeIdsIter, CrossScopeImport, CrossScopeImports, CrossScopeImportsIter,
    CrossScopeModules, CrossScopeRule, CrossScopeViolation, validate_cross_scope,
};
pub use frame_data::{FrameDataRecords, FrameDataRecordsIter, FrameDataSubsection};
pub use inlinee::{
    ChecksumOffsets, ChecksumOffsetsIter, InlineeLine, InlineeLines, InlineeLinesIter,
    InlineeSignature, encode_inlinee_line,
};
pub use module::{ModuleLines, ModuleSubsection};
pub use string_table::{StringTableIter, StringTableSubsection};
pub use symbol_records::SymbolsSubsection;
pub use token_map::{TokenMap, TokenMapEntries, TokenMapEntriesIter, TokenMapEntry, TokenMapValue};
pub use xfg_hash::{
    XfgTypeHash, XfgTypeHashes, XfgTypeHashesIter, XfgVirtualHash, XfgVirtualHashes,
    XfgVirtualHashesIter,
};

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
    /// [`DEBUG_S_SYMBOLS`]. This crate decodes it.
    Symbols,
    /// [`DEBUG_S_LINES`]. This crate decodes it.
    Lines,
    /// [`DEBUG_S_STRINGTABLE`]. This crate decodes it.
    StringTable,
    /// [`DEBUG_S_FILECHKSMS`]. This crate decodes it.
    FileChecksums,
    /// [`DEBUG_S_FRAMEDATA`]. This crate decodes it.
    FrameData,
    /// [`DEBUG_S_INLINEELINES`]. This crate decodes it.
    InlineeLines,
    /// [`DEBUG_S_CROSSSCOPEIMPORTS`]. This crate decodes it.
    CrossScopeImports,
    /// [`DEBUG_S_CROSSSCOPEEXPORTS`]. This crate decodes it.
    CrossScopeExports,
    /// [`DEBUG_S_IL_LINES`]. This crate decodes it.
    IlLines,
    /// [`DEBUG_S_FUNC_MDTOKEN_MAP`]. This crate decodes it.
    FunctionTokenMap,
    /// [`DEBUG_S_TYPE_MDTOKEN_MAP`]. This crate decodes it.
    TypeTokenMap,
    /// [`DEBUG_S_MERGED_ASSEMBLYINPUT`]. This crate decodes it.
    MergedAssemblyInput,
    /// [`DEBUG_S_COFF_SYMBOL_RVA`]. This crate decodes it.
    CoffSymbolRva,
    /// [`DEBUG_S_INLINEE_MDTOKEN_MAP`]. No available source defines the layout.
    InlineeTokenMap,
    /// [`DEBUG_S_XFGHASH_TYPE`]. This crate decodes it.
    XfgHashType,
    /// [`DEBUG_S_XFGHASH_VIRTUAL`]. This crate decodes it.
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

    /// Return the raw kind value.
    #[must_use]
    pub const fn raw(self) -> u32 {
        match self {
            Self::Symbols => DEBUG_S_SYMBOLS,
            Self::Lines => DEBUG_S_LINES,
            Self::StringTable => DEBUG_S_STRINGTABLE,
            Self::FileChecksums => DEBUG_S_FILECHKSMS,
            Self::FrameData => DEBUG_S_FRAMEDATA,
            Self::InlineeLines => DEBUG_S_INLINEELINES,
            Self::CrossScopeImports => DEBUG_S_CROSSSCOPEIMPORTS,
            Self::CrossScopeExports => DEBUG_S_CROSSSCOPEEXPORTS,
            Self::IlLines => DEBUG_S_IL_LINES,
            Self::FunctionTokenMap => DEBUG_S_FUNC_MDTOKEN_MAP,
            Self::TypeTokenMap => DEBUG_S_TYPE_MDTOKEN_MAP,
            Self::MergedAssemblyInput => DEBUG_S_MERGED_ASSEMBLYINPUT,
            Self::CoffSymbolRva => DEBUG_S_COFF_SYMBOL_RVA,
            Self::InlineeTokenMap => DEBUG_S_INLINEE_MDTOKEN_MAP,
            Self::XfgHashType => DEBUG_S_XFGHASH_TYPE,
            Self::XfgHashVirtual => DEBUG_S_XFGHASH_VIRTUAL,
            Self::Ignored(kind) | Self::Unknown(kind) => kind,
        }
    }

    /// Return `true` if [`LineSubsection::decode`] decodes this kind.
    ///
    /// The function returns `false` for [`SubsectionKind::Ignored`]: the
    /// decoder handles that class explicitly and does not decode the data.
    #[must_use]
    pub const fn is_decoded(self) -> bool {
        !matches!(
            self,
            Self::InlineeTokenMap | Self::Ignored(_) | Self::Unknown(_)
        )
    }
}

/// The data of one C13 subsection.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LineSubsection<'a> {
    /// A [`DEBUG_S_SYMBOLS`] subsection.
    Symbols(SymbolsSubsection<'a>),
    /// A [`DEBUG_S_LINES`] subsection.
    Lines(LinesSubsection<'a>),
    /// A [`DEBUG_S_STRINGTABLE`] subsection.
    StringTable(StringTableSubsection<'a>),
    /// A [`DEBUG_S_FILECHKSMS`] subsection.
    FileChecksums(FileChecksums<'a>),
    /// A [`DEBUG_S_FRAMEDATA`] subsection.
    FrameData(FrameDataSubsection<'a>),
    /// A [`DEBUG_S_INLINEELINES`] subsection.
    InlineeLines(InlineeLines<'a>),
    /// A [`DEBUG_S_CROSSSCOPEIMPORTS`] subsection.
    CrossScopeImports(CrossScopeImports<'a>),
    /// A [`DEBUG_S_CROSSSCOPEEXPORTS`] subsection.
    CrossScopeExports(CrossScopeExports<'a>),
    /// A [`DEBUG_S_IL_LINES`] subsection. The code offsets are IL offsets.
    IlLines(LinesSubsection<'a>),
    /// A [`DEBUG_S_FUNC_MDTOKEN_MAP`] subsection.
    FunctionTokenMap(TokenMap<'a>),
    /// A [`DEBUG_S_TYPE_MDTOKEN_MAP`] subsection.
    TypeTokenMap(TokenMap<'a>),
    /// A [`DEBUG_S_MERGED_ASSEMBLYINPUT`] subsection.
    MergedAssemblyInput(MergedAssemblyInput<'a>),
    /// A [`DEBUG_S_COFF_SYMBOL_RVA`] subsection.
    CoffSymbolRvas(CoffSymbolRvas<'a>),
    /// A [`DEBUG_S_XFGHASH_TYPE`] subsection.
    XfgTypeHashes(XfgTypeHashes<'a>),
    /// A [`DEBUG_S_XFGHASH_VIRTUAL`] subsection.
    XfgVirtualHashes(XfgVirtualHashes<'a>),
    /// A subsection with the [`DEBUG_S_IGNORE`] bit in its kind.
    ///
    /// `cvinfo.h` tells readers to ignore the contents, so the data stays
    /// raw for all values of the other kind bits.
    Ignored {
        /// The raw kind, with the [`DEBUG_S_IGNORE`] bit.
        kind: u32,
        /// The raw subsection data.
        data: &'a [u8],
    },
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
    /// Return [`LineSubsection::Ignored`] for a kind with the
    /// [`DEBUG_S_IGNORE`] bit. Return [`LineSubsection::Unsupported`] for
    /// another kind that [`SubsectionKind::is_decoded`] rejects.
    ///
    /// # Errors
    ///
    /// Returns the errors of the `decode` function of the kind, for example
    /// [`LinesSubsection::decode`]. The errors do not have a file offset or
    /// a stream location.
    pub fn decode(kind: u32, data: &'a [u8]) -> Result<Self> {
        Ok(match SubsectionKind::from_raw(kind) {
            SubsectionKind::Symbols => Self::Symbols(SymbolsSubsection::decode(data)?),
            SubsectionKind::Lines => Self::Lines(LinesSubsection::decode(data)?),
            SubsectionKind::StringTable => Self::StringTable(StringTableSubsection::decode(data)?),
            SubsectionKind::FileChecksums => Self::FileChecksums(FileChecksums::decode(data)?),
            SubsectionKind::FrameData => Self::FrameData(FrameDataSubsection::decode(data)?),
            SubsectionKind::InlineeLines => Self::InlineeLines(InlineeLines::decode(data)?),
            SubsectionKind::CrossScopeImports => {
                Self::CrossScopeImports(CrossScopeImports::decode(data)?)
            }
            SubsectionKind::CrossScopeExports => {
                Self::CrossScopeExports(CrossScopeExports::decode(data)?)
            }
            SubsectionKind::IlLines => Self::IlLines(LinesSubsection::decode(data)?),
            SubsectionKind::FunctionTokenMap => Self::FunctionTokenMap(TokenMap::decode(data)?),
            SubsectionKind::TypeTokenMap => Self::TypeTokenMap(TokenMap::decode(data)?),
            SubsectionKind::MergedAssemblyInput => {
                Self::MergedAssemblyInput(MergedAssemblyInput::decode(data)?)
            }
            SubsectionKind::CoffSymbolRva => Self::CoffSymbolRvas(CoffSymbolRvas::decode(data)?),
            SubsectionKind::XfgHashType => Self::XfgTypeHashes(XfgTypeHashes::decode(data)?),
            SubsectionKind::XfgHashVirtual => {
                Self::XfgVirtualHashes(XfgVirtualHashes::decode(data)?)
            }
            SubsectionKind::Ignored(kind) => Self::Ignored { kind, data },
            kind => Self::Unsupported { kind, data },
        })
    }

    /// Return the raw kind of the subsection.
    #[must_use]
    pub const fn kind(&self) -> u32 {
        match self {
            Self::Symbols(_) => DEBUG_S_SYMBOLS,
            Self::Lines(_) => DEBUG_S_LINES,
            Self::StringTable(_) => DEBUG_S_STRINGTABLE,
            Self::FileChecksums(_) => DEBUG_S_FILECHKSMS,
            Self::FrameData(_) => DEBUG_S_FRAMEDATA,
            Self::InlineeLines(_) => DEBUG_S_INLINEELINES,
            Self::CrossScopeImports(_) => DEBUG_S_CROSSSCOPEIMPORTS,
            Self::CrossScopeExports(_) => DEBUG_S_CROSSSCOPEEXPORTS,
            Self::IlLines(_) => DEBUG_S_IL_LINES,
            Self::FunctionTokenMap(_) => DEBUG_S_FUNC_MDTOKEN_MAP,
            Self::TypeTokenMap(_) => DEBUG_S_TYPE_MDTOKEN_MAP,
            Self::MergedAssemblyInput(_) => DEBUG_S_MERGED_ASSEMBLYINPUT,
            Self::CoffSymbolRvas(_) => DEBUG_S_COFF_SYMBOL_RVA,
            Self::XfgTypeHashes(_) => DEBUG_S_XFGHASH_TYPE,
            Self::XfgVirtualHashes(_) => DEBUG_S_XFGHASH_VIRTUAL,
            Self::Ignored { kind, .. } => *kind,
            Self::Unsupported { kind, .. } => kind.raw(),
        }
    }

    /// Append the subsection data to `out`, without the header and the
    /// alignment padding.
    ///
    /// The function encodes the decoded values again. For
    /// [`LineSubsection::Ignored`] and [`LineSubsection::Unsupported`], it
    /// appends the raw data. The output is equal to the data that
    /// [`LineSubsection::decode`] read.
    ///
    /// # Errors
    ///
    /// Returns the errors of the `encode` function of the kind, for example
    /// [`LinesSubsection::encode`]. Returns [`ErrorKind::LimitExceeded`] if
    /// the output cannot grow.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<()> {
        match self {
            Self::Symbols(value) => value.encode(out),
            Self::Lines(value) | Self::IlLines(value) => value.encode(out),
            Self::StringTable(value) => value.encode(out),
            Self::FileChecksums(value) => value.encode(out),
            Self::FrameData(value) => value.encode(out),
            Self::InlineeLines(value) => value.encode(out),
            Self::CrossScopeImports(value) => value.encode(out),
            Self::CrossScopeExports(value) => value.encode(out),
            Self::FunctionTokenMap(value) | Self::TypeTokenMap(value) => value.encode(out),
            Self::MergedAssemblyInput(value) => value.encode(out),
            Self::CoffSymbolRvas(value) => value.encode(out),
            Self::XfgTypeHashes(value) => value.encode(out),
            Self::XfgVirtualHashes(value) => value.encode(out),
            Self::Ignored { data, .. } | Self::Unsupported { data, .. } => {
                Output::new(out).bytes(data)
            }
        }
    }

    /// Append the complete subsection to `out`: the header, the data and
    /// zero bytes up to a 4-byte boundary.
    ///
    /// The header length is the data length without the padding. The
    /// subsection must start at a 4-byte boundary of the C13 line region.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`LineSubsection::encode`]. Returns
    /// [`ErrorKind::LimitExceeded`] if the data length does not fit in
    /// 32 bits.
    pub fn encode_framed(&self, out: &mut Vec<u8>) -> Result<()> {
        let start = out.len();
        Output::new(out).u32(self.kind())?;
        Output::new(out).u32(0)?;
        self.encode(out)?;
        // The header is 8 bytes, so `start + 8` is inside `out`.
        let size = out.len() - start - 8;
        let length = u32::try_from(size)
            .map_err(|_| too_large("C13 subsection data does not fit in 32 bits"))?;
        out[start + 4..start + 8].copy_from_slice(&length.to_le_bytes());
        pad4(out, size)
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

/// Append zero bytes to `out` after `size` bytes of an item that starts at
/// a 4-byte boundary, so that the next item also starts at a 4-byte
/// boundary.
fn pad4(out: &mut Vec<u8>, size: usize) -> Result<()> {
    let padding = (4 - size % 4) % 4;
    Output::new(out).zeros(padding)
}

/// Convert a count or a size to the `u32` of its field.
fn to_u32(value: usize, context: &'static str) -> Result<u32> {
    u32::try_from(value).map_err(|_| too_large(context))
}
