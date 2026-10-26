//! A synthetic Portable PDB with rows in each debug table and each custom debug information
//! kind.

use pdbrs::portable::build::{
    CustomDebugInfo, CustomDebugParent, DocumentInfo, ImportInfo, ImportScopeInfo,
    LocalConstantInfo, LocalScopeInfo, LocalVariableInfo, MethodInfo,
};
use pdbrs::portable::custom::{
    AsyncMethodSteppingInformation, AsyncStep, CompilationMetadataReferences, CompilationOption,
    CompilationOptions, CustomDebugKind, DynamicLocalVariables, EncLambdaAndClosureMap,
    EncLocalSlotMap, EncStateMachineStateMap, HoistedLocalScope, HoistedLocalScopes, Lambda,
    LambdaClosure, LocalSlot, MetadataReference, StateMachineState, TupleElementNames,
    TypeDefinitionDocuments, Utf8Bytes,
};
use pdbrs::portable::debug::{
    ConstantValue, HashAlgorithm, Language, LocalConstantSignature, LocalSignatureForm,
    PrimitiveConstant, SequencePoint, SourceSpan,
};
use pdbrs::portable::embedded::{EmbeddedFormat, encode};
use pdbrs::portable::metadata::{
    AssemblyRefId, DocumentId, IlOffset, ImportScopeId, MetadataToken, MethodDefId,
    StandAloneSigId, TableId,
};
use pdbrs::portable::write::ExternalRowCounts;
use pdbrs::portable::{Identity, IdentityPolicy, Pdb, PdbBuilder};

use crate::Outcome;

/// Make a custom debug information record with `value`.
fn information(
    parent: CustomDebugParent,
    kind: CustomDebugKind,
    value: Vec<u8>,
) -> CustomDebugInfo {
    let mut information = CustomDebugInfo::new(parent, kind);
    information.value = value;
    information
}

/// Make the sequence points of a method with a local signature row.
fn method(local_signature: u32, sequence_points: Vec<SequencePoint>) -> MethodInfo {
    let mut method = MethodInfo::default();
    method.local_signature = StandAloneSigId::new(local_signature);
    method.local_signature_form = LocalSignatureForm::Row;
    method.sequence_points = sequence_points;
    method
}

/// Make an import scope.
fn import_scope(parent: ImportScopeId, imports: Vec<ImportInfo>) -> ImportScopeInfo {
    let mut scope = ImportScopeInfo::default();
    scope.parent = parent;
    scope.imports = imports;
    scope
}

/// A sequence point with a span on one line.
fn point(offset: u32, document: DocumentId, line: u32) -> SequencePoint {
    SequencePoint {
        il_offset: IlOffset::new(offset),
        document,
        span: Some(SourceSpan {
            start_line: line,
            start_column: 5,
            end_line: line,
            end_column: 20,
        }),
    }
}

/// Add the documents and the methods with their sequence points.
fn documents_and_methods(builder: &mut PdbBuilder, source: &[u8]) -> Outcome<DocumentId> {
    let mut documents = Vec::new();
    for name in [&b"C:\\src\\Program.cs"[..], b"/src/Helper.cs"] {
        let mut document = DocumentInfo::default();
        document.name = name.to_vec();
        document.hash_algorithm = Some(HashAlgorithm::Sha256.guid());
        document.hash = vec![0x33; 32];
        document.language = Some(Language::CSharp.guid());
        documents.push(builder.add_document(document)?);
    }
    let hidden = SequencePoint {
        il_offset: IlOffset::new(6),
        document: documents[0],
        span: None,
    };
    builder.set_method(
        MethodDefId::new(1),
        method(
            1,
            vec![point(0, documents[0], 3), hidden, point(8, documents[0], 4)],
        ),
    );
    builder.set_method(
        MethodDefId::new(2),
        method(
            0,
            vec![point(0, documents[0], 10), point(4, documents[1], 2)],
        ),
    );
    builder.add_custom_debug_information(information(
        CustomDebugParent::Document(documents[0]),
        CustomDebugKind::EMBEDDED_SOURCE,
        encode(source, EmbeddedFormat::Deflate)?,
    ))?;
    builder.add_custom_debug_information(information(
        CustomDebugParent::Document(documents[1]),
        CustomDebugKind::EMBEDDED_SOURCE,
        encode(b"class Helper { }\n", EmbeddedFormat::Raw)?,
    ))?;
    Ok(documents[0])
}

/// Add an import scope, a local scope with a variable and constants, and a state machine.
fn scopes(builder: &mut PdbBuilder) -> Outcome {
    let root = builder.add_import_scope(import_scope(
        ImportScopeId::new(0),
        vec![ImportInfo::AssemblyReferenceAlias {
            alias: b"Ext".to_vec(),
        }],
    ))?;
    let type_token = MetadataToken::new(TableId::TYPE_REF, 1).ok_or("bad token")?;
    let imports = builder.add_import_scope(import_scope(
        root,
        vec![
            ImportInfo::Namespace {
                namespace: b"System".to_vec(),
            },
            ImportInfo::AliasType {
                alias: b"Con".to_vec(),
                target_type: type_token,
            },
            ImportInfo::AssemblyNamespace {
                assembly: AssemblyRefId::new(1),
                namespace: b"System.Linq".to_vec(),
            },
        ],
    ))?;
    let mut scope = LocalScopeInfo::default();
    scope.method = MethodDefId::new(1);
    scope.import_scope = imports;
    scope.start_offset = IlOffset::new(0);
    scope.length = 12;
    let scope = builder.add_local_scope(scope)?;
    let mut variable = LocalVariableInfo::default();
    variable.name = b"count".to_vec();
    let variable = builder.add_local_variable(scope, variable)?;
    for (name, value) in [
        (
            &b"Answer"[..],
            ConstantValue::Primitive(PrimitiveConstant::I4(42)),
        ),
        (b"Pi", ConstantValue::Primitive(PrimitiveConstant::R8(3.25))),
        (
            b"Name",
            ConstantValue::Primitive(PrimitiveConstant::String(Some(b"h\0i\0"))),
        ),
    ] {
        let mut constant = LocalConstantInfo::default();
        constant.name = name.to_vec();
        constant.signature = LocalConstantSignature::encode(&[], &value)?;
        builder.add_local_constant(scope, constant)?;
    }
    builder.add_custom_debug_information(information(
        CustomDebugParent::LocalVariable(variable),
        CustomDebugKind::DYNAMIC_LOCAL_VARIABLES,
        DynamicLocalVariables::encode([false, true, true])?,
    ))?;
    builder.add_custom_debug_information(information(
        CustomDebugParent::LocalScope(scope),
        CustomDebugKind::TUPLE_ELEMENT_NAMES,
        TupleElementNames::encode([&b"first"[..], b"", b"second"])?,
    ))?;
    builder.add_state_machine_method(MethodDefId::new(3), MethodDefId::new(2))?;
    Ok(())
}

/// Add one value of each method-level custom debug information kind.
fn method_values(builder: &mut PdbBuilder, document: DocumentId) -> Outcome {
    let method = |row| -> Outcome<CustomDebugParent> {
        Ok(CustomDebugParent::Metadata(
            MethodDefId::new(row).token().ok_or("bad token")?,
        ))
    };
    let module = CustomDebugParent::Metadata(MetadataToken::new(TableId::MODULE, 1).ok_or("bad")?);
    let type_def =
        CustomDebugParent::Metadata(MetadataToken::new(TableId::TYPE_DEF, 1).ok_or("bad")?);
    let step = AsyncStep {
        yield_offset: IlOffset::new(4),
        resume_offset: IlOffset::new(8),
        move_next_method: MethodDefId::new(3),
    };
    let values = [
        (
            method(3)?,
            CustomDebugKind::ASYNC_METHOD_STEPPING_INFORMATION,
            AsyncMethodSteppingInformation::encode(Some(IlOffset::new(10)), [step])?,
        ),
        (
            method(3)?,
            CustomDebugKind::STATE_MACHINE_HOISTED_LOCAL_SCOPES,
            HoistedLocalScopes::encode([HoistedLocalScope {
                start: IlOffset::new(0),
                length: 12,
            }])?,
        ),
        (
            method(1)?,
            CustomDebugKind::ENC_LOCAL_SLOT_MAP,
            EncLocalSlotMap::encode([
                LocalSlot::Temporary,
                LocalSlot::Variable {
                    kind: 0,
                    syntax_offset: 12,
                    ordinal: Some(1),
                },
            ])?,
        ),
        (
            method(1)?,
            CustomDebugKind::ENC_LAMBDA_AND_CLOSURE_MAP,
            EncLambdaAndClosureMap::encode(
                0,
                [20],
                [Lambda {
                    syntax_offset: 24,
                    closure: LambdaClosure::Closure(0),
                }],
            )?,
        ),
        (
            method(3)?,
            CustomDebugKind::ENC_STATE_MACHINE_STATE_MAP,
            EncStateMachineStateMap::encode([StateMachineState {
                state_number: 0,
                syntax_offset: 30,
            }])?,
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
            CompilationOptions::encode([
                CompilationOption {
                    name: Utf8Bytes::new(b"language"),
                    value: Utf8Bytes::new(b"C#"),
                },
                CompilationOption {
                    name: Utf8Bytes::new(b"optimization"),
                    value: Utf8Bytes::new(b"debug"),
                },
            ])?,
        ),
        (
            module,
            CustomDebugKind::COMPILATION_METADATA_REFERENCES,
            CompilationMetadataReferences::encode([MetadataReference {
                file_name: Utf8Bytes::new(b"System.Runtime.dll"),
                aliases: Utf8Bytes::new(b""),
                flags: MetadataReference::ASSEMBLY,
                time_stamp: 0x1234_5678,
                file_size: 0x2000,
                mvid: [0x77; 16],
            }])?,
        ),
        (
            type_def,
            CustomDebugKind::TYPE_DEFINITION_DOCUMENTS,
            TypeDefinitionDocuments::encode([document])?,
        ),
    ];
    for (parent, kind, value) in values {
        builder.add_custom_debug_information(information(parent, kind, value))?;
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
    let mut builder = PdbBuilder::new(external);
    builder.set_entry_point(Some(MethodDefId::new(1)));
    let source = b"class Program { static void Main() { } }\n".repeat(8);
    let document = documents_and_methods(&mut builder, &source)?;
    scopes(&mut builder)?;
    method_values(&mut builder, document)?;
    let identity = Identity::new([0x5a; 16], 0x1234_5678);
    let bytes = builder.plan(IdentityPolicy::Explicit(identity))?.to_vec()?;
    let pdb = Pdb::parse(&bytes)?;
    for row in pdb.custom_debug_informations() {
        row?.decode()?;
    }
    Ok(bytes)
}
