//! External dependencies: files that hold data that a PDB names but does not contain.
//!
//! [`Pdb::external_dependencies`] lists these dependencies. It does not open, find or load
//! any external file. The caller decides what to load.
//!
//! - Type servers: the entries of the DBI type-server map. A module uses the type server in
//!   its type-server index (`iTSM`). Current linkers write an empty map and index 0.
//! - Precompiled types: a module with a non-zero `S_OBJNAME` signature was compiled with a
//!   precompiled header (`/Yc` or `/Yu`). The modules with the same signature share the
//!   precompiled types. In a full PDB, the linker merged these types into the TPI stream. In a
//!   FASTLINK PDB, the types stay in the object file of the `/Yc` module.
//! - FASTLINK objects (`/DEBUG:FASTLINK`, feature code `MINI`): `S_FASTLINK` records of a
//!   module refer to records that stay in the object file of the module. The module record
//!   gives the object file, or the library that holds it.
//! - Module type records (feature code `NOTM`, MSVC `/DEBUG:LAZY`): the linker does not merge
//!   types, and the type and ID indices of the module symbols refer to the records of the input
//!   object. The `S_MOD_TYPEREF` record of each module tells which stream of the PDB holds
//!   them ([`ModuleTypeSource`]). The records are in the PDB, but not in TPI and IPI.
//! - Precompiled types of `/DEBUG:LAZY` (feature code `NOTM`, `/Z7` with `/Yc` and `/Yu`): the
//!   linker does not copy the precompiled types of the `/Yc` object to the PDB
//!   ([`ModuleTypeSource::PrecompiledTypeOwner`] and
//!   [`ModuleTypeSource::PrecompiledTypeUser`]). They stay in the object file of the owner
//!   module ([`ObjectPrecompiledTypes`]).

use alloc::vec::Vec;

use super::Pdb;
use super::codeview::symbol::{ModuleTypeSource, S_FASTLINK, S_MOD_TYPEREF, S_OBJNAME, SymbolData};
use super::dbi::{Dbi, TypeServer};
use super::info::Feature;
use super::output::no_memory;
use crate::Result;
use crate::io::ReadAt;

/// The modules that share the precompiled types of one precompiled header.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct PrecompiledTypes {
    /// The `S_OBJNAME` signature that the modules share.
    pub signature: u32,
    /// The indices of the modules, in module order.
    pub modules: Vec<u32>,
}

/// A FASTLINK module whose object file keeps the full records.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct FastLinkObject {
    /// The index of the module.
    pub module: u32,
    /// The module name: the object file, or the member name in a library.
    pub module_name: Vec<u8>,
    /// The object file or the library that holds the object file.
    pub object_file: Vec<u8>,
    /// The number of `S_FASTLINK` records of the module.
    pub references: usize,
}

/// A module that uses an entry of the DBI type-server map.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ModuleTypeServer {
    /// The index of the module.
    pub module: u32,
    /// The index of the entry in [`ExternalDependencies::type_servers`] (`iTSM`).
    pub type_server: u8,
}

/// A module of a `NOTM` PDB whose type and ID indices refer to precompiled types that stay in
/// the object file of the owner module.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ObjectPrecompiledTypes {
    /// The index of the module.
    pub module: u32,
    /// The index of the module that owns the precompiled types (the `/Yc` object). It is
    /// `module` for the owner itself.
    pub owner: u32,
    /// The object file of the owner module, which holds the precompiled types in `.debug$P`.
    pub object_file: Vec<u8>,
}

/// The source of the type records of one module, from its `S_MOD_TYPEREF` record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ModuleTypeDependency {
    /// The index of the module.
    pub module: u32,
    /// Where the type records of the module are.
    pub source: ModuleTypeSource,
}

/// The external dependencies of a PDB.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct ExternalDependencies {
    /// `true` if the PDB has the `MINI` feature code of a FASTLINK PDB.
    pub mini_pdb: bool,
    /// `true` if the PDB has the `NOTM` feature code: the linker did not merge types.
    pub no_type_merge: bool,
    /// The entries of the DBI type-server map: PDBs that hold types of the modules.
    pub type_servers: Vec<TypeServer>,
    /// The modules with a non-zero type-server index, in module order.
    pub type_server_modules: Vec<ModuleTypeServer>,
    /// The precompiled-header groups, in the order of their first module.
    pub precompiled: Vec<PrecompiledTypes>,
    /// The modules with `S_FASTLINK` records, in module order.
    pub fastlink_objects: Vec<FastLinkObject>,
    /// The modules with an `S_MOD_TYPEREF` record, in module order.
    pub module_types: Vec<ModuleTypeDependency>,
    /// The modules whose type and ID indices refer to precompiled types in an object file, in
    /// module order (MSVC `/DEBUG:LAZY` with `/Z7` precompiled headers).
    pub object_precompiled_types: Vec<ObjectPrecompiledTypes>,
}

impl ExternalDependencies {
    /// Return `true` if the PDB has no external dependency of these kinds.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.type_servers.is_empty()
            && self.type_server_modules.is_empty()
            && self.precompiled.is_empty()
            && self.fastlink_objects.is_empty()
            && self.module_types.is_empty()
            && self.object_precompiled_types.is_empty()
    }
}

impl<S: ReadAt> Pdb<S> {
    /// List the external dependencies that the module streams and the Info stream name.
    ///
    /// The function reads each module stream once. It does not open external files.
    ///
    /// # Errors
    ///
    /// Returns the errors of [`Dbi::type_server_map`], [`Dbi::modules`], [`Pdb::module_stream`]
    /// and of the decoding of
    /// `S_OBJNAME` records, [`crate::ErrorKind::InvalidReference`] if an `S_MOD_TYPEREF` record
    /// names a missing owner module, and [`crate::ErrorKind::LimitExceeded`] if an allocation
    /// fails.
    pub fn external_dependencies(&self, dbi: &Dbi) -> Result<ExternalDependencies> {
        let mut dependencies = ExternalDependencies {
            mini_pdb: self.info().has_feature(Feature::MinimalDebugInfo),
            no_type_merge: self.info().has_feature(Feature::NoTypeMerge),
            type_servers: dbi.type_server_map()?,
            ..ExternalDependencies::default()
        };
        for module in dbi.modules() {
            let module = module?;
            let type_server = module.type_server_index();
            if type_server != 0 {
                let modules = &mut dependencies.type_server_modules;
                modules.try_reserve(1).map_err(|_| no_memory())?;
                modules.push(ModuleTypeServer {
                    module: module.index,
                    type_server,
                });
            }
            let Some(stream) = self.module_stream(&module)? else {
                continue;
            };
            let mut signature = 0;
            let mut references = 0_usize;
            for record in stream.symbols() {
                let record = record?;
                if record.kind == S_OBJNAME
                    && let SymbolData::ObjectName(name) = record.symbol()?.data()
                {
                    signature = name.signature;
                } else if record.kind == S_FASTLINK {
                    references += 1;
                } else if record.kind == S_MOD_TYPEREF
                    && let SymbolData::ModuleTypeReference(reference) = record.symbol()?.data()
                {
                    let modules = &mut dependencies.module_types;
                    modules.try_reserve(1).map_err(|_| no_memory())?;
                    modules.push(ModuleTypeDependency {
                        module: module.index,
                        source: reference.source(),
                    });
                }
            }
            if signature != 0 {
                add_precompiled(&mut dependencies.precompiled, signature, module.index)?;
            }
            if references != 0 {
                dependencies
                    .fastlink_objects
                    .try_reserve(1)
                    .map_err(|_| no_memory())?;
                dependencies.fastlink_objects.push(FastLinkObject {
                    module: module.index,
                    module_name: copy(module.name)?,
                    object_file: copy(module.object_file)?,
                    references,
                });
            }
        }
        object_precompiled_types(dbi, &mut dependencies)?;
        Ok(dependencies)
    }
}

/// Fill [`ExternalDependencies::object_precompiled_types`] from the module type sources.
fn object_precompiled_types(dbi: &Dbi, dependencies: &mut ExternalDependencies) -> Result<()> {
    for dependency in &dependencies.module_types {
        let owner = match dependency.source {
            ModuleTypeSource::PrecompiledTypeOwner { .. } => dependency.module,
            ModuleTypeSource::PrecompiledTypeUser { owner, .. } => u32::from(owner),
            _ => continue,
        };
        let object = dbi
            .modules()
            .nth(usize::try_from(owner).unwrap_or(usize::MAX))
            .ok_or_else(|| {
                crate::Error::new(crate::ErrorKind::InvalidReference, None)
                    .with_context("S_MOD_TYPEREF names a missing owner of precompiled types")
            })??;
        let list = &mut dependencies.object_precompiled_types;
        list.try_reserve(1).map_err(|_| no_memory())?;
        list.push(ObjectPrecompiledTypes {
            module: dependency.module,
            owner,
            object_file: copy(object.object_file)?,
        });
    }
    Ok(())
}

/// Add `module` to the group of `signature`.
fn add_precompiled(groups: &mut Vec<PrecompiledTypes>, signature: u32, module: u32) -> Result<()> {
    if let Some(group) = groups.iter_mut().find(|group| group.signature == signature) {
        group.modules.try_reserve(1).map_err(|_| no_memory())?;
        group.modules.push(module);
        return Ok(());
    }
    let mut modules = Vec::new();
    modules.try_reserve(1).map_err(|_| no_memory())?;
    modules.push(module);
    groups.try_reserve(1).map_err(|_| no_memory())?;
    groups.push(PrecompiledTypes { signature, modules });
    Ok(())
}

/// Copy bytes into an owned vector.
fn copy(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut owned = Vec::new();
    owned
        .try_reserve_exact(bytes.len())
        .map_err(|_| no_memory())?;
    owned.extend_from_slice(bytes);
    Ok(owned)
}
