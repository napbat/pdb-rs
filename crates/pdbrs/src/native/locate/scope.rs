//! The scopes of one procedure: blocks, inline sites and code chunks.

use alloc::vec::Vec;

use super::{
    BlockInfo, InlineFrame, InlineRange, InlineRanges, Location, ModuleData, ProcedureInfo,
    ProcedureKey, SourceLine, SymbolLocator, internal, push,
};
use crate::native::address::{AddressMap, Rva, SectionOffset};
use crate::native::codeview::Records;
use crate::native::codeview::index::IdIndex;
use crate::native::codeview::symbol::{Symbol, SymbolData, separated_chunks};
use crate::{Error, ErrorKind, Result};

/// A code chunk of a procedure.
///
/// Chunk 0 is the code of the procedure. Chunk `n` is the code of the
/// `n`-th `S_SEPCODE` record of the module whose parent address is the
/// start of the procedure, in stream order.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct CodeChunk {
    /// The module stream offset of the `S_SEPCODE` record, or `None` for
    /// the code of the procedure.
    pub offset: Option<u32>,
    /// The section-relative address of the first byte.
    pub address: SectionOffset,
    /// The RVA of the first byte, or `None` if the address is not in an
    /// image section.
    pub start: Option<Rva>,
    /// The length of the code in bytes.
    pub length: u32,
}

/// The line ranges of one `S_INLINESITE` record.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct InlineSiteRanges {
    /// The offset of the record in the module stream.
    pub offset: u32,
    /// The module stream offset of the enclosing scope record.
    pub parent: u32,
    /// The inlined function.
    pub inlinee: IdIndex,
    /// The ranges in annotation order. Each code offset is relative to the
    /// start of the code chunk of the range
    /// ([`SymbolLocator::code_chunks`]).
    pub ranges: Vec<InlineRange>,
}

/// A scope record inside a procedure.
#[derive(Clone, Debug)]
struct ScopeRecord<'a> {
    /// The offset of the record in the module stream.
    offset: u32,
    /// The module stream offset of the enclosing scope record.
    parent: u32,
    /// The kind and the data of the scope.
    kind: ScopeKind<'a>,
}

/// The kinds of scope records that the locator uses.
#[derive(Clone, Debug)]
enum ScopeKind<'a> {
    /// An inline site with its line ranges.
    Site {
        /// The inlined function.
        inlinee: IdIndex,
        /// The line ranges.
        ranges: Vec<InlineRange>,
    },
    /// A block with an RVA.
    Block(BlockInfo<'a>),
    /// Another scope, for example a block without an RVA.
    Other,
}

impl SymbolLocator {
    /// Decode the procedure record of `key`.
    pub(super) fn procedure_info(&self, key: ProcedureKey) -> Result<ProcedureInfo<'_>> {
        let module = self.module_data(key.module)?;
        let symbol = module_symbol(module, key.offset)?;
        let (address, length) = match symbol.data() {
            SymbolData::Procedure(value) => (value.address, value.code_length),
            SymbolData::ManagedProcedure(value) => (value.address, value.code_length),
            SymbolData::Thunk(value) => (value.address, u32::from(value.length)),
            _ => {
                return Err(Error::new(ErrorKind::InvalidReference, None)
                    .with_context("module symbol offset does not identify a procedure"));
            }
        };
        Ok(ProcedureInfo {
            module: key.module,
            offset: key.offset,
            kind: symbol.kind(),
            name: symbol.name().unwrap_or_default(),
            address,
            start: optional_rva(&self.address_map()?, address)?,
            length,
        })
    }

    /// Return the line ranges of the inline sites of a procedure.
    ///
    /// The sites come in record order, so a parent site comes before its
    /// children. Thunks and managed procedures have no inline sites.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the procedure is not in the
    ///   module stream, or if a site has no inlinee line entry in the module.
    /// - The decoding errors of the records and of [`InlineRanges`].
    pub fn inline_sites(&self, procedure: &ProcedureInfo<'_>) -> Result<Vec<InlineSiteRanges>> {
        let mut sites = Vec::new();
        for scope in self.scopes(procedure)? {
            if let ScopeKind::Site { inlinee, ranges } = scope.kind {
                let site = InlineSiteRanges {
                    offset: scope.offset,
                    parent: scope.parent,
                    inlinee,
                    ranges,
                };
                push(&mut sites, site, "inline site list cannot be allocated")?;
            }
        }
        Ok(sites)
    }

    /// Return the code chunks of a procedure.
    ///
    /// The index of a chunk in the list is its chunk number, the value of
    /// [`InlineRange::chunk`]. Chunk 0 is the code of the procedure. The
    /// next chunks come from the `S_SEPCODE` records of the module whose
    /// parent address is the start of the procedure, in stream order.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the module of the procedure is
    ///   not known.
    /// - The errors of the address translation.
    pub fn code_chunks(&self, procedure: &ProcedureInfo<'_>) -> Result<Vec<CodeChunk>> {
        const LIMIT: &str = "code chunk list cannot be allocated";
        let module = self.module_data(procedure.module)?;
        let map = self.address_map()?;
        let mut chunks = Vec::new();
        let main = CodeChunk {
            offset: None,
            address: procedure.address,
            start: procedure.start,
            length: procedure.length,
        };
        push(&mut chunks, main, LIMIT)?;
        for chunk in separated_chunks(&module.separated, procedure.address) {
            let separated = CodeChunk {
                offset: Some(chunk.offset),
                address: chunk.address,
                start: optional_rva(&map, chunk.address)?,
                length: chunk.length,
            };
            push(&mut chunks, separated, LIMIT)?;
        }
        Ok(chunks)
    }

    /// Decode the blocks and the inline sites of a procedure.
    fn scopes(&self, procedure: &ProcedureInfo<'_>) -> Result<Vec<ScopeRecord<'_>>> {
        let module = self.module_data(procedure.module)?;
        let mut scopes = Vec::new();
        let (data, end) = match module_symbol(module, procedure.offset)?.data() {
            SymbolData::Procedure(value) => (records_at(module, procedure.offset)?, value.end),
            _ => return Ok(scopes),
        };
        let map = self.address_map()?;
        let mut records = Records::new(data);
        // The first record is the procedure itself.
        records.next();
        for record in records {
            let record = record?;
            let offset = procedure
                .offset
                .checked_add(record.offset)
                .ok_or_else(|| internal("module symbol offset overflows 32 bits"))?;
            if offset >= end || module.walk_end.is_some_and(|stop| offset >= stop) {
                break;
            }
            let symbol = record.symbol()?;
            let Some(links) = symbol.scope() else {
                continue;
            };
            let kind = match symbol.data() {
                SymbolData::InlineSite(site) => ScopeKind::Site {
                    inlinee: site.inlinee,
                    ranges: inline_ranges(module, site.inlinee, &site.annotations)?,
                },
                SymbolData::Block(block) => match optional_rva(&map, block.address)? {
                    Some(start) => ScopeKind::Block(BlockInfo {
                        offset,
                        name: block.name,
                        start,
                        length: block.code_length,
                    }),
                    None => ScopeKind::Other,
                },
                _ => ScopeKind::Other,
            };
            let scope = ScopeRecord {
                offset,
                parent: links.parent,
                kind,
            };
            push(
                &mut scopes,
                scope,
                "procedure scope list cannot be allocated",
            )?;
        }
        Ok(scopes)
    }

    /// Add the innermost block and the inline call chain to `location`.
    pub(super) fn add_scopes<'s>(
        &'s self,
        location: &mut Location<'s>,
        procedure: &ProcedureInfo<'s>,
    ) -> Result<()> {
        let rva = location.rva;
        let scopes = self.scopes(procedure)?;
        location.block = scopes.iter().rev().find_map(|scope| match scope.kind {
            ScopeKind::Block(block)
                if block.start <= rva
                    && u64::from(rva.0) - u64::from(block.start.0) < u64::from(block.length) =>
            {
                Some(block)
            }
            _ => None,
        });
        let chunks = self.code_chunks(procedure)?;
        // The range that holds the address, with its RVA and length.
        let holding = |range: &InlineRange| {
            let (start, length) = range.rva_range(&chunks)?;
            (rva.0.checked_sub(start.0)? < length).then_some((*range, start, length))
        };
        let innermost = scopes.iter().rposition(|scope| match &scope.kind {
            ScopeKind::Site { ranges, .. } => ranges.iter().any(|range| holding(range).is_some()),
            _ => false,
        });
        let module = self.module_data(procedure.module)?;
        let mut next = innermost;
        while let Some(position) = next {
            let scope = &scopes[position];
            if let ScopeKind::Site { inlinee, ranges } = &scope.kind {
                let found = ranges.iter().find_map(holding);
                let (name, qualified_name) = self.inlinee_name(*inlinee)?;
                let line = found
                    .map(|(range, start, length)| self.range_line(module, start, length, &range))
                    .transpose()?;
                let frame = InlineFrame {
                    offset: scope.offset,
                    inlinee: *inlinee,
                    name,
                    qualified_name,
                    range: found.map(|(range, _, _)| range),
                    line,
                    call_site: None,
                };
                push(
                    &mut location.inline_frames,
                    frame,
                    "inline frame list cannot be allocated",
                )?;
            }
            if scope.parent == procedure.offset {
                break;
            }
            // The scope walk checked the parent links, so a parent inside
            // the procedure is an earlier scope record.
            let parent = scopes[..position]
                .binary_search_by_key(&scope.parent, |scope| scope.offset)
                .map_err(|_| internal("inline site parent is not a scope of the procedure"))?;
            next = Some(parent);
        }
        let mut call_site = location.line;
        for frame in location.inline_frames.iter_mut().rev() {
            frame.call_site = call_site;
            call_site = frame.line;
        }
        Ok(())
    }

    /// Return the source line of an inline range at `start` with `length`
    /// bytes.
    fn range_line(
        &self,
        module: &ModuleData,
        start: Rva,
        length: u32,
        range: &InlineRange,
    ) -> Result<SourceLine<'_>> {
        Ok(SourceLine {
            file: self.file_name(module, range.file)?,
            checksum_offset: range.file,
            line: range.line,
            end_line: range
                .line
                .saturating_add(range.line_count.saturating_sub(1)),
            column: range.column_start,
            end_column: range.column_end,
            start,
            length: Some(length),
            is_statement: range.is_statement,
        })
    }
}

/// Run the annotations of an inline site from the inlinee start line.
fn inline_ranges(
    module: &ModuleData,
    inlinee: IdIndex,
    annotations: &crate::native::codeview::symbol::BinaryAnnotations<'_>,
) -> Result<Vec<InlineRange>> {
    let position = module
        .inlinees
        .binary_search_by_key(&inlinee, |&(id, _, _)| id)
        .map_err(|_| {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("inline site inlinee has no inlinee line entry in the module")
        })?;
    let (_, file, line) = module.inlinees[position];
    let mut ranges = Vec::new();
    for range in InlineRanges::new(annotations, file, line) {
        push(&mut ranges, range?, "inline range list cannot be allocated")?;
    }
    Ok(ranges)
}

/// Return the symbol records of a module, from a module stream offset on.
fn records_at(module: &ModuleData, offset: u32) -> Result<&[u8]> {
    let stream = module
        .stream
        .as_ref()
        .ok_or_else(|| internal("module has no module stream"))?;
    usize::try_from(offset)
        .ok()
        .and_then(|offset| offset.checked_sub(stream.symbol_offset()))
        .and_then(|start| stream.symbol_data().get(start..))
        .filter(|data| !data.is_empty())
        .ok_or_else(|| {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("module symbol offset is outside the symbol records")
        })
}

/// Decode the symbol record at a module stream offset.
pub(super) fn module_symbol(module: &ModuleData, offset: u32) -> Result<Symbol<'_>> {
    let data = records_at(module, offset)?;
    let record = Records::new(data).next().ok_or_else(|| {
        Error::new(ErrorKind::InvalidReference, None)
            .with_context("module symbol offset is outside the symbol records")
    })??;
    record.symbol()
}

/// Translate an address that can lie outside the image sections.
///
/// Section 0, section numbers past the last header and an address without
/// an OMAP mapping give `None`, as for absolute symbols. Other translation
/// errors stay errors.
pub(super) fn optional_rva(map: &AddressMap<'_>, address: SectionOffset) -> Result<Option<Rva>> {
    match map.rva(address) {
        Ok(rva) => Ok(rva.mapped()),
        Err(_) if address.section == 0 || usize::from(address.section) > map.sections().len() => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}
