//! Tests of a new `native::PdbEditor`: the synthetic program, the rebuild
//! of a built PDB, determinism, reference checks and width checks.

#[path = "../examples/native_synthetic/program.rs"]
mod program;

use pdb_rs::native::address::SectionOffset;
use pdb_rs::native::codeview::id_record::{FunctionId, IdRecord, StringId};
use pdb_rs::native::codeview::index::{IdIndex, ModuleIndex, TypeIndex};
use pdb_rs::native::codeview::symbol::{Data, S_GDATA32, Symbol, SymbolData};
use pdb_rs::native::codeview::type_record::{ArgumentList, TypeLeaf};
use pdb_rs::native::dbi::{Module, SectionContribution};
use pdb_rs::native::lines::{LineSubsection, ModuleLines};
use pdb_rs::native::msf::MsfWriteOptions;
use pdb_rs::native::{EditOptions, Identity, IdentityPolicy, IndexSpace, Pdb, PdbEditor};
use pdb_rs::{Bytes, ErrorKind};

/// Build the synthetic program and return the PDB bytes.
fn synthetic() -> Vec<u8> {
    program::build()
        .and_then(|editor| editor.plan(EditOptions::default())?.to_vec())
        .expect("the synthetic program builds")
}

/// Return a data record `g` of `type_index` at the start of section 1.
fn data_g(type_index: TypeIndex) -> Data<'static> {
    let mut data = Data::default();
    data.type_index = type_index;
    data.address = SectionOffset {
        section: 1,
        offset: 0,
    };
    data.name = Bytes::new(b"g");
    data
}

/// Return a module record with a name and no section contribution.
fn module_record(name: &[u8]) -> Module<'static> {
    let mut contribution = SectionContribution::default();
    contribution.section = 0xffff;
    contribution.padding1 = 0;
    contribution.offset = -1;
    contribution.size = -1;
    contribution.characteristics = 0;
    contribution.module = ModuleIndex(0xffff);
    contribution.padding2 = 0;
    contribution.data_crc = Some(0);
    contribution.reloc_crc = Some(0);
    contribution.coff_section = None;
    let mut module = Module::default();
    module.unused1 = 0;
    module.section_contribution = contribution;
    module.flags = 0;
    module.stream = None;
    module.symbol_bytes = 0;
    module.c11_bytes = 0;
    module.c13_bytes = 0;
    module.source_file_count = 0;
    module.padding = 0;
    module.file_name_offset = 0;
    module.source_file_name_index = 0;
    module.pdb_file_path_index = 0;
    module.name = name.to_vec().into();
    module.object_file = name.to_vec().into();
    module.name_padding = [0; 3];
    module
}

#[test]
fn synthetic_program_reads_back() {
    let bytes = synthetic();
    let pdb = Pdb::parse(&bytes).expect("the output is a native PDB");
    assert_eq!(
        pdb.info().identity(),
        Identity::Modern {
            guid: program::GUID,
            age: 1
        }
    );
    let types = pdb.types().expect("TPI").expect("TPI present");
    assert_eq!(types.header().record_count(), 8);
    let ids = pdb.ids().expect("IPI").expect("IPI present");
    assert_eq!(ids.header().record_count(), 2);
    let dbi = pdb.debug_info().expect("DBI").expect("DBI present");
    assert_eq!(dbi.header().age, 1);
    let (index, _) = dbi.modules().next().expect("one module").expect("module");
    let stream = pdb
        .module(index)
        .expect("module stream")
        .expect("module stream present");
    let names: Vec<Vec<u8>> = stream
        .symbols()
        .filter_map(|record| {
            match Symbol::decode(record.expect("record"))
                .expect("symbol")
                .data()
            {
                SymbolData::Procedure(procedure) => Some(procedure.name.to_vec()),
                _ => None,
            }
        })
        .collect();
    assert_eq!(names, [b"sum".to_vec(), b"main".to_vec()]);
    let lines = ModuleLines::parse(&stream).expect("C13 subsections");
    lines.check().expect("file references resolve");
    let blocks = lines
        .subsections()
        .iter()
        .filter(|subsection| matches!(subsection.data, LineSubsection::Lines(_)))
        .count();
    assert_eq!(blocks, 2);
    let globals = pdb.globals().expect("GSI").expect("GSI present");
    assert_eq!(globals.index().hash_table().records().count(), 4);
    let publics = pdb.publics().expect("PSI").expect("PSI present");
    assert_eq!(publics.index().hash_table().lookup(b"main").count(), 1);
}

/// Decode every stream of `bytes` and plan the PDB again with `options` and
/// the identity policy `policy`. Return the output bytes.
fn rebuild(bytes: &[u8], policy: IdentityPolicy, options: EditOptions) -> Vec<u8> {
    let pdb = Pdb::parse(bytes).expect("parse");
    let mut editor = pdb.edit(policy);
    editor.decode_all().expect("decode");
    editor
        .plan(options)
        .and_then(|plan| plan.to_vec())
        .expect("rebuild")
}

/// Return the MSF page size of the PDB `bytes`.
fn page_size(bytes: &[u8]) -> u32 {
    Pdb::parse(bytes)
        .expect("parse")
        .container()
        .msf()
        .expect("MSF 7.00")
        .block_size()
}

#[test]
fn rebuild_keeps_the_source_page_size() {
    let options = EditOptions::default().with_msf(MsfWriteOptions::default().with_page_size(512));
    let bytes = program::build()
        .and_then(|editor| editor.plan(options)?.to_vec())
        .expect("the synthetic program builds with 512-byte pages");
    assert_eq!(page_size(&bytes), 512);
    // Without an MSF option, the rebuild uses the page size of the source.
    let rebuilt = rebuild(&bytes, IdentityPolicy::Preserve, EditOptions::default());
    assert_eq!(page_size(&rebuilt), 512);
    assert_eq!(rebuilt, bytes);
    // The caller can select another page size.
    let options = EditOptions::default().with_msf(MsfWriteOptions::default().with_page_size(4096));
    let larger = rebuild(&bytes, IdentityPolicy::Preserve, options);
    assert_eq!(page_size(&larger), 4096);
}

#[test]
fn rebuild_keeps_missing_hash_streams() {
    let options = EditOptions::default()
        .with_type_hash_stream(false)
        .with_id_hash_stream(false);
    let bytes = program::build()
        .and_then(|editor| editor.plan(options)?.to_vec())
        .expect("the synthetic program builds without hash streams");
    let pdb = Pdb::parse(&bytes).expect("parse");
    let types = pdb.types().expect("TPI").expect("TPI present");
    assert_eq!(types.header().hash_stream, None);
    let ids = pdb.ids().expect("IPI").expect("IPI present");
    assert_eq!(ids.header().hash_stream, None);
    // Without hash stream options, the rebuild keeps the convention of the source.
    let rebuilt = rebuild(&bytes, IdentityPolicy::Preserve, EditOptions::default());
    let pdb = Pdb::parse(&rebuilt).expect("parse");
    let types = pdb.types().expect("TPI").expect("TPI present");
    let ids = pdb.ids().expect("IPI").expect("IPI present");
    assert_eq!(
        (types.header().hash_stream, ids.header().hash_stream),
        (None, None)
    );
    assert_eq!(rebuilt, bytes);
}

#[test]
fn identical_input_gives_identical_bytes() {
    assert_eq!(synthetic(), synthetic());
}

#[test]
fn rebuild_of_a_built_pdb_gives_the_same_bytes() {
    let bytes = synthetic();
    let pdb = Pdb::parse(&bytes).expect("parse");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor.decode_all().expect("decode");
    let plan = editor.plan(EditOptions::default()).expect("plan");
    assert_eq!(plan.report().dropped().count(), 0);
    let rebuilt = plan.to_vec().expect("rebuild");
    assert_eq!(rebuilt, bytes);
}

#[test]
fn identity_policy_sets_both_ages() {
    let bytes = synthetic();
    let pdb = Pdb::parse(&bytes).expect("parse");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor.decode_all().expect("decode");
    let plan = editor.plan(EditOptions::default()).expect("plan");
    assert_eq!(
        plan.identity().expect("identity"),
        Identity::Modern {
            guid: program::GUID,
            age: 2
        }
    );
    let output = plan.to_vec().expect("output");
    let rebuilt = Pdb::parse(&output).expect("parse");
    assert_eq!(rebuilt.info().identity().age(), 2);
    let dbi = rebuilt.debug_info().expect("DBI").expect("DBI present");
    assert_eq!(dbi.header().age, 2);
}

/// Return an `LF_ARGLIST` record with the argument `argument`.
fn arguments(argument: TypeIndex) -> TypeLeaf<'static> {
    let index = argument.0.to_le_bytes();
    TypeLeaf::ArgumentList(ArgumentList::parse(&index).expect("list")).into_owned()
}

/// Return an `LF_FUNC_ID` record `f` in the scope `scope`.
fn function_in(scope: IdIndex) -> IdRecord<'static> {
    let mut function = FunctionId::default();
    function.scope = Some(scope);
    function.function_type = TypeIndex(0x74);
    function.name = Bytes::new(b"f");
    IdRecord::Function(function)
}

/// Plan a new PDB with the type record `LF_ARGLIST [target]` at 0x1000 and
/// a second record at 0x1001. Return the plan error.
fn plan_type_reference(target: TypeIndex) -> pdb_rs::Error {
    let mut editor = PdbEditor::new(1, [1; 16], 1).expect("editor");
    let types = editor.types_mut().expect("types");
    types.push(arguments(target)).expect("type");
    types.push(arguments(TypeIndex(0x74))).expect("type");
    editor
        .plan(EditOptions::default())
        .expect_err("the plan rejects the reference")
}

#[test]
fn forward_type_reference_is_rejected() {
    // A record can refer only to earlier records of its index space: not to a
    // later record and not to itself.
    for target in [0x1001, 0x1000] {
        let error = plan_type_reference(TypeIndex(target));
        assert_eq!(
            (error.kind(), error.index_space()),
            (ErrorKind::UnsupportedEdit, Some(IndexSpace::TypeIndex)),
            "{target:#x}: {error}"
        );
    }
    // A reference to an earlier record is valid.
    let mut editor = PdbEditor::new(1, [1; 16], 1).expect("editor");
    let types = editor.types_mut().expect("types");
    let first = types.push(arguments(TypeIndex(0x74))).expect("type");
    types.push(arguments(first)).expect("type");
    editor
        .plan(EditOptions::default())
        .expect("a backward reference is valid");
}

#[test]
fn type_reference_past_the_type_stream_is_rejected() {
    // The TPI stream has only the records 0x1000 and 0x1001.
    let error = plan_type_reference(TypeIndex(0x1002));
    assert_eq!(error.kind(), ErrorKind::InvalidReference, "{error}");
}

#[test]
fn forward_id_reference_is_rejected() {
    // `f` names the scope `ns`, a later record.
    let mut editor = PdbEditor::new(1, [1; 16], 1).expect("editor");
    let ids = editor.ids_mut().expect("IDs");
    ids.push(function_in(IdIndex(0x1001))).expect("ID");
    let mut scope = StringId::default();
    scope.substrings = None;
    scope.string = Bytes::new(b"ns");
    let scope = ids.push(IdRecord::String(scope)).expect("ID");
    assert_eq!(scope, IdIndex(0x1001));
    let error = editor
        .plan(EditOptions::default())
        .expect_err("the plan rejects a forward reference");
    assert_eq!(
        (error.kind(), error.index_space()),
        (ErrorKind::UnsupportedEdit, Some(IndexSpace::IdIndex)),
        "{error}"
    );
    // The same records in reference order are valid.
    let mut editor = PdbEditor::new(1, [1; 16], 1).expect("editor");
    let ids = editor.ids_mut().expect("IDs");
    let mut scope = StringId::default();
    scope.substrings = None;
    scope.string = Bytes::new(b"ns");
    let scope = ids.push(IdRecord::String(scope)).expect("ID");
    ids.push(function_in(scope)).expect("ID");
    editor
        .plan(EditOptions::default())
        .expect("a backward reference is valid");
}

#[test]
fn reference_past_the_type_stream_is_rejected() {
    let mut editor = PdbEditor::new(1, [1; 16], 1).expect("editor");
    editor
        .globals_mut()
        .expect("globals")
        .push(S_GDATA32, SymbolData::Data(data_g(TypeIndex(0x1000))), 1)
        .expect("symbol");
    let error = editor
        .plan(EditOptions::default())
        .expect_err("the TPI stream has no record 0x1000");
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
}

#[test]
fn module_count_past_16_bits_is_rejected() {
    let mut editor = PdbEditor::new(1, [1; 16], 1).expect("editor");
    let record = module_record(b"m");
    for _ in 0..=u32::from(u16::MAX) {
        let index = editor.add_module(record.clone()).expect("module");
        editor.module_mut(index).expect("module").set_stream(false);
    }
    let error = editor
        .plan(EditOptions::default())
        .expect_err("the DBI stream has a 16-bit module count");
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
}

#[test]
fn module_without_stream_rejects_symbols() {
    let mut editor = PdbEditor::new(1, [1; 16], 1).expect("editor");
    let index = editor.add_module(module_record(b"m")).expect("module");
    let module = editor.module_mut(index).expect("module");
    module
        .push(S_GDATA32, SymbolData::Data(data_g(TypeIndex(0x74))))
        .expect("symbol");
    module.set_stream(false);
    let error = editor
        .plan(EditOptions::default())
        .expect_err("a module without a stream has no symbols");
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
}
