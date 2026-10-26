//! The semantic builder: handles, row order, reference resolution and width changes.

use pdbrs::ErrorKind;
use pdbrs::portable::build::{
    CustomDebugInfo, CustomDebugParent, DocumentInfo, ImportInfo, ImportScopeInfo,
    LocalConstantInfo, LocalScopeInfo, LocalVariableInfo, MethodInfo,
};
use pdbrs::portable::custom::CustomDebugKind;
use pdbrs::portable::debug::{
    ConstantValue, Import, Language, LocalConstantSignature, LocalSignatureForm, PrimitiveConstant,
    SequencePoint, SourceSpan, ValidateOptions,
};
use pdbrs::portable::metadata::{
    CodedIndex, DocumentId, IlOffset, ImportScopeId, LocalScopeId, MetadataToken, MethodDefId,
    StandAloneSigId, TableId,
};
use pdbrs::portable::write::ExternalRowCounts;
use pdbrs::portable::{IdentityPolicy, Pdb, PdbBuilder};

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

/// Make a local scope record.
pub(crate) fn scope(method: u32, start: u32, length: u32) -> LocalScopeInfo {
    let mut scope = LocalScopeInfo::default();
    scope.method = MethodDefId(method);
    scope.import_scope = ImportScopeId(0);
    scope.start_offset = IlOffset(start);
    scope.length = length;
    scope
}

/// Make a local constant record with an `I4` value.
pub(crate) fn constant(name: &[u8], value: i32) -> LocalConstantInfo {
    let mut constant = LocalConstantInfo::default();
    constant.name = name.to_vec();
    LocalConstantSignature::encode_parts(
        &[],
        &ConstantValue::Primitive(PrimitiveConstant::I4(value)),
        &mut constant.signature,
    )
    .unwrap();
    constant
}

/// Make a custom debug information record with `value`.
pub(crate) fn information(
    parent: CustomDebugParent,
    kind: CustomDebugKind,
    value: Vec<u8>,
) -> CustomDebugInfo {
    let mut information = CustomDebugInfo::new(parent, kind);
    information.value = value;
    information
}

/// Make a document record with `name` and `language`.
fn document(name: &[u8], language: Option<[u8; 16]>) -> DocumentInfo {
    let mut document = DocumentInfo::default();
    document.name = name.to_vec();
    document.language = language;
    document
}

/// Make an import scope record.
fn import_scope(parent: ImportScopeId, imports: Vec<ImportInfo>) -> ImportScopeInfo {
    let mut scope = ImportScopeInfo::default();
    scope.parent = parent;
    scope.imports = imports;
    scope
}

/// Make the sequence points of a method.
fn method(sequence_points: Vec<SequencePoint>) -> MethodInfo {
    let mut method = MethodInfo::default();
    method.sequence_points = sequence_points;
    method
}

/// Make a builder with two documents, three methods with sequence points, unsorted local
/// scopes with variables and constants, two import scopes, a state machine and custom debug
/// information of each parent kind.
pub(crate) fn sample() -> PdbBuilder {
    let mut builder = PdbBuilder::new(external(4));
    builder.set_entry_point(Some(MethodDefId(1)));
    let first = builder
        .add_document(document(
            b"C:\\src\\Program.cs",
            Some(Language::CSharp.guid()),
        ))
        .unwrap();
    let mut other = document(b"/src/Other.cs", None);
    other.hash_algorithm = Some([3; 16]);
    other.hash = vec![0xAB; 32];
    let second = builder.add_document(other).unwrap();
    assert_eq!((first.0, second.0), (1, 2));
    let mut first_method = method(vec![point(0, 1, 10), point(4, 2, 3), point(9, 1, 12)]);
    first_method.local_signature = StandAloneSigId(1);
    first_method.local_signature_form = LocalSignatureForm::Row;
    builder.set_method(MethodDefId(1), first_method);
    builder.set_method(MethodDefId(3), method(vec![point(0, 2, 20)]));
    let imports = builder
        .add_import_scope(import_scope(
            ImportScopeId(0),
            vec![ImportInfo::Namespace {
                namespace: b"System".to_vec(),
            }],
        ))
        .unwrap();
    builder
        .add_import_scope(import_scope(
            imports,
            vec![ImportInfo::AliasNamespace {
                alias: b"IO".to_vec(),
                namespace: b"System.IO".to_vec(),
            }],
        ))
        .unwrap();
    // Added out of order: method 3 first, then the inner scope of method 1, then its outer
    // scope.
    let late = builder.add_local_scope(scope(3, 0, 8)).unwrap();
    let inner = builder.add_local_scope(scope(1, 2, 4)).unwrap();
    let outer = builder.add_local_scope(scope(1, 0, 12)).unwrap();
    let variable = |name: &[u8], index| {
        let mut variable = LocalVariableInfo::default();
        variable.index = index;
        variable.name = name.to_vec();
        variable
    };
    builder
        .add_local_variable(late, variable(b"late", 0))
        .unwrap();
    let inner_variable = builder
        .add_local_variable(inner, variable(b"inner", 1))
        .unwrap();
    builder
        .add_local_variable(outer, variable(b"outer", 0))
        .unwrap();
    let pi = builder
        .add_local_constant(inner, constant(b"Pi", 3))
        .unwrap();
    builder
        .add_state_machine_method(MethodDefId(4), MethodDefId(2))
        .unwrap();
    let parents = [
        CustomDebugParent::LocalVariable(inner_variable),
        CustomDebugParent::LocalConstant(pi),
        CustomDebugParent::LocalScope(inner),
        CustomDebugParent::Document(second),
        CustomDebugParent::ImportScope(imports),
        CustomDebugParent::Metadata(MetadataToken::new(TableId::MODULE, 1).unwrap()),
        CustomDebugParent::Metadata(MetadataToken::new(TableId::METHOD_DEF, 2).unwrap()),
    ];
    for (value, parent) in (1_u8..).zip(parents) {
        builder
            .add_custom_debug_information(information(parent, UNKNOWN, vec![value]))
            .unwrap();
    }
    builder
}

#[test]
fn builder_sorts_rows_and_resolves_handles() {
    let bytes = sample()
        .plan(IdentityPolicy::ContentDerived)
        .unwrap()
        .to_vec()
        .unwrap();
    let pdb = open(&bytes);
    pdb.validate_debug(ValidateOptions::default()).unwrap();
    assert_eq!(pdb.pdb_stream().entry_point(), Some(MethodDefId(1)));
    let names: Vec<Vec<u8>> = pdb
        .documents()
        .map(|document| document.unwrap().name().to_vec())
        .collect();
    assert_eq!(
        names,
        [b"C:\\src\\Program.cs".to_vec(), b"/src/Other.cs".to_vec()]
    );

    // Method 1 changes documents, method 3 has one document, methods 2 and 4 have none.
    let methods: Vec<_> = pdb
        .method_debug_informations()
        .map(Result::unwrap)
        .collect();
    assert_eq!(methods.len(), 4);
    assert_eq!(methods[0].document().0, 0);
    assert_eq!(methods[2].document().0, 2);
    assert_eq!(methods[1].sequence_points_blob(), []);
    let points: Vec<SequencePoint> = methods[0]
        .sequence_points()
        .unwrap()
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(points, [point(0, 1, 10), point(4, 2, 3), point(9, 1, 12)]);

    // Scopes in method, start and length order, each with its own run.
    let scopes: Vec<_> = pdb.local_scopes().map(Result::unwrap).collect();
    let keys: Vec<(u32, u32, u32)> = scopes
        .iter()
        .map(|scope| (scope.method().0, scope.start_offset().0, scope.length()))
        .collect();
    assert_eq!(keys, [(1, 0, 12), (1, 2, 4), (3, 0, 8)]);
    let variable = |scope: &pdbrs::portable::debug::LocalScope| -> Vec<Vec<u8>> {
        scope
            .variables()
            .map(|id| pdb.local_variable(id).unwrap().name().to_vec())
            .collect()
    };
    assert_eq!(variable(&scopes[0]), [b"outer".to_vec()]);
    assert_eq!(variable(&scopes[1]), [b"inner".to_vec()]);
    assert_eq!(variable(&scopes[2]), [b"late".to_vec()]);
    let constants: Vec<_> = scopes[1].constants().collect();
    assert_eq!(constants.len(), 1);
    assert_eq!(pdb.local_constant(constants[0]).unwrap().name(), b"Pi");

    let imports = pdb.import_scope(ImportScopeId(2)).unwrap();
    assert_eq!(imports.parent().0, 1);
    let imports: Vec<Import> = imports.imports().map(Result::unwrap).collect();
    let [Import::AliasNamespace { alias, namespace }] = imports[..] else {
        panic!("unexpected imports {imports:?}");
    };
    assert_eq!(pdb.blob(alias).unwrap(), b"IO");
    assert_eq!(pdb.blob(namespace).unwrap(), b"System.IO");

    // Custom debug information sorted by the coded parent, with resolved rows.
    let rows: Vec<(MetadataToken, Vec<u8>)> = pdb
        .custom_debug_informations()
        .map(|row| {
            let row = row.unwrap();
            (row.parent(), row.value().to_vec())
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
    let first = sample()
        .plan(IdentityPolicy::ContentDerived)
        .unwrap()
        .to_vec()
        .unwrap();
    let second = sample()
        .plan(IdentityPolicy::ContentDerived)
        .unwrap()
        .to_vec()
        .unwrap();
    assert_eq!(first, second);
}

#[test]
fn builder_widens_the_coded_parent_for_many_documents() {
    for (count, width) in [(0x7FF_u32, 2), (0x800, 4)] {
        let mut builder = PdbBuilder::new(external(1));
        let mut last = DocumentId::default();
        for index in 0..count {
            last = builder
                .add_document(document(format!("/src/{index}.cs").as_bytes(), None))
                .unwrap();
        }
        builder
            .add_custom_debug_information(information(
                CustomDebugParent::Document(last),
                UNKNOWN,
                b"last".to_vec(),
            ))
            .unwrap();
        let bytes = builder
            .plan(IdentityPolicy::ContentDerived)
            .unwrap()
            .to_vec()
            .unwrap();
        let pdb = Pdb::parse(&bytes).unwrap();
        let tables = pdb.tables();
        assert_eq!(
            tables.coded_index_width(CodedIndex::HasCustomDebugInformation),
            width
        );
        let row = pdb.custom_debug_informations().next().unwrap().unwrap();
        assert_eq!(row.parent().row(), count);
        let name = pdb.document(last).unwrap();
        assert_eq!(name.name(), format!("/src/{}.cs", count - 1).as_bytes());
    }
}

#[test]
fn builder_rejects_unknown_handles_before_output() {
    let mut builder = sample();
    builder.set_method(MethodDefId(2), method(vec![point(0, 3, 1)]));
    let error = builder.plan(IdentityPolicy::ContentDerived).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
    assert_eq!(
        location(&error).table(),
        Some(TableId::METHOD_DEBUG_INFORMATION)
    );
    assert_eq!(location(&error).row(), Some(2));

    let mut builder = sample();
    builder.set_method(MethodDefId(5), MethodInfo::default());
    let error = builder.plan(IdentityPolicy::ContentDerived).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);

    let mut builder = sample();
    builder
        .add_custom_debug_information(CustomDebugInfo::new(
            CustomDebugParent::Document(DocumentId(3)),
            UNKNOWN,
        ))
        .unwrap();
    let error = builder.plan(IdentityPolicy::ContentDerived).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
    assert_eq!(
        location(&error).table(),
        Some(TableId::CUSTOM_DEBUG_INFORMATION)
    );
}
