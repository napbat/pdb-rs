//! The modules of the editor model: DBI module records and module streams.

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::record::SymbolRecord;
use super::tree::{self, ModuleSymbol, Open, Pushed, Scope};
use crate::native::codeview::SymbolKind;
use crate::native::codeview::index::{ModuleIndex, ModuleSymbolHandle, SymbolHandle};
use crate::native::codeview::symbol::SymbolData;
use crate::native::dbi::{Module, ModuleStream};
use crate::native::lines::{LineSubsection, ModuleLines};
use crate::native::output::no_memory;
use crate::native::symbols::SymbolOffset;
use crate::{Bytes, Error, ErrorKind, Result};

/// The module stream offset of the first symbol record: after the signature.
pub(super) const SYMBOL_BASE: u32 = 4;

/// The symbol region of a module.
#[derive(Clone, Debug)]
pub(super) enum Symbols {
    /// The module stream has no symbol region.
    Absent,
    /// The records form a scope tree.
    Tree(Vec<ModuleSymbol>),
    /// The source records do not form a tree; the plan copies them.
    Opaque,
}

/// A global reference of a module: a record of the symbol record stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GlobalRef {
    /// A source offset. The plan writes the new offset of the record.
    Offset(SymbolOffset),
    /// A record of the editor model.
    Handle(SymbolHandle),
}

/// The source of a module that the editor decoded.
#[derive(Clone, Debug)]
pub(super) struct ModuleSource {
    /// The source module record.
    pub(super) record: Module<'static>,
    /// The source module stream, if the module has one.
    pub(super) stream: Option<ModuleStream>,
    /// The source file names.
    pub(super) files: Vec<Bytes<'static>>,
}

/// One module of the editor model.
///
/// The module has a DBI module record, the source file names of the DBI
/// source info substream and an optional module stream. The module stream
/// has the symbol records as a scope tree, the C13 line subsections and the
/// global references.
///
/// The plan sets these fields of the record: the module stream, the three
/// region sizes and the source file count. It keeps the other fields.
///
/// A module whose source records do not form a scope tree stays opaque (rule
/// 2 of #42). The plan copies its symbol records. [`ModuleContents::symbols`],
/// [`ModuleContents::symbols_mut`], [`ModuleContents::procedure_mut`] and
/// [`ModuleContents::push`] then fail with [`ErrorKind::UnsupportedEdit`].
/// Its lines stay editable.
#[derive(Clone, Debug)]
pub struct ModuleContents {
    /// The index of the module.
    index: ModuleIndex,
    /// The module record.
    record: Module<'static>,
    /// `true` if the module has a module stream.
    has_stream: bool,
    /// The symbol region.
    symbols: Symbols,
    /// The scopes that [`ModuleContents::push`] opened and did not close.
    open: Vec<Open>,
    /// The number of the next record.
    next_id: u32,
    /// The C13 subsections.
    lines: Vec<LineSubsection<'static>>,
    /// `true` if the caller had mutable access to the lines.
    lines_changed: bool,
    /// The source file names.
    files: Vec<Bytes<'static>>,
    /// The global references.
    global_refs: Vec<GlobalRef>,
    /// The source, or `None` for a new module.
    source: Option<Box<ModuleSource>>,
}

/// Return the error for a symbol edit of a module that does not form a tree.
fn opaque(source: Option<&ModuleSource>) -> Error {
    let error = Error::new(ErrorKind::UnsupportedEdit, None)
        .with_context("the symbols of the module do not form a scope tree, so they stay opaque");
    match source
        .and_then(|source| source.stream.as_ref())
        .map(ModuleStream::stream)
    {
        Some(stream) => error.with_stream(stream, 0),
        None => error,
    }
}

impl ModuleContents {
    /// Make a new module with `record`, a module stream and an empty symbol
    /// region.
    pub(super) const fn new(index: ModuleIndex, record: Module<'static>) -> Self {
        Self {
            index,
            record,
            has_stream: true,
            symbols: Symbols::Tree(Vec::new()),
            open: Vec::new(),
            next_id: 0,
            lines: Vec::new(),
            lines_changed: false,
            files: Vec::new(),
            global_refs: Vec::new(),
            source: None,
        }
    }

    /// Decode a source module: the record, the source files and the stream.
    pub(super) fn decode(
        index: ModuleIndex,
        record: Module<'static>,
        files: Vec<Bytes<'static>>,
        stream: Option<ModuleStream>,
    ) -> Result<Self> {
        let mut module = Self::new(index, record.clone());
        module.files.clone_from(&files);
        match &stream {
            None => {
                module.has_stream = false;
                module.symbols = Symbols::Absent;
            }
            Some(stream) => {
                module.symbols = if stream.symbol_offset() == 0 {
                    Symbols::Absent
                } else {
                    match tree::decode(stream.symbol_data(), symbol_base(stream))? {
                        Some((nodes, count)) => {
                            module.next_id = count;
                            Symbols::Tree(nodes)
                        }
                        None => Symbols::Opaque,
                    }
                };
                for subsection in ModuleLines::parse(stream)?.subsections() {
                    module.lines.try_reserve(1).map_err(|_| no_memory())?;
                    module.lines.push(subsection.data.clone().into_owned());
                }
                let refs = stream.global_refs()?;
                module
                    .global_refs
                    .try_reserve_exact(refs.len())
                    .map_err(|_| no_memory())?;
                module
                    .global_refs
                    .extend(refs.iter().map(GlobalRef::Offset));
            }
        }
        module.source = Some(Box::new(ModuleSource {
            record,
            stream,
            files,
        }));
        Ok(module)
    }

    /// Return the index of the module in the DBI module list.
    #[must_use]
    pub const fn index(&self) -> ModuleIndex {
        self.index
    }

    /// Return the module record.
    #[must_use]
    pub const fn record(&self) -> &Module<'static> {
        &self.record
    }

    /// Return the module record for a change, for example of its flags.
    ///
    /// The plan replaces the stream, the region sizes and the source file
    /// count.
    pub const fn record_mut(&mut self) -> &mut Module<'static> {
        &mut self.record
    }

    /// Return `true` if the module has a module stream.
    #[must_use]
    pub const fn has_stream(&self) -> bool {
        self.has_stream
    }

    /// Select whether the module has a module stream.
    ///
    /// A module without a module stream must have no symbols, no
    /// subsections and no global references. The plan checks this.
    pub const fn set_stream(&mut self, present: bool) {
        self.has_stream = present;
    }

    /// Select whether the module stream has a symbol region.
    ///
    /// A module stream without a symbol region has no signature. Remove the
    /// region only if the module has no symbols.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if `present` is `false` and the module
    ///   has symbols.
    /// - [`ErrorKind::UnsupportedEdit`] if the symbols of the module stay
    ///   opaque.
    pub fn set_symbol_region(&mut self, present: bool) -> Result<()> {
        match (&self.symbols, present) {
            (Symbols::Opaque, _) => Err(opaque(self.source.as_deref())),
            (Symbols::Tree(nodes), false) if !nodes.is_empty() || !self.open.is_empty() => {
                Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("module with symbols must have a symbol region"))
            }
            (_, false) => {
                self.symbols = Symbols::Absent;
                Ok(())
            }
            (Symbols::Absent, true) => {
                self.symbols = Symbols::Tree(Vec::new());
                Ok(())
            }
            (Symbols::Tree(_), true) => Ok(()),
        }
    }

    /// Return the symbol tree of the module. A module without a symbol
    /// region gives no symbols.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::UnsupportedEdit`] if the symbols stay opaque.
    pub fn symbols(&self) -> Result<&[ModuleSymbol]> {
        match &self.symbols {
            Symbols::Absent => Ok(&[]),
            Symbols::Tree(nodes) => Ok(nodes),
            Symbols::Opaque => Err(opaque(self.source.as_deref())),
        }
    }

    /// Return the symbol tree of the module for a change.
    ///
    /// The plan computes the parent and end links of each scope from the
    /// tree. A removed record must not be the target of a reference record
    /// of the symbol record stream: the plan then fails with
    /// [`ErrorKind::InvalidReference`].
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::UnsupportedEdit`] if the symbols stay opaque.
    /// - [`ErrorKind::InvalidFormat`] if the module stream has no symbol
    ///   region.
    pub fn symbols_mut(&mut self) -> Result<&mut Vec<ModuleSymbol>> {
        match &mut self.symbols {
            Symbols::Tree(nodes) => Ok(nodes),
            Symbols::Absent => Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("module stream has no symbol region")),
            Symbols::Opaque => Err(opaque(self.source.as_deref())),
        }
    }

    /// Return the scope of the procedure with the name `name`, at any depth.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the module has no such procedure.
    /// - The errors of [`ModuleContents::symbols_mut`].
    pub fn procedure_mut(&mut self, name: &[u8]) -> Result<&mut Scope> {
        tree::find_procedure(self.symbols_mut()?, name).ok_or_else(|| {
            Error::new(ErrorKind::InvalidReference, None)
                .with_context("the module has no procedure with this name")
        })
    }

    /// Append a symbol record of `kind` with `data` in stream order and
    /// return its handle.
    ///
    /// A record that opens a scope, for example `S_GPROC32`, starts a scope.
    /// A record that closes a scope (`S_END`, `S_PROC_ID_END` or
    /// `S_INLINESITE_END`) ends the innermost scope. The scope enters the
    /// tree when it ends. Other records go into the innermost scope. The plan
    /// rejects a module with a scope that did not end.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] for a closer without an open scope of
    ///   its kind, or if the module stream has no symbol region.
    /// - [`ErrorKind::UnsupportedEdit`] if the symbols stay opaque.
    /// - [`ErrorKind::LimitExceeded`] if the record count does not fit in 32
    ///   bits or an allocation fails.
    pub fn push(&mut self, kind: SymbolKind, data: SymbolData<'_>) -> Result<ModuleSymbolHandle> {
        let id = self.next_id;
        let next = id.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::LimitExceeded, None)
                .with_context("module has too many symbol records")
        })?;
        let mut record = SymbolRecord::new(kind, data);
        record.set_id(id);
        let module = self.index;
        let source = self.source.as_deref();
        let nodes = match &mut self.symbols {
            Symbols::Tree(nodes) => nodes,
            Symbols::Absent => {
                return Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("module stream has no symbol region"));
            }
            Symbols::Opaque => return Err(opaque(source)),
        };
        if tree::closes_scope(kind) && self.open.is_empty() {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("a scope end record has no open scope"));
        }
        if let Pushed::Mismatch = tree::push(nodes, &mut self.open, record, 0)? {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("a scope end record does not match the kind of its scope"));
        }
        self.next_id = next;
        Ok(ModuleSymbolHandle {
            module: module.0,
            position: id,
        })
    }

    /// Return the handle of `record`, a record of this module.
    ///
    /// A record from [`SymbolRecord::new`] has no handle. Use
    /// [`ModuleContents::push`] for a record that a reference record names.
    #[must_use]
    pub fn handle(&self, record: &SymbolRecord) -> Option<ModuleSymbolHandle> {
        record.id().map(|position| ModuleSymbolHandle {
            module: self.index.0,
            position,
        })
    }

    /// Return the C13 line subsections.
    #[must_use]
    pub fn lines(&self) -> &[LineSubsection<'static>] {
        &self.lines
    }

    /// Return the C13 line subsections for a change.
    ///
    /// The plan encodes each subsection with
    /// [`LineSubsection::encode_framed`]. A new file checksum goes to the end
    /// of the checksum table, so that the checksum offsets of the other
    /// files stay (rule 5 of #42).
    pub fn lines_mut(&mut self) -> &mut Vec<LineSubsection<'static>> {
        self.lines_changed = true;
        &mut self.lines
    }

    /// Return the source file names of the DBI source info substream.
    #[must_use]
    pub fn files(&self) -> &[Bytes<'static>] {
        &self.files
    }

    /// Return the source file names for a change.
    pub fn files_mut(&mut self) -> &mut Vec<Bytes<'static>> {
        &mut self.files
    }

    /// Add a global reference: a record of the symbol record stream that the
    /// module uses.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn push_global_ref(&mut self, symbol: SymbolHandle) -> Result<()> {
        self.global_refs.try_reserve(1).map_err(|_| no_memory())?;
        self.global_refs.push(GlobalRef::Handle(symbol));
        Ok(())
    }

    /// Return the number of global references.
    #[must_use]
    pub fn global_ref_count(&self) -> usize {
        self.global_refs.len()
    }

    /// Return the symbol region.
    pub(super) const fn symbol_region(&self) -> &Symbols {
        &self.symbols
    }

    /// Return the symbol region for a conversion.
    pub(super) const fn symbol_region_mut(&mut self) -> &mut Symbols {
        &mut self.symbols
    }

    /// Return `true` if a scope of [`ModuleContents::push`] did not end.
    pub(super) fn has_open_scope(&self) -> bool {
        !self.open.is_empty()
    }

    /// Return `true` if the caller had mutable access to the lines.
    pub(super) const fn lines_changed(&self) -> bool {
        self.lines_changed
    }

    /// Mark the lines as changed, so that the plan encodes them.
    pub(super) const fn set_lines_changed(&mut self) {
        self.lines_changed = true;
    }

    /// Return the global references.
    pub(super) fn global_refs(&self) -> &[GlobalRef] {
        &self.global_refs
    }

    /// Return the global references for a conversion.
    pub(super) const fn global_refs_mut(&mut self) -> &mut Vec<GlobalRef> {
        &mut self.global_refs
    }

    /// Return the source of the module.
    pub(super) fn source(&self) -> Option<&ModuleSource> {
        self.source.as_deref()
    }

    /// Return `true` if the source file names changed.
    pub(super) fn files_changed(&self) -> bool {
        self.source
            .as_deref()
            .is_none_or(|source| source.files != self.files)
    }
}

/// Return the module stream offset of the first symbol record of `stream`:
/// after the 4-byte signature.
pub(super) fn symbol_base(stream: &ModuleStream) -> u32 {
    u32::try_from(stream.symbol_offset()).unwrap_or(u32::MAX)
}
