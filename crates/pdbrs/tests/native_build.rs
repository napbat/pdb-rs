//! Tests of `native::PdbBuilder`: the synthetic program, the rebuild
//! of a built PDB, determinism, handle resolution and width checks (#33).

#[path = "../examples/native_synthetic/program.rs"]
mod program;

use pdbrs::native::address::SectionOffset;
use pdbrs::native::codeview::index::{ModuleIndex, TypeIndex};
use pdbrs::native::codeview::symbol::{Data, S_GDATA32, Symbol, SymbolData};
use pdbrs::native::codeview::type_record::{ArgumentList, TypeLeaf};
use pdbrs::native::dbi::{Module, SectionContribution};
use pdbrs::native::lines::{LineSubsection, ModuleLines};
use pdbrs::native::msf::MsfWriteOptions;
use pdbrs::native::{BuildOptions, Identity, IdentityPolicy, Pdb, PdbBuilder};
use pdbrs::{Bytes, ErrorKind};

/// Build the synthetic program and return the PDB bytes.
fn synthetic() -> Vec<u8> {
    program::build()
        .and_then(|builder| builder.plan(BuildOptions::default())?.to_vec())
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

#[test]
fn rebuild_keeps_the_source_page_size() {
    let options = BuildOptions::default().with_msf(MsfWriteOptions::default().with_page_size(512));
    let bytes = program::build()
        .and_then(|builder| builder.plan(options)?.to_vec())
        .expect("the synthetic program builds with 512-byte pages");
    let pdb = Pdb::parse(&bytes).expect("parse");
    assert_eq!(pdb.container().msf().expect("MSF 7.00").block_size(), 512);
    let contents = pdb.contents().expect("contents");
    let rebuild_options = contents.build_options();
    assert_eq!(rebuild_options.msf().page_size(), 512);
    let builder = contents
        .to_builder(IdentityPolicy::Preserve)
        .expect("builder");
    let rebuilt = builder
        .plan(rebuild_options)
        .and_then(|plan| plan.to_vec())
        .expect("rebuild");
    assert_eq!(rebuilt, bytes);
    // The caller can select another page size.
    let rebuild_options = rebuild_options.with_msf(MsfWriteOptions::default().with_page_size(4096));
    let larger = builder
        .plan(rebuild_options)
        .and_then(|plan| plan.to_vec())
        .expect("rebuild with 4096-byte pages");
    let larger = Pdb::parse(&larger).expect("parse");
    assert_eq!(
        larger.container().msf().expect("MSF 7.00").block_size(),
        4096
    );
}

#[test]
fn rebuild_keeps_missing_hash_streams() {
    let options = BuildOptions::default()
        .with_type_hash_stream(false)
        .with_id_hash_stream(false);
    let bytes = program::build()
        .and_then(|builder| builder.plan(options)?.to_vec())
        .expect("the synthetic program builds without hash streams");
    let pdb = Pdb::parse(&bytes).expect("parse");
    let types = pdb.types().expect("TPI").expect("TPI present");
    assert_eq!(types.header().hash_stream, None);
    let contents = pdb.contents().expect("contents");
    let rebuild_options = contents.build_options();
    assert!(!rebuild_options.type_hash_stream() && !rebuild_options.id_hash_stream());
    let rebuilt = contents
        .to_builder(IdentityPolicy::Preserve)
        .and_then(|builder| builder.plan(rebuild_options)?.to_vec())
        .expect("rebuild");
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
    let contents = pdb.contents().expect("contents");
    assert_eq!(contents.unclaimed_streams(), []);
    let rebuilt = contents
        .to_builder(IdentityPolicy::Preserve)
        .and_then(|builder| builder.plan(contents.build_options())?.to_vec())
        .expect("rebuild");
    assert_eq!(rebuilt, bytes);
}

#[test]
fn identity_policy_sets_both_ages() {
    let bytes = synthetic();
    let pdb = Pdb::parse(&bytes).expect("parse");
    let contents = pdb.contents().expect("contents");
    let builder = contents
        .to_builder(IdentityPolicy::IncrementAge)
        .expect("builder");
    let plan = builder.plan(contents.build_options()).expect("plan");
    assert_eq!(
        plan.identity(),
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

#[test]
fn reserved_handle_without_record_is_rejected() {
    let mut builder = PdbBuilder::new(1, [1; 16], 1).expect("builder");
    builder.reserve_type().expect("reserve");
    let error = builder
        .plan(BuildOptions::default())
        .expect_err("the plan needs a record");
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
}

#[test]
fn handle_of_another_builder_is_rejected() {
    let mut other = PdbBuilder::new(1, [1; 16], 1).expect("builder");
    other
        .add_type(TypeLeaf::ArgumentList(
            ArgumentList::parse(&[]).expect("list"),
        ))
        .expect("type");
    let foreign = other
        .add_type(TypeLeaf::ArgumentList(
            ArgumentList::parse(&[]).expect("list"),
        ))
        .expect("type");
    let mut builder = PdbBuilder::new(1, [1; 16], 1).expect("builder");
    builder
        .add_type_with(move |resolver, out| {
            let index = resolver.type_index(foreign)?.0.to_le_bytes();
            TypeLeaf::ArgumentList(ArgumentList::parse(&index)?).encode(out)
        })
        .expect("deferred type");
    let error = builder
        .plan(BuildOptions::default())
        .expect_err("the handle is not in this builder");
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
}

#[test]
fn reference_past_the_type_stream_is_rejected() {
    let mut builder = PdbBuilder::new(1, [1; 16], 1).expect("builder");
    builder
        .add_symbol_fields(S_GDATA32, SymbolData::Data(data_g(TypeIndex(0x1000))))
        .expect("symbol");
    let error = builder
        .plan(BuildOptions::default())
        .expect_err("the TPI stream has no record 0x1000");
    assert_eq!(error.kind(), ErrorKind::InvalidReference);
}

#[test]
fn module_count_past_16_bits_is_rejected() {
    let mut builder = PdbBuilder::new(1, [1; 16], 1).expect("builder");
    let record = module_record(b"m");
    for _ in 0..=u32::from(u16::MAX) {
        builder
            .add_module(record.clone())
            .expect("module")
            .set_stream(false);
    }
    let error = builder
        .plan(BuildOptions::default())
        .expect_err("the DBI stream has a 16-bit module count");
    assert_eq!(error.kind(), ErrorKind::LimitExceeded);
}

#[test]
fn module_without_stream_rejects_symbols() {
    let mut builder = PdbBuilder::new(1, [1; 16], 1).expect("builder");
    let module = builder.add_module(module_record(b"m")).expect("module");
    module
        .push_symbol_fields(S_GDATA32, SymbolData::Data(data_g(TypeIndex(0x74))))
        .expect("symbol");
    module.set_stream(false);
    let error = builder
        .plan(BuildOptions::default())
        .expect_err("a module without a stream has no symbols");
    assert_eq!(error.kind(), ErrorKind::InvalidFormat);
}
