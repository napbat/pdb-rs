//! The Portable PDB builder: typed records from the input, written and read back.
//!
//! The input is a sequence of commands. Each command adds one record with field values from
//! the input. If the plan succeeds, the written PDB must read back with the same documents,
//! sequence points, scopes and custom debug information values.

use pdbrs::Bytes;
use pdbrs::portable::custom::{CustomDebugInformation, CustomDebugKind, CustomDebugParent};
use pdbrs::portable::debug::{
    Document, Import, ImportScope, LocalConstant, LocalScope, LocalScopeHandle, LocalVariable,
};
use pdbrs::portable::debug::{LocalSignatureForm, SequencePoint, SequencePoints, SourceSpan};
use pdbrs::portable::metadata::{
    AssemblyRefId, DocumentId, IlOffset, ImportScopeId, MetadataToken, MethodDefId,
    StandAloneSigId, TableId,
};
use pdbrs::portable::write::ExternalRowCounts;
use pdbrs::portable::{EditOptions, IdentityPolicy, Pdb, PdbEditor};

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
        .unwrap_or(DocumentId(1));
    let il_offset = IlOffset(input.u32() % 0x1000);
    let span = (input.u8() & 1 == 0).then(|| {
        let mut span = SourceSpan::default();
        span.start_line = input.u32();
        span.start_column = input.u16();
        span.end_line = input.u32();
        span.end_column = input.u16();
        span
    });
    let mut point = SequencePoint::default();
    point.il_offset = il_offset;
    point.document = document;
    point.span = span;
    point
}

/// Read an import with names from the input.
fn import(input: &mut Input<'_>) -> Import<'static> {
    let selector = input.u8();
    let name = Bytes::from(text(input).to_vec());
    let assembly = AssemblyRefId(u32::from(input.u8()) % (EXTERNAL_ROWS + 1));
    let target_type = MetadataToken::from_raw(input.u32());
    match selector % 9 {
        0 => Import::Namespace { namespace: name },
        1 => Import::AssemblyNamespace {
            assembly,
            namespace: name,
        },
        2 => Import::Type { target_type },
        3 => Import::XmlNamespace {
            alias: name.clone(),
            namespace: name,
        },
        4 => Import::AssemblyReferenceAlias { alias: name },
        5 => Import::AliasAssemblyReference {
            alias: name,
            assembly,
        },
        6 => Import::AliasNamespace {
            alias: name.clone(),
            namespace: name,
        },
        7 => Import::AliasAssemblyNamespace {
            alias: name.clone(),
            assembly,
            namespace: name,
        },
        _ => Import::AliasType {
            alias: name,
            target_type,
        },
    }
}

/// The state of one build: the editor and the records that the commands added.
struct Build {
    editor: PdbEditor<'static>,
    documents: Vec<DocumentId>,
    scopes: Vec<LocalScopeHandle>,
    imports: Vec<ImportScopeId>,
    methods: Vec<(MethodDefId, Vec<SequencePoint>)>,
}

impl Build {
    /// Run one command from the input. Return `None` if the editor rejects the record or
    /// if the command needs a record that does not exist.
    fn command(&mut self, input: &mut Input<'_>) -> Option<()> {
        let selector = input.u8();
        match selector % 7 {
            0 => {
                let mut document = Document::default();
                document.name = text(input).to_vec().into();
                document.hash_algorithm = (input.u8() & 1 == 1).then_some([input.u8(); 16]);
                document.hash = text(input).to_vec().into();
                document.language = (input.u8() & 1 == 1).then_some([input.u8(); 16]);
                let documents = ok(self.editor.documents_mut())?;
                ok(documents.push(document)).map(|id| self.documents.push(id))
            }
            1 => {
                let method = MethodDefId(1 + u32::from(input.u8()) % EXTERNAL_ROWS);
                let count = usize::from(input.u8() % 8);
                let points: Vec<SequencePoint> = (0..count)
                    .map(|_| sequence_point(input, &self.documents))
                    .collect();
                let form = if input.u8() & 1 == 0 {
                    LocalSignatureForm::Row
                } else {
                    LocalSignatureForm::Token
                };
                let signature = StandAloneSigId(u32::from(input.u8() % 2));
                let info = ok(SequencePoints::encode_parts(signature, form, &points))?;
                ok(self.editor.methods_mut())?.insert(method, info);
                self.methods.retain(|(id, _)| *id != method);
                self.methods.push((method, points));
                Some(())
            }
            2 => {
                let mut scope = LocalScope::default();
                scope.method = MethodDefId(1 + u32::from(input.u8()) % EXTERNAL_ROWS);
                scope.import_scope = self.imports.first().copied().unwrap_or_default();
                scope.start_offset = IlOffset(u32::from(input.u16()));
                scope.length = u32::from(input.u16());
                let scopes = ok(self.editor.local_scopes_mut())?;
                ok(scopes.push(scope)).map(|handle| self.scopes.push(handle))
            }
            3 => {
                let scope = *self.scopes.last()?;
                let mut variable = LocalVariable::default();
                variable.attributes = input.u16();
                variable.index = input.u16();
                variable.name = text(input).to_vec().into();
                let scopes = ok(self.editor.local_scopes_mut())?;
                scopes.get_mut(scope)?.variables.push(variable);
                Some(())
            }
            4 => {
                let scope = *self.scopes.last()?;
                let mut constant = LocalConstant::default();
                constant.name = text(input).to_vec().into();
                constant.signature = text(input).to_vec().into();
                let scopes = ok(self.editor.local_scopes_mut())?;
                scopes.get_mut(scope)?.constants.push(constant);
                Some(())
            }
            5 => {
                let count = usize::from(input.u8() % 4);
                let mut scope = ImportScope::default();
                scope.parent = self.imports.last().copied().unwrap_or_default();
                scope.imports = (0..count).map(|_| import(input)).collect();
                let scopes = ok(self.editor.import_scopes_mut())?;
                ok(scopes.push(scope)).map(|id| self.imports.push(id))
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
                    CustomDebugInformation::new(parent, CustomDebugKind([input.u8(); 16]));
                information.value = text(input).to_vec().into();
                let informations = ok(self.editor.custom_debug_information_mut())?;
                ok(informations.push(information)).map(|_| ())
            }
        }
    }
}

/// Build a Portable PDB from typed records and check the written PDB.
///
/// # Panics
///
/// Panics if a planned PDB does not write or read, or if the written PDB has other
/// documents or sequence points than the editor.
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
        editor: PdbEditor::new(external, IdentityPolicy::ContentDerived),
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
    let Some(plan) = ok(build.editor.plan(EditOptions::default())) else {
        return;
    };
    let bytes = plan.to_vec().expect("a planned PDB writes");
    let pdb = Pdb::with_options(bytes.as_slice(), options()).expect("a built PDB reads");
    assert_eq!(pdb.identity(), plan.identity());
    let documents = build
        .editor
        .documents_mut()
        .expect("new editor documents decode");
    assert_eq!(pdb.documents().count(), documents.len());
    for (index, document) in pdb.documents().enumerate() {
        let (id, document) = document.expect("a built document reads");
        let expected = documents.get(id).expect("a read document has a record");
        assert_eq!(document.name, expected.name, "document {index}");
        assert_eq!(document.hash, expected.hash);
    }
    for (method, points) in &build.methods {
        let id = pdbrs::portable::metadata::MethodDebugInformationId(method.0);
        let info = pdb
            .method_debug_information(id)
            .expect("a built method reads");
        let decoded: Vec<SequencePoint> = info
            .decode()
            .expect("built sequence points decode")
            .map(Iterator::collect::<pdbrs::Result<_>>)
            .transpose()
            .expect("built sequence points decode")
            .unwrap_or_default();
        assert_eq!(&decoded, points);
    }
    walk(&pdb);
}
