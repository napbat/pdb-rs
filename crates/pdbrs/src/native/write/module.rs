//! The modules of a builder: DBI module records and module streams.

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::entry::{Entry, SymbolEntry};
use super::handle::{ModuleHandle, ModuleSymbolHandle, Resolver, SymbolHandle};
use crate::native::codeview::symbol::{Symbol, SymbolData};
use crate::native::dbi::OwnedModule;
use crate::native::lines::LineSubsection;
use crate::native::output::no_memory;
use crate::{Error, ErrorKind, Result};

/// One module of a [`PdbBuilder`](super::PdbBuilder).
///
/// The module has a DBI module record, a list of source file names for the
/// DBI source info substream and an optional module stream. The module
/// stream has the symbol records, the C13 line subsections and the global
/// references.
///
/// The write plan sets these fields of the record: the module stream, the
/// three region sizes and the source file count. It keeps the other fields.
#[derive(Debug)]
pub struct ModuleBuilder<'a> {
    /// The position of the module.
    pub(super) index: u32,
    /// The module record. The plan sets the stream and the sizes.
    pub(super) record: OwnedModule,
    /// `true` if the module has a module stream.
    pub(super) has_stream: bool,
    /// The symbol records, or `None` for a module stream without a symbol region.
    pub(super) symbols: Option<Vec<SymbolEntry<'a>>>,
    /// The C13 subsections.
    pub(super) subsections: Vec<Entry<'a, LineSubsection<'a>>>,
    /// The global references.
    pub(super) global_refs: Vec<SymbolHandle>,
    /// The source file names.
    pub(super) files: Vec<&'a [u8]>,
}

impl<'a> ModuleBuilder<'a> {
    /// Create a module with `record`, a module stream and an empty symbol region.
    pub(super) const fn new(index: u32, record: OwnedModule) -> Self {
        Self {
            index,
            record,
            has_stream: true,
            symbols: Some(Vec::new()),
            subsections: Vec::new(),
            global_refs: Vec::new(),
            files: Vec::new(),
        }
    }

    /// Return the handle of the module.
    #[must_use]
    pub const fn handle(&self) -> ModuleHandle {
        ModuleHandle(self.index)
    }

    /// Return the module record.
    #[must_use]
    pub const fn record(&self) -> &OwnedModule {
        &self.record
    }

    /// Return the module record for a change, for example of its flags.
    ///
    /// The plan replaces the stream, the region sizes and the source file count.
    pub const fn record_mut(&mut self) -> &mut OwnedModule {
        &mut self.record
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
    /// Returns [`ErrorKind::InvalidFormat`] if `present` is `false` and the
    /// module has symbols.
    pub fn set_symbol_region(&mut self, present: bool) -> Result<()> {
        match (&self.symbols, present) {
            (Some(symbols), false) if !symbols.is_empty() => {
                Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("module with symbols must have a symbol region"))
            }
            (_, false) => {
                self.symbols = None;
                Ok(())
            }
            (None, true) => {
                self.symbols = Some(Vec::new());
                Ok(())
            }
            (Some(_), true) => Ok(()),
        }
    }

    /// Add a decoded symbol record. The encoder keeps its padding.
    ///
    /// The plan sets the parent and end links of scope records (see
    /// [`ModuleSymbolWriter`](crate::native::codeview::symbol::ModuleSymbolWriter)).
    ///
    /// # Errors
    ///
    /// Returns the errors of [`ModuleBuilder::push_symbol_fields`].
    pub fn push_symbol(&mut self, symbol: Symbol<'a>) -> Result<ModuleSymbolHandle> {
        self.push(SymbolEntry::Decoded(symbol))
    }

    /// Add a symbol record of `kind` with `data`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if the module has no symbol region.
    /// - [`ErrorKind::LimitExceeded`] if the record count does not fit in
    ///   32 bits or an allocation fails.
    pub fn push_symbol_fields(
        &mut self,
        kind: u16,
        data: SymbolData<'a>,
    ) -> Result<ModuleSymbolHandle> {
        self.push(SymbolEntry::Fields(kind, data))
    }

    /// Add a symbol record that `encode` appends when the plan resolves handles.
    ///
    /// The resolver of a module symbol record gives type indices, ID
    /// indices, stream IDs and module numbers. It does not give symbol
    /// offsets.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`ModuleBuilder::push_symbol_fields`].
    pub fn push_symbol_with(
        &mut self,
        encode: impl Fn(&Resolver<'_>, &mut Vec<u8>) -> Result<()> + 'a,
    ) -> Result<ModuleSymbolHandle> {
        self.push(SymbolEntry::Deferred(Box::new(encode)))
    }

    /// Add one symbol entry.
    fn push(&mut self, entry: SymbolEntry<'a>) -> Result<ModuleSymbolHandle> {
        let symbols = self.symbols.as_mut().ok_or_else(|| {
            Error::new(ErrorKind::InvalidFormat, None)
                .with_context("module stream has no symbol region")
        })?;
        let position = position(symbols.len(), "module has too many symbol records")?;
        symbols.try_reserve(1).map_err(|_| no_memory())?;
        symbols.push(entry);
        Ok(ModuleSymbolHandle {
            module: self.index,
            position,
        })
    }

    /// Add a decoded C13 subsection.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn push_subsection(&mut self, subsection: LineSubsection<'a>) -> Result<()> {
        self.subsections.try_reserve(1).map_err(|_| no_memory())?;
        self.subsections.push(Entry::Value(subsection));
        Ok(())
    }

    /// Add a C13 subsection that `encode` appends with its header and padding.
    ///
    /// Use [`LineSubsection::encode_framed`] or the typed part encoders of
    /// [`crate::native::lines`].
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn push_subsection_with(
        &mut self,
        encode: impl Fn(&Resolver<'_>, &mut Vec<u8>) -> Result<()> + 'a,
    ) -> Result<()> {
        self.subsections.try_reserve(1).map_err(|_| no_memory())?;
        self.subsections.push(Entry::Deferred(Box::new(encode)));
        Ok(())
    }

    /// Add a global reference: a record of the symbol record stream that the
    /// module uses.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn push_global_ref(&mut self, symbol: SymbolHandle) -> Result<()> {
        self.global_refs.try_reserve(1).map_err(|_| no_memory())?;
        self.global_refs.push(symbol);
        Ok(())
    }

    /// Add a source file name for the DBI source info substream.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`] if an allocation fails.
    pub fn push_file(&mut self, name: &'a [u8]) -> Result<()> {
        self.files.try_reserve(1).map_err(|_| no_memory())?;
        self.files.push(name);
        Ok(())
    }
}

/// Convert a list length to the position of the next item.
pub(super) fn position(length: usize, context: &'static str) -> Result<u32> {
    u32::try_from(length)
        .ok()
        .filter(|&position| position != u32::MAX)
        .ok_or_else(|| Error::new(ErrorKind::LimitExceeded, None).with_context(context))
}
