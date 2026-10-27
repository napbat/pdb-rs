//! A synthetic Portable PDB with rows in each debug table and each custom debug information
//! kind.

use pdbrs::Bytes;
use pdbrs::portable::custom::{
    AsyncMethodSteppingInformation, AsyncStep, CompilationMetadataReferences, CompilationOption,
    CompilationOptions, CustomDebugInformation, CustomDebugKind, CustomDebugParent,
    DynamicLocalVariables, EncLambdaAndClosureMap, EncLocalSlotMap, EncStateMachineStateMap,
    HoistedLocalScope, HoistedLocalScopes, Lambda, LambdaClosure, LocalSlot, MetadataReference,
    StateMachineState, TupleElementNames, TypeDefinitionDocuments,
};
use pdbrs::portable::debug::{
    ConstantValue, Document, HashAlgorithm, Import, ImportScope, Language, LocalConstant,
    LocalConstantSignature, LocalScope, LocalSignatureForm, LocalVariable, LocalVariableHandle,
    MethodDebugInformation, PrimitiveConstant, SequencePoint, SequencePoints, SourceSpan,
    StateMachineMethod,
};
use pdbrs::portable::embedded::{EmbeddedFormat, EmbeddedSource};
use pdbrs::portable::metadata::{
    AssemblyRefId, DocumentId, IlOffset, ImportScopeId, MetadataToken, MethodDefId,
    StandAloneSigId, TableId,
};
use pdbrs::portable::write::ExternalRowCounts;
use pdbrs::portable::{EditOptions, Identity, IdentityPolicy, Pdb, PdbEditor};

use crate::Outcome;

/// Run an encoder that appends to a buffer, and return the bytes.
fn blob(encode: impl FnOnce(&mut Vec<u8>) -> pdbrs::Result<()>) -> pdbrs::Result<Vec<u8>> {
    let mut out = Vec::new();
    encode(&mut out)?;
    Ok(out)
}

/// Make a custom debug information record with `value`.
fn information(
    parent: CustomDebugParent,
    kind: CustomDebugKind,
    value: Vec<u8>,
) -> CustomDebugInformation<'static> {
    let mut information = CustomDebugInformation::new(parent, kind);
    information.value = value.into();
    information
}

/// Make the sequence points of a method with a local signature row.
fn method(
    local_signature: u32,
    sequence_points: &[SequencePoint],
) -> pdbrs::Result<MethodDebugInformation<'static>> {
    SequencePoints::encode_parts(
        StandAloneSigId(local_signature),
        LocalSignatureForm::Row,
        sequence_points,
    )
}

/// Make an import scope.
fn import_scope(parent: ImportScopeId, imports: Vec<Import<'static>>) -> ImportScope<'static> {
    let mut scope = ImportScope::default();
    scope.parent = parent;
    scope.imports = imports;
    scope
}

/// A sequence point with a span on one line, or a hidden sequence point without a line.
fn point(offset: u32, document: DocumentId, line: Option<u32>) -> SequencePoint {
    let mut point = SequencePoint::default();
    point.il_offset = IlOffset(offset);
    point.document = document;
    point.span = line.map(|line| {
        let mut span = SourceSpan::default();
        span.start_line = line;
        span.start_column = 5;
        span.end_line = line;
        span.end_column = 20;
        span
    });
    point
}

/// Add the documents and the methods with their sequence points.
fn documents_and_methods(editor: &mut PdbEditor<'static>, source: &[u8]) -> Outcome<DocumentId> {
    let mut documents = Vec::new();
    for name in [&b"C:\\src\\Program.cs"[..], b"/src/Helper.cs"] {
        let mut document = Document::default();
        document.name = name.to_vec().into();
        document.hash_algorithm = Some(HashAlgorithm::Sha256.guid());
        document.hash = vec![0x33; 32].into();
        document.language = Some(Language::CSharp.guid());
        documents.push(editor.documents_mut()?.push(document)?);
    }
    let hidden = point(6, documents[0], None);
    let methods = editor.methods_mut()?;
    methods.insert(
        MethodDefId(1),
        method(
            1,
            &[
                point(0, documents[0], Some(3)),
                hidden,
                point(8, documents[0], Some(4)),
            ],
        )?,
    );
    methods.insert(
        MethodDefId(2),
        method(
            0,
            &[
                point(0, documents[0], Some(10)),
                point(4, documents[1], Some(2)),
            ],
        )?,
    );
    let informations = editor.custom_debug_information_mut()?;
    informations.push(information(
        CustomDebugParent::Document(documents[0]),
        CustomDebugKind::EMBEDDED_SOURCE,
        blob(|out| EmbeddedSource::encode_parts(source, EmbeddedFormat::Deflate, out))?,
    ))?;
    informations.push(information(
        CustomDebugParent::Document(documents[1]),
        CustomDebugKind::EMBEDDED_SOURCE,
        blob(|out| EmbeddedSource::encode_parts(b"class Helper { }\n", EmbeddedFormat::Raw, out))?,
    ))?;
    Ok(documents[0])
}

/// Add an import scope, a local scope with a variable and constants, and a state machine.
fn scopes(editor: &mut PdbEditor<'static>) -> Outcome {
    let import_scopes = editor.import_scopes_mut()?;
    let root = import_scopes.push(import_scope(
        ImportScopeId(0),
        vec![Import::AssemblyReferenceAlias {
            alias: Bytes::new(b"Ext"),
        }],
    ))?;
    let type_token = MetadataToken::new(TableId::TYPE_REF, 1).ok_or("bad token")?;
    let imports = import_scopes.push(import_scope(
        root,
        vec![
            Import::Namespace {
                namespace: Bytes::new(b"System"),
            },
            Import::AliasType {
                alias: Bytes::new(b"Con"),
                target_type: type_token,
            },
            Import::AssemblyNamespace {
                assembly: AssemblyRefId(1),
                namespace: Bytes::new(b"System.Linq"),
            },
        ],
    ))?;
    let mut scope = LocalScope::default();
    scope.method = MethodDefId(1);
    scope.import_scope = imports;
    scope.start_offset = IlOffset(0);
    scope.length = 12;
    let mut variable = LocalVariable::default();
    variable.name = Bytes::new(b"count");
    scope.variables.push(variable);
    for (name, value) in [
        (
            &b"Answer"[..],
            ConstantValue::Primitive(PrimitiveConstant::I4(42)),
        ),
        (b"Pi", ConstantValue::Primitive(PrimitiveConstant::R8(3.25))),
        (
            b"Name",
            ConstantValue::Primitive(PrimitiveConstant::String(Some(b"h\0i\0".into()))),
        ),
    ] {
        let mut constant = LocalConstant::default();
        constant.name = name.to_vec().into();
        constant.signature =
            blob(|out| LocalConstantSignature::encode_parts(&[], &value, out))?.into();
        scope.constants.push(constant);
    }
    let scope = editor.local_scopes_mut()?.push(scope)?;
    let variable = LocalVariableHandle { scope, index: 0 };
    let informations = editor.custom_debug_information_mut()?;
    informations.push(information(
        CustomDebugParent::LocalVariable(variable),
        CustomDebugKind::DYNAMIC_LOCAL_VARIABLES,
        blob(|out| DynamicLocalVariables::encode_parts([false, true, true], out))?,
    ))?;
    informations.push(information(
        CustomDebugParent::LocalScope(scope),
        CustomDebugKind::TUPLE_ELEMENT_NAMES,
        blob(|out| TupleElementNames::encode_parts([&b"first"[..], b"", b"second"], out))?,
    ))?;
    let mut machine = StateMachineMethod::default();
    machine.move_next_method = MethodDefId(3);
    machine.kickoff_method = MethodDefId(2);
    editor.state_machine_methods_mut()?.push(machine);
    Ok(())
}

/// Add one value of each method-level custom debug information kind.
fn method_values(editor: &mut PdbEditor<'static>, document: DocumentId) -> Outcome {
    let method = |row| -> Outcome<CustomDebugParent> {
        Ok(CustomDebugParent::Metadata(
            MethodDefId(row).token().ok_or("bad token")?,
        ))
    };
    let module = CustomDebugParent::Metadata(MetadataToken::new(TableId::MODULE, 1).ok_or("bad")?);
    let type_def =
        CustomDebugParent::Metadata(MetadataToken::new(TableId::TYPE_DEF, 1).ok_or("bad")?);
    let mut step = AsyncStep::default();
    step.yield_offset = IlOffset(4);
    step.resume_offset = IlOffset(8);
    step.move_next_method = MethodDefId(3);
    let mut scope = HoistedLocalScope::default();
    scope.length = 12;
    let mut lambda = Lambda::default();
    lambda.syntax_offset = 24;
    lambda.closure = LambdaClosure::Closure(0);
    let mut state = StateMachineState::default();
    state.syntax_offset = 30;
    let option = |name: &'static [u8], value: &'static [u8]| {
        let mut option = CompilationOption::default();
        option.name = pdbrs::Bytes::new(name);
        option.value = pdbrs::Bytes::new(value);
        option
    };
    let mut reference = MetadataReference::default();
    reference.file_name = pdbrs::Bytes::new(b"System.Runtime.dll");
    reference.flags = MetadataReference::ASSEMBLY;
    reference.time_stamp = 0x1234_5678;
    reference.file_size = 0x2000;
    reference.mvid = [0x77; 16];
    let values = [
        (
            method(3)?,
            CustomDebugKind::ASYNC_METHOD_STEPPING_INFORMATION,
            blob(|out| {
                AsyncMethodSteppingInformation::encode_parts(Some(IlOffset(10)), [step], out)
            })?,
        ),
        (
            method(3)?,
            CustomDebugKind::STATE_MACHINE_HOISTED_LOCAL_SCOPES,
            blob(|out| HoistedLocalScopes::encode_parts([scope], out))?,
        ),
        (
            method(1)?,
            CustomDebugKind::ENC_LOCAL_SLOT_MAP,
            blob(|out| {
                EncLocalSlotMap::encode_parts(
                    [
                        LocalSlot::Temporary,
                        LocalSlot::Variable {
                            kind: 0,
                            syntax_offset: 12,
                            ordinal: Some(1),
                        },
                    ],
                    out,
                )
            })?,
        ),
        (
            method(1)?,
            CustomDebugKind::ENC_LAMBDA_AND_CLOSURE_MAP,
            blob(|out| EncLambdaAndClosureMap::encode_parts(0, [20], [lambda], out))?,
        ),
        (
            method(3)?,
            CustomDebugKind::ENC_STATE_MACHINE_STATE_MAP,
            blob(|out| EncStateMachineStateMap::encode_parts([state], out))?,
        ),
        (
            method(2)?,
            CustomDebugKind::PRIMARY_CONSTRUCTOR_INFORMATION,
            Vec::new(),
        ),
        (module, CustomDebugKind::DEFAULT_NAMESPACE, b"Demo".to_vec()),
        (
            module,
            CustomDebugKind::SOURCE_LINK,
            br#"{"documents":{"C:\\src\\*":"https://example.com/*"}}"#.to_vec(),
        ),
        (
            module,
            CustomDebugKind::COMPILATION_OPTIONS,
            blob(|out| {
                CompilationOptions::encode_parts(
                    [
                        option(b"language", b"C#"),
                        option(b"optimization", b"debug"),
                    ],
                    out,
                )
            })?,
        ),
        (
            module,
            CustomDebugKind::COMPILATION_METADATA_REFERENCES,
            blob(|out| CompilationMetadataReferences::encode_parts([reference], out))?,
        ),
        (
            type_def,
            CustomDebugKind::TYPE_DEFINITION_DOCUMENTS,
            blob(|out| TypeDefinitionDocuments::encode_parts([document], out))?,
        ),
    ];
    let informations = editor.custom_debug_information_mut()?;
    for (parent, kind, value) in values {
        informations.push(information(parent, kind, value))?;
    }
    Ok(())
}

/// Build the synthetic Portable PDB and check that it reads back.
pub fn portable_pdb() -> Outcome<Vec<u8>> {
    let mut external = ExternalRowCounts::new();
    for (table, count) in [
        (TableId::MODULE, 1),
        (TableId::TYPE_REF, 2),
        (TableId::TYPE_DEF, 2),
        (TableId::METHOD_DEF, 3),
        (TableId::STAND_ALONE_SIG, 1),
        (TableId::ASSEMBLY_REF, 1),
    ] {
        external.set(table, count);
    }
    let identity = Identity::new([0x5a; 16], 0x1234_5678);
    let mut editor = PdbEditor::new(external, IdentityPolicy::Explicit(identity));
    editor.set_entry_point(Some(MethodDefId(1)));
    let source = b"class Program { static void Main() { } }\n".repeat(8);
    let document = documents_and_methods(&mut editor, &source)?;
    scopes(&mut editor)?;
    method_values(&mut editor, document)?;
    let bytes = editor.plan(EditOptions::default())?.to_vec()?;
    let pdb = Pdb::parse(&bytes)?;
    for row in pdb.custom_debug_informations() {
        row?.1.decode()?;
    }
    Ok(bytes)
}
