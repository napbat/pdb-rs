//! Resolve the handles of an editor model and make the raw parts.

use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Reverse;

use super::Methods;
use super::collections::{
    CustomDebugInformations, Documents, EmptyListForm, ImportScopes, LocalScopes,
};
use crate::PortableLocation;
use crate::io::ReadAt;
use crate::portable::Pdb;
use crate::portable::custom::CustomDebugParent;
use crate::portable::debug::{DocumentName, Imports, StateMachineMethod};
use crate::portable::metadata::{
    BlobOffset, CodedIndex, CustomDebugInformationRow, DocumentId, DocumentRow, GuidIndex,
    ImportScopeRow, LocalConstantId, LocalConstantRow, LocalScopeRow, LocalVariableId,
    LocalVariableRow, MetadataStream, MetadataToken, MethodDebugInformationRow, MethodDefId,
    StateMachineMethodRow, StringOffset, TableId,
};
use crate::portable::write::{
    BlobHeap, DebugTables, ExternalRowCounts, GuidHeap, PdbParts, StringHeap, StringHeapBuilder,
    UserStringHeap,
};
use crate::{Error, ErrorKind, Result};

/// The heaps of a source Portable PDB. A plan with a base keeps each entry at its offset.
pub(super) struct Base {
    /// The `#Strings` heap.
    strings: StringHeap,
    /// The `#US` heap.
    user_strings: UserStringHeap,
    /// The `#Blob` heap.
    blobs: BlobHeap,
    /// The `#GUID` heap.
    guids: GuidHeap,
}

impl Base {
    /// Read the heaps of `pdb`. Record the offset of each local variable and constant name,
    /// so that a name in the middle of another string keeps its source offset.
    pub(super) fn read<S: ReadAt>(pdb: &Pdb<S>) -> Result<Self> {
        let heap =
            |stream| -> Result<Vec<u8>> { Ok(pdb.stream_bytes(stream)?.unwrap_or_default()) };
        let mut strings = StringHeap::from_bytes(heap(MetadataStream::Strings)?)?;
        for row in pdb.rows::<LocalVariableRow>() {
            let (_, row) = row?;
            strings.register(&pdb.string(row.name)?, row.name);
        }
        for row in pdb.rows::<LocalConstantRow>() {
            let (_, row) = row?;
            strings.register(&pdb.string(row.name)?, row.name);
        }
        Ok(Self {
            strings,
            user_strings: UserStringHeap::from_bytes(heap(MetadataStream::UserStrings)?)?,
            blobs: BlobHeap::from_bytes(heap(MetadataStream::Blob)?)?,
            guids: GuidHeap::from_bytes(heap(MetadataStream::Guid)?)?,
        })
    }
}

/// The records of an editor, decoded or read for the plan.
pub(super) struct Model<'m> {
    /// The metadata version string.
    pub(super) version: &'m [u8],
    /// The entry point.
    pub(super) entry_point: Option<MethodDefId>,
    /// The external row counts.
    pub(super) external: ExternalRowCounts,
    /// The documents.
    pub(super) documents: &'m Documents,
    /// The methods with sequence points.
    pub(super) methods: &'m Methods,
    /// The local scopes.
    pub(super) scopes: &'m LocalScopes,
    /// The import scopes.
    pub(super) import_scopes: &'m ImportScopes,
    /// The state machine methods.
    pub(super) state_machines: &'m [StateMachineMethod],
    /// The custom debug information.
    pub(super) custom: &'m CustomDebugInformations,
}

/// The parts of a model and the rows of each local scope.
pub(super) struct Resolved {
    /// The raw parts.
    pub(super) parts: PdbParts,
    /// The row ID of each local scope, by handle.
    pub(super) scope_rows: Vec<u32>,
    /// The first `LocalVariable` row and the first `LocalConstant` row of each local scope,
    /// by handle.
    pub(super) list_rows: Vec<[u32; 2]>,
}

/// Return the location of a cell of a debug table. `row` is a zero-based index.
fn cell(table: TableId, row: usize, column: &'static str) -> PortableLocation {
    PortableLocation::new()
        .with_stream(MetadataStream::Tables)
        .with_table(table)
        .with_row(row_id(row))
        .with_column(column)
}

/// Make an [`ErrorKind::InvalidReference`] error at a cell.
fn reference(table: TableId, row: usize, column: &'static str, context: &'static str) -> Error {
    Error::new(ErrorKind::InvalidReference, None)
        .with_context(context)
        .with_portable(cell(table, row, column))
}

/// Return a row ID from a zero-based position. The editor limits each table to
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
        guid.map_or(Ok(GuidIndex(0)), |guid| self.guids.add(guid))
    }
}

/// Resolve the handles of `model`. With a base, the heaps start with the base heaps.
pub(super) fn resolve(model: &Model<'_>, base: Option<Base>) -> Result<Resolved> {
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
    for scope in &model.scopes.records {
        for variable in &scope.variables {
            strings.add(&variable.name)?;
        }
        for constant in &scope.constants {
            strings.add(&constant.name)?;
        }
    }
    let mut heaps = Heaps {
        strings: strings.finish()?,
        blobs,
        guids,
    };
    let mut tables = DebugTables {
        documents: documents(model.documents, &mut heaps)?,
        method_debug_information: methods(model, &mut heaps)?,
        ..DebugTables::default()
    };
    let (scope_rows, list_rows) = scopes(model.scopes, &mut heaps, &mut tables)?;
    tables.import_scopes = import_scopes(model.import_scopes, &mut heaps)?;
    tables.state_machine_methods = model
        .state_machines
        .iter()
        .map(|method| StateMachineMethodRow {
            move_next_method: method.move_next_method,
            kickoff_method: method.kickoff_method,
        })
        .collect();
    tables
        .state_machine_methods
        .sort_by_key(|row| row.move_next_method);
    let rows = Targets {
        documents: model.documents.len(),
        import_scopes: model.import_scopes.len(),
        scopes: model.scopes,
        scope_rows: &scope_rows,
        list_rows: &list_rows,
    };
    tables.custom_debug_information = custom(model.custom, &mut heaps, &rows)?;
    let Heaps {
        strings,
        blobs,
        guids,
    } = heaps;
    Ok(Resolved {
        parts: PdbParts {
            version: model.version.to_vec(),
            entry_point: model.entry_point,
            external: model.external,
            strings,
            user_strings,
            blobs,
            guids,
            tables,
        },
        scope_rows,
        list_rows,
    })
}

/// Make the `Document` rows.
fn documents(documents: &Documents, heaps: &mut Heaps) -> Result<Vec<DocumentRow>> {
    let mut rows = Vec::with_capacity(documents.len());
    let mut name = Vec::new();
    for (row, (_, document)) in documents.iter().enumerate() {
        let location = cell(TableId::DOCUMENT, row, "Name");
        let split = crate::portable::debug::split_document_name(&document.name);
        let separator = split.separator();
        let parts = split
            .map(|part| heaps.blobs.add(part))
            .collect::<Result<Vec<_>>>()?;
        name.clear();
        DocumentName::encode_parts(Some(separator), &parts, &mut name)
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
///
/// Each sequence point blob keeps its bytes. The function decodes it only to check that its
/// documents exist.
fn methods(model: &Model<'_>, heaps: &mut Heaps) -> Result<Vec<MethodDebugInformationRow>> {
    let methods = &model.methods.records;
    if !model.methods.table && methods.is_empty() {
        return Ok(Vec::new());
    }
    let table = TableId::METHOD_DEBUG_INFORMATION;
    let count = model.external.get(TableId::METHOD_DEF);
    if let Some((&method, _)) = methods.last_key_value()
        && method.0 > count
    {
        return Err(reference(
            table,
            usize::try_from(method.0).unwrap_or(usize::MAX) - 1,
            "Document",
            "the method is after the external MethodDef rows",
        ));
    }
    let documents = model.documents.len();
    let exists = |document: DocumentId| {
        usize::try_from(document.0).is_ok_and(|document| document <= documents)
    };
    let mut rows = Vec::new();
    for method in 1..=count {
        let row = usize::try_from(method - 1).unwrap_or(usize::MAX);
        let Some(information) = methods.get(&MethodDefId(method)) else {
            rows.push(MethodDebugInformationRow {
                document: DocumentId(0),
                sequence_points: BlobOffset(0),
            });
            continue;
        };
        let at = |error: Error| error.with_portable(cell(table, row, "SequencePoints"));
        if let Some(points) = information.decode().map_err(at)? {
            for point in points {
                if !exists(point.map_err(at)?.document) {
                    return Err(reference(
                        table,
                        row,
                        "SequencePoints",
                        "a sequence point document is not a document of the editor",
                    ));
                }
            }
        }
        if !exists(information.document) {
            return Err(reference(
                table,
                row,
                "Document",
                "the document is not a document of the editor",
            ));
        }
        let sequence_points = if information.sequence_points.is_empty() {
            BlobOffset(0)
        } else {
            heaps.blobs.add(&information.sequence_points)?
        };
        rows.push(MethodDebugInformationRow {
            document: information.document,
            sequence_points,
        });
    }
    Ok(rows)
}

/// The rows of each local scope: the scope row and the first variable and constant rows.
type ScopeRows = (Vec<u32>, Vec<[u32; 2]>);

/// Sort the local scopes, give each scope one run of variables and of constants, and make
/// the rows. Return the row ID and the list rows of each scope handle.
fn scopes(scopes: &LocalScopes, heaps: &mut Heaps, tables: &mut DebugTables) -> Result<ScopeRows> {
    let records = &scopes.records;
    let mut order: Vec<usize> = (0..records.len()).collect();
    order.sort_by_key(|&index| {
        let scope = &records[index];
        (scope.method, scope.start_offset, Reverse(scope.length))
    });
    let any_variables = records.iter().any(|scope| !scope.variables.is_empty());
    let any_constants = records.iter().any(|scope| !scope.constants.is_empty());
    let mut scope_rows = vec![0; records.len()];
    let mut list_rows = vec![[0; 2]; records.len()];
    let [empty_variables, empty_constants] = scopes.empty_lists.map(EmptyListForm::value);
    for (position, &index) in order.iter().enumerate() {
        scope_rows[index] = row_id(position);
        let scope = &records[index];
        let first_variable = row_id(tables.local_variables.len());
        let first_constant = row_id(tables.local_constants.len());
        list_rows[index] = [first_variable, first_constant];
        for variable in &scope.variables {
            tables.local_variables.push(LocalVariableRow {
                attributes: variable.attributes,
                index: variable.index,
                name: heaps.string(&variable.name),
            });
        }
        for constant in &scope.constants {
            tables.local_constants.push(LocalConstantRow {
                name: heaps.string(&constant.name),
                signature: heaps.blobs.add(&constant.signature)?,
            });
        }
        tables.local_scopes.push(LocalScopeRow {
            method: scope.method,
            import_scope: scope.import_scope,
            variable_list: LocalVariableId(if any_variables {
                first_variable
            } else {
                empty_variables
            }),
            constant_list: LocalConstantId(if any_constants {
                first_constant
            } else {
                empty_constants
            }),
            start_offset: scope.start_offset,
            length: scope.length,
        });
    }
    Ok((scope_rows, list_rows))
}

/// Make the `ImportScope` rows.
fn import_scopes(scopes: &ImportScopes, heaps: &mut Heaps) -> Result<Vec<ImportScopeRow>> {
    let mut rows = Vec::with_capacity(scopes.len());
    let mut blob = Vec::new();
    for (row, (_, scope)) in scopes.iter().enumerate() {
        let imports = scope
            .imports
            .iter()
            .map(|import| import.to_raw(|name| heaps.blobs.add(name)))
            .collect::<Result<Vec<_>>>()?;
        blob.clear();
        Imports::encode_parts(&imports, &mut blob)
            .map_err(|error| error.with_portable(cell(TableId::IMPORT_SCOPE, row, "Imports")))?;
        rows.push(ImportScopeRow {
            parent: scope.parent,
            imports: heaps.blobs.add(&blob)?,
        });
    }
    Ok(rows)
}

/// The row counts and the scope rows that custom debug information parents need.
struct Targets<'a> {
    documents: usize,
    import_scopes: usize,
    scopes: &'a LocalScopes,
    scope_rows: &'a [u32],
    list_rows: &'a [[u32; 2]],
}

impl Targets<'_> {
    /// Return the row of the entry `index` of the list `list` (0: variables, 1: constants) of
    /// a scope, or `None` if the scope or the entry does not exist.
    fn local(&self, scope: usize, index: u32, list: usize) -> Option<u32> {
        let record = self.scopes.records.get(scope)?;
        let len = if list == 0 {
            record.variables.len()
        } else {
            record.constants.len()
        };
        let index_usize = usize::try_from(index).ok()?;
        (index_usize < len).then(|| self.list_rows[scope][list] + index)
    }

    /// Return the token of a parent, or `None` if the editor has no such record.
    fn token(&self, parent: CustomDebugParent) -> Option<MetadataToken> {
        let within = |id: u32, count: usize| {
            usize::try_from(id)
                .ok()
                .filter(|&row| row <= count)
                .map(|_| id)
        };
        let (table, row) = match parent {
            CustomDebugParent::Metadata(token) => {
                let table = token.table().filter(|table| table.is_type_system())?;
                (table, Some(token.row()))
            }
            CustomDebugParent::Document(id) => (TableId::DOCUMENT, within(id.0, self.documents)),
            CustomDebugParent::ImportScope(id) => {
                (TableId::IMPORT_SCOPE, within(id.0, self.import_scopes))
            }
            CustomDebugParent::LocalScope(handle) => (
                TableId::LOCAL_SCOPE,
                self.scope_rows.get(handle.index()).copied(),
            ),
            CustomDebugParent::LocalVariable(handle) => (
                TableId::LOCAL_VARIABLE,
                self.local(handle.scope.index(), handle.index, 0),
            ),
            CustomDebugParent::LocalConstant(handle) => (
                TableId::LOCAL_CONSTANT,
                self.local(handle.scope.index(), handle.index, 1),
            ),
        };
        row.filter(|&row| row != 0)
            .and_then(|row| MetadataToken::new(table, row))
    }
}

/// Make the `CustomDebugInformation` rows, sorted by the coded parent.
fn custom(
    custom: &CustomDebugInformations,
    heaps: &mut Heaps,
    rows: &Targets<'_>,
) -> Result<Vec<CustomDebugInformationRow>> {
    let table = TableId::CUSTOM_DEBUG_INFORMATION;
    let mut keyed = Vec::with_capacity(custom.records.len());
    for (index, information) in custom.records.iter().enumerate() {
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
                "parent is not a record of the editor or a HasCustomDebugInformation row",
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
