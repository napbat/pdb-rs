//! Loading the streams and building the indexes of a [`SymbolLocator`].

use alloc::vec::Vec;

use super::inlinee::{MERGED_IDS, inlinee_names};
use super::range::{RangeIndex, Span};
use super::scope::{optional_pdb_rva, optional_rva};
use super::{
    LineKey, MAX_SCOPE_DEPTH, ModuleData, ProcedureKey, SeparatedChunk, SymbolLocator, push,
};
use crate::io::ReadAt;
use crate::native::Pdb;
use crate::native::address::{AddressMap, SectionOffset};
use crate::native::codeview::index::{IdIndex, ModuleIndex, ModuleSymbolOffset};
use crate::native::codeview::symbol::{ModuleSymbols, ModuleTypeSource, Symbol, SymbolData};
use crate::native::dbi::Module;
use crate::native::info::Feature;
use crate::native::lines::{LineSubsection, ModuleLines};
use crate::native::symbols::PublicSymbolIndex;
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
    /// The separated code ranges, with the procedure of each range.
    separated: Vec<Span<ProcedureKey>>,
    /// The line subsection ranges.
    lines: Vec<Span<LineKey>>,
    /// The inlinees of all inline sites, with duplicates: the ID scope (the
    /// module in a `NOTM` PDB, else [`MERGED_IDS`]) and the inlinee.
    inlinees: Vec<(u32, IdIndex)>,
    /// The `S_MOD_TYPEREF` source of each module that has one.
    sources: Vec<(ModuleIndex, ModuleTypeSource)>,
    /// `true` if the PDB has the `NOTM` feature: the inlinee IDs of a module
    /// refer to the ID records of the module.
    no_type_merge: bool,
    /// The modules whose walk stopped at an undecoded scope kind.
    incomplete: Vec<ModuleIndex>,
}

impl SymbolLocator {
    /// Load the streams of `pdb` and build the indexes.
    ///
    /// The function loads the DBI stream, the section headers, `/names`, all
    /// module streams, the symbol record stream and the global and public
    /// symbol indexes. It loads the IPI and TPI streams to name the
    /// inlinees of `S_INLINESITE` records. In a PDB without type merging
    /// (`NOTM`), it loads the ID records that the `S_MOD_TYPEREF` record of
    /// the module names instead.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::MissingStream`] if the PDB has no DBI stream or no
    ///   section header stream, or if it has inline sites but no IPI stream.
    /// - [`ErrorKind::InvalidReference`] if an inlinee is not a function ID,
    ///   if a member function ID does not name a class, union or enum, or if
    ///   a module of a `NOTM` PDB has inline sites but no ID records.
    /// - [`ErrorKind::LimitExceeded`] if an index cannot be allocated.
    /// - The loading and decoding errors of the streams, the module records,
    ///   the line subsections and the address translation.
    pub fn new<S: ReadAt>(pdb: &Pdb<S>) -> Result<Self> {
        let dbi = pdb.debug_info()?.ok_or_else(|| missing("DBI"))?;
        let tables = pdb
            .address_tables()?
            .ok_or_else(|| missing("section header"))?;
        let strings = pdb
            .names()?
            .map(|table| copy(table.strings()))
            .transpose()?;
        let map = tables.map();
        let no_type_merge = pdb.info().has_feature(Feature::NO_TYPE_MERGE);
        let mut lists = Lists {
            no_type_merge,
            ..Lists::default()
        };
        for module in dbi.modules() {
            let (index, module) = module?;
            lists.module(pdb, index, &module, &map)?;
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
                if let Some(start) = optional_pdb_rva(&map, address)? {
                    let span = span(start.0, size, contribution.module);
                    push(&mut contributions, span, LIMIT)?;
                }
            }
        }
        let inlinees = inlinee_names(pdb, &mut lists.inlinees, &lists.sources)?;
        let symbol_records = copy(pdb.symbol_records()?.unwrap_or_default())?;
        let global_index = pdb.globals()?.map(|globals| globals.index().clone());
        let public_index = pdb.publics()?.map(|publics| publics.index().clone());
        let publics = match &public_index {
            Some(index) => public_spans(index, &symbol_records, &map)?,
            None => Vec::new(),
        };
        Ok(Self {
            procedures: RangeIndex::new(lists.procedures)?,
            separated: RangeIndex::new(lists.separated)?,
            lines: RangeIndex::new(lists.lines)?,
            contributions: RangeIndex::new(contributions)?,
            publics: RangeIndex::new(publics)?,
            tables,
            strings,
            modules: lists.modules,
            inlinees,
            no_type_merge,
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
        index: ModuleIndex,
        module: &Module<'_>,
        map: &AddressMap<'_>,
    ) -> Result<()> {
        let mut data = ModuleData {
            name: copy(&module.name)?,
            stream: None,
            files: Vec::new(),
            inlinees: Vec::new(),
            walk_end: None,
            separated: Vec::new(),
        };
        if let Some(stream) = pdb.load_module(module)? {
            self.lines_of(index, &stream, &mut data, map)?;
            data.walk_end = self.symbols_of(index, &stream, &mut data, map)?;
            data.inlinees.sort_by_key(|&(id, _, _)| id);
            data.inlinees.dedup_by_key(|&mut (id, _, _)| id);
            data.stream = Some(stream);
        }
        push(&mut self.modules, data, LIMIT)
    }

    /// Index the line subsections, files and inlinee lines of a module.
    fn lines_of(
        &mut self,
        index: ModuleIndex,
        stream: &crate::native::dbi::ModuleStream,
        data: &mut ModuleData,
        map: &AddressMap<'_>,
    ) -> Result<()> {
        let lines = ModuleLines::parse(stream)?;
        lines.check()?;
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
                    let Some(start) = optional_pdb_rva(map, address)? else {
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

    /// Index the procedures and the separated code, and collect the
    /// inlinees of a module.
    ///
    /// Return the module stream offset of the record that stopped the walk,
    /// if a scope kind that this crate does not decode stopped it.
    fn symbols_of(
        &mut self,
        index: ModuleIndex,
        stream: &crate::native::dbi::ModuleStream,
        data: &mut ModuleData,
        map: &AddressMap<'_>,
    ) -> Result<Option<ModuleSymbolOffset>> {
        let base = u32::try_from(stream.symbol_offset())
            .map_err(|_| invalid("module symbol offset does not fit in 32 bits"))?;
        let symbols = ModuleSymbols::new(
            stream.symbol_data(),
            stream.stream(),
            ModuleSymbolOffset(base),
            MAX_SCOPE_DEPTH,
        );
        // The address and the record offset of each procedure.
        let mut starts = Vec::new();
        let mut walk_end = None;
        for scoped in symbols {
            let scoped = match scoped {
                Ok(scoped) => scoped,
                Err(error) if error.kind() == ErrorKind::UnsupportedVersion => {
                    push(&mut self.incomplete, index, LIMIT)?;
                    let offset = error.stream_location().map_or(0, |(_, offset)| offset);
                    walk_end = Some(ModuleSymbolOffset(
                        u32::try_from(offset).unwrap_or(u32::MAX),
                    ));
                    break;
                }
                Err(error) => return Err(error),
            };
            let (address, length) = match scoped.symbol.data() {
                SymbolData::Procedure(value) => {
                    push(&mut starts, (value.address, scoped.offset), LIMIT)?;
                    (value.address, value.code_length)
                }
                SymbolData::ManagedProcedure(value) => (value.address, value.code_length),
                SymbolData::Thunk(value) => (value.address, u32::from(value.length)),
                SymbolData::SeparatedCode(value) => {
                    let chunk = SeparatedChunk {
                        offset: scoped.offset,
                        address: value.address,
                        length: value.code_length,
                    };
                    push(&mut data.separated, (value.parent_address, chunk), LIMIT)?;
                    continue;
                }
                SymbolData::InlineSite(site) => {
                    let scope = if self.no_type_merge {
                        index.0
                    } else {
                        MERGED_IDS
                    };
                    push(&mut self.inlinees, (scope, site.inlinee), LIMIT)?;
                    continue;
                }
                SymbolData::ModuleTypeReference(reference) => {
                    push(&mut self.sources, (index, reference.source()), LIMIT)?;
                    continue;
                }
                _ => continue,
            };
            if let Some(start) = optional_pdb_rva(map, address)? {
                let key = ProcedureKey {
                    module: index,
                    offset: scoped.offset,
                };
                push(&mut self.procedures, span(start.0, length, key), LIMIT)?;
            }
        }
        // MSVC writes an `S_SEPCODE` record after the end of its procedure,
        // so the walk links the records after it read all procedures.
        for (parent, chunk) in &data.separated {
            let procedure = starts.iter().find(|(address, _)| address == parent);
            if let (Some(&(_, offset)), Some(start)) =
                (procedure, optional_pdb_rva(map, chunk.address)?)
            {
                let key = ProcedureKey {
                    module: index,
                    offset,
                };
                push(&mut self.separated, span(start.0, chunk.length, key), LIMIT)?;
            }
        }
        Ok(walk_end)
    }
}

/// Return a range of length 1 for each public symbol with an RVA.
fn public_spans(
    index: &PublicSymbolIndex,
    symbol_records: &[u8],
    map: &AddressMap<'_>,
) -> Result<Vec<Span<crate::native::symbols::SymbolOffset>>> {
    let mut spans = Vec::new();
    for offset in index.address_map() {
        let symbol = Symbol::decode(offset.record(symbol_records)?)?;
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
