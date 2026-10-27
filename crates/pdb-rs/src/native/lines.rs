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
//! available source or producer defines their layout. [`ModuleLines`]
//! decodes all subsections of one module stream and validates the file
//! checksum references.
//!
//! Each decoded value has an `encode` function that appends the subsection
//! data again. [`LineSubsection::encode_framed`] adds the subsection header
//! and the alignment padding. Encoders for new data take typed parts, for
//! example [`LinesHeader::encode`] and [`LineBlock::encode_parts`].
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
use core::fmt;

use crate::native::output::Output;
use crate::{Bytes, Error, ErrorKind, Result};

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
};
pub use checksums::{ChecksumKind, FileChecksum, FileChecksums, FileChecksumsIter};
pub use coff_symbol_rva::{CoffSymbolRvas, CoffSymbolRvasIter};
pub use cross_scope::{
    CrossScopeContext, CrossScopeExport, CrossScopeExports, CrossScopeExportsIter, CrossScopeIds,
    CrossScopeIdsIter, CrossScopeImport, CrossScopeImports, CrossScopeImportsIter,
    CrossScopeModules, CrossScopeRule, CrossScopeViolation, validate_cross_scope,
};
pub use frame_data::{FrameDataRecords, FrameDataRecordsIter, FrameDataSubsection};
pub use inlinee::{
    ChecksumOffsets, ChecksumOffsetsIter, InlineeLine, InlineeLines, InlineeLinesIter,
    InlineeSignature,
};
pub use module::{ModuleLines, ModuleSubsection};
pub use string_table::{StringTableSubsection, StringTableSubsectionIter};
pub use symbol_records::SymbolsSubsection;
pub use token_map::{TokenMap, TokenMapEntries, TokenMapEntriesIter, TokenMapEntry, TokenMapValue};
pub use xfg_hash::{
    XfgTypeHash, XfgTypeHashes, XfgTypeHashesIter, XfgVirtualHash, XfgVirtualHashes,
    XfgVirtualHashesIter,
};

/// The kind of a C13 subsection (`DEBUG_S_SUBSECTION_TYPE` in `cvinfo.h`).
///
/// The type keeps values that this crate does not know. `Debug` prints the
/// `cvinfo.h` name of a known kind and the hexadecimal value of another
/// kind. `LowerHex` and `UpperHex` format the raw value.
#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SubsectionKind(pub u32);

/// The kind bit that tells a reader to ignore the subsection.
pub const DEBUG_S_IGNORE: SubsectionKind = SubsectionKind(0x8000_0000);
/// Symbol records (`DEBUG_S_SYMBOLS`).
pub const DEBUG_S_SYMBOLS: SubsectionKind = SubsectionKind(0xf1);
/// Line blocks (`DEBUG_S_LINES`).
pub const DEBUG_S_LINES: SubsectionKind = SubsectionKind(0xf2);
/// A string table (`DEBUG_S_STRINGTABLE`).
pub const DEBUG_S_STRINGTABLE: SubsectionKind = SubsectionKind(0xf3);
/// File checksums (`DEBUG_S_FILECHKSMS`).
pub const DEBUG_S_FILECHKSMS: SubsectionKind = SubsectionKind(0xf4);
/// Frame data (`DEBUG_S_FRAMEDATA`).
pub const DEBUG_S_FRAMEDATA: SubsectionKind = SubsectionKind(0xf5);
/// Inlinee source lines (`DEBUG_S_INLINEELINES`).
pub const DEBUG_S_INLINEELINES: SubsectionKind = SubsectionKind(0xf6);
/// Cross-scope imports (`DEBUG_S_CROSSSCOPEIMPORTS`).
pub const DEBUG_S_CROSSSCOPEIMPORTS: SubsectionKind = SubsectionKind(0xf7);
/// Cross-scope exports (`DEBUG_S_CROSSSCOPEEXPORTS`).
pub const DEBUG_S_CROSSSCOPEEXPORTS: SubsectionKind = SubsectionKind(0xf8);
/// IL lines (`DEBUG_S_IL_LINES`).
pub const DEBUG_S_IL_LINES: SubsectionKind = SubsectionKind(0xf9);
/// A function metadata token map (`DEBUG_S_FUNC_MDTOKEN_MAP`).
pub const DEBUG_S_FUNC_MDTOKEN_MAP: SubsectionKind = SubsectionKind(0xfa);
/// A type metadata token map (`DEBUG_S_TYPE_MDTOKEN_MAP`).
pub const DEBUG_S_TYPE_MDTOKEN_MAP: SubsectionKind = SubsectionKind(0xfb);
/// Merged assembly input (`DEBUG_S_MERGED_ASSEMBLYINPUT`).
pub const DEBUG_S_MERGED_ASSEMBLYINPUT: SubsectionKind = SubsectionKind(0xfc);
/// COFF symbol RVAs (`DEBUG_S_COFF_SYMBOL_RVA`).
pub const DEBUG_S_COFF_SYMBOL_RVA: SubsectionKind = SubsectionKind(0xfd);
/// An inlinee metadata token map (`DEBUG_S_INLINEE_MDTOKEN_MAP`).
pub const DEBUG_S_INLINEE_MDTOKEN_MAP: SubsectionKind = SubsectionKind(0xfe);
/// XFG type hashes (`DEBUG_S_XFGHASH_TYPE`).
pub const DEBUG_S_XFGHASH_TYPE: SubsectionKind = SubsectionKind(0xff);
/// XFG virtual hashes (`DEBUG_S_XFGHASH_VIRTUAL`).
pub const DEBUG_S_XFGHASH_VIRTUAL: SubsectionKind = SubsectionKind(0x100);

impl SubsectionKind {
    /// Return the `cvinfo.h` name of the kind, or `None` for an unknown
    /// value.
    ///
    /// A kind with the [`DEBUG_S_IGNORE`] bit and other bits has no name.
    #[must_use]
    pub const fn name(self) -> Option<&'static str> {
        Some(match self {
            DEBUG_S_IGNORE => "DEBUG_S_IGNORE",
            DEBUG_S_SYMBOLS => "DEBUG_S_SYMBOLS",
            DEBUG_S_LINES => "DEBUG_S_LINES",
            DEBUG_S_STRINGTABLE => "DEBUG_S_STRINGTABLE",
            DEBUG_S_FILECHKSMS => "DEBUG_S_FILECHKSMS",
            DEBUG_S_FRAMEDATA => "DEBUG_S_FRAMEDATA",
            DEBUG_S_INLINEELINES => "DEBUG_S_INLINEELINES",
            DEBUG_S_CROSSSCOPEIMPORTS => "DEBUG_S_CROSSSCOPEIMPORTS",
            DEBUG_S_CROSSSCOPEEXPORTS => "DEBUG_S_CROSSSCOPEEXPORTS",
            DEBUG_S_IL_LINES => "DEBUG_S_IL_LINES",
            DEBUG_S_FUNC_MDTOKEN_MAP => "DEBUG_S_FUNC_MDTOKEN_MAP",
            DEBUG_S_TYPE_MDTOKEN_MAP => "DEBUG_S_TYPE_MDTOKEN_MAP",
            DEBUG_S_MERGED_ASSEMBLYINPUT => "DEBUG_S_MERGED_ASSEMBLYINPUT",
            DEBUG_S_COFF_SYMBOL_RVA => "DEBUG_S_COFF_SYMBOL_RVA",
            DEBUG_S_INLINEE_MDTOKEN_MAP => "DEBUG_S_INLINEE_MDTOKEN_MAP",
            DEBUG_S_XFGHASH_TYPE => "DEBUG_S_XFGHASH_TYPE",
            DEBUG_S_XFGHASH_VIRTUAL => "DEBUG_S_XFGHASH_VIRTUAL",
            _ => return None,
        })
    }

    /// Return `true` if the kind has the [`DEBUG_S_IGNORE`] bit.
    #[must_use]
    pub const fn is_ignored(self) -> bool {
        self.0 & DEBUG_S_IGNORE.0 != 0
    }

    /// Return the kind without the [`DEBUG_S_IGNORE`] bit.
    #[must_use]
    pub const fn without_ignore_flag(self) -> Self {
        Self(self.0 & !DEBUG_S_IGNORE.0)
    }

    /// Return `true` if [`LineSubsection::decode`] decodes this kind.
    ///
    /// The function returns `false` for a kind with the [`DEBUG_S_IGNORE`]
    /// bit: the decoder handles these kinds explicitly and does not decode
    /// the data.
    #[must_use]
    pub const fn is_decoded(self) -> bool {
        matches!(
            self,
            DEBUG_S_SYMBOLS
                | DEBUG_S_LINES
                | DEBUG_S_STRINGTABLE
                | DEBUG_S_FILECHKSMS
                | DEBUG_S_FRAMEDATA
                | DEBUG_S_INLINEELINES
                | DEBUG_S_CROSSSCOPEIMPORTS
                | DEBUG_S_CROSSSCOPEEXPORTS
                | DEBUG_S_IL_LINES
                | DEBUG_S_FUNC_MDTOKEN_MAP
                | DEBUG_S_TYPE_MDTOKEN_MAP
                | DEBUG_S_MERGED_ASSEMBLYINPUT
                | DEBUG_S_COFF_SYMBOL_RVA
                | DEBUG_S_XFGHASH_TYPE
                | DEBUG_S_XFGHASH_VIRTUAL
        )
    }
}

impl fmt::Debug for SubsectionKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => formatter.write_str(name),
            None => write!(formatter, "SubsectionKind({:#010x})", self.0),
        }
    }
}

impl fmt::LowerHex for SubsectionKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, formatter)
    }
}

impl fmt::UpperHex for SubsectionKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::UpperHex::fmt(&self.0, formatter)
    }
}

/// The data of one C13 subsection.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
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
        /// The kind, with the [`DEBUG_S_IGNORE`] bit.
        kind: SubsectionKind,
        /// The raw subsection data.
        data: Bytes<'a>,
    },
    /// A subsection of a kind that this crate does not decode.
    Unsupported {
        /// The kind.
        kind: SubsectionKind,
        /// The raw subsection data.
        data: Bytes<'a>,
    },
}

impl LineSubsection<'_> {
    /// Return a record that owns its bytes. Copy borrowed bytes.
    #[must_use]
    pub fn into_owned(self) -> LineSubsection<'static> {
        match self {
            Self::Symbols(value) => LineSubsection::Symbols(value.into_owned()),
            Self::Lines(value) => LineSubsection::Lines(value.into_owned()),
            Self::StringTable(value) => LineSubsection::StringTable(value.into_owned()),
            Self::FileChecksums(value) => LineSubsection::FileChecksums(value.into_owned()),
            Self::FrameData(value) => LineSubsection::FrameData(value.into_owned()),
            Self::InlineeLines(value) => LineSubsection::InlineeLines(value.into_owned()),
            Self::CrossScopeImports(value) => LineSubsection::CrossScopeImports(value.into_owned()),
            Self::CrossScopeExports(value) => LineSubsection::CrossScopeExports(value.into_owned()),
            Self::IlLines(value) => LineSubsection::IlLines(value.into_owned()),
            Self::FunctionTokenMap(value) => LineSubsection::FunctionTokenMap(value.into_owned()),
            Self::TypeTokenMap(value) => LineSubsection::TypeTokenMap(value.into_owned()),
            Self::MergedAssemblyInput(value) => {
                LineSubsection::MergedAssemblyInput(value.into_owned())
            }
            Self::CoffSymbolRvas(value) => LineSubsection::CoffSymbolRvas(value.into_owned()),
            Self::XfgTypeHashes(value) => LineSubsection::XfgTypeHashes(value.into_owned()),
            Self::XfgVirtualHashes(value) => LineSubsection::XfgVirtualHashes(value.into_owned()),
            Self::Ignored { kind, data } => LineSubsection::Ignored {
                kind,
                data: data.into_owned(),
            },
            Self::Unsupported { kind, data } => LineSubsection::Unsupported {
                kind,
                data: data.into_owned(),
            },
        }
    }
}

impl crate::bytes::IntoOwned for LineSubsection<'_> {
    type Owned = LineSubsection<'static>;

    fn to_static(self) -> LineSubsection<'static> {
        self.into_owned()
    }
}

impl<'a> LineSubsection<'a> {
    /// Decode the data of a subsection of `kind`.
    ///
    /// `data` excludes the subsection header and the alignment padding
    /// after the data, as in [`Subsection::data`](crate::native::dbi::Subsection::data).
    /// Return [`LineSubsection::Ignored`] for a kind with the
    /// [`DEBUG_S_IGNORE`] bit. Return [`LineSubsection::Unsupported`] for
    /// another kind that [`SubsectionKind::is_decoded`] rejects.
    ///
    /// # Errors
    ///
    /// Returns the errors of the `parse` function of the kind, for example
    /// [`LinesSubsection::parse`]. The errors do not have a file offset or
    /// a stream location.
    pub fn decode(kind: SubsectionKind, data: &'a [u8]) -> Result<Self> {
        Ok(match kind {
            DEBUG_S_SYMBOLS => Self::Symbols(SymbolsSubsection::parse(data)?),
            DEBUG_S_LINES => Self::Lines(LinesSubsection::parse(data)?),
            DEBUG_S_STRINGTABLE => Self::StringTable(StringTableSubsection::parse(data)?),
            DEBUG_S_FILECHKSMS => Self::FileChecksums(FileChecksums::parse(data)?),
            DEBUG_S_FRAMEDATA => Self::FrameData(FrameDataSubsection::parse(data)?),
            DEBUG_S_INLINEELINES => Self::InlineeLines(InlineeLines::parse(data)?),
            DEBUG_S_CROSSSCOPEIMPORTS => Self::CrossScopeImports(CrossScopeImports::parse(data)?),
            DEBUG_S_CROSSSCOPEEXPORTS => Self::CrossScopeExports(CrossScopeExports::parse(data)?),
            DEBUG_S_IL_LINES => Self::IlLines(LinesSubsection::parse(data)?),
            DEBUG_S_FUNC_MDTOKEN_MAP => Self::FunctionTokenMap(TokenMap::parse(data)?),
            DEBUG_S_TYPE_MDTOKEN_MAP => Self::TypeTokenMap(TokenMap::parse(data)?),
            DEBUG_S_MERGED_ASSEMBLYINPUT => {
                Self::MergedAssemblyInput(MergedAssemblyInput::parse(data)?)
            }
            DEBUG_S_COFF_SYMBOL_RVA => Self::CoffSymbolRvas(CoffSymbolRvas::parse(data)?),
            DEBUG_S_XFGHASH_TYPE => Self::XfgTypeHashes(XfgTypeHashes::parse(data)?),
            DEBUG_S_XFGHASH_VIRTUAL => Self::XfgVirtualHashes(XfgVirtualHashes::parse(data)?),
            _ if kind.is_ignored() => Self::Ignored {
                kind,
                data: Bytes::new(data),
            },
            _ => Self::Unsupported {
                kind,
                data: Bytes::new(data),
            },
        })
    }

    /// Return the kind of the subsection.
    #[must_use]
    pub const fn kind(&self) -> SubsectionKind {
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
            Self::Ignored { kind, .. } | Self::Unsupported { kind, .. } => *kind,
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
        Output::new(out).u32(self.kind().0)?;
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
        #[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
        pub struct $list<'a> {
            /// The serialized entries, without a partial entry at the end.
            entries: $crate::Bytes<'a>,
        }

        $crate::bytes::into_owned!(struct $list { entries; });

        impl<'a> $list<'a> {
            /// Wrap serialized entries. Ignore a partial entry at the end.
            pub(super) fn new(bytes: &'a [u8]) -> Self {
                let whole = bytes.len() - bytes.len() % $size;
                Self {
                    entries: $crate::Bytes::new(&bytes[..whole]),
                }
            }

            /// Return the serialized entries.
            fn chunks(&self) -> &[[u8; $size]] {
                self.entries.as_chunks::<{ $size }>().0
            }

            /// Return the number of entries.
            #[must_use]
            pub fn len(&self) -> usize {
                self.entries.len() / $size
            }

            /// Return `true` if the list has no entries.
            #[must_use]
            pub fn is_empty(&self) -> bool {
                self.entries.is_empty()
            }

            /// Return the entry at `position`, or `None` past the end.
            #[must_use]
            pub fn get(&self, position: usize) -> Option<$entry> {
                self.chunks().get(position).copied().map($parse)
            }

            /// Return an iterator over the entries.
            #[must_use]
            pub fn iter(&self) -> $iter<'_> {
                $iter {
                    entries: self.chunks().iter(),
                }
            }
        }

        impl<'s> IntoIterator for &'s $list<'_> {
            type Item = $entry;
            type IntoIter = $iter<'s>;

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
