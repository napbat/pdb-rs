//! Tests of PDBs without type merging (feature `NOTM`, MSVC `/DEBUG:LAZY`).
//!
//! The `S_MOD_TYPEREF` record of a module names the stream with the type
//! records of the module. The builder checks the module symbols against
//! these records, the rebuild carries the streams, and the editor treats
//! them as streams of the module (#32, #33, #34).

use pdbrs::native::address::SectionOffset;
use pdbrs::native::codeview::index::{ModuleIndex, TypeIndex};
use pdbrs::native::codeview::symbol::{
    Data, ModuleTypeReference, ModuleTypeReferenceFlags, ModuleTypeSource, Public, PublicFlags,
    S_LDATA32, S_MOD_TYPEREF, S_PUB32, SymbolData,
};
use pdbrs::native::codeview::type_record::{ArgumentList, TypeLeaf};
use pdbrs::native::dbi::{Module, SectionContribution};
use pdbrs::native::info::Feature;
use pdbrs::native::msf::StreamId;
use pdbrs::native::{BuildOptions, EditOptions, IdentityPolicy, Pdb, PdbBuilder};
use pdbrs::{Bytes, ErrorKind};

/// The flags of a module that owns its type records (`fOwnTMR`).
const OWN_TYPE_RECORDS: u32 = 0x08;
/// The flags of a module that uses the records of another module (`fRefTM`).
const OTHER_MODULE: u32 = 0x20;

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

/// Return `count` encoded type records without a stream header.
fn type_records(count: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..count {
        TypeLeaf::ArgumentList(ArgumentList::parse(&[]).expect("list"))
            .encode(&mut out)
            .expect("record");
    }
    out
}

/// Return a module-local data record of `type_index`.
fn local(type_index: TypeIndex) -> SymbolData<'static> {
    let mut data = Data::default();
    data.type_index = type_index;
    data.address = SectionOffset {
        section: 1,
        offset: 0,
    };
    data.name = Bytes::new(b"local");
    SymbolData::Data(data)
}

/// Return a module type reference with `flags` and zero words.
fn type_reference(flags: u32) -> ModuleTypeReference {
    let mut reference = ModuleTypeReference::default();
    reference.flags = ModuleTypeReferenceFlags(flags);
    reference
}

/// Build a PDB like `link /DEBUG:LAZY` writes: empty TPI and IPI, module 0
/// with two type records in its own stream, and module 1 with the records
/// of module 0. The local data of the modules has `first` and `second`.
fn builder(first: TypeIndex, second: TypeIndex) -> PdbBuilder<'static> {
    let mut builder = PdbBuilder::new(7, [7; 16], 1).expect("builder");
    builder
        .set_features(&[Feature::VC140, Feature::NO_TYPE_MERGE])
        .expect("features");
    let types = builder.add_stream(type_records(2)).expect("type stream");
    let own = builder
        .add_module(module_record(b"own.obj"))
        .expect("module");
    own.push_symbol_with(move |resolver, out| {
        let mut reference = type_reference(OWN_TYPE_RECORDS);
        reference.map_streams(|_| resolver.stream(types))?;
        SymbolData::ModuleTypeReference(reference).encode(S_MOD_TYPEREF, out)
    })
    .expect("type reference");
    own.push_symbol_fields(S_LDATA32, local(first))
        .expect("local");
    let other = builder
        .add_module(module_record(b"other.obj"))
        .expect("module");
    other
        .push_symbol_fields(
            S_MOD_TYPEREF,
            SymbolData::ModuleTypeReference(type_reference(OTHER_MODULE)),
        )
        .expect("type reference");
    other
        .push_symbol_fields(S_LDATA32, local(second))
        .expect("local");
    for name in [&b"pub"[..], b"later"] {
        let mut record = Public::default();
        record.flags = PublicFlags(0);
        record.address = SectionOffset {
            section: 1,
            offset: 0,
        };
        record.name = Bytes::new(name);
        let public = builder
            .add_symbol_fields(S_PUB32, SymbolData::Public(record))
            .expect("public");
        builder.index_public(public).expect("index");
    }
    builder
}

/// Build the PDB with valid module type indices.
fn notm() -> Vec<u8> {
    builder(TypeIndex(0x1000), TypeIndex(0x1001))
        .plan(BuildOptions::default())
        .and_then(|plan| plan.to_vec())
        .expect("the NOTM PDB builds")
}

/// Return the type record streams of the modules of `pdb`, by module.
fn module_type_streams(pdb: &Pdb<&[u8]>) -> Vec<(u32, ModuleTypeSource)> {
    let dbi = pdb.debug_info().expect("DBI").expect("DBI stream");
    pdb.external_dependencies(&dbi)
        .expect("dependencies")
        .module_types
        .iter()
        .map(|dependency| (dependency.module.0, dependency.source))
        .collect()
}

/// Return the bytes of stream `id`, or `None` for a nil or absent stream.
fn stream(pdb: &Pdb<&[u8]>, id: StreamId) -> Option<Vec<u8>> {
    if id.0 >= pdb.container().stream_count() {
        return None;
    }
    pdb.container()
        .stream(id)
        .expect("stream directory")
        .map(|reader| reader.read_all(usize::MAX).expect("stream bytes"))
}

/// Return the stream of the `S_MOD_TYPEREF` record of module 0.
fn own_stream(pdb: &Pdb<&[u8]>) -> StreamId {
    match module_type_streams(pdb).first() {
        Some(&(0, ModuleTypeSource::TypeRecords { stream })) => stream,
        other => panic!("module 0 has no own type records: {other:?}"),
    }
}

#[test]
fn module_symbols_resolve_against_their_type_records() {
    let bytes = notm();
    let pdb = Pdb::parse(bytes.as_slice()).expect("PDB");
    assert!(pdb.info().has_feature(Feature::NO_TYPE_MERGE));
    assert_eq!(
        module_type_streams(&pdb)[1],
        (
            1,
            ModuleTypeSource::OtherModule {
                module: ModuleIndex(0)
            }
        )
    );
    assert_eq!(stream(&pdb, own_stream(&pdb)), Some(type_records(2)));
}

#[test]
fn module_type_index_past_the_module_records_is_rejected() {
    for (first, second) in [
        (TypeIndex(0x1002), TypeIndex(0x1000)),
        (TypeIndex(0x1000), TypeIndex(0x1002)),
    ] {
        let error = builder(first, second)
            .plan(BuildOptions::default())
            .expect_err("module 0 has two type records");
        assert_eq!(error.kind(), ErrorKind::InvalidReference);
    }
}

#[test]
fn rebuild_carries_the_module_type_streams() {
    let bytes = notm();
    let pdb = Pdb::parse(bytes.as_slice()).expect("PDB");
    let contents = pdb.contents().expect("contents");
    assert_eq!(contents.unclaimed_streams(), []);
    let builder = contents
        .to_builder(IdentityPolicy::Preserve)
        .expect("builder");
    let rebuilt = builder
        .plan(contents.build_options())
        .and_then(|plan| plan.to_vec())
        .expect("the rebuild plans");
    let rebuilt = Pdb::parse(rebuilt.as_slice()).expect("rebuilt");
    assert!(rebuilt.info().has_feature(Feature::NO_TYPE_MERGE));
    assert_eq!(module_type_streams(&rebuilt), module_type_streams(&pdb));
    assert_eq!(
        stream(&rebuilt, own_stream(&rebuilt)),
        Some(type_records(2))
    );
}

/// Build a PDB like `link /DEBUG:LAZY` writes for `/Z7` objects with a
/// precompiled header: module 0 is the `/Yc` object (`fOwnTMR | fOwnTMPCT`)
/// and module 1 a `/Yu` object (`fOwnTMR | fRefTMPCT`, owner 0). Each has two
/// own records in a stream. The local data uses index `0x1506`, an index
/// after 1286 precompiled types that the PDB does not hold.
fn precompiled() -> Vec<u8> {
    let mut builder = PdbBuilder::new(7, [7; 16], 1).expect("builder");
    builder
        .set_features(&[Feature::VC140, Feature::NO_TYPE_MERGE])
        .expect("features");
    for (name, flags) in [(&b"pch.obj"[..], 0x0c), (b"use.obj", 0x0a)] {
        let types = builder.add_stream(type_records(2)).expect("type stream");
        let module = builder.add_module(module_record(name)).expect("module");
        module
            .push_symbol_with(move |resolver, out| {
                let mut reference = type_reference(flags);
                reference.map_streams(|_| resolver.stream(types))?;
                SymbolData::ModuleTypeReference(reference).encode(S_MOD_TYPEREF, out)
            })
            .expect("type reference");
        module
            .push_symbol_fields(S_LDATA32, local(TypeIndex(0x1506)))
            .expect("local");
    }
    builder
        .plan(BuildOptions::default())
        .and_then(|plan| plan.to_vec())
        .expect("indices of precompiled-type modules are not checked")
}

/// The precompiled types of `/DEBUG:LAZY` stay in the object file of the
/// `/Yc` module (#30, #39: `msvc-x64-pch-lazy`). The dependency list names it,
/// and the rebuild carries the own records of both modules.
#[test]
fn precompiled_types_of_lazy_pdbs_are_external_dependencies() {
    let bytes = precompiled();
    let pdb = Pdb::parse(bytes.as_slice()).expect("PDB");
    let dbi = pdb.debug_info().expect("DBI").expect("DBI stream");
    let dependencies = pdb.external_dependencies(&dbi).expect("dependencies");
    let objects: Vec<_> = dependencies
        .object_precompiled_types
        .iter()
        .map(|entry| (entry.module.0, entry.owner.0, entry.object_file.clone()))
        .collect();
    assert_eq!(
        objects,
        [(0, 0, b"pch.obj".to_vec()), (1, 0, b"pch.obj".to_vec())]
    );
    assert!(matches!(
        module_type_streams(&pdb)[1],
        (
            1,
            ModuleTypeSource::PrecompiledTypeUser {
                owner: ModuleIndex(0),
                ..
            }
        )
    ));
    let contents = pdb.contents().expect("contents");
    assert_eq!(contents.unclaimed_streams(), []);
    let rebuilt = contents
        .to_builder(IdentityPolicy::Preserve)
        .expect("builder")
        .plan(contents.build_options())
        .and_then(|plan| plan.to_vec())
        .expect("the rebuild plans");
    let rebuilt = Pdb::parse(rebuilt.as_slice()).expect("rebuilt");
    let sources = module_type_streams(&rebuilt);
    assert_eq!(sources.len(), 2);
    for (_, source) in sources {
        let stream_id = source.streams().next().expect("own records");
        assert_eq!(stream(&rebuilt, stream_id), Some(type_records(2)));
    }
}

#[test]
fn edit_that_moves_records_keeps_the_module_type_streams() {
    let bytes = notm();
    let pdb = Pdb::parse(bytes.as_slice()).expect("PDB");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .rename_public(b"pub", b"pub_with_a_longer_name")
        .expect("recorded");
    let plan = editor
        .plan(EditOptions::default())
        .expect("the module type stream has an owner");
    let own = own_stream(&pdb);
    assert!(!plan.changed_streams().contains(&own));
    let edited = plan.to_vec().expect("output");
    let edited = Pdb::parse(edited.as_slice()).expect("edited");
    assert_eq!(stream(&edited, own), Some(type_records(2)));
}

#[test]
fn strip_removes_the_module_type_streams_and_the_feature() {
    let bytes = notm();
    let pdb = Pdb::parse(bytes.as_slice()).expect("PDB");
    let mut editor = pdb.edit(IdentityPolicy::Preserve);
    editor.strip_private();
    let plan = editor.plan(EditOptions::default()).expect("strip");
    let own = own_stream(&pdb);
    assert!(plan.changed_streams().contains(&own));
    let stripped = plan.to_vec().expect("output");
    let stripped = Pdb::parse(stripped.as_slice()).expect("stripped");
    assert_eq!(stream(&stripped, own), None);
    assert!(!stripped.info().has_feature(Feature::NO_TYPE_MERGE));
    assert!(stripped.info().has_feature(Feature::VC140));
    assert_eq!(module_type_streams(&stripped), []);
}
