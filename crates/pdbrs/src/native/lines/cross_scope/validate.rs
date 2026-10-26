//! Reference validation of the cross-scope subsections of a module.
//!
//! The rules come from `CrossScopeReferences`, `LocalIdAndGlobalIdPair` and
//! `DecoratedItemId` in `cvinfo.h`, and from
//! `Mod1::fUpdateTiOrIdReferencedByNonSyms` in `PDB/dbi/mod.cpp` of
//! `microsoft/microsoft-pdb`. When the linker merges types, it writes the
//! global ID of each export: the local ID without bit 31, mapped through the
//! type map of the module. Bit 31 of the local ID (`DecoratedItemId`) tells
//! that the ID is a function ID in the IPI stream; without it the ID is a TPI
//! index. The linker maps the module name offset of each import into
//! `/names`. It does not map the exports when the PDB has the `NOTM` or the
//! `MINI` feature.
//!
//! [`validate_cross_scope`] reports each broken rule as a
//! [`CrossScopeViolation`] and does not stop at the first one.
//! [`CrossScopeModules`] holds the names and the exported local IDs of all
//! modules, so that an import can name a later module.

use alloc::vec::Vec;

use super::super::{LineSubsection, ModuleLines};
use super::CrossScopeExports;
use crate::Result;
use crate::native::codeview::index::{FIRST_RECORD_INDEX, IdIndex, TypeIndex};
use crate::native::names::StringTable;
use crate::native::output::no_memory;

/// The bit of a local ID that marks a function ID (`DecoratedItemId`).
const FUNCTION_ID_BIT: u32 = 0x8000_0000;

/// A rule that a cross-scope subsection can break.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum CrossScopeRule {
    /// The global ID of an export is not in the stream that bit 31 of its
    /// local ID selects: the IPI stream with the bit, the TPI stream without
    /// it. A TPI value below `0x1000` must be a known primitive type.
    ExportGlobalId,
    /// Two exports of the module have the same local ID.
    ExportDuplicate,
    /// The module name offset of an import does not identify a `/names`
    /// string.
    ImportModuleName,
    /// The module name of an import is not the module name or the object
    /// file name of a module. ASCII case is ignored, as for Windows paths.
    ImportModuleUnknown,
    /// The named module has a `DEBUG_S_CROSSSCOPEEXPORTS` subsection, and no
    /// export has the imported ID as its local ID.
    ImportNotExported,
}

impl CrossScopeRule {
    /// Return a short description of the rule.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::ExportGlobalId => "cross-scope export global ID is outside its type stream",
            Self::ExportDuplicate => "cross-scope exports repeat a local ID",
            Self::ImportModuleName => "cross-scope import module name is not a /names string",
            Self::ImportModuleUnknown => "cross-scope import names no module",
            Self::ImportNotExported => "cross-scope import ID is not exported by its module",
        }
    }
}

/// One broken rule in a cross-scope subsection.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct CrossScopeViolation {
    /// The broken rule.
    pub rule: CrossScopeRule,
    /// The offset of the subsection header in the C13 line region
    /// ([`ModuleSubsection::offset`](super::super::ModuleSubsection::offset)).
    pub subsection: u32,
    /// The position of the export, or of the import entry, in the subsection.
    pub entry: usize,
    /// The ID that breaks the rule: the global ID of an export, the module
    /// name offset of an import, or the imported ID.
    pub value: u32,
}

/// The names and the exported local IDs of one module.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ScopeModule {
    /// The module name.
    module_name: Vec<u8>,
    /// The object file name.
    object_name: Vec<u8>,
    /// The sorted local IDs of the exports, or `None` if the module has no
    /// export subsection.
    exports: Option<Vec<u32>>,
}

impl ScopeModule {
    /// Return `true` if `name` is the module name or the object file name.
    fn is_named(&self, name: &[u8]) -> bool {
        name.eq_ignore_ascii_case(&self.module_name) || name.eq_ignore_ascii_case(&self.object_name)
    }
}

/// The modules that cross-scope imports can name.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CrossScopeModules {
    /// The modules in the order of [`CrossScopeModules::add`].
    modules: Vec<ScopeModule>,
}

/// Copy `bytes` with a checked allocation.
fn copy<T: Copy>(bytes: &[T]) -> Result<Vec<T>> {
    let mut owned = Vec::new();
    owned
        .try_reserve_exact(bytes.len())
        .map_err(|_| no_memory())?;
    owned.extend_from_slice(bytes);
    Ok(owned)
}

impl CrossScopeModules {
    /// Create an empty list.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            modules: Vec::new(),
        }
    }

    /// Add a module with its DBI module name, its object file name and its
    /// export subsection, if it has one.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded)
    /// if an allocation fails.
    pub fn add(
        &mut self,
        module_name: &[u8],
        object_name: &[u8],
        exports: Option<&CrossScopeExports<'_>>,
    ) -> Result<()> {
        let exports = match exports {
            Some(exports) => {
                let mut ids = Vec::new();
                ids.try_reserve_exact(exports.len())
                    .map_err(|_| no_memory())?;
                ids.extend(exports.iter().map(|export| export.local_id));
                ids.sort_unstable();
                Some(ids)
            }
            None => None,
        };
        let module = ScopeModule {
            module_name: copy(module_name)?,
            object_name: copy(object_name)?,
            exports,
        };
        self.modules.try_reserve(1).map_err(|_| no_memory())?;
        self.modules.push(module);
        Ok(())
    }

    /// Return the number of modules.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.modules.len()
    }

    /// Return `true` if the list has no modules.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }
}

/// The data from other streams that [`validate_cross_scope`] uses.
///
/// [`CrossScopeContext::new`] gives the TPI and IPI index ends. Add the
/// optional data with the `with_` functions. A check that needs data that
/// is not present is not done.
#[derive(Clone, Copy, Debug)]
pub struct CrossScopeContext<'c> {
    /// The index after the last TPI record.
    type_index_end: TypeIndex,
    /// The index after the last IPI record.
    id_index_end: IdIndex,
    /// `false` if the linker did not map the export global IDs.
    exports_mapped: bool,
    /// The `/names` stream.
    names: Option<&'c StringTable<'c>>,
    /// The modules of the PDB.
    modules: Option<&'c CrossScopeModules>,
}

impl<'c> CrossScopeContext<'c> {
    /// Create a context with the index ends of the TPI and IPI streams.
    ///
    /// The export check is on. Turn it off with
    /// [`CrossScopeContext::without_export_mapping`] for a PDB with the `NOTM`
    /// or the `MINI` feature.
    #[must_use]
    pub const fn new(type_index_end: TypeIndex, id_index_end: IdIndex) -> Self {
        Self {
            type_index_end,
            id_index_end,
            exports_mapped: true,
            names: None,
            modules: None,
        }
    }

    /// Turn off the check of the export global IDs. The linker does not map
    /// them when it does not merge types (`NOTM`, `MINI`).
    #[must_use]
    pub const fn without_export_mapping(mut self) -> Self {
        self.exports_mapped = false;
        self
    }

    /// Add the `/names` stream for the import module names.
    #[must_use]
    pub const fn with_names(mut self, names: &'c StringTable<'c>) -> Self {
        self.names = Some(names);
        self
    }

    /// Add the modules for the import checks. The module checks need the
    /// `/names` stream too.
    #[must_use]
    pub const fn with_modules(mut self, modules: &'c CrossScopeModules) -> Self {
        self.modules = Some(modules);
        self
    }
}

/// Return `true` if `global` is a valid global ID for the decorated `local`
/// ID.
fn valid_global(local: u32, global: u32, context: &CrossScopeContext<'_>) -> bool {
    if local & FUNCTION_ID_BIT != 0 {
        return (FIRST_RECORD_INDEX..context.id_index_end.0).contains(&global);
    }
    let index = TypeIndex(global);
    if index.is_primitive() {
        index.primitive().is_ok()
    } else {
        index < context.type_index_end
    }
}

/// Check the cross-scope subsections of one module.
///
/// The function returns the violations in subsection order.
///
/// # Errors
///
/// Returns [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if
/// an allocation fails.
pub fn validate_cross_scope(
    lines: &ModuleLines<'_>,
    context: &CrossScopeContext<'_>,
) -> Result<Vec<CrossScopeViolation>> {
    let mut violations = Vec::new();
    let mut add = |rule, subsection, entry, value| -> Result<()> {
        violations.try_reserve(1).map_err(|_| no_memory())?;
        violations.push(CrossScopeViolation {
            rule,
            subsection,
            entry,
            value,
        });
        Ok(())
    };
    for subsection in lines.subsections() {
        let offset = subsection.offset;
        match &subsection.data {
            LineSubsection::CrossScopeExports(exports) => {
                let mut locals = Vec::new();
                locals
                    .try_reserve_exact(exports.len())
                    .map_err(|_| no_memory())?;
                for (entry, export) in exports.iter().enumerate() {
                    if context.exports_mapped
                        && !valid_global(export.local_id, export.global_id, context)
                    {
                        add(
                            CrossScopeRule::ExportGlobalId,
                            offset,
                            entry,
                            export.global_id,
                        )?;
                    }
                    locals.push((export.local_id, entry));
                }
                locals.sort_unstable();
                for pair in locals.windows(2) {
                    if pair[0].0 == pair[1].0 {
                        add(
                            CrossScopeRule::ExportDuplicate,
                            offset,
                            pair[1].1,
                            pair[1].0,
                        )?;
                    }
                }
            }
            LineSubsection::CrossScopeImports(imports) => {
                let Some(names) = context.names else {
                    continue;
                };
                for (entry, import) in imports.iter().enumerate() {
                    let Ok(name) = names.get(import.module_name_offset) else {
                        add(
                            CrossScopeRule::ImportModuleName,
                            offset,
                            entry,
                            import.module_name_offset,
                        )?;
                        continue;
                    };
                    let Some(modules) = context.modules else {
                        continue;
                    };
                    let mut matches = modules
                        .modules
                        .iter()
                        .filter(|module| module.is_named(name));
                    let Some(first) = matches.next() else {
                        add(
                            CrossScopeRule::ImportModuleUnknown,
                            offset,
                            entry,
                            import.module_name_offset,
                        )?;
                        continue;
                    };
                    let candidates = || core::iter::once(first).chain(matches.clone());
                    for id in import.ids {
                        let exported = candidates().any(|module| {
                            module
                                .exports
                                .as_ref()
                                .is_none_or(|exports| exports.binary_search(&id).is_ok())
                        });
                        if !exported {
                            add(CrossScopeRule::ImportNotExported, offset, entry, id)?;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(violations)
}
