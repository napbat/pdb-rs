//! Resolve the handles of an editor model and make the raw parts.

use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Reverse;

use super::Methods;
use super::collections::{
    CustomDebugInformations, Documents, EmptyListForm, ImportScopes, LocalScopes, SourceRun,
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
    /// Read the heaps of `pdb`. If `locals` is `true`, record the offset of each local
    /// variable and constant name, so that a name in the middle of another string keeps its
    /// source offset.
    pub(super) fn read<S: ReadAt>(pdb: &Pdb<S>, locals: bool) -> Result<Self> {
        let heap =
            |stream| -> Result<Vec<u8>> { Ok(pdb.stream_bytes(stream)?.unwrap_or_default()) };
        let mut strings = StringHeap::from_bytes(heap(MetadataStream::Strings)?)?;
        if locals {
            for row in pdb.rows::<LocalVariableRow>() {
                let (_, row) = row?;
                strings.register(&pdb.string(row.name)?, row.name);
            }
            for row in pdb.rows::<LocalConstantRow>() {
                let (_, row) = row?;
                strings.register(&pdb.string(row.name)?, row.name);
            }
        }
        Ok(Self {
            strings,
            user_strings: UserStringHeap::from_bytes(heap(MetadataStream::UserStrings)?)?,
            blobs: BlobHeap::from_bytes(heap(MetadataStream::Blob)?)?,
            guids: GuidHeap::from_bytes(heap(MetadataStream::Guid)?)?,
        })
    }
}

/// The records of an editor for the plan. `None` selects the raw rows of the source for a
/// table group that the editor did not decode.
pub(super) struct Model<'m> {
    /// The metadata version string.
    pub(super) version: &'m [u8],
    /// The entry point.
    pub(super) entry_point: Option<MethodDefId>,
    /// The external row counts.
    pub(super) external: ExternalRowCounts,
    /// The documents.
    pub(super) documents: Option<&'m Documents>,
    /// The methods with sequence points.
    pub(super) methods: Option<&'m Methods>,
    /// The local scopes.
    pub(super) scopes: Option<&'m LocalScopes>,
    /// The import scopes.
    pub(super) import_scopes: Option<&'m ImportScopes>,
    /// The state machine methods.
    pub(super) state_machines: Option<&'m [StateMachineMethod]>,
}

/// The parts of a model without the custom debug information, and the rows of each local
/// scope.
pub(super) struct Resolved {
    /// The raw parts.
    pub(super) parts: PdbParts,
    /// The rows of each local scope, by handle.
    pub(super) scopes: ScopeRows,
}

/// The output rows of the local scopes, by handle. Custom debug information parents refer
/// to them.
#[derive(Default)]
pub(super) struct ScopeRows {
    /// The row ID of each local scope.
    pub(super) rows: Vec<u32>,
    /// The first `LocalVariable` row and the first `LocalConstant` row of each local scope.
    pub(super) lists: Vec<[u32; 2]>,
    /// The number of local variables and of local constants of each local scope.
    lengths: Vec<[u32; 2]>,
}

impl ScopeRows {
    /// Return the rows of local scopes that keep their source rows, from the source runs.
    pub(super) fn from_source(runs: &[[SourceRun; 2]]) -> Self {
        Self {
            rows: (1..).take(runs.len()).collect(),
            lists: runs.iter().map(|runs| runs.map(|run| run.start)).collect(),
            lengths: runs.iter().map(|runs| runs.map(|run| run.count)).collect(),
        }
    }
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

/// Resolve the handles of `model` and make the rows of all tables but the custom debug
/// information. With a base, the heaps start with the base heaps. `raw` has the source rows
/// of each table group that `model` does not have (see [`decode::raw_rows`]).
///
/// [`decode::raw_rows`]: super::decode::raw_rows
pub(super) fn resolve(model: &Model<'_>, base: Option<Base>, raw: DebugTables) -> Result<Resolved> {
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
    for scope in model.scopes.iter().flat_map(|scopes| &scopes.records) {
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
    let mut tables = raw;
    if let Some(model_documents) = model.documents {
        tables.documents = documents(model_documents, &mut heaps)?;
    }
    if let Some(model_methods) = model.methods {
        let documents = tables.documents.len();
        tables.method_debug_information = methods(model, model_methods, documents, &mut heaps)?;
    }
    let scope_rows = match model.scopes {
        Some(model_scopes) => scopes(model_scopes, &mut heaps, &mut tables)?,
        None => ScopeRows::default(),
    };
    if let Some(model_imports) = model.import_scopes {
        tables.import_scopes = import_scopes(model_imports, &mut heaps)?;
    }
    if let Some(state_machines) = model.state_machines {
        tables.state_machine_methods = state_machines
            .iter()
            .map(|method| StateMachineMethodRow {
                move_next_method: method.move_next_method,
                kickoff_method: method.kickoff_method,
            })
            .collect();
        tables
            .state_machine_methods
            .sort_by_key(|row| row.move_next_method);
    }
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
        scopes: scope_rows,
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
/// The output has `documents` documents.
///
/// Each sequence point blob keeps its bytes. The function decodes it only to check that its
/// documents exist.
fn methods(
    model: &Model<'_>,
    model_methods: &Methods,
    documents: usize,
    heaps: &mut Heaps,
) -> Result<Vec<MethodDebugInformationRow>> {
    let methods = &model_methods.records;
    if !model_methods.table && methods.is_empty() {
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

/// Sort the local scopes, give each scope one run of variables and of constants, and make
/// the rows. Return the rows of each scope handle.
fn scopes(scopes: &LocalScopes, heaps: &mut Heaps, tables: &mut DebugTables) -> Result<ScopeRows> {
    let records = &scopes.records;
    let mut order: Vec<usize> = (0..records.len()).collect();
    order.sort_by_key(|&index| {
        let scope = &records[index];
        (scope.method, scope.start_offset, Reverse(scope.length))
    });
    let any_variables = records.iter().any(|scope| !scope.variables.is_empty());
    let any_constants = records.iter().any(|scope| !scope.constants.is_empty());
    let length = |len: usize| u32::try_from(len).unwrap_or(u32::MAX);
    let mut rows = ScopeRows {
        rows: vec![0; records.len()],
        lists: vec![[0; 2]; records.len()],
        lengths: records
            .iter()
            .map(|scope| [length(scope.variables.len()), length(scope.constants.len())])
            .collect(),
    };
    let [empty_variables, empty_constants] = scopes.empty_lists.map(EmptyListForm::value);
    for (position, &index) in order.iter().enumerate() {
        rows.rows[index] = row_id(position);
        let scope = &records[index];
        let first_variable = row_id(tables.local_variables.len());
        let first_constant = row_id(tables.local_constants.len());
        rows.lists[index] = [first_variable, first_constant];
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
    Ok(rows)
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
    scopes: &'a ScopeRows,
}

impl Targets<'_> {
    /// Return the row of the entry `index` of the list `list` (0: variables, 1: constants) of
    /// a scope, or `None` if the scope or the entry does not exist.
    fn local(&self, scope: usize, index: u32, list: usize) -> Option<u32> {
        let length = self.scopes.lengths.get(scope)?[list];
        (index < length).then(|| self.scopes.lists[scope][list] + index)
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
                self.scopes.rows.get(handle.index()).copied(),
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

impl Resolved {
    /// Make the `CustomDebugInformation` rows of `custom`, sorted by the coded parent.
    pub(super) fn add_custom(&mut self, custom: &CustomDebugInformations) -> Result<()> {
        let parts = &mut self.parts;
        let rows = Targets {
            documents: parts.tables.documents.len(),
            import_scopes: parts.tables.import_scopes.len(),
            scopes: &self.scopes,
        };
        let table = TableId::CUSTOM_DEBUG_INFORMATION;
        let mut keyed = Vec::with_capacity(custom.records.len());
        for (index, information) in custom.records.iter().enumerate() {
            let Some(information) = information else {
                continue;
            };
            let parent = rows.token(information.parent);
            let coded =
                parent.and_then(|token| CodedIndex::HasCustomDebugInformation.encode(token));
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
                kind: parts.guids.add(information.kind.0)?,
                value: parts.blobs.add(&information.value)?,
            };
            keyed.push((coded, row));
        }
        keyed.sort_by_key(|&(coded, _)| coded);
        parts.tables.custom_debug_information = keyed.into_iter().map(|(_, row)| row).collect();
        Ok(())
    }
}
