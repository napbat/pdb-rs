//! The Portable PDB builder: typed records from the input, written and read back.
//!
//! The input is a sequence of commands. Each command adds one record with field values from
//! the input. If the plan succeeds, the written PDB must read back with the same documents,
//! sequence points, scopes and custom debug information values.

use pdbrs::portable::build::{
    CustomDebugInfo, CustomDebugParent, DocumentInfo, ImportInfo, ImportScopeInfo,
    LocalConstantInfo, LocalScopeHandle, LocalScopeInfo, LocalVariableInfo, MethodInfo,
};
use pdbrs::portable::custom::CustomDebugKind;
use pdbrs::portable::debug::{LocalSignatureForm, SequencePoint, SourceSpan};
use pdbrs::portable::metadata::{
    AssemblyRefId, DocumentId, IlOffset, ImportScopeId, MetadataToken, MethodDefId,
    StandAloneSigId, TableId,
};
use pdbrs::portable::write::ExternalRowCounts;
use pdbrs::portable::{IdentityPolicy, Pdb, PdbBuilder};

use crate::input::Input;
use crate::ok;
use crate::portable::{options, walk};

/// The most commands that one input runs.
const MAX_COMMANDS: usize = 64;

/// The row count of each external table that the records can name.
const EXTERNAL_ROWS: u32 = 8;

/// Read a short byte string: a length byte and the bytes.
fn text<'a>(input: &mut Input<'a>) -> &'a [u8] {
    let length = usize::from(input.u8() % 32);
    input.bytes(length)
}

/// Read a sequence point for one of `documents`.
fn sequence_point(input: &mut Input<'_>, documents: &[DocumentId]) -> SequencePoint {
    let document = documents
        .get(usize::from(input.u8()) % documents.len().max(1))
        .copied()
        .unwrap_or(DocumentId::new(1));
    let il_offset = IlOffset::new(input.u32() % 0x1000);
    let span = (input.u8() & 1 == 0).then(|| SourceSpan {
        start_line: input.u32(),
        start_column: input.u16(),
        end_line: input.u32(),
        end_column: input.u16(),
    });
    SequencePoint {
        il_offset,
        document,
        span,
    }
}

/// Read an import with names from the input.
fn import(input: &mut Input<'_>) -> ImportInfo {
    let selector = input.u8();
    let name = text(input).to_vec();
    let assembly = AssemblyRefId::new(u32::from(input.u8()) % (EXTERNAL_ROWS + 1));
    let target_type = MetadataToken::from_raw(input.u32());
    match selector % 9 {
        0 => ImportInfo::Namespace { namespace: name },
        1 => ImportInfo::AssemblyNamespace {
            assembly,
            namespace: name,
        },
        2 => ImportInfo::Type { target_type },
        3 => ImportInfo::XmlNamespace {
            alias: name.clone(),
            namespace: name,
        },
        4 => ImportInfo::AssemblyReferenceAlias { alias: name },
        5 => ImportInfo::AliasAssemblyReference {
            alias: name,
            assembly,
        },
        6 => ImportInfo::AliasNamespace {
            alias: name.clone(),
            namespace: name,
        },
        7 => ImportInfo::AliasAssemblyNamespace {
            alias: name.clone(),
            assembly,
            namespace: name,
        },
        _ => ImportInfo::AliasType {
            alias: name,
            target_type,
        },
    }
}

/// The state of one build: the builder and the records that the commands added.
struct Build {
    builder: PdbBuilder,
    documents: Vec<DocumentId>,
    scopes: Vec<LocalScopeHandle>,
    imports: Vec<ImportScopeId>,
    methods: Vec<(MethodDefId, Vec<SequencePoint>)>,
}

impl Build {
    /// Run one command from the input. Return `None` if the builder rejects the record or
    /// if the command needs a record that does not exist.
    fn command(&mut self, input: &mut Input<'_>) -> Option<()> {
        let selector = input.u8();
        match selector % 7 {
            0 => {
                let mut document = DocumentInfo::default();
                document.name = text(input).to_vec();
                document.hash_algorithm = (input.u8() & 1 == 1).then_some([input.u8(); 16]);
                document.hash = text(input).to_vec();
                document.language = (input.u8() & 1 == 1).then_some([input.u8(); 16]);
                ok(self.builder.add_document(document)).map(|id| self.documents.push(id))
            }
            1 => {
                let method = MethodDefId::new(1 + u32::from(input.u8()) % EXTERNAL_ROWS);
                let count = usize::from(input.u8() % 8);
                let points: Vec<SequencePoint> = (0..count)
                    .map(|_| sequence_point(input, &self.documents))
                    .collect();
                let form = if input.u8() & 1 == 0 {
                    LocalSignatureForm::Row
                } else {
                    LocalSignatureForm::Token
                };
                let mut info = MethodInfo::default();
                info.local_signature = StandAloneSigId::new(u32::from(input.u8() % 2));
                info.local_signature_form = form;
                info.sequence_points = points.clone();
                self.builder.set_method(method, info);
                self.methods.retain(|(id, _)| *id != method);
                self.methods.push((method, points));
                Some(())
            }
            2 => {
                let mut scope = LocalScopeInfo::default();
                scope.method = MethodDefId::new(1 + u32::from(input.u8()) % EXTERNAL_ROWS);
                scope.import_scope = self.imports.first().copied().unwrap_or_default();
                scope.start_offset = IlOffset::new(u32::from(input.u16()));
                scope.length = u32::from(input.u16());
                ok(self.builder.add_local_scope(scope)).map(|handle| self.scopes.push(handle))
            }
            3 => {
                let scope = *self.scopes.last()?;
                let mut variable = LocalVariableInfo::default();
                variable.attributes = input.u16();
                variable.index = input.u16();
                variable.name = text(input).to_vec();
                ok(self.builder.add_local_variable(scope, variable)).map(|_| ())
            }
            4 => {
                let scope = *self.scopes.last()?;
                let mut constant = LocalConstantInfo::default();
                constant.name = text(input).to_vec();
                constant.signature = text(input).to_vec();
                ok(self.builder.add_local_constant(scope, constant)).map(|_| ())
            }
            5 => {
                let count = usize::from(input.u8() % 4);
                let mut scope = ImportScopeInfo::default();
                scope.parent = self.imports.last().copied().unwrap_or_default();
                scope.imports = (0..count).map(|_| import(input)).collect();
                ok(self.builder.add_import_scope(scope)).map(|id| self.imports.push(id))
            }
            _ => {
                let parent = match input.u8() % 3 {
                    0 => CustomDebugParent::Metadata(
                        MetadataToken::new(TableId::METHOD_DEF, 1)
                            .unwrap_or(MetadataToken::from_raw(0)),
                    ),
                    1 => CustomDebugParent::Document(
                        self.documents.first().copied().unwrap_or_default(),
                    ),
                    _ => CustomDebugParent::LocalScope(*self.scopes.first()?),
                };
                let mut information =
                    CustomDebugInfo::new(parent, CustomDebugKind([input.u8(); 16]));
                information.value = text(input).to_vec();
                ok(self.builder.add_custom_debug_information(information)).map(|_| ())
            }
        }
    }
}

/// Build a Portable PDB from typed records and check the written PDB.
///
/// # Panics
///
/// Panics if a planned PDB does not write or read, or if the written PDB has other
/// documents or sequence points than the builder.
pub fn build(data: &[u8]) {
    let mut external = ExternalRowCounts::new();
    for table in [
        TableId::MODULE,
        TableId::TYPE_REF,
        TableId::TYPE_DEF,
        TableId::METHOD_DEF,
        TableId::STAND_ALONE_SIG,
        TableId::ASSEMBLY_REF,
    ] {
        external.set(table, EXTERNAL_ROWS);
    }
    let mut build = Build {
        builder: PdbBuilder::new(external),
        documents: Vec::new(),
        scopes: Vec::new(),
        imports: Vec::new(),
        methods: Vec::new(),
    };
    let mut input = Input::new(data);
    for _ in 0..MAX_COMMANDS {
        if input.is_empty() {
            break;
        }
        let _ = build.command(&mut input);
    }
    let Some(plan) = ok(build.builder.plan(IdentityPolicy::ContentDerived)) else {
        return;
    };
    let bytes = plan.to_vec().expect("a planned PDB writes");
    let pdb = Pdb::with_options(bytes.as_slice(), options()).expect("a built PDB reads");
    assert_eq!(pdb.identity(), plan.identity());
    assert_eq!(pdb.documents().count(), build.builder.document_count());
    for (id, document) in pdb.documents().enumerate() {
        let document = document.expect("a built document reads");
        let expected = build
            .builder
            .document(document.id())
            .expect("a read document has a record");
        assert_eq!(document.name(), expected.name, "document {id}");
        assert_eq!(document.hash(), expected.hash);
    }
    for (method, points) in &build.methods {
        let id = pdbrs::portable::metadata::MethodDebugInformationId::new(method.get());
        let info = pdb
            .method_debug_information(id)
            .expect("a built method reads");
        let decoded: Vec<SequencePoint> = info
            .sequence_points()
            .expect("built sequence points decode")
            .map(Iterator::collect::<pdbrs::Result<_>>)
            .transpose()
            .expect("built sequence points decode")
            .unwrap_or_default();
        assert_eq!(&decoded, points);
    }
    walk(&pdb);
}
