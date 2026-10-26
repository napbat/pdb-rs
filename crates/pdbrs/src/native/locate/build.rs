//! Loading the streams and building the indexes of a [`SymbolLocator`].

use alloc::vec::Vec;

use super::range::{RangeIndex, Span};
use super::scope::optional_rva;
use super::{InlineeName, LineKey, MAX_SCOPE_DEPTH, ModuleData, ProcedureKey, SymbolLocator, push};
use crate::io::ReadAt;
use crate::native::Pdb;
use crate::native::address::{AddressMap, SectionOffset};
use crate::native::codeview::id_record::{IdRecord, IdTable};
use crate::native::codeview::index::IdIndex;
use crate::native::codeview::symbol::{ModuleSymbols, SymbolData};
use crate::native::codeview::type_record::TypeLeaf;
use crate::native::dbi::{DebugStream, Module};
use crate::native::lines::{LineSubsection, ModuleLines};
use crate::native::names::StringTable;
use crate::native::section::SectionHeaders;
use crate::native::symbols::PublicSymbolIndex;
use crate::native::types::TypeTable;
use crate::{Error, ErrorKind, Result};

/// The size of a C13 subsection header.
const SUBSECTION_HEADER_SIZE: usize = 8;

/// The lists that the module walk fills.
#[derive(Default)]
struct Lists {
    /// The modules in DBI order.
    modules: Vec<ModuleData>,
    /// The procedure ranges.
    procedures: Vec<Span<ProcedureKey>>,
    /// The line subsection ranges.
    lines: Vec<Span<LineKey>>,
    /// The inlinees of all inline sites, with duplicates.
    inlinees: Vec<IdIndex>,
    /// The modules whose walk stopped at an undecoded scope kind.
    incomplete: Vec<u32>,
}

impl SymbolLocator {
    /// Load the streams of `pdb` and build the indexes.
    ///
    /// The function loads the DBI stream, the section headers, `/names`, all
    /// module streams, the symbol record stream and the global and public
    /// symbol indexes. It loads the IPI and TPI streams to name the
    /// inlinees of `S_INLINESITE` records.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the PDB has no DBI stream or no
    ///   section header stream, or if it has inline sites but no IPI stream.
    /// - [`ErrorKind::UnsupportedVersion`] if the DBI stream names an OMAP stream.
    /// - [`ErrorKind::InvalidReference`] if an inlinee is not a function ID,
    ///   or if a member function ID does not name a class, union or enum.
    /// - [`ErrorKind::LimitExceeded`] if an index cannot be allocated.
    /// - The loading and decoding errors of the streams, the module records,
    ///   the line subsections and the address translation.
    pub fn new<S: ReadAt>(pdb: &Pdb<S>) -> Result<Self> {
        let dbi = pdb.debug_info()?.ok_or_else(|| missing("DBI"))?;
        let headers_stream = dbi
            .debug_stream(DebugStream::SectionHeader)
            .ok_or_else(|| missing("section header"))?;
        let section_headers = pdb.stream_bytes(headers_stream)?;
        let strings = pdb
            .named_stream_bytes(b"/names")?
            .map(|bytes| copy(StringTable::parse(&bytes)?.strings()))
            .transpose()?;
        let map = AddressMap::new(SectionHeaders::parse(&section_headers)?, &dbi);
        let mut lists = Lists::default();
        for module in dbi.modules() {
            lists.module(pdb, &module?, &map)?;
        }
        let mut contributions = Vec::new();
        if let Some(table) = dbi.section_contributions()? {
            for contribution in &table {
                let offset = u32::try_from(contribution.offset)
                    .map_err(|_| invalid("section contribution offset is negative"))?;
                let size = u32::try_from(contribution.size)
                    .map_err(|_| invalid("section contribution size is negative"))?;
                let address = SectionOffset {
                    section: contribution.section,
                    offset,
                };
                if let Some(start) = optional_rva(&map, address)? {
                    let span = span(start.0, size, u32::from(contribution.module));
                    push(&mut contributions, span, LIMIT)?;
                }
            }
        }
        let inlinees = inlinee_names(pdb, &mut lists.inlinees)?;
        let symbol_records = dbi
            .header()
            .symbol_stream
            .map(|id| pdb.stream_bytes(id))
            .transpose()?
            .unwrap_or_default();
        let global_index = pdb.global_symbols(&dbi)?;
        let public_index = pdb.public_symbols(&dbi)?;
        let publics = match &public_index {
            Some(index) => public_spans(index, &symbol_records, &map)?,
            None => Vec::new(),
        };
        Ok(Self {
            procedures: RangeIndex::new(lists.procedures)?,
            lines: RangeIndex::new(lists.lines)?,
            contributions: RangeIndex::new(contributions)?,
            publics: RangeIndex::new(publics)?,
            dbi,
            section_headers,
            strings,
            modules: lists.modules,
            inlinees,
            symbol_records,
            global_index,
            public_index,
            incomplete: lists.incomplete,
        })
    }
}

impl Lists {
    /// Load one module stream and index its procedures and lines.
    fn module<S: ReadAt>(
        &mut self,
        pdb: &Pdb<S>,
        module: &Module<'_>,
        map: &AddressMap<'_>,
    ) -> Result<()> {
        let mut data = ModuleData {
            name: copy(module.name)?,
            stream: None,
            files: Vec::new(),
            inlinees: Vec::new(),
            walk_end: None,
        };
        if let Some(stream) = pdb.module_stream(module)? {
            self.lines_of(module.index, &stream, &mut data, map)?;
            data.walk_end = self.symbols_of(module.index, &stream, map)?;
            data.inlinees.sort_by_key(|&(id, _, _)| id);
            data.inlinees.dedup_by_key(|&mut (id, _, _)| id);
            data.stream = Some(stream);
        }
        push(&mut self.modules, data, LIMIT)
    }

    /// Index the line subsections, files and inlinee lines of a module.
    fn lines_of(
        &mut self,
        index: u32,
        stream: &crate::native::dbi::ModuleStream,
        data: &mut ModuleData,
        map: &AddressMap<'_>,
    ) -> Result<()> {
        let lines = ModuleLines::parse(stream)?;
        lines.validate()?;
        if let Some(checksums) = lines.checksums() {
            for entry in checksums {
                push(&mut data.files, (entry.offset, entry.name_offset), LIMIT)?;
            }
        }
        // `ModuleLines::parse` decodes the framed subsections in order.
        for (framed, decoded) in stream.c13_subsections().zip(lines.subsections()) {
            let framed = framed?;
            match &decoded.data {
                LineSubsection::Lines(subsection) => {
                    let header = subsection.header();
                    let address = SectionOffset {
                        section: header.segment,
                        offset: header.code_offset,
                    };
                    let Some(start) = optional_rva(map, address)? else {
                        continue;
                    };
                    let header_offset = usize::try_from(framed.offset)
                        .map_err(|_| invalid("C13 subsection offset does not fit in memory"))?;
                    let key = LineKey {
                        module: index,
                        start: header_offset + SUBSECTION_HEADER_SIZE,
                        length: framed.data.len(),
                    };
                    push(&mut self.lines, span(start.0, header.code_size, key), LIMIT)?;
                }
                LineSubsection::InlineeLines(entries) => {
                    for entry in entries {
                        let value = (entry.inlinee, entry.checksum_offset, entry.source_line);
                        push(&mut data.inlinees, value, LIMIT)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Index the procedures and collect the inlinees of a module.
    ///
    /// Return the module stream offset of the record that stopped the walk,
    /// if a scope kind that this crate does not decode stopped it.
    fn symbols_of(
        &mut self,
        index: u32,
        stream: &crate::native::dbi::ModuleStream,
        map: &AddressMap<'_>,
    ) -> Result<Option<u32>> {
        let base = u32::try_from(stream.symbol_offset())
            .map_err(|_| invalid("module symbol offset does not fit in 32 bits"))?;
        let symbols =
            ModuleSymbols::new(stream.symbol_data(), stream.stream(), base, MAX_SCOPE_DEPTH);
        for scoped in symbols {
            let scoped = match scoped {
                Ok(scoped) => scoped,
                Err(error) if error.kind() == ErrorKind::UnsupportedVersion => {
                    push(&mut self.incomplete, index, LIMIT)?;
                    let offset = error.stream_location().map_or(0, |(_, offset)| offset);
                    return Ok(Some(u32::try_from(offset).unwrap_or(u32::MAX)));
                }
                Err(error) => return Err(error),
            };
            let (address, length) = match scoped.symbol.data() {
                SymbolData::Procedure(value) => (value.address, value.code_length),
                SymbolData::ManagedProcedure(value) => (value.address, value.code_length),
                SymbolData::Thunk(value) => (value.address, u32::from(value.length)),
                SymbolData::InlineSite(site) => {
                    push(&mut self.inlinees, site.inlinee, LIMIT)?;
                    continue;
                }
                _ => continue,
            };
            if let Some(start) = optional_rva(map, address)? {
                let key = ProcedureKey {
                    module: index,
                    offset: scoped.offset,
                };
                push(&mut self.procedures, span(start.0, length, key), LIMIT)?;
            }
        }
        Ok(None)
    }
}

/// Resolve the qualified names of the inlinees in `ids`.
fn inlinee_names<S: ReadAt>(pdb: &Pdb<S>, ids: &mut Vec<IdIndex>) -> Result<Vec<InlineeName>> {
    ids.sort_unstable();
    ids.dedup();
    let mut names = Vec::new();
    if ids.is_empty() {
        return Ok(names);
    }
    let ipi = pdb.ids()?.ok_or_else(|| missing("IPI"))?;
    let table = IdTable::new(&ipi)?;
    let tpi = pdb.types()?;
    let types = tpi.as_ref().map(TypeTable::new).transpose()?;
    for &id in ids.iter() {
        let mut qualified = Vec::new();
        let name = match table.decode(id)? {
            IdRecord::Function(function) => {
                if let Some(scope) = function.scope {
                    table.resolve_string(scope, &mut qualified)?;
                    extend(&mut qualified, b"::")?;
                }
                function.name
            }
            IdRecord::MemberFunction(function) => {
                let types = types.as_ref().ok_or_else(|| missing("TPI"))?;
                let class = match types.leaf(function.parent_type)? {
                    TypeLeaf::Class(value) => value.name,
                    TypeLeaf::Union(value) => value.name,
                    TypeLeaf::Enum(value) => value.name,
                    _ => {
                        return Err(Error::new(ErrorKind::InvalidReference, None).with_context(
                            "member function ID does not name a class, union or enum",
                        ));
                    }
                };
                extend(&mut qualified, class)?;
                extend(&mut qualified, b"::")?;
                function.name
            }
            _ => {
                return Err(Error::new(ErrorKind::InvalidReference, None)
                    .with_context("inline site inlinee is not a function ID"));
            }
        };
        let name_start = qualified.len();
        extend(&mut qualified, name)?;
        let entry = InlineeName {
            id,
            qualified,
            name_start,
        };
        push(&mut names, entry, LIMIT)?;
    }
    Ok(names)
}

/// Return a range of length 1 for each public symbol with an RVA.
fn public_spans(
    index: &PublicSymbolIndex,
    symbol_records: &[u8],
    map: &AddressMap<'_>,
) -> Result<Vec<Span<crate::native::symbols::SymbolOffset>>> {
    let mut spans = Vec::new();
    for offset in index.address_map() {
        let symbol = offset.record(symbol_records)?.symbol()?;
        let SymbolData::Public(public) = symbol.data() else {
            return Err(Error::new(ErrorKind::InvalidReference, None)
                .with_context("public address map entry is not an S_PUB32 record"));
        };
        if let Some(start) = optional_rva(map, public.address)? {
            push(&mut spans, span(start.0, 1, offset), LIMIT)?;
        }
    }
    Ok(spans)
}

/// The error context for an index that cannot be allocated.
const LIMIT: &str = "symbol locator index cannot be allocated";

/// Create a range of `length` bytes at `start`.
fn span<T>(start: u32, length: u32, value: T) -> Span<T> {
    Span {
        start,
        end: u64::from(start) + u64::from(length),
        value,
    }
}

/// Copy bytes into a new vector.
fn copy(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut copy = Vec::new();
    extend(&mut copy, bytes)?;
    Ok(copy)
}

/// Append bytes to a vector, or return [`ErrorKind::LimitExceeded`].
fn extend(target: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    target
        .try_reserve(bytes.len())
        .map_err(|_| Error::new(ErrorKind::LimitExceeded, None).with_context(LIMIT))?;
    target.extend_from_slice(bytes);
    Ok(())
}

/// Create an error for a missing stream.
fn missing(stream: &'static str) -> Error {
    Error::new(ErrorKind::MissingStream, None).with_context(stream)
}

/// Create an [`ErrorKind::InvalidFormat`] error.
fn invalid(context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidFormat, None).with_context(context)
}
