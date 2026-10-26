//! Address and name queries over the symbols and lines of a native PDB.
//!
//! [`SymbolLocator::new`] loads the streams that the queries need and makes
//! these indexes:
//!
//! - The procedures, managed procedures and thunks of all modules, by RVA.
//! - The `DEBUG_S_LINES` code contributions of all modules, by RVA.
//! - The DBI section contributions, by RVA.
//! - The public symbols, by RVA.
//! - The file checksum entries and the inlinee start lines of each module.
//! - The qualified names of all inlinees of `S_INLINESITE` records.
//!
//! [`SymbolLocator::locate`] gives the [`Location`] of an RVA: the module,
//! the procedure, the innermost block, the inline call chain and the source
//! line. [`SymbolLocator::find_name`] finds the global and public symbols
//! with a name through the hash tables of the global and public symbol
//! indexes. [`SymbolLocator::public_at`] gives the public symbol at or before
//! an RVA.
//!
//! # Inline call chains
//!
//! An inline site has the line ranges of the inlined code
//! ([`InlineRanges`]). The innermost inline frame is the last inline site
//! of the procedure with a range that holds the address. Its parent links
//! give the outer inline frames. Each [`InlineFrame`] has the line in the
//! inlined function at the address, and the call site: the line of the
//! parent frame at the address. The parent frame of the outermost inline
//! frame is the procedure. Its line comes from the `DEBUG_S_LINES`
//! subsections ([`Location::line`]).
//!
//! # Separated code
//!
//! A compiler can move a part of a procedure to a separated code chunk, for
//! example cold code after profile-guided optimization. An `S_SEPCODE`
//! record gives the chunk. Its parent address is the start of the
//! procedure. MSVC writes it at the top level, after the end of the
//! procedure. [`SymbolLocator::code_chunks`] gives the chunks of a
//! procedure. An address in a chunk gives the procedure of the chunk, as DIA
//! does, and the inline frames from the ranges of the chunk.
//!
//! # Limits
//!
//! The locator translates addresses through the OMAP tables of an image
//! that a tool rewrote. An address without an OMAP mapping has no location.
//! The locator indexes only the records before a record that this crate
//! cannot decode, for example a constant with an unknown numeric leaf.
//! [`SymbolLocator::incomplete_modules`] lists the modules with such a
//! record. A module symbol scope can have at most [`MAX_SCOPE_DEPTH`] levels.
//!
//! The locator keeps names as bytes. It does not demangle or undecorate them.

use alloc::vec::Vec;

use super::address::{AddressMap, Rva, SectionOffset};
use super::codeview::SymbolKind;
use super::codeview::index::{
    ChecksumOffset, IdIndex, ModuleIndex, ModuleSymbolOffset, NameOffset,
};
use super::codeview::symbol::Public;
use super::dbi::{Dbi, ModuleStream};
use super::section::SectionHeaders;
use super::symbols::{GlobalSymbolIndex, PublicSymbolIndex, SymbolOffset};
use crate::{Error, ErrorKind, Result};

mod build;
mod inline;
mod inlinee;
mod lines;
mod name;
mod range;
mod scope;

pub use inline::{InlineRange, InlineRanges};
pub use lines::{LineMatch, find_line};
pub use name::NameMatch;
pub use scope::{CodeChunk, InlineSiteRanges};

use range::RangeIndex;

/// The deepest symbol scope nesting that the locator accepts in a module.
pub const MAX_SCOPE_DEPTH: usize = 256;

/// Address and name queries over one native PDB.
///
/// The value owns the loaded streams. It does not borrow the [`Pdb`](super::Pdb).
#[derive(Clone, Debug)]
pub struct SymbolLocator {
    /// The DBI stream.
    dbi: Dbi,
    /// The section header stream.
    section_headers: Vec<u8>,
    /// The string buffer of the `/names` stream, or `None` without `/names`.
    strings: Option<Vec<u8>>,
    /// The modules in DBI order.
    modules: Vec<ModuleData>,
    /// The procedures and thunks by RVA.
    procedures: RangeIndex<ProcedureKey>,
    /// The separated code chunks by RVA, with the procedure of each chunk.
    separated: RangeIndex<ProcedureKey>,
    /// The `DEBUG_S_LINES` subsections by RVA.
    lines: RangeIndex<LineKey>,
    /// The section contributions by RVA, with the module index.
    contributions: RangeIndex<ModuleIndex>,
    /// The public symbols by RVA. Each range has a length of 1.
    publics: RangeIndex<SymbolOffset>,
    /// The inlinee names, sorted by ID scope and ID.
    inlinees: Vec<InlineeName>,
    /// `true` if the PDB has the `NOTM` feature: the inlinee IDs of a module
    /// refer to the ID records of the module, not to IPI.
    no_type_merge: bool,
    /// The symbol record stream.
    symbol_records: Vec<u8>,
    /// The global symbol index, if the DBI stream names one.
    global_index: Option<GlobalSymbolIndex>,
    /// The public symbol index, if the DBI stream names one.
    public_index: Option<PublicSymbolIndex>,
    /// The modules whose symbol walk stopped at an undecoded scope kind.
    incomplete: Vec<ModuleIndex>,
}

/// The data of one module.
#[derive(Clone, Debug)]
struct ModuleData {
    /// The module name.
    name: Vec<u8>,
    /// The module stream, if the module has one.
    stream: Option<ModuleStream>,
    /// The file checksum offsets with their `/names` offsets, sorted.
    files: Vec<(ChecksumOffset, NameOffset)>,
    /// The inlinee lines: the inlinee, the file checksum offset and the
    /// start line. Sorted by inlinee; the first entry of an inlinee wins.
    inlinees: Vec<(IdIndex, ChecksumOffset, u32)>,
    /// The module stream offset of the record that stopped the symbol walk:
    /// a scope kind that this crate does not decode. The locator does not
    /// read records at or after it.
    walk_end: Option<ModuleSymbolOffset>,
    /// The parent address and the chunk of each `S_SEPCODE` record, in
    /// stream order.
    separated: Vec<(SectionOffset, SeparatedChunk)>,
}

/// An `S_SEPCODE` record of a module.
#[derive(Clone, Copy, Debug)]
struct SeparatedChunk {
    /// The module stream offset of the record.
    offset: ModuleSymbolOffset,
    /// The address of the separated code.
    address: SectionOffset,
    /// The length of the separated code in bytes.
    length: u32,
}

/// The place of a procedure record.
#[derive(Clone, Copy, Debug)]
struct ProcedureKey {
    /// The module index.
    module: ModuleIndex,
    /// The module stream offset of the record.
    offset: ModuleSymbolOffset,
}

/// The place of a `DEBUG_S_LINES` subsection.
#[derive(Clone, Copy, Debug)]
struct LineKey {
    /// The module index.
    module: ModuleIndex,
    /// The offset of the subsection data in the C13 line region.
    start: usize,
    /// The length of the subsection data.
    length: usize,
}

/// The name of one inlinee.
#[derive(Clone, Debug)]
struct InlineeName {
    /// The ID scope: the module whose ID records hold the inlinee in a `NOTM`
    /// PDB, else `inlinee::MERGED_IDS` (IPI).
    scope: u32,
    /// The inlinee.
    id: IdIndex,
    /// The qualified name.
    qualified: Vec<u8>,
    /// The start of the unqualified name in `qualified`.
    name_start: usize,
}

/// A module with its zero-based index.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ModuleRef<'a> {
    /// The zero-based index of the module in the DBI stream.
    pub index: ModuleIndex,
    /// The module name.
    pub name: &'a [u8],
}

/// A procedure, managed procedure or thunk record of a module.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ProcedureInfo<'a> {
    /// The zero-based module index.
    pub module: ModuleIndex,
    /// The offset of the record in the module stream.
    pub offset: ModuleSymbolOffset,
    /// The record kind, for example `S_GPROC32_ID`.
    pub kind: SymbolKind,
    /// The name.
    pub name: &'a [u8],
    /// The section-relative address of the first byte.
    pub address: SectionOffset,
    /// The RVA of the first byte, or `None` if the address is not in an
    /// image section.
    pub start: Option<Rva>,
    /// The length of the code in bytes.
    pub length: u32,
}

/// An `S_BLOCK32` scope inside a procedure.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct BlockInfo<'a> {
    /// The offset of the record in the module stream.
    pub offset: ModuleSymbolOffset,
    /// The block name. It is often empty.
    pub name: &'a [u8],
    /// The RVA of the first byte.
    pub start: Rva,
    /// The length of the block in bytes.
    pub length: u32,
}

/// A source line with its code range.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct SourceLine<'a> {
    /// The file name from the `/names` stream.
    pub file: &'a [u8],
    /// The byte offset of the file entry in the `DEBUG_S_FILECHKSMS`
    /// subsection of the module.
    pub checksum_offset: ChecksumOffset,
    /// The first line.
    ///
    /// `0xfeefee` and `0xf00f00` are special values; see
    /// [`ALWAYS_STEP_INTO_LINE`](crate::native::lines::ALWAYS_STEP_INTO_LINE).
    pub line: u32,
    /// The last line. It equals `line` for a range of one line.
    pub end_line: u32,
    /// The first column, if the data has columns.
    pub column: Option<u32>,
    /// The column after the end, if the data gives it.
    pub end_column: Option<u32>,
    /// The RVA of the first byte of the code range of the line.
    pub start: Rva,
    /// The length of the code range, or `None` if the data does not give it.
    pub length: Option<u32>,
    /// `true` if the range starts a statement.
    pub is_statement: bool,
}

/// One inlined function in an inline call chain.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct InlineFrame<'a> {
    /// The offset of the `S_INLINESITE` record in the module stream.
    pub offset: ModuleSymbolOffset,
    /// The `LF_FUNC_ID` or `LF_MFUNC_ID` record of the inlined function.
    pub inlinee: IdIndex,
    /// The name of the ID record.
    pub name: &'a [u8],
    /// The name with its scope: the `LF_STRING_ID` scope of `LF_FUNC_ID`,
    /// or the class name of `LF_MFUNC_ID`, then `::` and the name.
    pub qualified_name: &'a [u8],
    /// The annotation range that holds the address, if one holds it.
    pub range: Option<InlineRange>,
    /// The line in the inlined function, from `range`.
    pub line: Option<SourceLine<'a>>,
    /// The line of the call in the parent frame at the address.
    pub call_site: Option<SourceLine<'a>>,
}

/// The result of an address query.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct Location<'a> {
    /// The RVA of the query.
    pub rva: Rva,
    /// The section-relative address of the query.
    pub address: SectionOffset,
    /// The module of the section contribution that holds the address, or
    /// else the module of the procedure.
    pub module: Option<ModuleRef<'a>>,
    /// The procedure or thunk that holds the address. For an address in a
    /// separated code chunk, the procedure of the chunk.
    pub procedure: Option<ProcedureInfo<'a>>,
    /// The innermost `S_BLOCK32` scope of the procedure that holds the address.
    pub block: Option<BlockInfo<'a>>,
    /// The inline call chain from the innermost to the outermost frame.
    pub inline_frames: Vec<InlineFrame<'a>>,
    /// The line from the `DEBUG_S_LINES` subsections. In inlined code, it is
    /// the line of the call in the procedure.
    pub line: Option<SourceLine<'a>>,
}

/// A public symbol at or before an address.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct PublicMatch<'a> {
    /// The offset of the record in the symbol record stream.
    pub offset: SymbolOffset,
    /// The decoded record.
    pub public: Public<'a>,
    /// The RVA of the public symbol.
    pub rva: Rva,
    /// The number of bytes from the public symbol to the query address.
    pub displacement: u32,
}

impl SymbolLocator {
    /// Return the number of modules.
    #[must_use]
    pub fn module_count(&self) -> usize {
        self.modules.len()
    }

    /// Return the module with a zero-based index, if it exists.
    #[must_use]
    pub fn module(&self, index: ModuleIndex) -> Option<ModuleRef<'_>> {
        let data = self.modules.get(usize::try_from(index.0).ok()?)?;
        Some(ModuleRef {
            index,
            name: &data.name,
        })
    }

    /// Return the modules whose symbol walk stopped at a scope kind that
    /// this crate does not decode. The locator does not index the records
    /// after that point.
    #[must_use]
    pub fn incomplete_modules(&self) -> &[ModuleIndex] {
        &self.incomplete
    }

    /// Translate a section-relative address to an RVA.
    ///
    /// Return `None` for section 0 and for a section number past the last
    /// section header, as for absolute symbols.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`AddressMap::rva`] for other addresses.
    pub fn rva(&self, address: SectionOffset) -> Result<Option<Rva>> {
        scope::optional_rva(&self.address_map()?, address)
    }

    /// Return the indexed procedures and thunks in RVA order.
    ///
    /// Procedures with the same RVA come in module order.
    pub fn procedures(&self) -> impl Iterator<Item = Result<ProcedureInfo<'_>>> + '_ {
        self.procedures
            .spans()
            .iter()
            .map(|span| self.procedure_info(span.value))
    }

    /// Return the address map of the image sections.
    fn address_map(&self) -> Result<AddressMap<'_>> {
        Ok(AddressMap::new(
            SectionHeaders::parse(&self.section_headers)?,
            &self.dbi,
        ))
    }

    /// Find the source location of an RVA.
    ///
    /// Return `None` if no image section holds the RVA, or if the OMAP
    /// tables of an image that a tool rewrote give no mapping for it. The
    /// fields of the location are `None` or empty where the PDB has no data
    /// for the RVA, for example in a gap between procedures.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if a record refers to a missing file
    ///   checksum, `/names` string or inlinee line entry.
    /// - [`ErrorKind::MissingStream`] if line data needs `/names` and the PDB
    ///   has no `/names` stream.
    /// - The errors of the address translation, and the decoding errors of
    ///   the module records and line subsections.
    pub fn locate(&self, rva: Rva) -> Result<Option<Location<'_>>> {
        match self.address_map()?.section_offset(rva) {
            Ok(address) => address
                .mapped()
                .map(|address| self.locate_at(rva, address))
                .transpose(),
            Err(error) if error.kind() == ErrorKind::InvalidReference => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Find the source location of a section-relative address.
    ///
    /// Return `None` if the address has no RVA.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`SymbolLocator::locate`].
    pub fn locate_section_offset(&self, address: SectionOffset) -> Result<Option<Location<'_>>> {
        match self.address_map()?.rva(address) {
            Ok(rva) => match rva.mapped() {
                Some(rva) => self.locate(rva),
                None => Ok(None),
            },
            Err(error) if error.kind() == ErrorKind::InvalidReference => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Build the location of an RVA in an image section.
    ///
    /// With identical code folding, more than one procedure can start at the
    /// RVA. Prefer the procedures of the module of the section contribution,
    /// and of these the last one in module and record order. DIA
    /// (`findSymbolByRVA`) and `llvm-symbolizer` give the same procedure for
    /// the folded procedures of the fixtures of #27.
    ///
    /// An address in a separated code chunk and in no procedure gives the
    /// procedure of the chunk, with the same preference.
    fn locate_at(&self, rva: Rva, address: SectionOffset) -> Result<Location<'_>> {
        let contribution = self.contributions.find(rva.0).map(|span| span.value);
        let prefer = |key: &ProcedureKey| Some(key.module) == contribution;
        let procedure = self
            .procedures
            .find_last_preferred(rva.0, prefer)
            .or_else(|| self.separated.find_last_preferred(rva.0, prefer))
            .map(|span| self.procedure_info(span.value))
            .transpose()?;
        let module_index = contribution.or(procedure.map(|procedure| procedure.module));
        let preferred = procedure.map_or(module_index, |procedure| Some(procedure.module));
        let line = self.line_at(rva, preferred)?;
        let mut location = Location {
            rva,
            address,
            module: module_index.and_then(|index| self.module(index)),
            procedure,
            block: None,
            inline_frames: Vec::new(),
            line,
        };
        if let Some(procedure) = procedure {
            self.add_scopes(&mut location, &procedure)?;
        }
        Ok(location)
    }

    /// Return the `DEBUG_S_LINES` line at an RVA.
    ///
    /// With identical code folding, more than one subsection can start at
    /// the RVA. Prefer the subsections of the module `preferred`, and of
    /// these the first one in module and stream order. DIA
    /// (`findLinesByRVA`) gives the same line for the folded code of the
    /// fixtures of #27.
    fn line_at(&self, rva: Rva, preferred: Option<ModuleIndex>) -> Result<Option<SourceLine<'_>>> {
        let Some(span) = self
            .lines
            .find_first_preferred(rva.0, |key| Some(key.module) == preferred)
        else {
            return Ok(None);
        };
        let key = span.value;
        let module = self.module_data(key.module)?;
        let data = module
            .stream
            .as_ref()
            .and_then(|stream| stream.c13_data().get(key.start..key.start + key.length))
            .ok_or_else(|| internal("line subsection range is outside the module stream"))?;
        let subsection = crate::native::lines::LinesSubsection::parse(data)?;
        // The span starts at the code offset of the subsection, so the
        // difference fits in `u32`.
        let Some(found) = find_line(&subsection, rva.0 - span.start) else {
            return Ok(None);
        };
        let column = found.column.map(|column| u32::from(column.start));
        let end_column = found
            .column
            .and_then(|column| (column.end != 0).then_some(u32::from(column.end)));
        Ok(Some(SourceLine {
            file: self.file_name(module, found.checksum_offset)?,
            checksum_offset: found.checksum_offset,
            line: found.entry.line_start,
            end_line: found.entry.line_start + u32::from(found.entry.end_delta),
            column,
            end_column,
            start: Rva(span.start + found.entry.offset),
            length: Some(found.length),
            is_statement: found.entry.is_statement,
        }))
    }

    /// Return the module data of an index.
    fn module_data(&self, index: ModuleIndex) -> Result<&ModuleData> {
        usize::try_from(index.0)
            .ok()
            .and_then(|index| self.modules.get(index))
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidReference, None)
                    .with_context("module index is past the last module")
            })
    }

    /// Return the file name of a file checksum offset of a module.
    fn file_name<'s>(
        &'s self,
        module: &ModuleData,
        checksum_offset: ChecksumOffset,
    ) -> Result<&'s [u8]> {
        let position = module
            .files
            .binary_search_by_key(&checksum_offset, |&(offset, _)| offset)
            .map_err(|_| {
                Error::new(ErrorKind::InvalidReference, None)
                    .with_context("file reference does not identify a file checksum entry")
            })?;
        let name_offset = module.files[position].1;
        let strings = self.strings.as_deref().ok_or_else(|| {
            Error::new(ErrorKind::MissingStream, None)
                .with_context("line data needs the /names stream")
        })?;
        usize::try_from(name_offset.0)
            .ok()
            .and_then(|start| strings.get(start..))
            .and_then(|rest| {
                rest.iter()
                    .position(|&byte| byte == 0)
                    .map(|end| &rest[..end])
            })
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidReference, None)
                    .with_context("string table offset does not refer to a NUL-terminated name")
            })
    }

    /// Return the name of an inlinee of the module `module`: the unqualified
    /// and the qualified name.
    fn inlinee_name(&self, module: ModuleIndex, id: IdIndex) -> Result<(&[u8], &[u8])> {
        let scope = if self.no_type_merge {
            module.0
        } else {
            inlinee::MERGED_IDS
        };
        let position = self
            .inlinees
            .binary_search_by_key(&(scope, id), |name| (name.scope, name.id))
            .map_err(|_| internal("inlinee name was not resolved"))?;
        let name = &self.inlinees[position];
        Ok((&name.qualified[name.name_start..], &name.qualified))
    }
}

/// Create an error for an index that the locator built itself.
fn internal(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidReference, None).with_context(context)
}

/// Append `value` to `list`, or return [`ErrorKind::LimitExceeded`].
fn push<T>(list: &mut Vec<T>, value: T, context: &'static str) -> Result<()> {
    list.try_reserve(1)
        .map_err(|_| Error::new(ErrorKind::LimitExceeded, None).with_context(context))?;
    list.push(value);
    Ok(())
}
