//! The scopes of one procedure: blocks and inline sites.

use alloc::vec::Vec;

use super::{
    BlockInfo, InlineFrame, InlineRange, InlineRanges, Location, ModuleData, ProcedureInfo,
    ProcedureKey, SourceLine, SymbolLocator, internal, push,
};
use crate::native::address::{AddressMap, Rva, SectionOffset};
use crate::native::codeview::Records;
use crate::native::codeview::index::IdIndex;
use crate::native::codeview::symbol::{Symbol, SymbolData};
use crate::{Error, ErrorKind, Result};

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
    /// start of the procedure.
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
        let Some(start) = procedure.start else {
            return Ok(());
        };
        let Some(target) = location.rva.0.checked_sub(start.0) else {
            return Ok(());
        };
        let scopes = self.scopes(procedure)?;
        location.block = scopes.iter().rev().find_map(|scope| match scope.kind {
            ScopeKind::Block(block)
                if block.start <= location.rva
                    && u64::from(location.rva.0) - u64::from(block.start.0)
                        < u64::from(block.length) =>
            {
                Some(block)
            }
            _ => None,
        });
        let innermost = scopes.iter().rposition(|scope| match &scope.kind {
            ScopeKind::Site { ranges, .. } => ranges.iter().any(|range| range.contains(target)),
            _ => false,
        });
        let module = self.module_data(procedure.module)?;
        let mut next = innermost;
        while let Some(position) = next {
            let scope = &scopes[position];
            if let ScopeKind::Site { inlinee, ranges } = &scope.kind {
                let range = ranges.iter().copied().find(|range| range.contains(target));
                let (name, qualified_name) = self.inlinee_name(*inlinee)?;
                let line = range
                    .map(|range| self.range_line(module, start, &range))
                    .transpose()?;
                let frame = InlineFrame {
                    offset: scope.offset,
                    inlinee: *inlinee,
                    name,
                    qualified_name,
                    range,
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

    /// Return the source line of an inline range.
    fn range_line(
        &self,
        module: &ModuleData,
        procedure_start: Rva,
        range: &InlineRange,
    ) -> Result<SourceLine<'_>> {
        let start = procedure_start
            .0
            .checked_add(range.code_offset)
            .ok_or_else(|| {
                Error::new(ErrorKind::OffsetOverflow, None)
                    .with_context("inline range RVA overflows 32 bits")
            })?;
        Ok(SourceLine {
            file: self.file_name(module, range.file)?,
            checksum_offset: range.file,
            line: range.line,
            end_line: range
                .line
                .saturating_add(range.line_count.saturating_sub(1)),
            column: range.column_start,
            end_column: range.column_end,
            start: Rva(start),
            length: range.code_length,
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
/// Section 0 and section numbers past the last header give `None`, as for
/// absolute symbols. Other translation errors stay errors.
pub(super) fn optional_rva(map: &AddressMap<'_>, address: SectionOffset) -> Result<Option<Rva>> {
    match map.rva(address) {
        Ok(rva) => Ok(Some(rva)),
        Err(_) if address.section == 0 || usize::from(address.section) > map.sections().len() => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}
