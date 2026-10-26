//! An editor of an existing Portable PDB.

use alloc::vec::Vec;
use core::ops::{Deref, DerefMut};

use super::resolve::{Base, Resolved, resolve};
use super::{
    CustomDebugInfo, CustomDebugParent, DocumentInfo, ImportInfo, ImportScopeInfo,
    LocalConstantHandle, LocalConstantInfo, LocalScopeHandle, LocalScopeInfo, LocalVariableHandle,
    LocalVariableInfo, MethodInfo, PdbBuilder,
};
use crate::PortableLocation;
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::custom::{self, CustomDebugValue};
use crate::portable::debug::Import;
use crate::portable::metadata::{BlobOffset, DocumentId, ImportScopeId, MetadataStream, TableId};
use crate::portable::write::{
    BlobHeap, ExternalRowCounts, GuidHeap, IdPolicy, PdbParts, StringHeap, UserStringHeap,
    WritePlan,
};
use crate::{Error, ErrorKind, Result};

/// The number of source records of the tables whose rows a plan can move.
#[derive(Clone, Copy, Debug)]
struct SourceRows {
    scopes: usize,
    variables: usize,
    constants: usize,
}

/// A [`PdbBuilder`] with the records and the heaps of an existing Portable PDB.
///
/// [`Pdb::edit`] makes the editor. The editor dereferences to its builder, so the builder
/// functions read and change the records. The editor gives these guarantees:
///
/// - The heaps of the output start with the bytes of the source heaps. Each source heap entry
///   keeps its offset, and a value that the source has gets its source offset.
/// - Documents and import scopes keep their row IDs. The builder adds new documents and
///   import scopes after them.
/// - Without a change, each record keeps its row. For a Portable PDB that
///   `System.Reflection.Metadata` wrote, the output then has the same bytes as the source.
/// - Custom debug information of an unknown kind keeps its bytes. If a change gives a source
///   local scope, local variable or local constant a new row ID, such a value can contain the
///   old row ID. [`PdbEditor::plan`] then fails before output. Remove the unknown values or
///   add the new records after the source records of the same method to keep the rows.
///
/// The editor does not keep the source unchanged in other ways: for example, a decoded and
/// encoded blob can have other bytes if the source producer used another encoding. The editor
/// then adds the new blob and the source blob stays in the heap.
#[derive(Clone, Debug)]
pub struct PdbEditor {
    builder: PdbBuilder,
    base: Base,
    source: SourceRows,
}

impl Deref for PdbEditor {
    type Target = PdbBuilder;

    fn deref(&self) -> &PdbBuilder {
        &self.builder
    }
}

impl DerefMut for PdbEditor {
    fn deref_mut(&mut self) -> &mut PdbBuilder {
        &mut self.builder
    }
}

impl PdbEditor {
    /// Resolve the handles with the source heaps and check the unknown custom debug
    /// information.
    fn resolve(&self) -> Result<Resolved> {
        let resolved = resolve(&self.builder, Some(self.base.clone()))?;
        self.check_opaque(&resolved)?;
        Ok(resolved)
    }

    /// Fail if a source row moves and an unknown custom debug information value exists.
    fn check_opaque(&self, resolved: &Resolved) -> Result<()> {
        let moved = |rows: &[u32], count: usize| {
            (1..)
                .zip(rows.iter().take(count))
                .any(|(source, &row)| row != source)
        };
        let source = self.source;
        let moved = moved(&resolved.scope_rows, source.scopes)
            || moved(&resolved.variable_rows, source.variables)
            || moved(&resolved.constant_rows, source.constants);
        if !moved {
            return Ok(());
        }
        for (handle, information) in self.builder.custom_debug_informations() {
            let opaque = !matches!(
                custom::decode(information.kind, &information.value),
                Ok(value) if !matches!(value, CustomDebugValue::Unknown { .. })
            );
            if opaque {
                return Err(Error::new(ErrorKind::InvalidReference, None)
                    .with_context(
                        "the edit moves source local scopes, variables or constants, and an unknown custom debug information value can refer to their rows",
                    )
                    .with_portable(
                        PortableLocation::new()
                            .with_stream(MetadataStream::Tables)
                            .with_table(TableId::CUSTOM_DEBUG_INFORMATION)
                            .with_row(handle.0 + 1)
                            .with_column("Value"),
                    ));
            }
        }
        Ok(())
    }

    /// Resolve the handles and make the raw parts with the source heaps.
    ///
    /// # Errors
    ///
    /// The errors of [`PdbEditor::plan`] that occur before [`PdbParts::plan`].
    pub fn to_parts(&self) -> Result<PdbParts> {
        Ok(self.resolve()?.parts)
    }

    /// Resolve the handles, check the records and freeze the layout.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidReference`] if the edit gives a source local scope, local variable
    ///   or local constant a new row ID and a custom debug information value of an unknown
    ///   kind (or a value that does not decode) exists.
    /// - The errors of [`PdbBuilder::plan`].
    pub fn plan(&self, id: IdPolicy) -> Result<WritePlan> {
        self.to_parts()?.plan(id)
    }
}

/// Return the zero-based index of a row ID as a handle number.
fn handle(row: u32) -> u32 {
    row.saturating_sub(1)
}

impl<S: ReadAt> Pdb<S> {
    /// Read all records of the Portable PDB into an editor.
    ///
    /// See [`PdbEditor`] for the guarantees of an edit.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidFormat`] if a heap is not valid, a blob does not decode (for
    ///   example an import of an unknown kind), or a local variable or constant is not in the
    ///   run of a local scope.
    /// - The errors of the row, heap and blob accessors.
    pub fn edit(&self) -> Result<PdbEditor> {
        let heap =
            |stream| -> Result<Vec<u8>> { Ok(self.stream_bytes(stream)?.unwrap_or_default()) };
        let mut base = Base {
            strings: StringHeap::from_bytes(heap(MetadataStream::Strings)?)?,
            user_strings: UserStringHeap::from_bytes(heap(MetadataStream::UserStrings)?)?,
            blobs: BlobHeap::from_bytes(heap(MetadataStream::Blob)?)?,
            guids: GuidHeap::from_bytes(heap(MetadataStream::Guid)?)?,
        };
        let mut builder = PdbBuilder::new(ExternalRowCounts::from_pdb_stream(self.pdb_stream()));
        builder.version = self.metadata().version_string().to_vec();
        builder.entry_point = self.pdb_stream().entry_point();
        for document in self.documents() {
            let document = document?;
            builder.documents.push(DocumentInfo {
                name: document.name().to_vec(),
                hash_algorithm: document.hash_algorithm_guid().copied(),
                hash: document.hash().to_vec(),
                language: document.language_guid().copied(),
            });
        }
        self.read_methods(&mut builder)?;
        self.read_scopes(&mut builder, &mut base.strings)?;
        for scope in self.import_scopes() {
            let scope = scope?;
            let imports = scope
                .imports()
                .map(|import| self.import_info(import.map_err(|error| scope.blob_error(error))?))
                .collect::<Result<Vec<_>>>()?;
            builder.import_scopes.push(ImportScopeInfo {
                parent: scope.parent(),
                imports,
            });
        }
        for method in self.state_machine_methods() {
            builder.state_machines.push(*method?.row());
        }
        for information in self.custom_debug_informations() {
            let information = information?;
            let token = information.parent();
            let row = token.row();
            let parent = match TableId::new(token.table()) {
                Some(TableId::DOCUMENT) => CustomDebugParent::Document(DocumentId::new(row)),
                Some(TableId::IMPORT_SCOPE) => {
                    CustomDebugParent::ImportScope(ImportScopeId::new(row))
                }
                Some(TableId::LOCAL_SCOPE) => {
                    CustomDebugParent::LocalScope(LocalScopeHandle(handle(row)))
                }
                Some(TableId::LOCAL_VARIABLE) => {
                    CustomDebugParent::LocalVariable(LocalVariableHandle(handle(row)))
                }
                Some(TableId::LOCAL_CONSTANT) => {
                    CustomDebugParent::LocalConstant(LocalConstantHandle(handle(row)))
                }
                _ => CustomDebugParent::Metadata(token),
            };
            builder.custom.push(Some(CustomDebugInfo {
                parent,
                kind: information.kind(),
                value: information.value().to_vec(),
            }));
        }
        let source = SourceRows {
            scopes: builder.scopes.len(),
            variables: builder.variables.len(),
            constants: builder.constants.len(),
        };
        Ok(PdbEditor {
            builder,
            base,
            source,
        })
    }

    /// Read the sequence points of each method into `builder`.
    fn read_methods(&self, builder: &mut PdbBuilder) -> Result<()> {
        builder.method_table = self.tables().row_count(TableId::METHOD_DEBUG_INFORMATION) > 0;
        for information in self.method_debug_informations() {
            let information = information?;
            let Some(points) = information.sequence_points()? else {
                continue;
            };
            let local_signature = points.local_signature();
            let local_signature_form = points.local_signature_form();
            let sequence_points = points
                .collect::<Result<Vec<_>>>()
                .map_err(|error| information.blob_error(error))?;
            builder.methods.insert(
                information.id().get(),
                MethodInfo {
                    local_signature,
                    local_signature_form,
                    sequence_points,
                },
            );
        }
        Ok(())
    }

    /// Read the local scopes with their variables and constants into `builder`. Record the
    /// offset of each name in `strings`.
    fn read_scopes(&self, builder: &mut PdbBuilder, strings: &mut StringHeap) -> Result<()> {
        let tables = self.tables();
        let mut first = true;
        for scope in self.local_scopes() {
            let scope = scope?;
            if first {
                // Keep the list value of an empty table: Roslyn writes 1, the F# compiler
                // writes 0.
                first = false;
                let list = |table, value: u32| {
                    if tables.row_count(table) == 0 {
                        value
                    } else {
                        1
                    }
                };
                builder.empty_lists = [
                    list(TableId::LOCAL_VARIABLE, scope.row().variable_list.get()),
                    list(TableId::LOCAL_CONSTANT, scope.row().constant_list.get()),
                ];
            }
            let handle = builder.add_local_scope(LocalScopeInfo {
                method: scope.method(),
                import_scope: scope.import_scope(),
                start_offset: scope.start_offset(),
                length: scope.length(),
            })?;
            for id in scope.variables() {
                let variable = self.local_variable(id)?;
                strings.register(variable.name(), variable.row().name);
                builder.add_local_variable(
                    handle,
                    LocalVariableInfo {
                        attributes: variable.attributes(),
                        index: variable.index(),
                        name: variable.name().to_vec(),
                    },
                )?;
            }
            for id in scope.constants() {
                let constant = self.local_constant(id)?;
                strings.register(constant.name(), constant.row().name);
                builder.add_local_constant(
                    handle,
                    LocalConstantInfo {
                        name: constant.name().to_vec(),
                        signature: constant.signature_blob().to_vec(),
                    },
                )?;
            }
        }
        let complete = [
            (TableId::LOCAL_VARIABLE, builder.variables.len()),
            (TableId::LOCAL_CONSTANT, builder.constants.len()),
        ];
        for (table, count) in complete {
            if u32::try_from(count).ok() != Some(tables.row_count(table)) {
                return Err(Error::new(ErrorKind::InvalidFormat, None)
                    .with_context("a row is not in the run of a local scope")
                    .with_portable(
                        PortableLocation::new()
                            .with_stream(MetadataStream::Tables)
                            .with_table(table),
                    ));
            }
        }
        Ok(())
    }

    /// Read the names of an import from the `#Blob` heap.
    fn import_info(&self, import: Import) -> Result<ImportInfo> {
        let blob = |offset: BlobOffset| self.blob(offset);
        Ok(match import {
            Import::Namespace { namespace } => ImportInfo::Namespace {
                namespace: blob(namespace)?,
            },
            Import::AssemblyNamespace {
                assembly,
                namespace,
            } => ImportInfo::AssemblyNamespace {
                assembly,
                namespace: blob(namespace)?,
            },
            Import::Type { target_type } => ImportInfo::Type { target_type },
            Import::XmlNamespace { alias, namespace } => ImportInfo::XmlNamespace {
                alias: blob(alias)?,
                namespace: blob(namespace)?,
            },
            Import::AssemblyReferenceAlias { alias } => ImportInfo::AssemblyReferenceAlias {
                alias: blob(alias)?,
            },
            Import::AliasAssemblyReference { alias, assembly } => {
                ImportInfo::AliasAssemblyReference {
                    alias: blob(alias)?,
                    assembly,
                }
            }
            Import::AliasNamespace { alias, namespace } => ImportInfo::AliasNamespace {
                alias: blob(alias)?,
                namespace: blob(namespace)?,
            },
            Import::AliasAssemblyNamespace {
                alias,
                assembly,
                namespace,
            } => ImportInfo::AliasAssemblyNamespace {
                alias: blob(alias)?,
                assembly,
                namespace: blob(namespace)?,
            },
            Import::AliasType { alias, target_type } => ImportInfo::AliasType {
                alias: blob(alias)?,
                target_type,
            },
        })
    }
}
