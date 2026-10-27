//! Builds with `PdbEditor::new`: handles, row order, reference resolution and width changes.

use pdb_rs::ErrorKind;
use pdb_rs::io::ReadAt;
use pdb_rs::portable::custom::{CustomDebugInformation, CustomDebugKind, CustomDebugParent};
use pdb_rs::portable::debug::{
    ConstantValue, Document, Import, ImportScope, Language, LocalConstant, LocalConstantHandle,
    LocalConstantSignature, LocalScope, LocalSignatureForm, LocalVariable, LocalVariableHandle,
    MethodDebugInformation, PrimitiveConstant, SequencePoint, SequencePoints, SourceSpan,
    StateMachineMethod, ValidateOptions,
};
use pdb_rs::portable::metadata::{
    CodedIndex, CustomDebugInformationRow, DocumentId, IlOffset, ImportScopeId, LocalScopeId,
    MetadataToken, MethodDefId, StandAloneSigId, TableId,
};
use pdb_rs::portable::write::ExternalRowCounts;
use pdb_rs::portable::{EditOptions, IdentityPolicy, Pdb, PdbEditor};

use crate::{location, open};

/// An unknown custom debug information kind.
pub(crate) const UNKNOWN: CustomDebugKind = CustomDebugKind([0x5A; 16]);

/// Return external row counts with `methods` `MethodDef` rows and one `Module` row.
pub(crate) fn external(methods: u32) -> ExternalRowCounts {
    let mut external = ExternalRowCounts::new();
    external.set(TableId::METHOD_DEF, methods);
    external.set(TableId::MODULE, 1);
    external.set(TableId::STAND_ALONE_SIG, 2);
    external
}

/// Plan an editor with the content-derived ID and return the output bytes.
pub(crate) fn write<S: ReadAt>(editor: &PdbEditor<'_, S>) -> Vec<u8> {
    editor
        .plan(EditOptions::default())
        .unwrap()
        .to_vec()
        .unwrap()
}

/// Make a visible sequence point.
pub(crate) fn point(il: u32, document: u32, line: u32) -> SequencePoint {
    let mut span = SourceSpan::default();
    span.start_line = line;
    span.start_column = 5;
    span.end_line = line;
    span.end_column = 20;
    let mut point = SequencePoint::default();
    point.il_offset = IlOffset(il);
    point.document = DocumentId(document);
    point.span = Some(span);
    point
}

/// Make a local scope record without variables and constants.
pub(crate) fn scope(method: u32, start: u32, length: u32) -> LocalScope<'static> {
    let mut scope = LocalScope::default();
    scope.method = MethodDefId(method);
    scope.import_scope = ImportScopeId(0);
    scope.start_offset = IlOffset(start);
    scope.length = length;
    scope
}

/// Make a local variable record.
pub(crate) fn variable(name: &[u8], index: u16) -> LocalVariable<'static> {
    let mut variable = LocalVariable::default();
    variable.index = index;
    variable.name = name.to_vec().into();
    variable
}

/// Make a local constant record with an `I4` value.
pub(crate) fn constant(name: &[u8], value: i32) -> LocalConstant<'static> {
    let mut signature = Vec::new();
    LocalConstantSignature::encode_parts(
        &[],
        &ConstantValue::Primitive(PrimitiveConstant::I4(value)),
        &mut signature,
    )
    .unwrap();
    let mut constant = LocalConstant::default();
    constant.name = name.to_vec().into();
    constant.signature = signature.into();
    constant
}

/// Make a custom debug information record with `value`.
pub(crate) fn information(
    parent: CustomDebugParent,
    kind: CustomDebugKind,
    value: Vec<u8>,
) -> CustomDebugInformation<'static> {
    let mut information = CustomDebugInformation::new(parent, kind);
    information.value = value.into();
    information
}

/// Make a document record with `name` and `language`.
pub(crate) fn document(name: &[u8], language: Option<[u8; 16]>) -> Document<'static> {
    let mut document = Document::default();
    document.name = name.to_vec().into();
    document.language = language;
    document
}

/// Make an import scope record.
fn import_scope(parent: ImportScopeId, imports: Vec<Import<'static>>) -> ImportScope<'static> {
    let mut scope = ImportScope::default();
    scope.parent = parent;
    scope.imports = imports;
    scope
}

/// Encode the sequence points of a method with the local signature row `signature`.
pub(crate) fn method(signature: u32, points: &[SequencePoint]) -> MethodDebugInformation<'static> {
    SequencePoints::encode_parts(StandAloneSigId(signature), LocalSignatureForm::Row, points)
        .unwrap()
}

/// Make an editor without a source with two documents, two methods with sequence points,
/// unsorted local scopes with variables and constants, two import scopes, a state machine
/// and custom debug information of each parent kind.
pub(crate) fn sample() -> PdbEditor<'static> {
    let mut editor = PdbEditor::new(external(4), IdentityPolicy::ContentDerived);
    editor.set_entry_point(Some(MethodDefId(1)));
    let documents = editor.documents_mut().unwrap();
    let first = documents
        .push(document(
            b"C:\\src\\Program.cs",
            Some(Language::CSharp.guid()),
        ))
        .unwrap();
    let mut other = document(b"/src/Other.cs", None);
    other.hash_algorithm = Some([3; 16]);
    other.hash = vec![0xAB; 32].into();
    let second = documents.push(other).unwrap();
    assert_eq!((first.0, second.0), (1, 2));
    let methods = editor.methods_mut().unwrap();
    methods.insert(
        MethodDefId(1),
        method(1, &[point(0, 1, 10), point(4, 2, 3), point(9, 1, 12)]),
    );
    methods.insert(MethodDefId(3), method(0, &[point(0, 2, 20)]));
    let import_scopes = editor.import_scopes_mut().unwrap();
    let imports = import_scopes
        .push(import_scope(
            ImportScopeId(0),
            vec![Import::Namespace {
                namespace: b"System".into(),
            }],
        ))
        .unwrap();
    import_scopes
        .push(import_scope(
            imports,
            vec![Import::AliasNamespace {
                alias: b"IO".into(),
                namespace: b"System.IO".into(),
            }],
        ))
        .unwrap();
    // Added out of order: method 3 first, then the inner scope of method 1, then its outer
    // scope.
    let scopes = editor.local_scopes_mut().unwrap();
    let mut late = scope(3, 0, 8);
    late.variables.push(variable(b"late", 0));
    scopes.push(late).unwrap();
    let mut inner = scope(1, 2, 4);
    inner.variables.push(variable(b"inner", 1));
    inner.constants.push(constant(b"Pi", 3));
    let inner = scopes.push(inner).unwrap();
    let mut outer = scope(1, 0, 12);
    outer.variables.push(variable(b"outer", 0));
    scopes.push(outer).unwrap();
    let mut machine = StateMachineMethod::default();
    machine.move_next_method = MethodDefId(4);
    machine.kickoff_method = MethodDefId(2);
    editor.state_machine_methods_mut().unwrap().push(machine);
    let parents = [
        CustomDebugParent::LocalVariable(LocalVariableHandle {
            scope: inner,
            index: 0,
        }),
        CustomDebugParent::LocalConstant(LocalConstantHandle {
            scope: inner,
            index: 0,
        }),
        CustomDebugParent::LocalScope(inner),
        CustomDebugParent::Document(second),
        CustomDebugParent::ImportScope(imports),
        CustomDebugParent::Metadata(MetadataToken::new(TableId::MODULE, 1).unwrap()),
        CustomDebugParent::Metadata(MetadataToken::new(TableId::METHOD_DEF, 2).unwrap()),
    ];
    let custom = editor.custom_debug_information_mut().unwrap();
    for (value, parent) in (1_u8..).zip(parents) {
        custom
            .push(information(parent, UNKNOWN, vec![value]))
            .unwrap();
    }
    editor
}

/// Return the names of the variables of a scope.
fn variable_names(scope: &LocalScope<'_>) -> Vec<Vec<u8>> {
    scope
        .variables
        .iter()
        .map(|variable| variable.name.to_vec())
        .collect()
}

#[test]
fn builder_sorts_rows_and_resolves_handles() {
    let bytes = write(&sample());
    let pdb = open(&bytes);
    pdb.validate_debug(ValidateOptions::default()).unwrap();
    assert_eq!(pdb.pdb_stream().entry_point(), Some(MethodDefId(1)));
    let names: Vec<Vec<u8>> = pdb
        .documents()
        .map(|document| document.unwrap().1.name.into_vec())
        .collect();
    assert_eq!(
        names,
        [b"C:\\src\\Program.cs".to_vec(), b"/src/Other.cs".to_vec()]
    );

    // Method 1 changes documents, method 3 has one document, methods 2 and 4 have none.
    let methods: Vec<_> = pdb
        .method_debug_informations()
        .map(|method| method.unwrap().1)
        .collect();
    assert_eq!(methods.len(), 4);
    assert_eq!(methods[0].document.0, 0);
    assert_eq!(methods[2].document.0, 2);
    assert!(methods[1].sequence_points.is_empty());
    let points: Vec<SequencePoint> = methods[0]
        .decode()
        .unwrap()
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(points, [point(0, 1, 10), point(4, 2, 3), point(9, 1, 12)]);

    // Scopes in method, start and length order, each with its own run.
    let scopes: Vec<_> = pdb.local_scopes().map(|scope| scope.unwrap().1).collect();
    let keys: Vec<(u32, u32, u32)> = scopes
        .iter()
        .map(|scope| (scope.method.0, scope.start_offset.0, scope.length))
        .collect();
    assert_eq!(keys, [(1, 0, 12), (1, 2, 4), (3, 0, 8)]);
    assert_eq!(variable_names(&scopes[0]), [b"outer".to_vec()]);
    assert_eq!(variable_names(&scopes[1]), [b"inner".to_vec()]);
    assert_eq!(variable_names(&scopes[2]), [b"late".to_vec()]);
    assert_eq!(scopes[1].constants.len(), 1);
    assert_eq!(scopes[1].constants[0].name, b"Pi");

    let imports = pdb.import_scope(ImportScopeId(2)).unwrap();
    assert_eq!(imports.parent.0, 1);
    let [Import::AliasNamespace { alias, namespace }] = &imports.imports[..] else {
        panic!("unexpected imports {:?}", imports.imports);
    };
    assert_eq!(alias, b"IO");
    assert_eq!(namespace, b"System.IO");

    // Custom debug information sorted by the coded parent, with resolved rows.
    let rows: Vec<(MetadataToken, Vec<u8>)> = pdb
        .rows::<CustomDebugInformationRow>()
        .map(|row| {
            let row = row.unwrap().1;
            (row.parent, pdb.blob(row.value).unwrap())
        })
        .collect();
    let coded: Vec<u32> = rows
        .iter()
        .map(|(parent, _)| {
            CodedIndex::HasCustomDebugInformation
                .encode(*parent)
                .unwrap()
        })
        .collect();
    assert!(coded.is_sorted());
    let parent = |value: u8| rows.iter().find(|(_, bytes)| bytes == &[value]).unwrap().0;
    assert_eq!(
        parent(1),
        MetadataToken::new(TableId::LOCAL_VARIABLE, 2).unwrap()
    );
    assert_eq!(
        parent(2),
        MetadataToken::new(TableId::LOCAL_CONSTANT, 1).unwrap()
    );
    assert_eq!(parent(3), LocalScopeId(2).token().unwrap());
    assert_eq!(parent(4), MetadataToken::new(TableId::DOCUMENT, 2).unwrap());
    assert_eq!(
        parent(5),
        MetadataToken::new(TableId::IMPORT_SCOPE, 1).unwrap()
    );
}

#[test]
fn identical_builders_give_identical_bytes() {
    assert_eq!(write(&sample()), write(&sample()));
}

#[test]
fn builder_widens_the_coded_parent_for_many_documents() {
    for (count, width) in [(0x7FF_u32, 2), (0x800, 4)] {
        let mut editor = PdbEditor::new(external(1), IdentityPolicy::ContentDerived);
        let documents = editor.documents_mut().unwrap();
        let mut last = DocumentId::default();
        for index in 0..count {
            last = documents
                .push(document(format!("/src/{index}.cs").as_bytes(), None))
                .unwrap();
        }
        editor
            .custom_debug_information_mut()
            .unwrap()
            .push(information(
                CustomDebugParent::Document(last),
                UNKNOWN,
                b"last".to_vec(),
            ))
            .unwrap();
        let bytes = write(&editor);
        let pdb = Pdb::parse(&bytes).unwrap();
        let tables = pdb.tables();
        assert_eq!(
            tables.coded_index_width(CodedIndex::HasCustomDebugInformation),
            width
        );
        let (_, information) = pdb.custom_debug_informations().next().unwrap().unwrap();
        assert_eq!(information.parent, CustomDebugParent::Document(last));
        assert_eq!(last.0, count);
        let name = pdb.document(last).unwrap();
        assert_eq!(name.name, format!("/src/{}.cs", count - 1).as_bytes());
    }
}

#[test]
fn builder_rejects_unknown_handles_before_output() {
    let mut editor = sample();
    editor
        .methods_mut()
        .unwrap()
        .insert(MethodDefId(2), method(0, &[point(0, 3, 1)]));
    let error = editor.plan(EditOptions::default()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
    assert_eq!(
        location(&error).table(),
        Some(TableId::METHOD_DEBUG_INFORMATION)
    );
    assert_eq!(location(&error).row(), Some(2));

    let mut editor = sample();
    editor
        .methods_mut()
        .unwrap()
        .insert(MethodDefId(5), method(0, &[point(0, 1, 1)]));
    let error = editor.plan(EditOptions::default()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);

    let mut editor = sample();
    editor
        .custom_debug_information_mut()
        .unwrap()
        .push(CustomDebugInformation::new(
            CustomDebugParent::Document(DocumentId(3)),
            UNKNOWN,
        ))
        .unwrap();
    let error = editor.plan(EditOptions::default()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
    assert_eq!(
        location(&error).table(),
        Some(TableId::CUSTOM_DEBUG_INFORMATION)
    );
}
