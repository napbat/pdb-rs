//! [`PdbEditor::rename_symbols`].
//!
//! The task has two steps. [`stage`] reads the source, decodes the parts
//! into local values, calls the function and checks every new name. Then
//! [`PdbEditor::apply`] stores the parts and changes the records. Only the
//! first step can fail, so an error leaves the model unchanged.

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use super::super::module::{ModuleContents, Symbols};
use super::super::record::SymbolRecord;
use super::super::symbols::SharedSymbols;
use super::super::tree::{self, ModuleSymbol};
use super::super::{ModuleSlot, PdbEditor, missing, source};
use crate::io::ReadAt;
use crate::native::Pdb;
use crate::native::codeview::Records;
use crate::native::codeview::index::ModuleIndex;
use crate::native::codeview::symbol::{ModuleSymbolTarget, ScopeLinks, Symbol, SymbolData};
use crate::native::msf::StreamId;
use crate::native::output::no_memory;
use crate::{Bytes, Error, ErrorKind, Result};

/// The new name of each renamed name.
type NewNames = BTreeMap<Vec<u8>, Vec<u8>>;

/// The new names and the parts that the rename decodes.
struct Staged {
    /// The new name of each renamed name.
    names: NewNames,
    /// The symbol record stream, if the model does not have it yet.
    shared: Option<SharedSymbols>,
    /// The module list, if the model does not have it yet.
    slots: Option<Vec<ModuleSlot>>,
    /// The source modules with a renamed procedure.
    modules: Vec<ModuleContents>,
}

/// Return the name of a record of the symbol record stream that the task
/// renames. `S_DATAREF` and `S_ANNOTATIONREF` keep their names: the task
/// does not rename their targets.
fn shared_name(record: &SymbolRecord) -> Option<&[u8]> {
    if matches!(record.data(), SymbolData::SymbolReference(_)) {
        return None;
    }
    record.name()
}

/// Return the name of a module record that the task renames: a procedure.
fn procedure_name(record: &SymbolRecord) -> Option<&[u8]> {
    if tree::is_procedure(record.data()) {
        record.name()
    } else {
        None
    }
}

/// The names that the function sees: each name, with the source modules
/// whose undecoded symbols have a procedure with that name.
type Candidates<'n> = BTreeMap<Cow<'n, [u8]>, Vec<u32>>;

/// Add the procedure names of the undecoded symbol records `records` of
/// module `index` to `candidates`.
fn scan(records: &[u8], index: u32, candidates: &mut Candidates<'_>) -> Result<()> {
    for record in Records::new(records) {
        let symbol = Symbol::decode(record?)?;
        if !tree::is_procedure(symbol.data()) {
            continue;
        }
        let Some(name) = symbol.name() else {
            continue;
        };
        let modules = match candidates.get_mut(name.as_bytes()) {
            Some(modules) => modules,
            None => candidates
                .entry(Cow::Owned(name.as_bytes().to_vec()))
                .or_default(),
        };
        if modules.last() != Some(&index) {
            modules.try_reserve(1).map_err(|_| no_memory())?;
            modules.push(index);
        }
    }
    Ok(())
}

/// Collect the names of the symbol record stream and of the module
/// procedures.
fn candidates<'n, S: ReadAt>(
    pdb: Option<&Pdb<S>>,
    shared: &'n SharedSymbols,
    slots: &'n [ModuleSlot],
) -> Result<Candidates<'n>> {
    let mut candidates = Candidates::new();
    for name in shared.records.iter().flatten().filter_map(shared_name) {
        candidates.entry(Cow::Borrowed(name)).or_default();
    }
    for (index, slot) in slots.iter().enumerate() {
        let index = u32::try_from(index).map_err(|_| no_memory())?;
        match slot {
            ModuleSlot::Decoded(module) => match module.symbol_region() {
                Symbols::Tree(nodes) => tree::walk(nodes, &mut |record| {
                    if let Some(name) = procedure_name(record) {
                        candidates.entry(Cow::Borrowed(name)).or_default();
                    }
                    Ok(())
                })?,
                // The procedures of an opaque module stay in its source bytes.
                Symbols::Opaque => {
                    if let Some(stream) = module.source().and_then(|source| source.stream.as_ref())
                    {
                        scan(stream.symbol_data(), index, &mut candidates)?;
                    }
                }
                Symbols::Absent => {}
            },
            ModuleSlot::Source => {
                let pdb = pdb.ok_or_else(|| missing("the editor has no source for this module"))?;
                if let Some(stream) = pdb.module(ModuleIndex(index))? {
                    scan(stream.symbol_data(), index, &mut candidates)?;
                }
            }
        }
    }
    Ok(candidates)
}

/// Call `rename` once for each name of `candidates`, in byte order. Return
/// the new names and the modules that have a renamed procedure.
fn new_names(
    candidates: &Candidates<'_>,
    rename: &mut impl FnMut(&[u8]) -> Option<Vec<u8>>,
) -> Result<(NewNames, BTreeSet<u32>)> {
    let mut names = BTreeMap::new();
    let mut modules = BTreeSet::new();
    for (name, sources) in candidates {
        let Some(new) = rename(name) else {
            continue;
        };
        if new.as_slice() == name.as_ref() {
            continue;
        }
        if new.contains(&0) {
            return Err(Error::new(ErrorKind::InvalidFormat, None)
                .with_context("new symbol name contains a NUL byte"));
        }
        modules.extend(sources.iter().copied());
        names.insert(name.to_vec(), new);
    }
    Ok((names, modules))
}

/// Return the module of the model or of `staged` with the index `index`.
fn find_module<'m>(
    slots: &'m [ModuleSlot],
    staged: &'m [ModuleContents],
    index: ModuleIndex,
) -> Option<&'m ModuleContents> {
    staged
        .iter()
        .find(|module| module.index() == index)
        .or_else(|| match slots.get(index.0 as usize)? {
            ModuleSlot::Decoded(module) => Some(&**module),
            ModuleSlot::Source => None,
        })
}

/// Check that each renamed procedure reference names a procedure that gets
/// the same new name.
fn check_references(
    shared: &SharedSymbols,
    slots: &[ModuleSlot],
    staged: &[ModuleContents],
    names: &NewNames,
    stream: Option<StreamId>,
) -> Result<()> {
    for record in shared.records.iter().flatten() {
        let SymbolData::ProcedureReference(reference) = record.data() else {
            continue;
        };
        let Some(new) = names.get(reference.name.as_bytes()) else {
            continue;
        };
        let target = match reference.target {
            ModuleSymbolTarget::Handle(handle) => Some(handle.module()),
            ModuleSymbolTarget::Position { module, .. } => module,
        };
        let renamed = target
            .and_then(|index| find_module(slots, staged, index))
            .and_then(|module| module.target_name(reference.target))
            .map(|name| names.get(name).map_or(name, Vec::as_slice));
        if renamed != Some(new.as_slice()) {
            let error = Error::new(ErrorKind::UnsupportedEdit, None).with_context(
                "a procedure reference and its procedure do not get the same new name",
            );
            return Err(
                match (stream, record.origin().map(|origin| origin.record().offset)) {
                    (Some(stream), Some(offset)) => error.with_stream(stream, u64::from(offset)),
                    _ => error,
                },
            );
        }
    }
    Ok(())
}

/// Encode `record` with the name `name` into `scratch`, to find a record that
/// the new name makes too long.
fn check_encoding(record: &SymbolRecord, name: &[u8], scratch: &mut Vec<u8>) -> Result<()> {
    let mut data: SymbolData<'_> = record.data().clone();
    if let Some(field) = data.name_mut() {
        *field = Bytes::from(name);
    }
    if let SymbolData::ProcedureReference(reference) = &mut data {
        // Only the plan resolves a handle; the check needs the size only.
        reference.target = ModuleSymbolTarget::default();
    }
    scratch.clear();
    if data.starts_scope() {
        data.encode_scope(record.kind(), ScopeLinks::default(), scratch)
    } else {
        data.encode(record.kind(), scratch)
    }
}

/// Check the encoding of each record that gets a new name.
fn check_sizes(
    shared: &SharedSymbols,
    slots: &[ModuleSlot],
    staged: &[ModuleContents],
    names: &NewNames,
) -> Result<()> {
    let mut scratch = Vec::new();
    for record in shared.records.iter().flatten() {
        if let Some(new) = shared_name(record).and_then(|name| names.get(name)) {
            check_encoding(record, new, &mut scratch)?;
        }
    }
    let decoded = slots.iter().filter_map(|slot| match slot {
        ModuleSlot::Decoded(module) => Some(&**module),
        ModuleSlot::Source => None,
    });
    for module in decoded.chain(staged) {
        if let Symbols::Tree(nodes) = module.symbol_region() {
            tree::walk(nodes, &mut |record| match procedure_name(record)
                .and_then(|name| names.get(name))
            {
                Some(new) => check_encoding(record, new, &mut scratch),
                None => Ok(()),
            })?;
        }
    }
    Ok(())
}

/// Read and check everything that the rename needs. Change nothing.
fn stage<S: ReadAt>(
    editor: &PdbEditor<'_, S>,
    rename: &mut impl FnMut(&[u8]) -> Option<Vec<u8>>,
) -> Result<Staged> {
    let pdb = editor.source;
    let shared = match &editor.symbols {
        Some(_) => None,
        None => Some(source::shared_symbols(editor.cow_source()?)?),
    };
    let slots = match &editor.modules {
        Some(_) => None,
        None => Some(source::module_slots(editor.cow_source()?)?),
    };
    let model_shared = editor
        .symbols
        .as_ref()
        .or(shared.as_ref())
        .ok_or_else(|| missing("symbol record stream"))?;
    let model_slots = editor
        .modules
        .as_deref()
        .or(slots.as_deref())
        .ok_or_else(|| missing("modules"))?;
    let (names, decode) = new_names(&candidates(pdb, model_shared, model_slots)?, rename)?;
    let mut modules = Vec::new();
    for index in decode {
        let index = ModuleIndex(index);
        match model_slots.get(index.0 as usize) {
            Some(ModuleSlot::Decoded(module)) => {
                // Only an opaque module gives its source procedures here.
                module.symbols()?;
            }
            Some(ModuleSlot::Source) => {
                let pdb = pdb.ok_or_else(|| missing("the editor has no source for this module"))?;
                let module = source::module(pdb, index)?;
                module.symbols()?;
                modules.try_reserve(1).map_err(|_| no_memory())?;
                modules.push(module);
            }
            None => return Err(missing("module")),
        }
    }
    let stream = match pdb {
        Some(pdb) => pdb.debug_info()?.and_then(|dbi| dbi.header().symbol_stream),
        None => None,
    };
    check_references(model_shared, model_slots, &modules, &names, stream)?;
    check_sizes(model_shared, model_slots, &modules, &names)?;
    Ok(Staged {
        names,
        shared,
        slots,
        modules,
    })
}

/// Give each procedure of `nodes` with a renamed name its new name. Return
/// the number of renamed records.
fn rename_nodes(nodes: &mut [ModuleSymbol], names: &NewNames) -> usize {
    let mut count = 0;
    for node in nodes {
        match node {
            ModuleSymbol::Record(record) => count += rename_record(record, names, procedure_name),
            ModuleSymbol::Scope(scope) => {
                count += rename_record(&mut scope.opener, names, procedure_name);
                count += rename_nodes(&mut scope.children, names);
            }
        }
    }
    count
}

/// Give `record` its new name if `select` gives a renamed name. Return 1 for
/// a renamed record, else 0.
fn rename_record(
    record: &mut SymbolRecord,
    names: &NewNames,
    select: fn(&SymbolRecord) -> Option<&[u8]>,
) -> usize {
    let Some(new) = select(record).and_then(|name| names.get(name)) else {
        return 0;
    };
    let new = Bytes::from(new.clone());
    match record.data_mut().name_mut() {
        Some(field) => {
            *field = new;
            1
        }
        None => 0,
    }
}

impl<S: ReadAt> PdbEditor<'_, S> {
    /// Rename public symbols, global symbols and module procedures with the
    /// function `rename`, and return the number of renamed records.
    ///
    /// The task calls `rename` once for each distinct name, in byte order of
    /// the names. `rename` returns the new name, or `None` to keep the name.
    /// Each record with that name gets the new name:
    ///
    /// - each record of the symbol record stream with a name, for example
    ///   `S_PUB32`, `S_GDATA32`, `S_LDATA32`, `S_CONSTANT`, `S_UDT`,
    ///   `S_PROCREF`, `S_LPROCREF` and `S_TOKENREF`. `S_DATAREF` and
    ///   `S_ANNOTATIONREF` keep their names.
    /// - each procedure record of a module (the native procedures of all
    ///   architectures and their `_ID` kinds, and the managed procedures).
    ///   Module data, locals, labels, blocks and thunks keep their names.
    ///
    /// A procedure reference such as `S_PROCREF` and its procedure have the
    /// same name, so they get the same new name. The task does not undecorate
    /// a name and does not relate two names: MSVC gives a C++ function a
    /// decorated public name (`?f@@YAXXZ`) and an undecorated procedure name
    /// (`f`), so `rename` must return a new name for each of them. The task
    /// does not change the IPI records (`LF_FUNC_ID`, `LF_MFUNC_ID`).
    ///
    /// The task decodes the symbol record stream and only the modules with a
    /// renamed procedure. A new name changes the record lengths, so the plan
    /// writes the moved offsets into the references; it rejects the edit if
    /// opaque data can refer to the moved records (see
    /// [`PdbEditor`](PdbEditor#rejected-edits)). Use
    /// [`IdentityPolicy::IncrementAge`](crate::native::IdentityPolicy::IncrementAge)
    /// for a rename.
    ///
    /// ```no_run
    /// use pdb_rs::native::{EditOptions, IdentityPolicy, Pdb};
    ///
    /// # #[cfg(feature = "std")]
    /// # fn main() -> pdb_rs::Result<()> {
    /// let pdb = Pdb::open("app.pdb")?;
    /// let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    /// // Rename the procedure `helper`, its `S_PROCREF` record and the
    /// // public symbol of a C function with the same name.
    /// let renamed = editor.rename_symbols(|name| (name == b"helper").then(|| b"helper_v2".to_vec()))?;
    /// let plan = editor.plan(EditOptions::default())?;
    /// plan.write_file("app.renamed.pdb")?;
    /// // Write the new identity into the RSDS debug entry of the image.
    /// let identity = plan.identity()?;
    /// # let _ = (renamed, identity);
    /// # Ok(())
    /// # }
    /// # #[cfg(not(feature = "std"))]
    /// # fn main() {}
    /// ```
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if a new name contains a NUL byte.
    /// - [`ErrorKind::LimitExceeded`] if a record does not encode with its new
    ///   name: the record is longer than its `u16` length field allows.
    /// - [`ErrorKind::UnsupportedEdit`] if a renamed procedure is in a module
    ///   whose symbols do not form a scope tree (the error has the module
    ///   stream), or if a renamed procedure reference names a procedure that
    ///   does not get the same new name (the error has the symbol record
    ///   stream and the offset of the reference).
    /// - [`ErrorKind::MissingStream`] if the PDB has no DBI stream or no symbol
    ///   record stream.
    /// - The errors of the stream readers and the record decoders.
    ///
    /// On an error, the model does not change.
    pub fn rename_symbols<F>(&mut self, mut rename: F) -> Result<usize>
    where
        F: FnMut(&[u8]) -> Option<Vec<u8>>,
    {
        let staged = stage(self, &mut rename)?;
        Ok(self.apply(staged))
    }

    /// Store the parts of `staged` and give the records their new names.
    fn apply(&mut self, staged: Staged) -> usize {
        self.clear_remap_output();
        let Staged {
            names,
            shared,
            slots,
            modules,
        } = staged;
        if shared.is_some() {
            self.symbols = shared;
        }
        if slots.is_some() {
            self.modules = slots;
        }
        let mut count = 0;
        if let Some(slots) = &mut self.modules {
            for module in modules {
                if let Some(slot) = slots.get_mut(module.index().0 as usize) {
                    *slot = ModuleSlot::Decoded(Box::new(module));
                }
            }
            for slot in slots {
                if let ModuleSlot::Decoded(module) = slot
                    && let Symbols::Tree(nodes) = module.symbol_region_mut()
                {
                    count += rename_nodes(nodes, &names);
                }
            }
        }
        if let Some(shared) = &mut self.symbols {
            for record in shared.records.iter_mut().flatten() {
                count += rename_record(record, &names, shared_name);
            }
        }
        count
    }
}
