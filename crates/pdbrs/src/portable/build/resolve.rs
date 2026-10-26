//! Resolve the handles of a [`PdbBuilder`] and make the raw parts.

use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Reverse;

use super::{CustomDebugParent, ImportInfo, PdbBuilder};
use crate::error::PortableLocation;
use crate::portable::debug::{DocumentName, Import, encode_imports, encode_sequence_points};
use crate::portable::metadata::{
    BlobOffset, CodedIndex, CustomDebugInformationRow, DocumentId, DocumentRow, GuidIndex,
    LocalConstantId, LocalConstantRow, LocalScopeRow, LocalVariableId, LocalVariableRow,
    MetadataStream, MetadataToken, MethodDebugInformationRow, TableId,
};
use crate::portable::metadata::{ImportScopeRow, StringOffset};
use crate::portable::write::{
    BlobHeap, DebugTables, GuidHeap, PdbParts, StringHeap, StringHeapBuilder, UserStringHeap,
};
use crate::{Error, ErrorKind, Result};

/// The heaps of a source Portable PDB. A plan with a base keeps each entry at its offset.
#[derive(Clone, Debug)]
pub(super) struct Base {
    /// The `#Strings` heap.
    pub(super) strings: StringHeap,
    /// The `#US` heap.
    pub(super) user_strings: UserStringHeap,
    /// The `#Blob` heap.
    pub(super) blobs: BlobHeap,
    /// The `#GUID` heap.
    pub(super) guids: GuidHeap,
}

/// The parts of a builder and the row of each handle.
pub(super) struct Resolved {
    /// The raw parts.
    pub(super) parts: PdbParts,
    /// The row ID of each local scope handle.
    pub(super) scope_rows: Vec<u32>,
    /// The row ID of each local variable handle.
    pub(super) variable_rows: Vec<u32>,
    /// The row ID of each local constant handle.
    pub(super) constant_rows: Vec<u32>,
}

/// Return the location of a cell of a debug table. `row` is a zero-based index.
fn cell(table: TableId, row: usize, column: &'static str) -> PortableLocation {
    PortableLocation::new()
        .with_stream(MetadataStream::Tables)
        .with_table(table)
        .with_row(u32::try_from(row + 1).unwrap_or(u32::MAX))
        .with_column(column)
}

/// Make an [`ErrorKind::InvalidReference`] error at a cell.
fn reference(table: TableId, row: usize, column: &'static str, context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidReference, None)
        .with_context(context)
        .with_portable(cell(table, row, column))
}

/// Return a row ID from a zero-based position. The builder limits each table to
/// [`MAX_ROW`](crate::portable::metadata::MAX_ROW) records.
fn row_id(position: usize) -> u32 {
    u32::try_from(position + 1).unwrap_or(u32::MAX)
}

/// The heaps during the resolution.
struct Heaps {
    strings: StringHeap,
    blobs: BlobHeap,
    guids: GuidHeap,
}

impl Heaps {
    /// Return the offset of a string that the strings pass added.
    fn string(&self, value: &[u8]) -> StringOffset {
        // The strings pass adds each name before the heap is laid out.
        self.strings.offset(value).unwrap_or_default()
    }

    /// Add an optional GUID.
    fn guid(&mut self, guid: Option<[u8; 16]>) -> Result<GuidIndex> {
        guid.map_or(Ok(GuidIndex::new(0)), |guid| self.guids.add(guid))
    }
}

/// Resolve the handles of `builder`. With a base, the heaps start with the base heaps.
pub(super) fn resolve(builder: &PdbBuilder, base: Option<Base>) -> Result<Resolved> {
    let (mut strings, user_strings, blobs, guids) = match base {
        Some(base) => (
            StringHeapBuilder::with_base(base.strings),
            base.user_strings,
            base.blobs,
            base.guids,
        ),
        None => (
            StringHeapBuilder::new(),
            UserStringHeap::new(),
            BlobHeap::new(),
            GuidHeap::new(),
        ),
    };
    for (_, variable) in &builder.variables {
        strings.add(&variable.name)?;
    }
    for (_, constant) in &builder.constants {
        strings.add(&constant.name)?;
    }
    let mut heaps = Heaps {
        strings: strings.finish()?,
        blobs,
        guids,
    };
    let mut tables = DebugTables {
        documents: documents(builder, &mut heaps)?,
        method_debug_information: methods(builder, &mut heaps)?,
        ..DebugTables::default()
    };
    let (scope_rows, variable_rows, constant_rows) = scopes(builder, &mut heaps, &mut tables)?;
    tables.import_scopes = import_scopes(builder, &mut heaps)?;
    tables
        .state_machine_methods
        .clone_from(&builder.state_machines);
    tables
        .state_machine_methods
        .sort_by_key(|row| row.move_next_method);
    let rows = Rows {
        documents: builder.documents.len(),
        import_scopes: builder.import_scopes.len(),
        scopes: &scope_rows,
        variables: &variable_rows,
        constants: &constant_rows,
    };
    tables.custom_debug_information = custom(builder, &mut heaps, &rows)?;
    let Heaps {
        strings,
        blobs,
        guids,
    } = heaps;
    Ok(Resolved {
        parts: PdbParts {
            version: builder.version.clone(),
            entry_point: builder.entry_point,
            external: builder.external,
            strings,
            user_strings,
            blobs,
            guids,
            tables,
        },
        scope_rows,
        variable_rows,
        constant_rows,
    })
}

/// Make the `Document` rows.
fn documents(builder: &PdbBuilder, heaps: &mut Heaps) -> Result<Vec<DocumentRow>> {
    let mut rows = Vec::with_capacity(builder.documents.len());
    for (row, document) in builder.documents.iter().enumerate() {
        let location = cell(TableId::DOCUMENT, row, "Name");
        let split = crate::portable::debug::split_document_name(&document.name);
        let separator = split.separator();
        let parts = split
            .map(|part| heaps.blobs.add(part))
            .collect::<Result<Vec<_>>>()?;
        let name = DocumentName::encode(Some(separator), &parts)
            .map_err(|error| error.with_portable(location))?;
        rows.push(DocumentRow {
            name: heaps.blobs.add(&name)?,
            hash_algorithm: heaps.guid(document.hash_algorithm)?,
            hash: heaps.blobs.add(&document.hash)?,
            language: heaps.guid(document.language)?,
        });
    }
    Ok(rows)
}

/// Make the `MethodDebugInformation` rows: none, or one for each external `MethodDef` row.
fn methods(builder: &PdbBuilder, heaps: &mut Heaps) -> Result<Vec<MethodDebugInformationRow>> {
    if !builder.method_table && builder.methods.is_empty() {
        return Ok(Vec::new());
    }
    let table = TableId::METHOD_DEBUG_INFORMATION;
    let count = builder.external.get(TableId::METHOD_DEF);
    if let Some((&method, _)) = builder.methods.last_key_value()
        && method > count
    {
        return Err(reference(
            table,
            usize::try_from(method).unwrap_or(usize::MAX) - 1,
            "Document",
            "the method is after the external MethodDef rows",
        ));
    }
    let documents = builder.documents.len();
    let mut rows = Vec::new();
    for method in 1..=count {
        let row = usize::try_from(method - 1).unwrap_or(usize::MAX);
        let Some(info) = builder.methods.get(&method) else {
            rows.push(MethodDebugInformationRow {
                document: DocumentId::new(0),
                sequence_points: BlobOffset::new(0),
            });
            continue;
        };
        let unknown = info.sequence_points.iter().any(|point| {
            usize::try_from(point.document.get()).map_or(true, |document| document > documents)
        });
        if unknown {
            return Err(reference(
                table,
                row,
                "SequencePoints",
                "a sequence point document is not a document of the builder",
            ));
        }
        let encoded = encode_sequence_points(
            info.local_signature,
            info.local_signature_form,
            &info.sequence_points,
        )
        .map_err(|error| error.with_portable(cell(table, row, "SequencePoints")))?;
        rows.push(MethodDebugInformationRow {
            document: encoded.document,
            sequence_points: heaps.blobs.add(&encoded.blob)?,
        });
    }
    Ok(rows)
}

/// The rows of each handle.
type HandleRows = (Vec<u32>, Vec<u32>, Vec<u32>);

/// Sort the local scopes, give each scope one run of variables and of constants, and make
/// the rows. Return the row ID of each scope, variable and constant handle.
fn scopes(builder: &PdbBuilder, heaps: &mut Heaps, tables: &mut DebugTables) -> Result<HandleRows> {
    let count = builder.scopes.len();
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by_key(|&index| {
        let scope = &builder.scopes[index];
        (scope.method, scope.start_offset, Reverse(scope.length))
    });
    let mut variables = vec![Vec::new(); count];
    for (index, (scope, _)) in builder.variables.iter().enumerate() {
        variables
            .get_mut(scope.index())
            .ok_or_else(|| missing_scope(TableId::LOCAL_VARIABLE, index))?
            .push(index);
    }
    let mut constants = vec![Vec::new(); count];
    for (index, (scope, _)) in builder.constants.iter().enumerate() {
        constants
            .get_mut(scope.index())
            .ok_or_else(|| missing_scope(TableId::LOCAL_CONSTANT, index))?
            .push(index);
    }
    let mut scope_rows = vec![0; count];
    let mut variable_rows = vec![0; builder.variables.len()];
    let mut constant_rows = vec![0; builder.constants.len()];
    let [empty_variables, empty_constants] = builder.empty_lists;
    for (position, &index) in order.iter().enumerate() {
        scope_rows[index] = row_id(position);
        let scope = builder.scopes[index];
        let variable_list = if builder.variables.is_empty() {
            empty_variables
        } else {
            row_id(tables.local_variables.len())
        };
        let constant_list = if builder.constants.is_empty() {
            empty_constants
        } else {
            row_id(tables.local_constants.len())
        };
        for &variable in &variables[index] {
            variable_rows[variable] = row_id(tables.local_variables.len());
            let info = &builder.variables[variable].1;
            tables.local_variables.push(LocalVariableRow {
                attributes: info.attributes,
                index: info.index,
                name: heaps.string(&info.name),
            });
        }
        for &constant in &constants[index] {
            constant_rows[constant] = row_id(tables.local_constants.len());
            let info = &builder.constants[constant].1;
            tables.local_constants.push(LocalConstantRow {
                name: heaps.string(&info.name),
                signature: heaps.blobs.add(&info.signature)?,
            });
        }
        tables.local_scopes.push(LocalScopeRow {
            method: scope.method,
            import_scope: scope.import_scope,
            variable_list: LocalVariableId::new(variable_list),
            constant_list: LocalConstantId::new(constant_list),
            start_offset: scope.start_offset,
            length: scope.length,
        });
    }
    Ok((scope_rows, variable_rows, constant_rows))
}

/// Make an error for a variable or a constant whose scope the builder does not have.
fn missing_scope(table: TableId, index: usize) -> Error {
    Error::new(ErrorKind::InvalidReference, None)
        .with_context("the scope of the record is not a scope of the builder")
        .with_portable(
            PortableLocation::new()
                .with_stream(MetadataStream::Tables)
                .with_table(table)
                .with_row(row_id(index)),
        )
}

/// Make the `ImportScope` rows.
fn import_scopes(builder: &PdbBuilder, heaps: &mut Heaps) -> Result<Vec<ImportScopeRow>> {
    let mut rows = Vec::with_capacity(builder.import_scopes.len());
    for (row, scope) in builder.import_scopes.iter().enumerate() {
        let imports = scope
            .imports
            .iter()
            .map(|import| encode_import(import, &mut heaps.blobs))
            .collect::<Result<Vec<_>>>()?;
        let blob = encode_imports(&imports)
            .map_err(|error| error.with_portable(cell(TableId::IMPORT_SCOPE, row, "Imports")))?;
        rows.push(ImportScopeRow {
            parent: scope.parent,
            imports: heaps.blobs.add(&blob)?,
        });
    }
    Ok(rows)
}

/// Add the names of an import to the `#Blob` heap and return the import.
fn encode_import(import: &ImportInfo, blobs: &mut BlobHeap) -> Result<Import> {
    Ok(match import {
        ImportInfo::Namespace { namespace } => Import::Namespace {
            namespace: blobs.add(namespace)?,
        },
        ImportInfo::AssemblyNamespace {
            assembly,
            namespace,
        } => Import::AssemblyNamespace {
            assembly: *assembly,
            namespace: blobs.add(namespace)?,
        },
        ImportInfo::Type { target_type } => Import::Type {
            target_type: *target_type,
        },
        ImportInfo::XmlNamespace { alias, namespace } => Import::XmlNamespace {
            alias: blobs.add(alias)?,
            namespace: blobs.add(namespace)?,
        },
        ImportInfo::AssemblyReferenceAlias { alias } => Import::AssemblyReferenceAlias {
            alias: blobs.add(alias)?,
        },
        ImportInfo::AliasAssemblyReference { alias, assembly } => Import::AliasAssemblyReference {
            alias: blobs.add(alias)?,
            assembly: *assembly,
        },
        ImportInfo::AliasNamespace { alias, namespace } => Import::AliasNamespace {
            alias: blobs.add(alias)?,
            namespace: blobs.add(namespace)?,
        },
        ImportInfo::AliasAssemblyNamespace {
            alias,
            assembly,
            namespace,
        } => Import::AliasAssemblyNamespace {
            alias: blobs.add(alias)?,
            assembly: *assembly,
            namespace: blobs.add(namespace)?,
        },
        ImportInfo::AliasType { alias, target_type } => Import::AliasType {
            alias: blobs.add(alias)?,
            target_type: *target_type,
        },
    })
}

/// The row counts and the handle rows that custom debug information parents need.
struct Rows<'a> {
    documents: usize,
    import_scopes: usize,
    scopes: &'a [u32],
    variables: &'a [u32],
    constants: &'a [u32],
}

impl Rows<'_> {
    /// Return the token of a parent, or `None` if the builder has no such record.
    fn token(&self, parent: CustomDebugParent) -> Option<MetadataToken> {
        let local = |rows: &[u32], index: usize| rows.get(index).copied();
        let (table, row) = match parent {
            CustomDebugParent::Metadata(token) => {
                let table = TableId::new(token.table()).filter(|table| table.is_type_system())?;
                (table, Some(token.row()))
            }
            CustomDebugParent::Document(id) => (
                TableId::DOCUMENT,
                usize::try_from(id.get())
                    .ok()
                    .filter(|&row| row <= self.documents)
                    .map(|_| id.get()),
            ),
            CustomDebugParent::ImportScope(id) => (
                TableId::IMPORT_SCOPE,
                usize::try_from(id.get())
                    .ok()
                    .filter(|&row| row <= self.import_scopes)
                    .map(|_| id.get()),
            ),
            CustomDebugParent::LocalScope(handle) => {
                (TableId::LOCAL_SCOPE, local(self.scopes, handle.index()))
            }
            CustomDebugParent::LocalVariable(handle) => (
                TableId::LOCAL_VARIABLE,
                local(self.variables, handle.index()),
            ),
            CustomDebugParent::LocalConstant(handle) => (
                TableId::LOCAL_CONSTANT,
                local(self.constants, handle.index()),
            ),
        };
        row.filter(|&row| row != 0)
            .and_then(|row| MetadataToken::new(table, row))
    }
}

/// Make the `CustomDebugInformation` rows, sorted by the coded parent.
fn custom(
    builder: &PdbBuilder,
    heaps: &mut Heaps,
    rows: &Rows<'_>,
) -> Result<Vec<CustomDebugInformationRow>> {
    let table = TableId::CUSTOM_DEBUG_INFORMATION;
    let mut keyed = Vec::with_capacity(builder.custom.len());
    for (index, information) in builder.custom.iter().enumerate() {
        let Some(information) = information else {
            continue;
        };
        let parent = rows.token(information.parent);
        let coded = parent.and_then(|token| CodedIndex::HasCustomDebugInformation.encode(token));
        let (Some(parent), Some(coded)) = (parent, coded) else {
            return Err(reference(
                table,
                index,
                "Parent",
                "parent is not a record of the builder or a HasCustomDebugInformation row",
            ));
        };
        let row = CustomDebugInformationRow {
            parent,
            kind: heaps.guids.add(information.kind.0)?,
            value: heaps.blobs.add(&information.value)?,
        };
        keyed.push((coded, row));
    }
    keyed.sort_by_key(|&(coded, _)| coded);
    Ok(keyed.into_iter().map(|(_, row)| row).collect())
}
