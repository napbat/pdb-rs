//! The removal of type and ID records with index compaction (#47).
//!
//! The synthetic program has the type records `0x1000` to `0x1007` and the ID
//! records `0x1000` and `0x1001`. [`source`] adds an unreferenced forward
//! reference (`0x1008`), an argument list of `Point` (`0x1009`), a global
//! `S_UDT` record, `/UDTSRCLINEUNDONE`, a type server map that `/TMCache`
//! names and a type server map without an owner. All of them refer to
//! `0x1009`. A removal of `0x1008` moves `0x1009` to `0x1008`.

use pdbrs::io::ReadAt;
use pdbrs::native::auxiliary::{TmCache, TmCacheBuilder, TypeMapStream, UdtSourceLineUndone};
use pdbrs::native::codeview::id_record::{FunctionId, IdRecord, StringId};
use pdbrs::native::codeview::index::{IdIndex, TypeIndex};
use pdbrs::native::codeview::symbol::{S_UDT, SymbolData, UserDefinedType};
use pdbrs::native::codeview::type_record::{ArgumentList, TypeLeaf};
use pdbrs::native::info::Feature;
use pdbrs::native::msf::StreamId;
use pdbrs::native::{EditOptions, IdentityPolicy, IndexSpace, Pdb, PdbEditor, StreamChange};
use pdbrs::{Bytes, Error, ErrorKind};

use crate::{build, forward, stream, synthetic};

/// The unreferenced forward reference of [`source`].
const UNUSED: TypeIndex = TypeIndex(0x1008);
/// The argument list of [`source`] before the removal of [`UNUSED`].
const LIST: TypeIndex = TypeIndex(0x1009);
/// The `Point` structure of the synthetic program.
const POINT: TypeIndex = TypeIndex(0x1005);
/// The name of the global `S_UDT` record of [`source`].
const LIST_NAME: &[u8] = b"PointList";

/// Return an `LF_ARGLIST` record with the one argument `argument`.
fn argument_list(argument: TypeIndex) -> TypeLeaf<'static> {
    TypeLeaf::ArgumentList(
        ArgumentList::parse(argument.0.to_le_bytes().to_vec()).expect("argument list"),
    )
}

/// Return the little-endian bytes of `words`.
fn words(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

/// Return a type server map (version 1) with one type, one ID and one
/// function ID pair.
fn type_server_map(types: &[u32]) -> Vec<u8> {
    let count = u32::try_from(types.len()).expect("count");
    let mut bytes = words(&[1, count, 1]);
    bytes.extend(words(types));
    bytes.extend(words(&[0x1001, 0x1000, LIST.0]));
    bytes
}

/// Return an object file map (version 2) with one index.
fn object_map() -> Vec<u8> {
    let mut bytes = words(&[2]);
    bytes.extend([0xff, 0xff, 0, 0]);
    bytes.extend(words(&[0, 0x1000, 1, LIST.0, 0, 8]));
    bytes
}

/// Add the records and streams of [`source`] to the synthetic program.
fn add_references(editor: &mut PdbEditor<'static>) -> pdbrs::Result<()> {
    let types = editor.types_mut()?;
    assert_eq!(types.push(forward(b"Unused"))?, UNUSED);
    assert_eq!(types.push(argument_list(POINT))?, LIST);
    let mut udt = UserDefinedType::default();
    udt.type_index = LIST;
    udt.name = Bytes::new(LIST_NAME);
    editor
        .globals_mut()?
        .push(S_UDT, SymbolData::UserDefinedType(udt), 1)?;
    let owned = editor
        .unnamed_streams_mut()?
        .push(type_server_map(&[LIST.0, 0]))?;
    let mut cache = TmCacheBuilder::new();
    let slot = cache.push_stream(Some(owned))?;
    cache.push_entry(slot, 1)?;
    let mut bytes = Vec::new();
    cache.encode(&mut bytes)?;
    let named = editor.named_streams_mut()?;
    named.insert(b"/TMCache", bytes)?;
    named.insert(b"/UDTSRCLINEUNDONE", words(&[LIST.0]))?;
    // The plan writes the streams without a name last.
    editor
        .unnamed_streams_mut()?
        .push(type_server_map(&[LIST.0]))
        .map(|_| ())
}

/// Build the source PDB of the compaction tests.
fn source() -> Vec<u8> {
    build(add_references)
}

/// Plan `editor` and return the error.
fn rejected<S: ReadAt>(editor: &PdbEditor<'_, S>) -> Error {
    editor
        .plan(EditOptions::default())
        .expect_err("rejected plan")
}

/// Return the type record `index` of `pdb`.
fn type_record<S: ReadAt>(pdb: &Pdb<S>, index: TypeIndex) -> TypeLeaf<'static> {
    let types = pdb.types().expect("TPI").expect("TPI present");
    let record = types
        .records()
        .map(|record| record.expect("TPI record"))
        .find(|record| record.index == index.0)
        .expect("record of the index");
    TypeLeaf::decode(record.record)
        .expect("type record")
        .into_owned()
}

/// Return the ID record `index` of `pdb`.
fn id_record<S: ReadAt>(pdb: &Pdb<S>, index: IdIndex) -> IdRecord<'static> {
    let ids = pdb.ids().expect("IPI").expect("IPI present");
    let record = ids
        .records()
        .map(|record| record.expect("IPI record"))
        .find(|record| record.index == index.0)
        .expect("record of the index");
    IdRecord::decode(record.record)
        .expect("ID record")
        .into_owned()
}

/// Return the arguments of the argument list `leaf`.
fn arguments(leaf: &TypeLeaf<'_>) -> Vec<TypeIndex> {
    match leaf {
        TypeLeaf::ArgumentList(list) => list.iter().collect(),
        other => panic!("not an argument list: {other:?}"),
    }
}

/// Return the bytes of the named stream `name` of `pdb`.
fn named(pdb: &Pdb<&[u8]>, name: &[u8]) -> (StreamId, Vec<u8>) {
    let id = pdb.info().named_stream(name).expect("named stream");
    (id, stream(pdb, id.0).expect("stream bytes"))
}

/// Check that the output `bytes` has [`LIST`] at `0x1008`, and that each
/// reference to it has `0x1008`. `unowned` is the stream of the type map
/// without an owner; `decode_all` drops that stream.
fn check_compacted(bytes: &[u8], unowned: Option<u32>) {
    let pdb = Pdb::parse(bytes).expect("output parses");
    let tpi = pdb.types().expect("TPI").expect("TPI present");
    assert_eq!(tpi.header().index_end, LIST.0);
    assert_eq!(arguments(&type_record(&pdb, UNUSED)), [POINT]);
    let globals = pdb.globals().expect("globals").expect("globals present");
    let udt = globals
        .symbols()
        .map(|symbol| symbol.expect("global symbol"))
        .find_map(|symbol| match symbol.data() {
            SymbolData::UserDefinedType(udt) if *udt.name == *LIST_NAME => Some(udt.type_index),
            _ => None,
        })
        .expect("global S_UDT");
    assert_eq!(udt, UNUSED);
    let (id, undone) = named(&pdb, b"/UDTSRCLINEUNDONE");
    let undone = UdtSourceLineUndone::parse(&undone, id).expect("/UDTSRCLINEUNDONE");
    assert_eq!(undone.type_indices().collect::<Vec<_>>(), [UNUSED]);
    let (id, cache) = named(&pdb, b"/TMCache");
    let cache = TmCache::parse(&cache, id).expect("/TMCache");
    let owned = cache
        .streams()
        .flatten()
        .next()
        .expect("type map of /TMCache");
    let owned = stream(&pdb, owned.0).expect("type map bytes");
    let map = TypeMapStream::parse(&owned, id).expect("type map");
    assert_eq!(map.types().collect::<Vec<_>>(), [UNUSED.0, 0]);
    assert_eq!(map.ids().collect::<Vec<_>>(), [0x1001]);
    assert_eq!(
        map.function_types().collect::<Vec<_>>(),
        [(0x1000, UNUSED.0)]
    );
    if let Some(unowned) = unowned {
        let bytes = stream(&pdb, unowned).expect("type map without an owner");
        let map = TypeMapStream::parse(&bytes, StreamId(unowned)).expect("type map");
        assert_eq!(map.types().collect::<Vec<_>>(), [UNUSED.0]);
    }
}

#[test]
fn compaction_writes_the_new_index_into_each_reference() {
    let source = source();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    for decode_all in [false, true] {
        let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
        if decode_all {
            editor.decode_all().expect("decode_all");
        }
        let types = editor.types_mut().expect("types");
        types.remove(UNUSED).expect("remove");
        assert!(types.get(UNUSED).is_none());
        assert_eq!(types.len(), 9);
        assert_eq!(types.end().expect("end"), TypeIndex(0x100a));
        assert_eq!(
            types.remove(UNUSED).expect_err("second removal").kind(),
            ErrorKind::InvalidReference
        );
        let plan = editor.plan(EditOptions::default()).expect("plan");
        assert_eq!(plan.type_index(LIST).expect("moved index"), UNUSED);
        assert_eq!(plan.type_index(POINT).expect("kept index"), POINT);
        assert_eq!(
            plan.type_index(TypeIndex(0x74)).expect("primitive"),
            TypeIndex(0x74)
        );
        for missing in [UNUSED, TypeIndex(0x100a)] {
            let error = plan.type_index(missing).expect_err("no output index");
            assert_eq!(error.kind(), ErrorKind::InvalidReference);
        }
        assert_eq!(
            plan.id_index(IdIndex(0x1001)).expect("IDs stay"),
            IdIndex(0x1001)
        );
        if !decode_all {
            let change = |id: u32| {
                plan.report()
                    .streams()
                    .iter()
                    .find(|report| report.output == Some(StreamId(id)))
                    .map(|report| report.change)
            };
            assert_eq!(change(2), Some(StreamChange::Edited));
            let symbols = pdb
                .debug_info()
                .expect("DBI")
                .expect("DBI present")
                .header()
                .symbol_stream
                .expect("symbol record stream");
            assert_eq!(
                change(symbols.0),
                Some(StreamChange::Remapped(IndexSpace::TypeIndex))
            );
        }
        let unowned = (!decode_all).then(|| pdb.container().stream_count() - 1);
        check_compacted(&plan.to_vec().expect("output"), unowned);
    }
}

#[test]
fn records_added_after_a_removal_resolve_through_their_handles() {
    let source = source();
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    let types = editor.types_mut().expect("types");
    types.remove(UNUSED).expect("remove");
    let handle = types.push(argument_list(TypeIndex(0x74))).expect("handle");
    assert_eq!(handle, TypeIndex(0x100a));
    let user = types.push(argument_list(handle)).expect("user");
    let ids = editor.ids_mut().expect("IDs");
    let mut string = StringId::default();
    string.substrings = None;
    string.string = Bytes::new(b"unused");
    let unused = ids
        .push(IdRecord::String(string.clone()))
        .expect("unused ID");
    string.string = Bytes::new(b"scope");
    let scope = ids.push(IdRecord::String(string)).expect("scope ID");
    ids.remove(IdIndex(0x1000)).expect("remove the ID of sum");
    ids.remove(unused).expect("remove the unused ID");
    let mut function = FunctionId::default();
    function.scope = Some(scope);
    function.function_type = user;
    function.name = Bytes::new(b"scoped");
    function.decorated_name_hash = None;
    let function = ids.push(IdRecord::Function(function)).expect("function");
    let plan = editor.plan(EditOptions::default()).expect("plan");
    assert_eq!(plan.type_index(handle).expect("handle"), TypeIndex(0x1009));
    assert_eq!(plan.type_index(user).expect("user"), TypeIndex(0x100a));
    assert_eq!(
        plan.id_index(IdIndex(0x1001)).expect("main"),
        IdIndex(0x1000)
    );
    assert_eq!(plan.id_index(scope).expect("scope"), IdIndex(0x1001));
    assert_eq!(plan.id_index(function).expect("function"), IdIndex(0x1002));
    let output = plan.to_vec().expect("output");
    let out = Pdb::parse(output.as_slice()).expect("output parses");
    assert_eq!(
        arguments(&type_record(&out, TypeIndex(0x100a))),
        [TypeIndex(0x1009)]
    );
    let IdRecord::Function(function) = id_record(&out, IdIndex(0x1002)) else {
        panic!("not a function ID");
    };
    assert_eq!(function.scope, Some(IdIndex(0x1001)));
    assert_eq!(function.function_type, TypeIndex(0x100a));
    let IdRecord::Function(main) = id_record(&out, IdIndex(0x1000)) else {
        panic!("not a function ID");
    };
    assert_eq!(*main.name, *b"main");
}

#[test]
fn removal_of_a_referenced_record_names_the_record_that_refers() {
    let source = synthetic();
    let pdb = Pdb::parse(source.as_slice()).expect("source");

    // `LF_PROCEDURE` 0x1003 refers to the argument list 0x1002.
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .types_mut()
        .expect("types")
        .remove(TypeIndex(0x1002))
        .expect("remove");
    let error = rejected(&editor);
    assert_eq!(error.kind(), ErrorKind::InvalidReference, "{error}");
    assert_eq!(error.record(), Some((IndexSpace::TypeIndex, 0x1003)));

    // `LF_FUNC_ID` 0x1001 of `main` refers to its type 0x1007.
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .types_mut()
        .expect("types")
        .remove(TypeIndex(0x1007))
        .expect("remove");
    let error = rejected(&editor);
    assert_eq!(error.kind(), ErrorKind::InvalidReference, "{error}");
    assert_eq!(error.record(), Some((IndexSpace::IdIndex, 0x1001)));

    // The global `S_UDT` record of `Point` refers to 0x1005.
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .types_mut()
        .expect("types")
        .remove(POINT)
        .expect("remove");
    let error = rejected(&editor);
    assert_eq!(error.kind(), ErrorKind::InvalidReference, "{error}");
    assert_eq!(error.index_space(), Some(IndexSpace::TypeIndex));
    let symbols = pdb
        .debug_info()
        .expect("DBI")
        .expect("DBI present")
        .header()
        .symbol_stream;
    assert_eq!(error.stream_location().map(|(id, _)| id), symbols);
}

/// Remove [`UNUSED`] from the output of [`build`] with `extra`, and return
/// the error of the plan.
fn rejected_removal(extra: impl FnOnce(&mut PdbEditor<'static>) -> pdbrs::Result<()>) -> Error {
    let source = build(|editor| {
        assert_eq!(editor.types_mut()?.push(forward(b"Unused"))?, UNUSED);
        extra(editor)
    });
    let pdb = Pdb::parse(source.as_slice()).expect("source");
    let mut editor = pdb.edit(IdentityPolicy::IncrementAge);
    editor
        .types_mut()
        .expect("types")
        .remove(UNUSED)
        .expect("remove");
    rejected(&editor)
}

#[test]
fn opaque_data_blocks_the_compaction() {
    let error =
        rejected_removal(|editor| editor.unnamed_streams_mut()?.push(object_map()).map(|_| ()));
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit, "{error}");
    assert_eq!(error.index_space(), Some(IndexSpace::TypeIndex));
    assert!(error.stream_location().is_some(), "{error}");

    let error = rejected_removal(|editor| {
        editor
            .named_streams_mut()?
            .insert(b"unknown-kind", b"opaque data".to_vec())
    });
    assert_eq!(error.kind(), ErrorKind::UnsupportedEdit, "{error}");
    assert!(error.stream_location().is_some(), "{error}");
}

#[test]
fn pdbs_with_external_type_records_refuse_the_compaction() {
    for feature in [Feature::NO_TYPE_MERGE, Feature::MINIMAL_DEBUG_INFO] {
        let error = rejected_removal(|editor| editor.set_features(&[Feature::VC140, feature]));
        assert_eq!(error.kind(), ErrorKind::UnsupportedEdit, "{error}");
        assert_eq!(error.index_space(), Some(IndexSpace::TypeIndex));
    }
}
